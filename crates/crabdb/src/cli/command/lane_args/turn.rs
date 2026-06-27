use std::path::PathBuf;

use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(in crate::cli::command) enum LaneTurnSubcommand {
    /// Start a new durable turn and attach context.
    Start(LaneTurnStartArgs),
    /// Show one turn report with linked messages and events.
    Show(LaneTurnShowArgs),
    /// Add a message event to a turn.
    Message(LaneTurnMessageArgs),
    /// Add a trace event to a turn.
    Event(LaneTurnEventArgs),
    /// Apply a structured patch linked to a turn.
    ApplyPatch(LaneTurnPatchArgs),
    /// Mark a turn finished with terminal status.
    End(LaneTurnEndArgs),
}

#[derive(Args)]
pub(in crate::cli::command) struct LaneTurnCommand {
    #[command(subcommand)]
    pub(in crate::cli::command) command: LaneTurnSubcommand,
}

#[derive(Args)]
pub(in crate::cli::command) struct LaneTurnStartArgs {
    pub(in crate::cli::command) name: String,
    #[arg(long)]
    pub(in crate::cli::command) from: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) title: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) base_change: Option<String>,
}

#[derive(Args)]
pub(in crate::cli::command) struct LaneTurnShowArgs {
    pub(in crate::cli::command) turn_id: String,
}

#[derive(Args)]
pub(in crate::cli::command) struct LaneTurnMessageArgs {
    pub(in crate::cli::command) turn_id: String,
    #[arg(long)]
    pub(in crate::cli::command) role: String,
    #[arg(long)]
    pub(in crate::cli::command) text: String,
}

#[derive(Args)]
pub(in crate::cli::command) struct LaneTurnEventArgs {
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
pub(in crate::cli::command) struct LaneTurnPatchArgs {
    pub(in crate::cli::command) turn_id: String,
    #[arg(long)]
    pub(in crate::cli::command) patch: PathBuf,
    #[arg(long)]
    pub(in crate::cli::command) allow_ignored: bool,
    #[arg(long)]
    pub(in crate::cli::command) allow_stale: bool,
}

#[derive(Args)]
pub(in crate::cli::command) struct LaneTurnEndArgs {
    pub(in crate::cli::command) turn_id: String,
    #[arg(long, default_value = "completed")]
    pub(in crate::cli::command) status: String,
}
