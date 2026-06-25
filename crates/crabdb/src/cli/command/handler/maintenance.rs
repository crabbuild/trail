use super::*;
use std::fs;
use std::net::{SocketAddr, TcpListener};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;

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
    let cache_warmup = db.start_daemon_worktree_cache()?;
    let addr: SocketAddr = format!("{}:{}", args.host, args.port)
        .parse()
        .map_err(|err| Error::InvalidInput(format!("invalid listen address: {err}")))?;
    let listener = TcpListener::bind(addr)?;
    let local_addr = listener.local_addr()?;
    let daemon_url = daemon_rpc::daemon_url_for_listener(local_addr);
    let endpoint_registration = DaemonEndpointRegistration::new(
        db.db_dir(),
        daemon_rpc::DaemonEndpointFile {
            version: 1,
            url: daemon_url.clone(),
            pid: std::process::id(),
            auth: !args.no_auth,
        },
    )?;
    let endpoint_writer = endpoint_registration.writer();
    let quiet = ctx.quiet;
    thread::spawn(move || {
        if let Err(err) = cache_warmup
            .run()
            .and_then(|_| endpoint_writer.write_if_alive())
        {
            if !quiet {
                eprintln!("CrabDB daemon cache warmup failed: {err}");
            }
        }
    });
    if !ctx.quiet {
        println!("CrabDB API listening on {daemon_url}");
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
    let _endpoint_registration = endpoint_registration;
    crabdb::server::serve_listener_with_auth(&mut db, listener, max_requests, auth)
}

struct DaemonEndpointRegistration {
    path: PathBuf,
    endpoint: daemon_rpc::DaemonEndpointFile,
    alive: Arc<AtomicBool>,
}

#[derive(Clone)]
struct DaemonEndpointWriter {
    path: PathBuf,
    endpoint: daemon_rpc::DaemonEndpointFile,
    alive: Arc<AtomicBool>,
}

impl DaemonEndpointRegistration {
    fn new(db_dir: &std::path::Path, endpoint: daemon_rpc::DaemonEndpointFile) -> Result<Self> {
        let path = daemon_rpc::daemon_endpoint_path(db_dir);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(Error::from(err)),
        }
        Ok(Self {
            path,
            endpoint,
            alive: Arc::new(AtomicBool::new(true)),
        })
    }

    fn writer(&self) -> DaemonEndpointWriter {
        DaemonEndpointWriter {
            path: self.path.clone(),
            endpoint: self.endpoint.clone(),
            alive: Arc::clone(&self.alive),
        }
    }
}

impl Drop for DaemonEndpointRegistration {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
        let Ok(bytes) = fs::read(&self.path) else {
            return;
        };
        let Ok(existing) = serde_json::from_slice::<daemon_rpc::DaemonEndpointFile>(&bytes) else {
            return;
        };
        if existing == self.endpoint {
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl DaemonEndpointWriter {
    fn write_if_alive(&self) -> Result<()> {
        if self.alive.load(Ordering::Acquire) {
            fs::write(&self.path, serde_json::to_vec_pretty(&self.endpoint)?)?;
        }
        Ok(())
    }
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
        IndexSubcommand::Rebuild(args) => {
            let mut db = open_db(ctx)?;
            let report = if args.rich_text {
                db.rebuild_indexes_with_rich_text()?
            } else {
                db.rebuild_indexes()?
            };
            render_index_rebuild(&report, ctx.json, ctx.quiet)
        }
        IndexSubcommand::Watch(args) => {
            if args.interval_ms == 0 {
                return Err(Error::InvalidInput(
                    "index watch --interval-ms must be greater than 0".to_string(),
                ));
            }
            let db = open_db(ctx)?;
            let interval = Duration::from_millis(args.interval_ms);
            let mut iterations = 0usize;
            loop {
                let report = db.refresh_worktree_index()?;
                iterations += 1;
                if matches!(ctx.format, OutputFormat::Ndjson) {
                    println!("{}", serde_json::to_string(&report)?);
                } else {
                    render_worktree_index(&report, ctx.json, ctx.quiet)?;
                }
                if args.once || args.iterations.is_some_and(|max| iterations >= max) {
                    break;
                }
                thread::sleep(interval);
            }
            Ok(())
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
