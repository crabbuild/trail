use std::path::PathBuf;

use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(super) enum GitSubcommand {
    /// Export a range as Git patch or commit.
    Export(GitExportArgs),
    /// Import current Git snapshot into CrabDB.
    ImportUpdate(GitImportUpdateArgs),
    /// List recent Git<->CrabDB mapping entries.
    Mappings(GitMappingsArgs),
}

#[derive(Args)]
pub(super) struct GitCommand {
    #[command(subcommand)]
    pub(super) command: GitSubcommand,
}

#[derive(Args)]
pub(super) struct GitExportArgs {
    pub(super) range: String,
    #[arg(short, long)]
    pub(super) message: Option<String>,
    #[arg(short, long)]
    pub(super) output: Option<PathBuf>,
}

#[derive(Args)]
pub(super) struct GitImportUpdateArgs {
    #[arg(short, long)]
    pub(super) message: Option<String>,
}

#[derive(Args)]
pub(super) struct GitMappingsArgs {
    #[arg(long, default_value_t = 30)]
    pub(super) limit: usize,
}

#[derive(Subcommand)]
pub(super) enum ApiSubcommand {
    /// Print or write the OpenAPI contract JSON.
    Openapi(ApiOpenapiArgs),
}

#[derive(Args)]
pub(super) struct ApiCommand {
    #[command(subcommand)]
    pub(super) command: ApiSubcommand,
}

#[derive(Args)]
pub(super) struct ApiOpenapiArgs {
    #[arg(short, long)]
    pub(super) output: Option<PathBuf>,
}

#[derive(Args)]
pub(super) struct DaemonArgs {
    #[arg(long, default_value = "127.0.0.1")]
    pub(super) host: String,
    #[arg(long, default_value_t = 8765)]
    pub(super) port: u16,
    #[arg(long)]
    pub(super) once: bool,
    #[arg(long)]
    pub(super) max_requests: Option<usize>,
    #[arg(long = "rate-limit-requests", default_value_t = 600)]
    pub(super) rate_limit_requests: usize,
    #[arg(long = "rate-limit-window-secs", default_value_t = 60)]
    pub(super) rate_limit_window_secs: u64,
    #[arg(long = "connection-timeout-secs", default_value_t = 30)]
    pub(super) connection_timeout_secs: u64,
    #[arg(long)]
    pub(super) auth_token: Option<String>,
    #[arg(long)]
    pub(super) auth_token_file: Option<PathBuf>,
    #[arg(long)]
    pub(super) no_auth: bool,
}

#[derive(Subcommand)]
pub(super) enum IndexSubcommand {
    /// Rebuild all derived indexes from current workspace state.
    Rebuild(IndexRebuildArgs),
    /// Continuously refresh the persisted worktree file index.
    Watch(IndexWatchArgs),
}

#[derive(Args)]
pub(super) struct IndexCommand {
    #[command(subcommand)]
    pub(super) command: IndexSubcommand,
}

#[derive(Args)]
pub(super) struct IndexRebuildArgs {
    #[arg(long = "rich-text")]
    pub(super) rich_text: bool,
}

#[derive(Args)]
pub(super) struct IndexWatchArgs {
    #[arg(long)]
    pub(super) once: bool,
    #[arg(long)]
    pub(super) iterations: Option<usize>,
    #[arg(long = "interval-ms", default_value_t = 1000)]
    pub(super) interval_ms: u64,
}

#[derive(Args)]
pub(super) struct GcArgs {
    #[arg(long)]
    pub(super) dry_run: bool,
}

#[derive(Subcommand)]
pub(super) enum BackupSubcommand {
    /// Create a portable workspace backup file.
    Create(BackupCreateArgs),
    /// Verify backup integrity before restore.
    Verify(BackupVerifyArgs),
    /// Restore workspace data from a backup archive.
    Restore(BackupRestoreArgs),
}

#[derive(Args)]
pub(super) struct BackupCommand {
    #[command(subcommand)]
    pub(super) command: BackupSubcommand,
}

#[derive(Args)]
pub(super) struct BackupCreateArgs {
    pub(super) output: PathBuf,
    #[arg(long)]
    pub(super) overwrite: bool,
}

#[derive(Args)]
pub(super) struct BackupVerifyArgs {
    pub(super) path: PathBuf,
}

#[derive(Args)]
pub(super) struct BackupRestoreArgs {
    pub(super) path: PathBuf,
    #[arg(long)]
    pub(super) force: bool,
}
