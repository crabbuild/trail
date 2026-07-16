use std::path::PathBuf;

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
    /// Build or reuse a frozen Node layer and bulk-replace an unmounted lane's dependency state.
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
pub(super) struct EnvironmentCommand {
    #[command(subcommand)]
    pub(super) command: EnvironmentSubcommand,
}

#[derive(Subcommand)]
pub(super) enum EnvironmentSubcommand {
    /// List registered environment adapters and their discovery metadata.
    Adapters,
    /// Install or remove capability-constrained external adapter packages.
    Plugin(EnvironmentPluginCommand),
    /// Detect environment components without running tools or repository code.
    Discover(EnvironmentDiscoverArgs),
    /// Show the validated desired component DAG in deterministic build order.
    Graph(EnvironmentGraphArgs),
    /// Show normalized component and adapter state for a layered lane.
    Status(EnvironmentStatusArgs),
    /// Show the exact active component/storage generation and its predecessor.
    Generation(EnvironmentStatusArgs),
    /// Explain every canonical input, tool, platform, or policy edge that made a component stale.
    Explain(EnvironmentExplainArgs),
    /// Preview the normalized component key, actions, outputs, and capabilities without executing.
    Plan(EnvironmentSyncArgs),
    /// Prepare shared and/or writable-private outputs and attach one component atomically.
    Sync(EnvironmentSyncArgs),
    /// Prepare all discovered components, then activate one atomic generation.
    SyncAll(EnvironmentDiscoverArgs),
    /// Inspect, reconcile, or stop lane-private runtime services.
    Runtime(EnvironmentRuntimeCommand),
}

#[derive(Args)]
pub(super) struct EnvironmentRuntimeCommand {
    #[command(subcommand)]
    pub(super) command: EnvironmentRuntimeSubcommand,
}

#[derive(Subcommand)]
pub(super) enum EnvironmentRuntimeSubcommand {
    /// Show the active generation's persisted runtime allocations and health.
    Status(EnvironmentStatusArgs),
    /// Idempotently create or adopt declared resources and wait for health.
    Reconcile(EnvironmentStatusArgs),
    /// Stop active containers while retaining their lane-private volumes and networks.
    Stop(EnvironmentStatusArgs),
}

#[derive(Args)]
pub(super) struct EnvironmentPluginCommand {
    #[command(subcommand)]
    pub(super) command: EnvironmentPluginSubcommand,
}

#[derive(Subcommand)]
pub(super) enum EnvironmentPluginSubcommand {
    /// Validate a package and print the exact payload/distribution digests without installing it.
    Inspect(EnvironmentPluginInstallArgs),
    /// Verify, content-address, and activate a local adapter package directory.
    Install(EnvironmentPluginInstallArgs),
    /// Deactivate an installed adapter identity without deleting provenance bytes.
    Remove(EnvironmentPluginRemoveArgs),
    /// Manage publisher public keys trusted to authenticate adapter packages.
    Trust(EnvironmentPluginTrustCommand),
}

#[derive(Args)]
pub(super) struct EnvironmentPluginInstallArgs {
    /// Directory containing trail-adapter.toml and its declared executable.
    pub(super) package: PathBuf,
}

#[derive(Args)]
pub(super) struct EnvironmentPluginRemoveArgs {
    /// Canonical adapter identity, for example acme/python@1.
    pub(super) identity: String,
}

#[derive(Args)]
pub(super) struct EnvironmentPluginTrustCommand {
    #[command(subcommand)]
    pub(super) command: EnvironmentPluginTrustSubcommand,
}

#[derive(Subcommand)]
pub(super) enum EnvironmentPluginTrustSubcommand {
    /// Add or rotate a publisher public key from a TOML document.
    Add(EnvironmentPluginTrustAddArgs),
    /// List currently trusted publisher keys.
    List,
    /// Revoke a publisher key; signed packages using it fail closed immediately.
    Remove(EnvironmentPluginTrustRemoveArgs),
}

#[derive(Args)]
pub(super) struct EnvironmentPluginTrustAddArgs {
    /// TOML document containing schema, publisher, and 32-byte Ed25519 public key.
    pub(super) key: PathBuf,
}

#[derive(Args)]
pub(super) struct EnvironmentPluginTrustRemoveArgs {
    /// Content-derived publisher key ID in sha256:<hex> form.
    pub(super) key_id: String,
}

#[derive(Args)]
pub(super) struct EnvironmentStatusArgs {
    pub(super) lane: String,
}

#[derive(Args)]
pub(super) struct EnvironmentExplainArgs {
    pub(super) lane: String,
    /// Stable logical component ID from `trail env discover` or `trail env status`.
    #[arg(long, value_name = "ID")]
    pub(super) component: String,
    /// Zero-based change offset for large monorepo explanations.
    #[arg(long, default_value_t = 0)]
    pub(super) offset: u64,
    /// Maximum changes returned in one page (1-1000).
    #[arg(long, default_value_t = 256)]
    pub(super) limit: u64,
}

#[derive(Args)]
pub(super) struct EnvironmentDiscoverArgs {
    pub(super) lane: String,
    /// Restrict discovery to one source-relative component root.
    #[arg(long, value_name = "ROOT")]
    pub(super) path: Option<String>,
}

#[derive(Args)]
pub(super) struct EnvironmentGraphArgs {
    pub(super) lane: String,
    /// Restrict discovery to one source-relative component root.
    #[arg(long, value_name = "ROOT")]
    pub(super) path: Option<String>,
    /// Zero-based topological node offset.
    #[arg(long, default_value_t = 0)]
    pub(super) offset: u64,
    /// Maximum target nodes returned in one page (1-1000).
    #[arg(long, default_value_t = 256)]
    pub(super) limit: u64,
}

#[derive(Args)]
pub(super) struct EnvironmentSyncArgs {
    pub(super) lane: String,
    /// Stable logical component ID from `trail env discover`.
    #[arg(long, value_name = "ID")]
    pub(super) component: Option<String>,
    /// Versioned environment adapter identity.
    #[arg(long, default_value = "auto")]
    pub(super) adapter: String,
    /// Adapter component root relative to the lane source root.
    #[arg(long, value_name = "ROOT")]
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
