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
