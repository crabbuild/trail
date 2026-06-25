use super::*;
use std::fs;
use std::net::{SocketAddr, TcpListener};

pub(super) fn handle_git_command(ctx: &RuntimeContext, git: GitCommand) -> Result<()> {
    match git.command {
        GitSubcommand::Export(args) => {
            if let Some(message) = args.message {
                if args.output.is_some() {
                    return Err(Error::InvalidInput(
                        "git export -m cannot be combined with --output".to_string(),
                    ));
                }
                let mut db = open_db(ctx)?;
                let report = db.git_export_commit(&args.range, &message)?;
                render_git_export(&report, ctx.json, ctx.quiet)
            } else if let Some(output) = args.output {
                let db = open_db(ctx)?;
                db.write_patch_to(&args.range, &output)?;
                if !ctx.quiet {
                    println!("Wrote patch: {}", output.display());
                }
                Ok(())
            } else {
                let db = open_db(ctx)?;
                let patch = db.export_patch(&args.range)?;
                print!("{patch}");
                Ok(())
            }
        }
        GitSubcommand::ImportUpdate(args) => {
            let mut db = open_db(ctx)?;
            let report = db.git_import_update(ctx.branch.as_deref(), args.message)?;
            render_git_import_update(&report, ctx.json, ctx.quiet)
        }
        GitSubcommand::Mappings(args) => {
            let db = open_db(ctx)?;
            let mappings = db.git_mappings(args.limit)?;
            render_git_mappings(&mappings, ctx.json, ctx.quiet)
        }
    }
}

pub(super) fn handle_api_command(_ctx: &RuntimeContext, api: ApiCommand) -> Result<()> {
    match api.command {
        ApiSubcommand::Openapi(args) => {
            let spec = crabdb::server::openapi_spec();
            let json = serde_json::to_string_pretty(&spec)?;
            if let Some(output) = args.output {
                fs::write(output, json)?;
            } else {
                println!("{json}");
            }
            Ok(())
        }
    }
}

pub(super) fn handle_daemon_command(ctx: &RuntimeContext, args: DaemonArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let (auth, token_file) = daemon_auth(&db, &args)?;
    let addr: SocketAddr = format!("{}:{}", args.host, args.port)
        .parse()
        .map_err(|err| Error::InvalidInput(format!("invalid listen address: {err}")))?;
    let listener = TcpListener::bind(addr)?;
    let local_addr = listener.local_addr()?;
    if !ctx.quiet {
        println!("CrabDB API listening on http://{local_addr}");
        if args.no_auth {
            println!("Daemon auth disabled");
        } else if let Some(path) = token_file {
            println!("Daemon token file: {}", path.display());
            println!(
                "Send requests with: Authorization: Bearer $(cat {})",
                path.display()
            );
        } else {
            println!("Daemon token configured from flag or CRABDB_DAEMON_TOKEN");
        }
    }
    let max_requests = if args.once {
        Some(1)
    } else {
        args.max_requests
    };
    crabdb::server::serve_listener_with_auth(&mut db, listener, max_requests, auth)
}

pub(super) fn handle_mcp_command(ctx: &RuntimeContext) -> Result<()> {
    let mut db = open_db(ctx)?;
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    crabdb::mcp::serve_stdio(&mut db, stdin.lock(), &mut stdout)
}

pub(super) fn handle_doctor_command(ctx: &RuntimeContext) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.doctor()?;
    render_doctor(&report, ctx.json, ctx.quiet)
}

pub(super) fn handle_backup_command(ctx: &RuntimeContext, backup: BackupCommand) -> Result<()> {
    match backup.command {
        BackupSubcommand::Create(args) => {
            let db = open_db(ctx)?;
            let report = db.create_backup(args.output, args.overwrite)?;
            render_backup_create(&report, ctx.json, ctx.quiet)
        }
        BackupSubcommand::Verify(args) => {
            let report = CrabDb::verify_backup(args.path)?;
            render_backup_verify(&report, ctx.json, ctx.quiet)
        }
        BackupSubcommand::Restore(args) => {
            let workspace = ctx
                .workspace
                .clone()
                .unwrap_or(std::env::current_dir().map_err(Error::from)?);
            let report = CrabDb::restore_backup(workspace, args.path, args.force)?;
            render_backup_restore(&report, ctx.json, ctx.quiet)
        }
    }
}

pub(super) fn handle_fsck_command(ctx: &RuntimeContext) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.fsck()?;
    render_fsck(&report, ctx.json, ctx.quiet)
}

pub(super) fn handle_index_command(ctx: &RuntimeContext, index: IndexCommand) -> Result<()> {
    match index.command {
        IndexSubcommand::Rebuild => {
            let mut db = open_db(ctx)?;
            let report = db.rebuild_indexes()?;
            render_index_rebuild(&report, ctx.json, ctx.quiet)
        }
    }
}

pub(super) fn handle_gc_command(ctx: &RuntimeContext, args: GcArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let report = db.gc(args.dry_run)?;
    render_gc(&report, ctx.json, ctx.quiet)
}

fn daemon_auth(
    db: &CrabDb,
    args: &DaemonArgs,
) -> Result<(crabdb::server::ServerAuth, Option<PathBuf>)> {
    if args.no_auth {
        if args.auth_token.is_some() || args.auth_token_file.is_some() {
            return Err(Error::InvalidInput(
                "--no-auth cannot be combined with --auth-token or --auth-token-file".to_string(),
            ));
        }
        return Ok((crabdb::server::ServerAuth::disabled(), None));
    }

    if let Some(token) = &args.auth_token {
        return Ok((crabdb::server::ServerAuth::bearer(token.clone())?, None));
    }
    if let Ok(token) = std::env::var("CRABDB_DAEMON_TOKEN") {
        return Ok((crabdb::server::ServerAuth::bearer(token)?, None));
    }

    let token_path = args
        .auth_token_file
        .clone()
        .unwrap_or_else(|| db.db_dir().join("daemon.token"));
    let token = if token_path.exists() {
        fs::read_to_string(&token_path)?.trim().to_string()
    } else {
        let token = generate_daemon_token()?;
        fs::write(&token_path, format!("{token}\n"))?;
        restrict_secret_file(&token_path)?;
        token
    };
    Ok((crabdb::server::ServerAuth::bearer(token)?, Some(token_path)))
}

fn generate_daemon_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|err| {
        Error::InvalidInput(format!("failed to generate daemon auth token: {err}"))
    })?;
    Ok(hex::encode(bytes))
}

fn restrict_secret_file(path: &std::path::Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(path, permissions)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
