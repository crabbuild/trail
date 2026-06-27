use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(in crate::cli::command) enum SessionSubcommand {
    /// Start a new session for a given lane.
    Start(SessionStartArgs),
    /// Show current session attachment for all lanes or one lane.
    Current(SessionCurrentArgs),
    /// List recent sessions, optionally filtered by lane.
    List(SessionListArgs),
    /// Show one session with context and linked records.
    Show(SessionShowArgs),
    /// Return bounded session context packet.
    Context(SessionContextArgs),
    /// End a session with explicit terminal status.
    End(SessionEndArgs),
}

#[derive(Args)]
pub(in crate::cli::command) struct SessionCommand {
    #[command(subcommand)]
    pub(in crate::cli::command) command: SessionSubcommand,
}

#[derive(Args)]
pub(in crate::cli::command) struct SessionStartArgs {
    pub(in crate::cli::command) lane: String,
    #[arg(long)]
    pub(in crate::cli::command) title: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) id: Option<String>,
}

#[derive(Args)]
pub(in crate::cli::command) struct SessionListArgs {
    #[arg(long)]
    pub(in crate::cli::command) lane: Option<String>,
}

#[derive(Args)]
pub(in crate::cli::command) struct SessionCurrentArgs {
    pub(in crate::cli::command) lane: Option<String>,
}

#[derive(Args)]
pub(in crate::cli::command) struct SessionShowArgs {
    pub(in crate::cli::command) session_id: String,
}

#[derive(Args)]
pub(in crate::cli::command) struct SessionContextArgs {
    pub(in crate::cli::command) session_id: String,
    #[arg(long, default_value_t = 50)]
    pub(in crate::cli::command) limit: usize,
}

#[derive(Args)]
pub(in crate::cli::command) struct SessionEndArgs {
    pub(in crate::cli::command) session_id: String,
    #[arg(long, default_value = "completed")]
    pub(in crate::cli::command) status: String,
}
