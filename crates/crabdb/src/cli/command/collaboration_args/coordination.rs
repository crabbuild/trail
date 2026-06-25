use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(in crate::cli::command) enum AnchorSubcommand {
    /// Create a stable anchor for a `path:line` selector.
    Create(AnchorCreateArgs),
    /// Resolve an anchor to current identifier in branch context.
    Resolve(AnchorResolveArgs),
    /// List existing anchors.
    List,
    /// Remove an anchor by id.
    Delete(AnchorDeleteArgs),
}

#[derive(Args)]
pub(in crate::cli::command) struct AnchorCommand {
    #[command(subcommand)]
    pub(in crate::cli::command) command: AnchorSubcommand,
}

#[derive(Args)]
pub(in crate::cli::command) struct AnchorCreateArgs {
    pub(in crate::cli::command) path_line: String,
    #[arg(long)]
    pub(in crate::cli::command) label: String,
}

#[derive(Args)]
pub(in crate::cli::command) struct AnchorResolveArgs {
    pub(in crate::cli::command) anchor_id: String,
}

#[derive(Args)]
pub(in crate::cli::command) struct AnchorDeleteArgs {
    pub(in crate::cli::command) anchor_id: String,
}

#[derive(Subcommand)]
pub(in crate::cli::command) enum LeaseSubcommand {
    /// Acquire a lease for a given path and mode.
    Acquire(LeaseAcquireArgs),
    /// List active (or all) advisory leases.
    List(LeaseListArgs),
    /// Release a single lease by id.
    Release(LeaseReleaseArgs),
}

#[derive(Args)]
pub(in crate::cli::command) struct LeaseCommand {
    #[command(subcommand)]
    pub(in crate::cli::command) command: LeaseSubcommand,
}

#[derive(Args)]
pub(in crate::cli::command) struct LeaseAcquireArgs {
    pub(in crate::cli::command) agent: String,
    #[arg(long)]
    pub(in crate::cli::command) path: String,
    #[arg(long, value_enum, default_value = "write")]
    pub(in crate::cli::command) mode: LeaseModeArg,
    #[arg(long, default_value_t = 3600)]
    pub(in crate::cli::command) ttl_secs: u64,
}

#[derive(Args)]
pub(in crate::cli::command) struct LeaseListArgs {
    #[arg(long)]
    pub(in crate::cli::command) all: bool,
}

#[derive(Args)]
pub(in crate::cli::command) struct LeaseReleaseArgs {
    pub(in crate::cli::command) lease_id: String,
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub(in crate::cli::command) enum LeaseModeArg {
    Read,
    Write,
}

impl LeaseModeArg {
    pub(in crate::cli::command) fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}
