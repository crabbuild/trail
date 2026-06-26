use std::path::PathBuf;

use clap::{Args, Subcommand};

mod run;
mod trace;
mod turn;

pub(super) use run::*;
pub(super) use trace::*;
pub(super) use turn::*;

#[derive(Subcommand)]
pub(super) enum AgentSubcommand {
    /// Create a new agent branch and optional materialized workdir.
    Spawn(AgentSpawnArgs),
    /// List all agent branches and metadata.
    List,
    /// Show one agent record and branch state.
    Show(AgentShowArgs),
    /// Show current status for an agent branch including readiness signals.
    Status(AgentStatusArgs),
    /// Produce a compact review packet for an agent branch.
    Review(AgentReviewArgs),
    /// Build an agent change review bundle with operation history.
    Contribution(AgentContributionArgs),
    /// List recent agent test/eval gate results by kind.
    Gates(AgentGatesArgs),
    /// Compute agent merge-readiness including blockers and warnings.
    Readiness(AgentReadinessArgs),
    /// Produce a handoff-ready transfer packet for an agent.
    Handoff(AgentHandoffArgs),
    /// Create a best-effort advisory claim for path-level work coordination.
    Claim(AgentClaimArgs),
    /// Add a message to an agent timeline.
    Message(AgentMessageArgs),
    /// Work directly with durable agent turns.
    Turn(AgentTurnCommand),
    /// Manage durable paused/resumed agent runs.
    Run(AgentRunCommand),
    /// Query structured trace events across agents, sessions, and turns.
    Events(AgentEventsArgs),
    /// Manage trace spans (start/end/list/summary/show).
    Trace(AgentTraceCommand),
    /// Record all current agent workdir changes as one operation.
    Record(AgentRecordArgs),
    /// Watch and record agent workdir changes continuously.
    Watch(AgentWatchArgs),
    /// Run a command in agent workdir and record test gate metadata.
    Test(AgentTestArgs),
    /// Run evaluation command in agent workdir and record eval gate metadata.
    Eval(AgentTestArgs),
    /// Read one file from an agent branch, lazily hydrating sparse workdirs by default.
    Read(AgentReadArgs),
    /// Print the resolved agent workdir path.
    Workdir(AgentWorkdirArgs),
    /// Re-sync agent workdir from the agent branch head.
    SyncWorkdir(AgentSyncWorkdirArgs),
    /// Apply a structured patch directly to an agent branch.
    ApplyPatch(AgentPatchArgs),
    /// Show current diff for an agent branch head vs base.
    Diff(AgentDiffArgs),
    /// List operations on an agent timeline.
    Timeline(AgentTimelineArgs),
    /// Preview or materialize an agent branch into workspace.
    Checkout(AgentCheckoutArgs),
    /// Remove an agent branch and associated workdir materialization.
    Rm(AgentRemoveArgs),
}

#[derive(Args)]
pub(super) struct AgentCommand {
    #[command(subcommand)]
    pub(super) command: AgentSubcommand,
}

#[derive(Args)]
pub(super) struct AgentSpawnArgs {
    pub(super) name: String,
    #[arg(long)]
    pub(super) from: Option<String>,
    #[arg(
        long,
        default_missing_value = "true",
        num_args = 0..=1,
        require_equals = true,
        conflicts_with = "no_materialize"
    )]
    pub(super) materialize: Option<bool>,
    #[arg(long = "no-materialize")]
    pub(super) no_materialize: bool,
    #[arg(long)]
    pub(super) workdir: Option<PathBuf>,
    #[arg(long, num_args = 1.., conflicts_with = "no_materialize")]
    pub(super) paths: Vec<String>,
    #[arg(long)]
    pub(super) include_neighbors: bool,
    #[arg(long)]
    pub(super) provider: Option<String>,
    #[arg(long)]
    pub(super) model: Option<String>,
}

#[derive(Args)]
pub(super) struct AgentPatchArgs {
    pub(super) name: String,
    #[arg(long)]
    pub(super) patch: PathBuf,
    #[arg(long)]
    pub(super) allow_ignored: bool,
}

#[derive(Args)]
pub(super) struct AgentClaimArgs {
    pub(super) name: String,
    pub(super) path: String,
    #[arg(long, default_value_t = 600)]
    pub(super) ttl_secs: u64,
}

#[derive(Args)]
pub(super) struct AgentShowArgs {
    pub(super) name: String,
}

#[derive(Args)]
pub(super) struct AgentStatusArgs {
    pub(super) name: String,
}

