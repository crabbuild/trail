use super::*;

pub(super) struct RuntimeContext {
    pub(super) workspace: Option<PathBuf>,
    pub(super) db_dir: Option<PathBuf>,
    pub(super) branch: Option<String>,
    pub(super) json: bool,
    pub(super) quiet: bool,
    pub(super) format: OutputFormat,
    pub(super) render: RenderOptions,
}

pub(super) fn resolve_output_format(cli_format: Option<OutputFormat>) -> Result<OutputFormat> {
    if let Some(format) = cli_format {
        return Ok(format);
    }
    let Some(value) = std::env::var("TRAIL_FORMAT").ok() else {
        return Ok(OutputFormat::Human);
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "human" => Ok(OutputFormat::Human),
        "plain" => Ok(OutputFormat::Plain),
        "json" => Ok(OutputFormat::Json),
        "ndjson" => Ok(OutputFormat::Ndjson),
        other => Err(Error::InvalidInput(format!(
            "TRAIL_FORMAT must be human, plain, json, or ndjson, got `{other}`"
        ))),
    }
}

pub(super) fn open_db(ctx: &RuntimeContext) -> Result<Trail> {
    // A concurrent, authenticated workspace mutation can advance SQLite's
    // WAL/SHM generation between preflight and opening the connection. The
    // preflight detects that race instead of accepting an unverified handle;
    // retry that one transient condition so ordinary CLI commands get the
    // same safe handoff behavior as ACP clients.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut delay = std::time::Duration::from_millis(2);
    loop {
        match open_db_once(ctx) {
            Ok(db) => return Ok(db),
            Err(error)
                if retryable_open_db_handoff(&error) && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(delay);
                delay = (delay * 2).min(std::time::Duration::from_millis(50));
            }
            Err(error) => return Err(error),
        }
    }
}

fn retryable_open_db_handoff(error: &Error) -> bool {
    match error {
        Error::SchemaReinitializeRequired { found, .. } => {
            matches!(
                found.as_str(),
                "schema main/WAL/SHM generation changed during mutable handoff"
                    | "schema main/WAL/SHM generation changed during snapshot validation"
                    | "schema generation changed during predecessor inspection"
            )
        }
        // Under a large cross-process CLI burst, preflight can exhaust its
        // deliberately short in-process WAL snapshot retry budget while
        // another process is still opening a verified connection. This is the
        // same transient handoff as the generation result above; retry only
        // this exact bounded diagnostic, never a generic workspace lock.
        Error::WorkspaceLocked(message) => {
            message == "SQLite WAL remained active throughout bounded schema snapshot validation; retry the command"
        }
        _ => false,
    }
}

fn open_db_once(ctx: &RuntimeContext) -> Result<Trail> {
    match (&ctx.workspace, &ctx.db_dir) {
        (Some(workspace), Some(db_dir)) => Trail::open_with_db_dir(workspace, db_dir),
        (Some(workspace), None) => Trail::open(workspace),
        (None, Some(db_dir)) => {
            let workspace = db_dir
                .parent()
                .ok_or_else(|| Error::WorkspaceNotFound(db_dir.clone()))?;
            Trail::open_with_db_dir(workspace, db_dir)
        }
        (None, None) => Trail::discover(std::env::current_dir().map_err(Error::from)?),
    }
}
