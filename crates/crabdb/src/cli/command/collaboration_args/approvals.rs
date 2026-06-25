use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(in crate::cli::command) enum ApprovalsSubcommand {
    /// Create a new approval request for a sensitive action.
    Request(ApprovalRequestArgs),
    /// List approval requests, optionally filtered by agent/status.
    List(ApprovalListArgs),
    /// Show one approval request record and decision state.
    Show(ApprovalShowArgs),
    /// Record a decision for an existing approval request.
    Decide(ApprovalDecideArgs),
}

#[derive(Args)]
pub(in crate::cli::command) struct ApprovalsCommand {
    #[command(subcommand)]
    pub(in crate::cli::command) command: ApprovalsSubcommand,
}

#[derive(Args)]
pub(in crate::cli::command) struct ApprovalRequestArgs {
    pub(in crate::cli::command) agent: String,
    #[arg(long)]
    pub(in crate::cli::command) action: String,
    #[arg(long)]
    pub(in crate::cli::command) summary: String,
    #[arg(long)]
    pub(in crate::cli::command) payload_json: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) session: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) turn: Option<String>,
}

#[derive(Args)]
pub(in crate::cli::command) struct ApprovalListArgs {
    #[arg(long)]
    pub(in crate::cli::command) agent: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) status: Option<String>,
}

#[derive(Args)]
pub(in crate::cli::command) struct ApprovalShowArgs {
    pub(in crate::cli::command) approval_id: String,
}

#[derive(Args)]
pub(in crate::cli::command) struct ApprovalDecideArgs {
    pub(in crate::cli::command) approval_id: String,
    #[arg(long, value_enum)]
    pub(in crate::cli::command) decision: ApprovalDecisionArg,
    #[arg(long)]
    pub(in crate::cli::command) reviewer: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) note: Option<String>,
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub(in crate::cli::command) enum ApprovalDecisionArg {
    Approved,
    Rejected,
    Cancelled,
}

impl ApprovalDecisionArg {
    pub(in crate::cli::command) fn as_str(&self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Cancelled => "cancelled",
        }
    }
}
