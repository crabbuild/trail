use std::path::PathBuf;

use clap::{Args, Subcommand};

#[derive(Args)]
pub(in crate::cli::command) struct MergeAgentArgs {
    pub(in crate::cli::command) name: String,
    #[arg(long, default_value = "main")]
    pub(in crate::cli::command) into: String,
    #[arg(long)]
    pub(in crate::cli::command) strategy: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) dry_run: bool,
}

#[derive(Subcommand)]
pub(in crate::cli::command) enum MergeQueueSubcommand {
    /// Add a source ref to the merge queue.
    Add(MergeQueueAddArgs),
    /// List queued merge candidates and states.
    List,
    /// Run queued merges up to optional item limit.
    Run(MergeQueueRunArgs),
    /// Remove a queued item before execution.
    Remove(MergeQueueRemoveArgs),
}

#[derive(Args)]
pub(in crate::cli::command) struct MergeQueueCommand {
    #[command(subcommand)]
    pub(in crate::cli::command) command: MergeQueueSubcommand,
}

#[derive(Args)]
pub(in crate::cli::command) struct MergeQueueAddArgs {
    pub(in crate::cli::command) source: String,
    #[arg(long)]
    pub(in crate::cli::command) into: String,
    #[arg(long, default_value_t = 0)]
    pub(in crate::cli::command) priority: i64,
}

#[derive(Args)]
pub(in crate::cli::command) struct MergeQueueRunArgs {
    #[arg(long)]
    pub(in crate::cli::command) limit: Option<usize>,
}

#[derive(Args)]
pub(in crate::cli::command) struct MergeQueueRemoveArgs {
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