#[derive(Args)]
pub(super) struct AgentReviewArgs {
    pub(super) name: String,
    #[arg(long, default_value_t = 50)]
    pub(super) limit: usize,
}

#[derive(Args)]
pub(super) struct AgentReadinessArgs {
    pub(super) name: String,
}

#[derive(Args)]
pub(super) struct AgentHandoffArgs {
    pub(super) name: String,
    #[arg(long, default_value_t = 50)]
    pub(super) limit: usize,
}

#[derive(Args)]
pub(super) struct AgentContributionArgs {
    pub(super) name: String,
    #[arg(long, default_value_t = 50)]
    pub(super) limit: usize,
}

#[derive(Args)]
pub(super) struct AgentGatesArgs {
    pub(super) name: String,
    #[arg(long)]
    pub(super) kind: Option<String>,
    #[arg(long, default_value_t = 50)]
    pub(super) limit: usize,
}

#[derive(Args)]
pub(super) struct AgentMessageArgs {
    pub(super) name: String,
    #[arg(long)]
    pub(super) role: String,
    #[arg(long)]
    pub(super) text: String,
    #[arg(long)]
    pub(super) session: Option<String>,
}

#[derive(Args)]
pub(super) struct AgentRecordArgs {
    pub(super) name: String,
    #[arg(short, long)]
    pub(super) message: Option<String>,
}

#[derive(Args)]
pub(super) struct AgentWatchArgs {
    pub(super) name: String,
    #[arg(short, long)]
    pub(super) message: Option<String>,
    #[arg(long, default_value_t = 2)]
    pub(super) interval_secs: u64,
    #[arg(long = "debounce-ms", alias = "debounce")]
    pub(super) debounce_ms: Option<u64>,
    #[arg(long = "include-untracked")]
    pub(super) include_untracked: bool,
    #[arg(long)]
    pub(super) once: bool,
}

#[derive(Args)]
pub(super) struct AgentTestArgs {
    pub(super) name: String,
    #[arg(long)]
    pub(super) turn: Option<String>,
    #[arg(long, default_value_t = 600)]
    pub(super) timeout_secs: u64,
    #[arg(long)]
    pub(super) suite: Option<String>,
    #[arg(long)]
    pub(super) score: Option<f64>,
    #[arg(long)]
    pub(super) threshold: Option<f64>,
    #[arg(last = true, num_args = 1.., required = true)]
    pub(super) command: Vec<String>,
}

#[derive(Args)]
pub(super) struct AgentReadArgs {
    pub(super) name: String,
    pub(super) path: String,
    #[arg(long, conflicts_with = "no_hydrate")]
    pub(super) hydrate: bool,
    #[arg(long = "no-hydrate")]
    pub(super) no_hydrate: bool,
    #[arg(long)]
    pub(super) force: bool,
    #[arg(long)]
    pub(super) include_neighbors: bool,
}

#[derive(Args)]
pub(super) struct AgentWorkdirArgs {
    pub(super) name: String,
}

#[derive(Args)]
pub(super) struct AgentSyncWorkdirArgs {
    pub(super) name: String,
    #[arg(long)]
    pub(super) force: bool,
    #[arg(long, num_args = 1..)]
    pub(super) paths: Vec<String>,
    #[arg(long)]
    pub(super) include_neighbors: bool,
}

#[derive(Args)]
pub(super) struct AgentDiffArgs {
    pub(super) name: String,
    #[arg(long)]
    pub(super) patch: bool,
    #[arg(long = "show-line-ids")]
    pub(super) show_line_ids: bool,
}

#[derive(Args)]
pub(super) struct AgentTimelineArgs {
    pub(super) name: String,
    #[arg(long, default_value_t = 30)]
    pub(super) limit: usize,
}

#[derive(Args)]
pub(super) struct AgentEventsArgs {
    #[arg(long)]
    pub(super) agent: Option<String>,
    #[arg(long)]
    pub(super) session: Option<String>,
    #[arg(long)]
    pub(super) turn: Option<String>,
    #[arg(long = "type")]
    pub(super) event_type: Option<String>,
    #[arg(long, default_value_t = 50)]
    pub(super) limit: usize,
}

#[derive(Args)]
pub(super) struct AgentCheckoutArgs {
    pub(super) name: String,
    #[arg(long)]
    pub(super) force: bool,
    #[arg(long)]
    pub(super) dry_run: bool,
    #[arg(long)]
    pub(super) workdir: Option<PathBuf>,
}

#[derive(Args)]
pub(super) struct AgentRemoveArgs {
    pub(super) name: String,
    #[arg(long)]
    pub(super) force: bool,
}
