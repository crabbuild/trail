use super::*;
use std::fs::{self, OpenOptions};
use std::io::Write;
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
                render_git_export(&report, ctx.json, &ctx.render)
            } else if let Some(output) = args.output {
                let db = open_db(ctx)?;
                db.write_patch_to(&args.range, &output)?;
                render_document(
                    &TerminalDocument::new("Patch written", UiTone::Success).block(
                        UiBlock::Metadata(vec![("Path".to_string(), output.display().to_string())]),
                    ),
                    &ctx.render,
                )
            } else {
                let db = open_db(ctx)?;
                let patch = db.export_patch(&args.range)?;
                render_raw_content(&patch, &ctx.render)
            }
        }
        GitSubcommand::ImportUpdate(args) => {
            let mut db = open_db(ctx)?;
            let report = db.git_import_update(ctx.branch.as_deref(), args.message)?;
            render_git_import_update(&report, ctx.json, &ctx.render)
        }
        GitSubcommand::Mappings(args) => {
            let db = open_db(ctx)?;
            let mappings = db.git_mappings(args.limit)?;
            render_git_mappings(&mappings, ctx.json, &ctx.render)
        }
    }
}

pub(super) fn handle_api_command(_ctx: &RuntimeContext, api: ApiCommand) -> Result<()> {
    match api.command {
        ApiSubcommand::Openapi(args) => {
            let spec = trail::server::openapi_spec();
            let json = serde_json::to_string_pretty(&spec)?;
            if let Some(output) = args.output {
                fs::write(output, json)?;
            } else {
                render_raw_content(
                    &format!("{json}\n"),
                    &RenderOptions {
                        mode: RenderMode::Plain,
                        color: false,
                        glyphs: GlyphSet::Ascii,
                        width: 80,
                        height: 24,
                        stdout_is_terminal: false,
                        stderr_is_terminal: false,
                        verbose: false,
                        quiet: false,
                        pager: PagerPolicy::Never,
                    },
                )?;
            }
            Ok(())
        }
    }
}

pub(super) fn handle_daemon_command(ctx: &RuntimeContext, args: DaemonArgs) -> Result<()> {
    let mut db = open_db(ctx)?;
    let addr = daemon_listen_addr(&args)?;
    validate_daemon_listen_security(&args, addr)?;
    let rate_limit = daemon_rate_limit(&args)?;
    let connection_timeout = daemon_connection_timeout(&args)?;
    let (auth, token_file) = daemon_auth(&db, &args)?;
    let cache_warmup = db.start_daemon_worktree_cache()?;
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
    let error_options = ctx.render.clone();
    thread::spawn(move || {
        if let Err(err) = cache_warmup
            .run()
            .and_then(|_| endpoint_writer.write_if_alive())
        {
            if !quiet {
                let diagnostic = UiDiagnostic {
                    code: err.code().to_string(),
                    summary: "Trail daemon cache warmup failed".to_string(),
                    cause: Some(err.to_string()),
                    consequence: Some(
                        "Requests may rebuild workspace cache entries before responding."
                            .to_string(),
                    ),
                    recovery: None,
                    alternatives: Vec::new(),
                };
                let _ = render_error_document(
                    &TerminalDocument::empty().block(UiBlock::Diagnostic(diagnostic)),
                    &error_options,
                );
            }
        }
    });
    if args.no_auth {
        render_error_document(
            &TerminalDocument::new("WARNING: daemon auth is disabled", UiTone::Attention)
                .context("Any local process can mutate this workspace through the daemon.")
                .block(UiBlock::Metadata(vec![(
                    "Endpoint".to_string(),
                    daemon_url.clone(),
                )]))
                .block(UiBlock::Notice(
                    "Keep the listener on loopback and use this only for trusted local automation."
                        .to_string(),
                )),
            &ctx.render,
        )?;
    }
    let mut metadata = vec![("Endpoint".to_string(), daemon_url.clone())];
    if !args.no_auth {
        if let Some(path) = token_file {
            metadata.push(("Token file".to_string(), path.display().to_string()));
            metadata.push((
                "Authorization".to_string(),
                format!("Bearer token from {}", path.display()),
            ));
        } else {
            metadata.push((
                "Authorization".to_string(),
                "token from --auth-token or TRAIL_DAEMON_TOKEN".to_string(),
            ));
        }
    }
    render_document(
        &TerminalDocument::new("Trail API daemon listening", UiTone::Success)
            .block(UiBlock::Metadata(metadata)),
        &ctx.render,
    )?;
    let max_requests = if args.once {
        Some(1)
    } else {
        args.max_requests
    };
    let _endpoint_registration = endpoint_registration;
    trail::server::serve_listener_with_auth_rate_limit_and_timeout(
        &mut db,
        listener,
        max_requests,
        auth,
        rate_limit,
        connection_timeout,
    )
}

