use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(in crate::cli::command) enum AgentTraceSubcommand {
    /// Start a new named trace span.
    Start(AgentTraceStartArgs),
    /// Close a span with final status and optional result payload.
    End(AgentTraceEndArgs),
    /// List recent spans with filtering and limits.
    List(AgentTraceListArgs),
    /// Summarize spans and durations for traces of interest.
    Summary(AgentTraceSummaryArgs),
    /// Show one full span report.
    Show(AgentTraceShowArgs),
}

#[derive(Args)]
pub(in crate::cli::command) struct AgentTraceCommand {
    #[command(subcommand)]
    pub(in crate::cli::command) command: AgentTraceSubcommand,
}

#[derive(Args)]
pub(in crate::cli::command) struct AgentTraceStartArgs {
    pub(in crate::cli::command) turn_id: String,
    #[arg(long = "type")]
    pub(in crate::cli::command) span_type: String,
    #[arg(long)]
    pub(in crate::cli::command) name: String,
    #[arg(long)]
    pub(in crate::cli::command) parent: Option<String>,
    #[arg(long = "trace-id")]
    pub(in crate::cli::command) trace_id: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) attributes_json: Option<String>,
}

#[derive(Args)]
pub(in crate::cli::command) struct AgentTraceEndArgs {
    pub(in crate::cli::command) span_id: String,
    #[arg(long, default_value = "completed")]
    pub(in crate::cli::command) status: String,
    #[arg(long)]
    pub(in crate::cli::command) result_json: Option<String>,
}

#[derive(Args)]
pub(in crate::cli::command) struct AgentTraceListArgs {
    #[arg(long)]
    pub(in crate::cli::command) agent: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) session: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) turn: Option<String>,
    #[arg(long = "trace-id")]
    pub(in crate::cli::command) trace_id: Option<String>,
    #[arg(long, default_value_t = 50)]
    pub(in crate::cli::command) limit: usize,
}

#[derive(Args)]
pub(in crate::cli::command) struct AgentTraceSummaryArgs {
    #[arg(long)]
    pub(in crate::cli::command) agent: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) session: Option<String>,
    #[arg(long)]
    pub(in crate::cli::command) turn: Option<String>,
    #[arg(long = "trace-id")]
    pub(in crate::cli::command) trace_id: Option<String>,
    #[arg(long = "slowest", default_value_t = 5)]
    pub(in crate::cli::command) slowest_limit: usize,
}

#[derive(Args)]
pub(in crate::cli::command) struct AgentTraceShowArgs {
    pub(in crate::cli::command) span_id: String,
}
