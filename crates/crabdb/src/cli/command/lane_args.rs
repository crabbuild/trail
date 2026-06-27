use std::path::PathBuf;

use clap::{Args, Subcommand};

mod run;
mod trace;
mod turn;

pub(super) use run::*;
pub(super) use trace::*;
pub(super) use turn::*;

#[derive(Subcommand)]
pub(super) enum LaneSubcommand {
    /// Create a new lane branch and optional materialized workdir.
    Spawn(LaneSpawnArgs),
    /// List all lane branches and metadata.
    List,
    /// Show one lane record and branch state.
    Show(LaneShowArgs),
    /// Show current status for a lane branch including readiness signals.
    Status(LaneStatusArgs),
    /// Produce a compact review packet for a lane branch.
    Review(LaneReviewArgs),
    /// Build a lane change review bundle with operation history.
    Contribution(LaneContributionArgs),
    /// List recent lane test/eval gate results by kind.
    Gates(LaneGatesArgs),
    /// Compute lane merge-readiness including blockers and warnings.
    Readiness(LaneReadinessArgs),
    /// Preview refreshing a lane onto a target branch before merge.
    RefreshPreview(LaneRefreshPreviewArgs),
    /// Produce a handoff-ready transfer packet for a lane.
    Handoff(LaneHandoffArgs),
    /// Create a best-effort advisory claim for path-level work coordination.
    Claim(LaneClaimArgs),
    /// Add a message to a lane timeline.
    Message(LaneMessageArgs),
    /// Work directly with durable lane turns.
    Turn(LaneTurnCommand),
    /// Manage durable paused/resumed lane runs.
    Run(LaneRunCommand),
    /// Query structured trace events across lanes, sessions, and turns.
    Events(LaneEventsArgs),
    /// Manage trace spans (start/end/list/summary/show).
    Trace(LaneTraceCommand),
    /// Record all current lane workdir changes as one operation.
    Record(LaneRecordArgs),
    /// Rewind a lane branch to a known-good change or root.
    Rewind(LaneRewindArgs),
    /// Watch and record lane workdir changes continuously.
    Watch(LaneWatchArgs),
    /// Run a command in lane workdir and record test gate metadata.
    Test(LaneTestArgs),
    /// Run evaluation command in lane workdir and record eval gate metadata.
    Eval(LaneTestArgs),
    /// Read one file from a lane branch, lazily hydrating sparse workdirs by default.
    Read(LaneReadArgs),
    /// Print the resolved lane workdir path.
    Workdir(LaneWorkdirArgs),
    /// Re-sync lane workdir from the lane branch head.
    SyncWorkdir(LaneSyncWorkdirArgs),
    /// Apply a structured patch directly to a lane branch.
    ApplyPatch(LanePatchArgs),
    /// Show current diff for a lane branch head vs base.
    Diff(LaneDiffArgs),
    /// List operations on a lane timeline.
    Timeline(LaneTimelineArgs),
    /// Preview or materialize a lane branch into workspace.
    Checkout(LaneCheckoutArgs),
    /// Remove a lane branch and associated workdir materialization.
    Rm(LaneRemoveArgs),
}

#[derive(Args)]
pub(super) struct LaneCommand {
    #[command(subcommand)]
    pub(super) command: LaneSubcommand,
}

#[derive(Args)]
pub(super) struct LaneSpawnArgs {
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
pub(super) struct LanePatchArgs {
    pub(super) name: String,
    #[arg(long)]
    pub(super) patch: PathBuf,
    #[arg(long)]
    pub(super) allow_ignored: bool,
    #[arg(long)]
    pub(super) allow_stale: bool,
}

#[derive(Args)]
pub(super) struct LaneClaimArgs {
    pub(super) name: String,
    pub(super) path: String,
    #[arg(long, default_value_t = 600)]
    pub(super) ttl_secs: u64,
}

#[derive(Args)]
pub(super) struct LaneShowArgs {
    pub(super) name: String,
}

#[derive(Args)]
pub(super) struct LaneStatusArgs {
    pub(super) name: String,
}

#[derive(Args)]
pub(super) struct LaneReviewArgs {
    pub(super) name: String,
    #[arg(long, default_value_t = 50)]
    pub(super) limit: usize,
}

#[derive(Args)]
pub(super) struct LaneReadinessArgs {
    pub(super) name: String,
}

#[derive(Args)]
pub(super) struct LaneRefreshPreviewArgs {
    pub(super) name: String,
    #[arg(long, default_value = "main")]
    pub(super) target: String,
}

#[derive(Args)]
pub(super) struct LaneHandoffArgs {
    pub(super) name: String,
    #[arg(long, default_value_t = 50)]
    pub(super) limit: usize,
}

#[derive(Args)]
pub(super) struct LaneContributionArgs {
    pub(super) name: String,
    #[arg(long, default_value_t = 50)]
    pub(super) limit: usize,
}

#[derive(Args)]
pub(super) struct LaneGatesArgs {
    pub(super) name: String,
    #[arg(long)]
    pub(super) kind: Option<String>,
    #[arg(long, default_value_t = 50)]
    pub(super) limit: usize,
}

#[derive(Args)]
pub(super) struct LaneMessageArgs {
    pub(super) name: String,
    #[arg(long)]
    pub(super) role: String,
    #[arg(long)]
    pub(super) text: String,
    #[arg(long)]
    pub(super) session: Option<String>,
}

#[derive(Args)]
pub(super) struct LaneRecordArgs {
    pub(super) name: String,
    #[arg(short, long)]
    pub(super) message: Option<String>,
    #[arg(long)]
    pub(super) preview: bool,
}

#[derive(Args)]
pub(super) struct LaneRewindArgs {
    pub(super) name: String,
    #[arg(long = "to")]
    pub(super) target: String,
    #[arg(long)]
    pub(super) record_current: bool,
    #[arg(long)]
    pub(super) sync_workdir: bool,
}

#[derive(Args)]
pub(super) struct LaneWatchArgs {
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
pub(super) struct LaneTestArgs {
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
pub(super) struct LaneReadArgs {
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
pub(super) struct LaneWorkdirArgs {
    pub(super) name: String,
}

#[derive(Args)]
pub(super) struct LaneSyncWorkdirArgs {
    pub(super) name: String,
    #[arg(long)]
    pub(super) force: bool,
    #[arg(long, num_args = 1..)]
    pub(super) paths: Vec<String>,
    #[arg(long)]
    pub(super) include_neighbors: bool,
}

#[derive(Args)]
pub(super) struct LaneDiffArgs {
    pub(super) name: String,
    #[arg(long)]
    pub(super) patch: bool,
    #[arg(long = "show-line-ids")]
    pub(super) show_line_ids: bool,
}

#[derive(Args)]
pub(super) struct LaneTimelineArgs {
    pub(super) name: String,
    #[arg(long, default_value_t = 30)]
    pub(super) limit: usize,
}

#[derive(Args)]
pub(super) struct LaneEventsArgs {
    #[arg(long)]
    pub(super) lane: Option<String>,
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
pub(super) struct LaneCheckoutArgs {
    pub(super) name: String,
    #[arg(long)]
    pub(super) force: bool,
    #[arg(long)]
    pub(super) dry_run: bool,
    #[arg(long)]
    pub(super) workdir: Option<PathBuf>,
}

#[derive(Args)]
pub(super) struct LaneRemoveArgs {
    pub(super) name: String,
    #[arg(long)]
    pub(super) force: bool,
}