fn daemon_listen_addr(args: &DaemonArgs) -> Result<SocketAddr> {
    format!("{}:{}", args.host, args.port)
        .parse()
        .map_err(|err| Error::InvalidInput(format!("invalid listen address: {err}")))
}

fn validate_daemon_listen_security(args: &DaemonArgs, addr: SocketAddr) -> Result<()> {
    if args.no_auth && !addr.ip().is_loopback() {
        return Err(Error::InvalidInput(format!(
            "daemon --no-auth requires a loopback --host; refusing to listen on {} without authentication",
            addr.ip()
        )));
    }
    Ok(())
}

fn daemon_rate_limit(args: &DaemonArgs) -> Result<trail::server::ServerRateLimit> {
    trail::server::ServerRateLimit::per_window(
        args.rate_limit_requests,
        Duration::from_secs(args.rate_limit_window_secs),
    )
}

fn daemon_connection_timeout(args: &DaemonArgs) -> Result<Duration> {
    let timeout = Duration::from_secs(args.connection_timeout_secs);
    if timeout.is_zero() {
        return Err(Error::InvalidInput(
            "daemon --connection-timeout-secs must be greater than zero".to_string(),
        ));
    }
    Ok(timeout)
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
    trail::mcp::serve_stdio(&mut db, stdin.lock(), &mut stdout)
}

pub(super) fn handle_doctor_command(ctx: &RuntimeContext) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.doctor()?;
    render_doctor(&report, ctx.json, &ctx.render)
}

pub(super) fn handle_backup_command(ctx: &RuntimeContext, backup: BackupCommand) -> Result<()> {
    match backup.command {
        BackupSubcommand::Create(args) => {
            let db = open_db(ctx)?;
            let report = db.create_backup(args.output, args.overwrite)?;
            render_backup_create(&report, ctx.json, &ctx.render)
        }
        BackupSubcommand::Verify(args) => {
            let report = Trail::verify_backup(args.path)?;
            render_backup_verify(&report, ctx.json, &ctx.render)
        }
        BackupSubcommand::Restore(args) => {
            let workspace = ctx
                .workspace
                .clone()
                .unwrap_or(std::env::current_dir().map_err(Error::from)?);
            let report = Trail::restore_backup(workspace, args.path, args.force)?;
            render_backup_restore(&report, ctx.json, &ctx.render)
        }
    }
}

pub(super) fn handle_fsck_command(ctx: &RuntimeContext) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.fsck()?;
    render_fsck(&report, ctx.json, &ctx.render)
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
            render_index_rebuild(&report, ctx.json, &ctx.render)
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
                    render_ndjson(&report)?;
                } else {
                    render_worktree_index(&report, ctx.json, &ctx.render)?;
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
    render_gc(&report, ctx.json, &ctx.render)
}

