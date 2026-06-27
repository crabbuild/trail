use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(in crate::cli::command) enum LaneTraceSubcommand {
    /// Start a new named trace span.
    Start(LaneTraceStartArgs),
    /// Close a span with final status and optional result payload.
    End(LaneTraceEndArgs),
    /// List recent spans with filtering and limits.
    List(LaneTraceListArgs),
    /// Summarize spans and durations for traces of interest.
    Summary(LaneTraceSummaryArgs),
    /// Show one full span report.
    Show(LaneTraceShowArgs),
}

#[derive(Args)]
pub(in crate::cli::command) struct LaneTraceCommand {
    #[command(subcommand)]
    pub(in crate::cli::command) command: LaneTraceSubcommand,
}

#[derive(Args)]
pub(in crate::cli::command) struct LaneTraceStartArgs {
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
pub(in crate::cli::command) struct LaneTraceEndArgs {
    pub(in crate::cli::command) span_id: String,
    #[arg(long, default_value = "completed")]
    pub(in crate::cli::command) status: String,
    #[arg(long)]
    pub(in crate::cli::command) result_json: Option<String>,
}

#[derive(Args)]
pub(in crate::cli::command) struct LaneTraceListArgs {
    #[arg(long)]
    pub(in crate::cli::command) lane: Option<String>,
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
pub(in crate::cli::command) struct LaneTraceSummaryArgs {
    #[arg(long)]
    pub(in crate::cli::command) lane: Option<String>,
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
pub(in crate::cli::command) struct LaneTraceShowArgs {
    pub(in crate::cli::command) span_id: String,
}
