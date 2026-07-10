use clap::{Args, Subcommand};

#[derive(Args)]
pub(super) struct DepsCommand {
    #[command(subcommand)]
    pub(super) command: DepsSubcommand,
}

#[derive(Subcommand)]
pub(super) enum DepsSubcommand {
    /// Show expected, attached, ready, stale, or failed dependency environments.
    Status(DepsStatusArgs),
    /// Build or reuse a frozen Node dependency layer and attach it to a lane.
    Sync(DepsSyncArgs),
}

#[derive(Args)]
pub(super) struct DepsStatusArgs {
    pub(super) lane: String,
}

#[derive(Args)]
pub(super) struct DepsSyncArgs {
    pub(super) lane: String,
    #[arg(long)]
    pub(super) path: Option<String>,
}

#[derive(Args)]
pub(super) struct CacheCommand {
    #[command(subcommand)]
    pub(super) command: CacheSubcommand,
}

#[derive(Subcommand)]
pub(super) enum CacheSubcommand {
    /// List immutable workspace layers and storage accounting.
    List,
    /// Verify one immutable layer against its content-addressed manifest.
    Verify(CacheLayerArgs),
    /// Inspect one immutable layer and verify it before reporting.
    Inspect(CacheLayerArgs),
    /// Reclaim unpinned immutable layers and rematerializable blob projections.
    Gc(CacheGcArgs),
}

#[derive(Args)]
pub(super) struct CacheGcArgs {
    /// Report exactly what would be reclaimed without mutating the cache.
    #[arg(long)]
    pub(super) dry_run: bool,
    /// Override the configured minimum age for unreferenced cache entries.
    #[arg(long)]
    pub(super) retention_secs: Option<u64>,
}

#[derive(Args)]
pub(super) struct CacheLayerArgs {
    pub(super) layer: String,
}
