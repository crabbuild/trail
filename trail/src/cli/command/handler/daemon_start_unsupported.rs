use std::path::{Path, PathBuf};

use trail::{Error, Result, Trail};

use super::RuntimeContext;

pub(super) fn retire_workspace_daemon_after_external_generation_change(
    _workspace: &Path,
) -> Result<()> {
    Ok(())
}

pub(super) fn is_auto_workspace_daemon() -> bool {
    false
}

pub(super) fn run_auto_workspace_daemon(_db: Trail) -> Result<()> {
    Err(Error::DaemonUnavailable(
        "automatic workspace daemon requires Linux or macOS".into(),
    ))
}

pub(super) fn workspace_from_context(ctx: &RuntimeContext) -> Result<PathBuf> {
    ctx.workspace
        .clone()
        .or_else(|| {
            ctx.db_dir
                .as_deref()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
        })
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| Error::InvalidInput("workspace path is unavailable".into()))
}