fn daemon_auth(
    db: &Trail,
    args: &DaemonArgs,
) -> Result<(trail::server::ServerAuth, Option<PathBuf>)> {
    if args.no_auth {
        if args.auth_token.is_some() || args.auth_token_file.is_some() {
            return Err(Error::InvalidInput(
                "--no-auth cannot be combined with --auth-token or --auth-token-file".to_string(),
            ));
        }
        return Ok((trail::server::ServerAuth::disabled(), None));
    }

    if let Some(token) = &args.auth_token {
        return Ok((trail::server::ServerAuth::bearer(token.clone())?, None));
    }
    if let Ok(token) = std::env::var("TRAIL_DAEMON_TOKEN") {
        return Ok((trail::server::ServerAuth::bearer(token)?, None));
    }

    let token_path = args
        .auth_token_file
        .clone()
        .unwrap_or_else(|| db.db_dir().join("daemon.token"));
    let token = read_or_create_daemon_token_file(&token_path)?;
    Ok((trail::server::ServerAuth::bearer(token)?, Some(token_path)))
}

fn read_or_create_daemon_token_file(path: &std::path::Path) -> Result<String> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            restrict_secret_file(path)?;
            let token = fs::read_to_string(path)?.trim().to_string();
            if token.is_empty() {
                return Err(Error::InvalidInput(format!(
                    "daemon auth token file `{}` cannot be empty",
                    path.display()
                )));
            }
            Ok(token)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => create_daemon_token_file(path),
        Err(err) => Err(Error::from(err)),
    }
}

fn create_daemon_token_file(path: &std::path::Path) -> Result<String> {
    let token = generate_daemon_token()?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(format!("{token}\n").as_bytes())?;
            file.sync_all()?;
            restrict_secret_file(path)?;
            Ok(token)
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            read_or_create_daemon_token_file(path)
        }
        Err(err) => Err(Error::from(err)),
    }
}

fn generate_daemon_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|err| {
        Error::InvalidInput(format!("failed to generate daemon auth token: {err}"))
    })?;
    Ok(hex::encode(bytes))
}

fn restrict_secret_file(path: &std::path::Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(Error::InvalidInput(format!(
            "daemon auth token file `{}` cannot be a symlink",
            path.display()
        )));
    }
    if !metadata.is_file() {
        return Err(Error::InvalidInput(format!(
            "daemon auth token file `{}` must be a regular file",
            path.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(path, permissions)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn daemon_args_with_token_file(token_file: PathBuf) -> DaemonArgs {
        DaemonArgs {
            host: "127.0.0.1".to_string(),
            port: 0,
            once: true,
            max_requests: Some(1),
            rate_limit_requests: 600,
            rate_limit_window_secs: 60,
            connection_timeout_secs: 30,
            auth_token: None,
            auth_token_file: Some(token_file),
            no_auth: false,
        }
    }

    #[test]
    fn daemon_auth_restricts_existing_token_file() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let token_path = db.db_dir().join("daemon.token");
        fs::write(&token_path, "existing-token\n").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&token_path).unwrap().permissions();
            permissions.set_mode(0o644);
            fs::set_permissions(&token_path, permissions).unwrap();
        }

        let (auth, token_file) = daemon_auth(&db, &daemon_args_with_token_file(token_path.clone()))
            .expect("existing token file should be accepted");

        assert!(auth.is_required());
        assert_eq!(token_file.as_deref(), Some(token_path.as_path()));
        assert_eq!(fs::read_to_string(&token_path).unwrap(), "existing-token\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&token_path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[cfg(unix)]
    #[test]
    fn daemon_auth_rejects_symlink_token_file() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let target_path = temp.path().join("outside-token");
        fs::write(&target_path, "linked-token\n").unwrap();
        let token_path = db.db_dir().join("daemon.token");
        symlink(&target_path, &token_path).unwrap();

        let err = daemon_auth(&db, &daemon_args_with_token_file(token_path))
            .expect_err("symlink token files must be rejected");

        assert!(
            matches!(err, Error::InvalidInput(ref message) if message.contains("cannot be a symlink")),
            "unexpected error: {err:?}"
        );
        assert_eq!(fs::read_to_string(&target_path).unwrap(), "linked-token\n");
    }
}
