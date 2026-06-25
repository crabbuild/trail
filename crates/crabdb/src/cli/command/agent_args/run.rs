use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(in crate::cli::command) enum AgentRunSubcommand {
    /// Pause an agent run with optional interruption state.
    Pause(AgentRunPauseArgs),
    /// List paused or active agent runs.
    List(AgentRunListArgs),
    /// Show one agent run checkpoint.
    Show(AgentRunShowArgs),
    /// Resume a paused run after approval or state review.
    Resume(AgentRunResumeArgs),
}

#[derive(Args)]
pub(in crate::cli::command) struct AgentRunCommand {
    #[command(subcommand)]
    pub(in crate::cli::command) command: AgentRunSubcommand,
}

#[derive(Args)]
pub(in crate::cli::command) struct AgentRunPauseArgs {
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
pub(in crate::cli::command) struct AgentRunListArgs {
    #[arg(long)]
    pub(in crate::cli::command) agent: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) status: Option<String>,
}

#[derive(Args)]
pub(in crate::cli::command) struct AgentRunShowArgs {
    pub(in crate::cli::command) run_id: String,
}

#[derive(Args)]
pub(in crate::cli::command) struct AgentRunResumeArgs {
    pub(in crate::cli::command) run_id: String,
    #[arg(long)]
    pub(in crate::cli::command) reviewer: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) note: Option<String>,
}
