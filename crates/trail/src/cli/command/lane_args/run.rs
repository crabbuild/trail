use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(in crate::cli::command) enum LaneRunSubcommand {
    /// Pause a lane run with optional interruption state.
    Pause(LaneRunPauseArgs),
    /// List paused or active lane runs.
    List(LaneRunListArgs),
    /// Show one lane run checkpoint.
    Show(LaneRunShowArgs),
    /// Resume a paused run after approval or state review.
    Resume(LaneRunResumeArgs),
}

#[derive(Args)]
pub(in crate::cli::command) struct LaneRunCommand {
    #[command(subcommand)]
    pub(in crate::cli::command) command: LaneRunSubcommand,
}

#[derive(Args)]
pub(in crate::cli::command) struct LaneRunPauseArgs {
    pub(in crate::cli::command) name: String,
    #[arg(long)]
    pub(in crate::cli::command) reason: String,
    #[arg(long)]
    pub(in crate::cli::command) summary: String,
    #[arg(long = "state-json")]
    pub(in crate::cli::command) state_json: Option<String>,
    #[arg(long = "interruption-json")]
    pub(in crate::cli::command) interruption_json: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) session: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) turn: Option<String>,
}

#[derive(Args)]
pub(in crate::cli::command) struct LaneRunListArgs {
    #[arg(long)]
    pub(in crate::cli::command) lane: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) status: Option<String>,
}

#[derive(Args)]
pub(in crate::cli::command) struct LaneRunShowArgs {
    pub(in crate::cli::command) run_id: String,
}

#[derive(Args)]
pub(in crate::cli::command) struct LaneRunResumeArgs {
    pub(in crate::cli::command) run_id: String,
    #[arg(long)]
    pub(in crate::cli::command) reviewer: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) note: Option<String>,
}
