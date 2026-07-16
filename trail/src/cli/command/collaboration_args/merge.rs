use std::path::PathBuf;

use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(in crate::cli::command) enum LaneMergeQueueSubcommand {
    /// Add a lane to the serialized merge queue.
    Add(LaneMergeQueueAddArgs),
    /// List queued merge candidates and states.
    List,
    /// Explain why a queued merge is ready or blocked.
    Explain(LaneMergeQueueExplainArgs),
    /// Run queued merges up to optional item limit.
    Run(LaneMergeQueueRunArgs),
    /// Remove a queued item before execution.
    Remove(LaneMergeQueueRemoveArgs),
}

#[derive(Args)]
pub(in crate::cli::command) struct LaneMergeQueueCommand {
    #[command(subcommand)]
    pub(in crate::cli::command) command: LaneMergeQueueSubcommand,
}

#[derive(Args)]
pub(in crate::cli::command) struct LaneMergeQueueAddArgs {
    pub(in crate::cli::command) lane: String,
    #[arg(long)]
    pub(in crate::cli::command) into: String,
    #[arg(long, default_value_t = 0)]
    pub(in crate::cli::command) priority: i64,
}

#[derive(Args)]
pub(in crate::cli::command) struct LaneMergeQueueRunArgs {
    #[arg(long)]
    pub(in crate::cli::command) limit: Option<usize>,
}

#[derive(Args)]
pub(in crate::cli::command) struct LaneMergeQueueExplainArgs {
    pub(in crate::cli::command) selector: String,
}

#[derive(Args)]
pub(in crate::cli::command) struct LaneMergeQueueRemoveArgs {
    pub(in crate::cli::command) selector: String,
}

#[derive(Subcommand)]
pub(in crate::cli::command) enum ConflictsSubcommand {
    /// List recent unresolved or historical conflict sets.
    List,
    /// Show details for one conflict set.
    Show(ConflictShowArgs),
    /// Resolve a conflict by taking source/target or manual file map.
    Resolve(ConflictResolveArgs),
}

#[derive(Args)]
pub(in crate::cli::command) struct ConflictsCommand {
    #[command(subcommand)]
    pub(in crate::cli::command) command: ConflictsSubcommand,
}

#[derive(Args)]
pub(in crate::cli::command) struct ConflictShowArgs {
    pub(in crate::cli::command) conflict_set_id: String,
    #[arg(long, default_value_t = 50)]
    pub(in crate::cli::command) limit: usize,
}

#[derive(Args)]
pub(in crate::cli::command) struct ConflictResolveArgs {
    pub(in crate::cli::command) conflict_set_id: String,
    #[arg(
        long,
        value_enum,
        required_unless_present = "manual",
        conflicts_with = "manual"
    )]
    pub(in crate::cli::command) take: Option<ConflictTakeArg>,
    #[arg(long, value_name = "JSON")]
    pub(in crate::cli::command) manual: Option<PathBuf>,
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub(in crate::cli::command) enum ConflictTakeArg {
    Source,
    Target,
}

impl ConflictTakeArg {
    pub(in crate::cli::command) fn as_str(&self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Target => "target",
        }
    }
}
