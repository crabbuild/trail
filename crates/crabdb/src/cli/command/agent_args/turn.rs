use std::path::PathBuf;

use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(in crate::cli::command) enum AgentTurnSubcommand {
    /// Start a new durable turn and attach context.
    Start(AgentTurnStartArgs),
    /// Show one turn report with linked messages and events.
    Show(AgentTurnShowArgs),
    /// Add a message event to a turn.
    Message(AgentTurnMessageArgs),
    /// Add a trace event to a turn.
    Event(AgentTurnEventArgs),
    /// Apply a structured patch linked to a turn.
    ApplyPatch(AgentTurnPatchArgs),
    /// Mark a turn finished with terminal status.
    End(AgentTurnEndArgs),
}

#[derive(Args)]
pub(in crate::cli::command) struct AgentTurnCommand {
    #[command(subcommand)]
    pub(in crate::cli::command) command: AgentTurnSubcommand,
}

#[derive(Args)]
pub(in crate::cli::command) struct AgentTurnStartArgs {
    pub(in crate::cli::command) name: String,
    #[arg(long)]
    pub(in crate::cli::command) from: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) title: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) base_change: Option<String>,
}

#[derive(Args)]
pub(in crate::cli::command) struct AgentTurnShowArgs {
    pub(in crate::cli::command) turn_id: String,
}

#[derive(Args)]
pub(in crate::cli::command) struct AgentTurnMessageArgs {
    pub(in crate::cli::command) turn_id: String,
    #[arg(long)]
    pub(in crate::cli::command) role: String,
    #[arg(long)]
    pub(in crate::cli::command) text: String,
}

#[derive(Args)]
pub(in crate::cli::command) struct AgentTurnEventArgs {
    pub(in crate::cli::command) turn_id: String,
    #[arg(long)]
    pub(in crate::cli::command) event_type: String,
    #[arg(long)]
    pub(in crate::cli::command) payload_json: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) change: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) message: Option<String>,
}

#[derive(Args)]
pub(in crate::cli::command) struct AgentTurnPatchArgs {
    pub(in crate::cli::command) turn_id: String,
    #[arg(long)]
    pub(in crate::cli::command) patch: PathBuf,
    #[arg(long)]
    pub(in crate::cli::command) allow_ignored: bool,
}

#[derive(Args)]
pub(in crate::cli::command) struct AgentTurnEndArgs {
    pub(in crate::cli::command) turn_id: String,
    #[arg(long, default_value = "completed")]
    pub(in crate::cli::command) status: String,
}
