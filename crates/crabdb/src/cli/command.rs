use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

mod handler;
mod render;

pub(crate) fn run() {
    handler::run_cli();
}

#[derive(Parser)]
#[command(name = "crabdb")]
#[command(about = "Local-first operation database for code and text worktrees")]
struct Cli {
    #[arg(long, global = true)]
    workspace: Option<PathBuf>,
    #[arg(long, global = true)]
    db: Option<PathBuf>,
    #[arg(long, global = true)]
    branch: Option<String>,
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true)]
    quiet: bool,
    #[arg(long, global = true)]
    verbose: bool,
    #[arg(long, global = true)]
    trace: bool,
    #[arg(long, global = true)]
    no_color: bool,
    #[arg(long, global = true)]
    format: Option<OutputFormat>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum OutputFormat {
    Human,
    Json,
    Ndjson,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize a new CrabDB workspace and default branch state.
    /// Use this once per repository to create `.crabdb`, default config,
    /// `.crabignore`, and baseline root metadata.
    Init(InitArgs),
    /// Inspect and edit workspace configuration values.
    /// Use `get` and `set` to read typed keys and adjust behavior safely.
    Config(ConfigCommand),
    /// Manage `.crabignore` rules that shield files from recording.
    /// Add/remove patterns or check whether a path is currently ignored.
    Ignore(IgnoreCommand),
    /// Run policy checks for proposed agent actions.
    /// Use `check` to preflight approval, denylist, and ignore decisions.
    Guardrails(GuardrailsCommand),
    /// Show current branch state, root object, dirty status, and recent changes.
    Status(StatusArgs),
    /// Scan worktree changes and record a new operation on the active branch.
    /// Supports partial recording via `--paths` and optional session links.
    Record(RecordArgs),
    /// Poll the worktree on a timer and record cleanly detected changes.
    /// Useful for background recording in a local loop or automation scripts.
    Watch(WatchArgs),
    /// List recent operations across workspace, branch, session, or agent scope.
    Timeline(TimelineArgs),
    /// Print details for an operation, message, ref, or object id.
    /// Supports human and JSON output.
    Show(ShowArgs),
    /// Inspect generic CrabDB object metadata and summarize known object kinds.
    Object(ObjectCommand),
    /// Decode and display a `WorktreeRoot` object in detail.
    Root(RootCommand),
    /// Decode and display a `TextContent` object including line identities.
    Text(TextCommand),
    /// Inspect low-level prolly map roots and byte ranges.
    /// Supports several map decoding modes and key-address forms.
    Map(MapCommand),
    /// Compare two refs or roots and optionally print a patch.
    Diff(DiffArgs),
    /// Materialize a branch, ref, operation, or root into the workspace.
    /// Supports safe dry-run and alternate workdir workflows.
    Checkout(CheckoutArgs),
    /// Create, rename, list, or delete branch refs in local history.
    Branch(BranchArgs),
    /// Merge a source branch/ref into a target with conflict-aware checks.
    /// Returns planned paths on dry run and conflict sets on failure.
    Merge(MergeArgs),
    /// Resolve authorship and history for a specific path or stable line.
    /// Useful for provenance questions before changing a location.
    Why(WhyArgs),
    /// Show file and line edit history from derived indexes.
    History(HistoryArgs),
    /// Find source operations and changed paths from a message, session, or agent.
    CodeFrom(CodeFromArgs),
    /// Manage agent branches, metadata, sessions, patches, tests, and traces.
    /// This command group covers the full CLI-facing agent workflow.
    Agent(AgentCommand),
    /// Create and manage agent sessions, context packets, and lifecycle.
    Session(SessionCommand),
    /// Handle sensitive action approvals and reviewer decisions.
    Approvals(ApprovalsCommand),
    /// Merge an agent branch into a standard branch with readiness checks.
    /// Applies the same conflict rules as queue/manual merges.
    MergeAgent(MergeAgentArgs),
    /// Schedule and run controlled serialized merges using a queue.
    /// Merge items pause on first conflict and keep audit history.
    MergeQueue(MergeQueueCommand),
    /// Inspect and resolve conflict sets opened by merge operations.
    Conflicts(ConflictsCommand),
    /// Create and resolve stable anchors that survive nearby line churn.
    Anchor(AnchorCommand),
    /// Manage advisory read/write leases used for path coordination.
    /// Helps prevent overlapping agent writes across workdirs.
    Lease(LeaseCommand),
    /// Move data between CrabDB refs and Git, and inspect mapping metadata.
    Git(GitCommand),
    /// Inspect and render local API schemas for integrations.
    Api(ApiCommand),
    /// Run the JSON HTTP API daemon for editor and agent integrations.
    Daemon(DaemonArgs),
    /// Start the MCP stdio server for agent host tool discovery.
    Mcp,
    /// Run full local diagnostics for workspace and integration readiness.
    Doctor,
    /// Export, verify, and restore workspace backup bundles.
    Backup(BackupCommand),
    /// Verify repository integrity and report structural or reference issues.
    Fsck,
    /// Rebuild searchable indexes used by history and provenance queries.
    Index(IndexCommand),
    /// Prune unused objects and stale index references.
    Gc(GcArgs),
}

#[derive(Subcommand)]
enum ConfigSubcommand {
    /// List all currently configured workspace keys.
    List,
    /// Print one typed workspace config value.
    Get(ConfigGetArgs),
    /// Set one typed workspace config value after validation.
    Set(ConfigSetArgs),
}

#[derive(Args)]
struct ConfigCommand {
    #[command(subcommand)]
    command: ConfigSubcommand,
}

#[derive(Args)]
struct ConfigGetArgs {
    key: String,
}

#[derive(Args)]
struct ConfigSetArgs {
    key: String,
    value: String,
}

#[derive(Subcommand)]
enum IgnoreSubcommand {
    /// Print the active ignore patterns.
    List,
    /// Add a path pattern to `.crabignore`.
    Add(IgnorePatternArgs),
    /// Remove a path pattern from `.crabignore`.
    Remove(IgnorePatternArgs),
    /// Check whether a path is currently ignored.
    Check(IgnoreCheckArgs),
}

#[derive(Args)]
struct IgnoreCommand {
    #[command(subcommand)]
    command: IgnoreSubcommand,
}

#[derive(Args)]
struct IgnorePatternArgs {
    pattern: String,
}

#[derive(Args)]
struct IgnoreCheckArgs {
    path: String,
}

#[derive(Subcommand)]
enum GuardrailsSubcommand {
    /// Preflight an agent action against policy and ignore checks.
    Check(GuardrailCheckArgs),
}

#[derive(Args)]
struct GuardrailsCommand {
    #[command(subcommand)]
    command: GuardrailsSubcommand,
}

#[derive(Args)]
struct GuardrailCheckArgs {
    #[arg(long)]
    agent: Option<String>,
    #[arg(long)]
    action: String,
    #[arg(long)]
    summary: Option<String>,
    #[arg(long = "payload-json")]
    payload_json: Option<String>,
    #[arg(long = "path")]
    paths: Vec<String>,
}

#[derive(Args)]
struct InitArgs {
    #[arg(long)]
    from_git: bool,
    #[arg(long)]
    working_tree: bool,
    #[arg(long, default_value = "main")]
    branch: String,
    #[arg(long = "text-policy", value_enum)]
    text_policy: Option<TextPolicyArg>,
    #[arg(long)]
    force: bool,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum TextPolicyArg {
    Minimal,
    Balanced,
    Full,
}

impl TextPolicyArg {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Balanced => "balanced",
            Self::Full => "full",
        }
    }
}

#[derive(Args)]
struct StatusArgs {
    #[arg(long)]
    branch: Option<String>,
}

#[derive(Args)]
struct RecordArgs {
    #[arg(short, long)]
    message: Option<String>,
    #[arg(long, num_args = 1..)]
    paths: Vec<String>,
    #[arg(long)]
    kind: Option<String>,
    #[arg(long)]
    session: Option<String>,
    #[arg(long)]
    allow_ignored: bool,
}

#[derive(Args)]
struct WatchArgs {
    #[arg(short, long)]
    message: Option<String>,
    #[arg(long)]
    session: Option<String>,
    #[arg(long, default_value_t = 2)]
    interval_secs: u64,
    #[arg(long = "debounce-ms", alias = "debounce")]
    debounce_ms: Option<u64>,
    #[arg(long = "include-untracked")]
    include_untracked: bool,
    #[arg(long)]
    once: bool,
}

#[derive(Args)]
struct TimelineArgs {
    #[arg(long, default_value_t = 30)]
    limit: usize,
    #[arg(long)]
    branch: Option<String>,
    #[arg(long)]
    session: Option<String>,
    #[arg(long)]
    agent: Option<String>,
}

#[derive(Args)]
struct ShowArgs {
    selector: String,
}

#[derive(Subcommand)]
enum ObjectSubcommand {
    /// Show a structured object summary for a specific object id.
    Show(ObjectShowArgs),
}

#[derive(Args)]
struct ObjectCommand {
    #[command(subcommand)]
    command: ObjectSubcommand,
}

#[derive(Args)]
struct ObjectShowArgs {
    object_id: String,
}

#[derive(Subcommand)]
enum RootSubcommand {
    /// Show a `WorktreeRoot` object with stable file metadata.
    Show(RootShowArgs),
}

#[derive(Args)]
struct RootCommand {
    #[command(subcommand)]
    command: RootSubcommand,
}

#[derive(Args)]
struct RootShowArgs {
    root_id: String,
}

#[derive(Subcommand)]
enum TextSubcommand {
    /// Show a `TextContent` object and stable line identifiers.
    Show(TextShowArgs),
}

#[derive(Args)]
struct TextCommand {
    #[command(subcommand)]
    command: TextSubcommand,
}

#[derive(Args)]
struct TextShowArgs {
    text_id: String,
    #[arg(long, default_value_t = 50)]
    limit: usize,
}

#[derive(Subcommand)]
enum MapSubcommand {
    /// Show a mapped range from a prolly map root.
    Range(MapRangeArgs),
    /// Diff two prolly map roots with optional range filtering.
    Diff(MapDiffArgs),
}

#[derive(Args)]
struct MapCommand {
    #[command(subcommand)]
    command: MapSubcommand,
}

#[derive(Args)]
struct MapRangeArgs {
    map_id: String,
    #[arg(long = "map-type", value_enum, default_value = "raw")]
    map_type: MapTypeArg,
    #[arg(long)]
    start: Option<String>,
    #[arg(long)]
    end: Option<String>,
    #[arg(long, default_value_t = 50)]
    limit: usize,
}

#[derive(Args)]
struct MapDiffArgs {
    left_map_id: String,
    right_map_id: String,
    #[arg(long = "map-type", value_enum, default_value = "raw")]
    map_type: MapTypeArg,
    #[arg(long)]
    start: Option<String>,
    #[arg(long)]
    end: Option<String>,
    #[arg(long, default_value_t = 50)]
    limit: usize,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum MapTypeArg {
    Raw,
    Path,
    FileIndex,
    TextOrder,
    LineIndex,
}

impl MapTypeArg {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Path => "path",
            Self::FileIndex => "file-index",
            Self::TextOrder => "text-order",
            Self::LineIndex => "line-index",
        }
    }
}

#[derive(Args)]
struct DiffArgs {
    range: Option<String>,
    #[arg(long)]
    patch: bool,
    #[arg(long)]
    stat: bool,
    #[arg(long)]
    dirty: bool,
    #[arg(long)]
    root: Option<String>,
    #[arg(long = "show-line-ids")]
    show_line_ids: bool,
}

#[derive(Args)]
struct CheckoutArgs {
    target: String,
    #[arg(long)]
    force: bool,
    #[arg(long)]
    record_dirty: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    workdir: Option<PathBuf>,
}

#[derive(Args)]
struct BranchArgs {
    name: Option<String>,
    #[arg(long)]
    from: Option<String>,
    #[arg(long)]
    delete: Option<String>,
    #[arg(long)]
    rename: Option<String>,
    #[arg(long)]
    to: Option<String>,
}

#[derive(Args)]
struct MergeArgs {
    source: String,
    #[arg(long)]
    into: String,
    #[arg(long)]
    strategy: Option<String>,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct WhyArgs {
    path_line: Option<String>,
    #[arg(long)]
    at: Option<String>,
    #[arg(long = "line-id")]
    line_id: Option<String>,
}

#[derive(Args)]
struct HistoryArgs {
    selector: Option<String>,
    #[arg(long)]
    file_id: Option<String>,
    #[arg(long)]
    line_id: Option<String>,
}

#[derive(Args)]
struct CodeFromArgs {
    selector: String,
}

#[derive(Subcommand)]
enum AgentSubcommand {
    /// Create a new agent branch and optional materialized workdir.
    Spawn(AgentSpawnArgs),
    /// List all agent branches and metadata.
    List,
    /// Show one agent record and branch state.
    Show(AgentShowArgs),
    /// Show current status for an agent branch including readiness signals.
    Status(AgentStatusArgs),
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
struct AgentCommand {
    #[command(subcommand)]
    command: AgentSubcommand,
}

#[derive(Args)]
struct AgentSpawnArgs {
    name: String,
    #[arg(long)]
    from: Option<String>,
    #[arg(
        long,
        default_value_t = true,
        default_missing_value = "true",
        num_args = 0..=1,
        require_equals = true,
        conflicts_with = "no_materialize"
    )]
    materialize: bool,
    #[arg(long = "no-materialize")]
    no_materialize: bool,
    #[arg(long)]
    workdir: Option<PathBuf>,
    #[arg(long)]
    provider: Option<String>,
    #[arg(long)]
    model: Option<String>,
}

#[derive(Args)]
struct AgentPatchArgs {
    name: String,
    #[arg(long)]
    patch: PathBuf,
    #[arg(long)]
    allow_ignored: bool,
}

#[derive(Args)]
struct AgentClaimArgs {
    name: String,
    path: String,
    #[arg(long, default_value_t = 600)]
    ttl_secs: u64,
}

#[derive(Args)]
struct AgentShowArgs {
    name: String,
}

#[derive(Args)]
struct AgentStatusArgs {
    name: String,
}

#[derive(Args)]
struct AgentReadinessArgs {
    name: String,
}

#[derive(Args)]
struct AgentHandoffArgs {
    name: String,
    #[arg(long, default_value_t = 50)]
    limit: usize,
}

#[derive(Args)]
struct AgentContributionArgs {
    name: String,
    #[arg(long, default_value_t = 50)]
    limit: usize,
}

#[derive(Args)]
struct AgentGatesArgs {
    name: String,
    #[arg(long)]
    kind: Option<String>,
    #[arg(long, default_value_t = 50)]
    limit: usize,
}

#[derive(Args)]
struct AgentMessageArgs {
    name: String,
    #[arg(long)]
    role: String,
    #[arg(long)]
    text: String,
    #[arg(long)]
    session: Option<String>,
}

#[derive(Subcommand)]
enum AgentTurnSubcommand {
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
struct AgentTurnCommand {
    #[command(subcommand)]
    command: AgentTurnSubcommand,
}

#[derive(Args)]
struct AgentTurnStartArgs {
    name: String,
    #[arg(long)]
    from: Option<String>,
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    base_change: Option<String>,
}

#[derive(Args)]
struct AgentTurnShowArgs {
    turn_id: String,
}

#[derive(Args)]
struct AgentTurnMessageArgs {
    turn_id: String,
    #[arg(long)]
    role: String,
    #[arg(long)]
    text: String,
}

#[derive(Args)]
struct AgentTurnEventArgs {
    turn_id: String,
    #[arg(long)]
    event_type: String,
    #[arg(long)]
    payload_json: Option<String>,
    #[arg(long)]
    change: Option<String>,
    #[arg(long)]
    message: Option<String>,
}

#[derive(Args)]
struct AgentTurnPatchArgs {
    turn_id: String,
    #[arg(long)]
    patch: PathBuf,
    #[arg(long)]
    allow_ignored: bool,
}

#[derive(Args)]
struct AgentTurnEndArgs {
    turn_id: String,
    #[arg(long, default_value = "completed")]
    status: String,
}

#[derive(Subcommand)]
enum AgentRunSubcommand {
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
struct AgentRunCommand {
    #[command(subcommand)]
    command: AgentRunSubcommand,
}

#[derive(Args)]
struct AgentRunPauseArgs {
    name: String,
    #[arg(long)]
    reason: String,
    #[arg(long)]
    summary: String,
    #[arg(long = "state-json")]
    state_json: Option<String>,
    #[arg(long = "interruption-json")]
    interruption_json: Option<String>,
    #[arg(long)]
    session: Option<String>,
    #[arg(long)]
    turn: Option<String>,
}

#[derive(Args)]
struct AgentRunListArgs {
    #[arg(long)]
    agent: Option<String>,
    #[arg(long)]
    status: Option<String>,
}

#[derive(Args)]
struct AgentRunShowArgs {
    run_id: String,
}

#[derive(Args)]
struct AgentRunResumeArgs {
    run_id: String,
    #[arg(long)]
    reviewer: Option<String>,
    #[arg(long)]
    note: Option<String>,
}

#[derive(Args)]
struct AgentRecordArgs {
    name: String,
    #[arg(short, long)]
    message: Option<String>,
}

#[derive(Args)]
struct AgentWatchArgs {
    name: String,
    #[arg(short, long)]
    message: Option<String>,
    #[arg(long, default_value_t = 2)]
    interval_secs: u64,
    #[arg(long = "debounce-ms", alias = "debounce")]
    debounce_ms: Option<u64>,
    #[arg(long = "include-untracked")]
    include_untracked: bool,
    #[arg(long)]
    once: bool,
}

#[derive(Args)]
struct AgentTestArgs {
    name: String,
    #[arg(long)]
    turn: Option<String>,
    #[arg(long, default_value_t = 600)]
    timeout_secs: u64,
    #[arg(long)]
    suite: Option<String>,
    #[arg(long)]
    score: Option<f64>,
    #[arg(long)]
    threshold: Option<f64>,
    #[arg(last = true, num_args = 1.., required = true)]
    command: Vec<String>,
}

#[derive(Args)]
struct AgentWorkdirArgs {
    name: String,
}

#[derive(Args)]
struct AgentSyncWorkdirArgs {
    name: String,
    #[arg(long)]
    force: bool,
}

#[derive(Args)]
struct AgentDiffArgs {
    name: String,
    #[arg(long)]
    patch: bool,
    #[arg(long = "show-line-ids")]
    show_line_ids: bool,
}

#[derive(Args)]
struct AgentTimelineArgs {
    name: String,
    #[arg(long, default_value_t = 30)]
    limit: usize,
}

#[derive(Args)]
struct AgentEventsArgs {
    #[arg(long)]
    agent: Option<String>,
    #[arg(long)]
    session: Option<String>,
    #[arg(long)]
    turn: Option<String>,
    #[arg(long = "type")]
    event_type: Option<String>,
    #[arg(long, default_value_t = 50)]
    limit: usize,
}

#[derive(Subcommand)]
enum AgentTraceSubcommand {
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
struct AgentTraceCommand {
    #[command(subcommand)]
    command: AgentTraceSubcommand,
}

#[derive(Args)]
struct AgentTraceStartArgs {
    turn_id: String,
    #[arg(long = "type")]
    span_type: String,
    #[arg(long)]
    name: String,
    #[arg(long)]
    parent: Option<String>,
    #[arg(long = "trace-id")]
    trace_id: Option<String>,
    #[arg(long)]
    attributes_json: Option<String>,
}

#[derive(Args)]
struct AgentTraceEndArgs {
    span_id: String,
    #[arg(long, default_value = "completed")]
    status: String,
    #[arg(long)]
    result_json: Option<String>,
}

#[derive(Args)]
struct AgentTraceListArgs {
    #[arg(long)]
    agent: Option<String>,
    #[arg(long)]
    session: Option<String>,
    #[arg(long)]
    turn: Option<String>,
    #[arg(long = "trace-id")]
    trace_id: Option<String>,
    #[arg(long, default_value_t = 50)]
    limit: usize,
}

#[derive(Args)]
struct AgentTraceSummaryArgs {
    #[arg(long)]
    agent: Option<String>,
    #[arg(long)]
    session: Option<String>,
    #[arg(long)]
    turn: Option<String>,
    #[arg(long = "trace-id")]
    trace_id: Option<String>,
    #[arg(long = "slowest", default_value_t = 5)]
    slowest_limit: usize,
}

#[derive(Args)]
struct AgentTraceShowArgs {
    span_id: String,
}

#[derive(Args)]
struct AgentCheckoutArgs {
    name: String,
    #[arg(long)]
    force: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    workdir: Option<PathBuf>,
}

#[derive(Args)]
struct AgentRemoveArgs {
    name: String,
    #[arg(long)]
    force: bool,
}

#[derive(Subcommand)]
enum SessionSubcommand {
    /// Start a new session for a given agent.
    Start(SessionStartArgs),
    /// Show current session attachment for all agents or one agent.
    Current(SessionCurrentArgs),
    /// List recent sessions, optionally filtered by agent.
    List(SessionListArgs),
    /// Show one session with context and linked records.
    Show(SessionShowArgs),
    /// Return bounded session context packet.
    Context(SessionContextArgs),
    /// End a session with explicit terminal status.
    End(SessionEndArgs),
}

#[derive(Args)]
struct SessionCommand {
    #[command(subcommand)]
    command: SessionSubcommand,
}

#[derive(Args)]
struct SessionStartArgs {
    agent: String,
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    id: Option<String>,
}

#[derive(Args)]
struct SessionListArgs {
    #[arg(long)]
    agent: Option<String>,
}

#[derive(Args)]
struct SessionCurrentArgs {
    agent: Option<String>,
}

#[derive(Args)]
struct SessionShowArgs {
    session_id: String,
}

#[derive(Args)]
struct SessionContextArgs {
    session_id: String,
    #[arg(long, default_value_t = 50)]
    limit: usize,
}

#[derive(Args)]
struct SessionEndArgs {
    session_id: String,
    #[arg(long, default_value = "completed")]
    status: String,
}

#[derive(Subcommand)]
enum ApprovalsSubcommand {
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
struct ApprovalsCommand {
    #[command(subcommand)]
    command: ApprovalsSubcommand,
}

#[derive(Args)]
struct ApprovalRequestArgs {
    agent: String,
    #[arg(long)]
    action: String,
    #[arg(long)]
    summary: String,
    #[arg(long)]
    payload_json: Option<String>,
    #[arg(long)]
    session: Option<String>,
    #[arg(long)]
    turn: Option<String>,
}

#[derive(Args)]
struct ApprovalListArgs {
    #[arg(long)]
    agent: Option<String>,
    #[arg(long)]
    status: Option<String>,
}

#[derive(Args)]
struct ApprovalShowArgs {
    approval_id: String,
}

#[derive(Args)]
struct ApprovalDecideArgs {
    approval_id: String,
    #[arg(long, value_enum)]
    decision: ApprovalDecisionArg,
    #[arg(long)]
    reviewer: Option<String>,
    #[arg(long)]
    note: Option<String>,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum ApprovalDecisionArg {
    Approved,
    Rejected,
    Cancelled,
}

impl ApprovalDecisionArg {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Args)]
struct MergeAgentArgs {
    name: String,
    #[arg(long, default_value = "main")]
    into: String,
    #[arg(long)]
    strategy: Option<String>,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Subcommand)]
enum MergeQueueSubcommand {
    /// Add a source ref to the merge queue.
    Add(MergeQueueAddArgs),
    /// List queued merge candidates and states.
    List,
    /// Run queued merges up to optional item limit.
    Run(MergeQueueRunArgs),
    /// Remove a queued item before execution.
    Remove(MergeQueueRemoveArgs),
}

#[derive(Args)]
struct MergeQueueCommand {
    #[command(subcommand)]
    command: MergeQueueSubcommand,
}

#[derive(Args)]
struct MergeQueueAddArgs {
    source: String,
    #[arg(long)]
    into: String,
    #[arg(long, default_value_t = 0)]
    priority: i64,
}

#[derive(Args)]
struct MergeQueueRunArgs {
    #[arg(long)]
    limit: Option<usize>,
}

#[derive(Args)]
struct MergeQueueRemoveArgs {
    selector: String,
}

#[derive(Subcommand)]
enum ConflictsSubcommand {
    /// List recent unresolved or historical conflict sets.
    List,
    /// Show details for one conflict set.
    Show(ConflictShowArgs),
    /// Resolve a conflict by taking source/target or manual file map.
    Resolve(ConflictResolveArgs),
}

#[derive(Args)]
struct ConflictsCommand {
    #[command(subcommand)]
    command: ConflictsSubcommand,
}

#[derive(Args)]
struct ConflictShowArgs {
    conflict_set_id: String,
}

#[derive(Args)]
struct ConflictResolveArgs {
    conflict_set_id: String,
    #[arg(
        long,
        value_enum,
        required_unless_present = "manual",
        conflicts_with = "manual"
    )]
    take: Option<ConflictTakeArg>,
    #[arg(long, value_name = "JSON")]
    manual: Option<PathBuf>,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum ConflictTakeArg {
    Source,
    Target,
}

impl ConflictTakeArg {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Target => "target",
        }
    }
}

#[derive(Subcommand)]
enum AnchorSubcommand {
    /// Create a stable anchor for a `path:line` selector.
    Create(AnchorCreateArgs),
    /// Resolve an anchor to current identifier in branch context.
    Resolve(AnchorResolveArgs),
    /// List existing anchors.
    List,
    /// Remove an anchor by id.
    Delete(AnchorDeleteArgs),
}

#[derive(Args)]
struct AnchorCommand {
    #[command(subcommand)]
    command: AnchorSubcommand,
}

#[derive(Args)]
struct AnchorCreateArgs {
    path_line: String,
    #[arg(long)]
    label: String,
}

#[derive(Args)]
struct AnchorResolveArgs {
    anchor_id: String,
}

#[derive(Args)]
struct AnchorDeleteArgs {
    anchor_id: String,
}

#[derive(Subcommand)]
enum LeaseSubcommand {
    /// Acquire a lease for a given path and mode.
    Acquire(LeaseAcquireArgs),
    /// List active (or all) advisory leases.
    List(LeaseListArgs),
    /// Release a single lease by id.
    Release(LeaseReleaseArgs),
}

#[derive(Args)]
struct LeaseCommand {
    #[command(subcommand)]
    command: LeaseSubcommand,
}

#[derive(Args)]
struct LeaseAcquireArgs {
    agent: String,
    #[arg(long)]
    path: String,
    #[arg(long, value_enum, default_value = "write")]
    mode: LeaseModeArg,
    #[arg(long, default_value_t = 3600)]
    ttl_secs: u64,
}

#[derive(Args)]
struct LeaseListArgs {
    #[arg(long)]
    all: bool,
}

#[derive(Args)]
struct LeaseReleaseArgs {
    lease_id: String,
}

#[derive(Clone, Debug, clap::ValueEnum)]
enum LeaseModeArg {
    Read,
    Write,
}

impl LeaseModeArg {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

#[derive(Subcommand)]
enum GitSubcommand {
    /// Export a range as Git patch or commit.
    Export(GitExportArgs),
    /// Import current Git snapshot into CrabDB.
    ImportUpdate(GitImportUpdateArgs),
    /// List recent Git<->CrabDB mapping entries.
    Mappings(GitMappingsArgs),
}

#[derive(Args)]
struct GitCommand {
    #[command(subcommand)]
    command: GitSubcommand,
}

#[derive(Args)]
struct GitExportArgs {
    range: String,
    #[arg(short, long)]
    message: Option<String>,
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Args)]
struct GitImportUpdateArgs {
    #[arg(short, long)]
    message: Option<String>,
}

#[derive(Args)]
struct GitMappingsArgs {
    #[arg(long, default_value_t = 30)]
    limit: usize,
}

#[derive(Subcommand)]
enum ApiSubcommand {
    /// Print or write the OpenAPI contract JSON.
    Openapi(ApiOpenapiArgs),
}

#[derive(Args)]
struct ApiCommand {
    #[command(subcommand)]
    command: ApiSubcommand,
}

#[derive(Args)]
struct ApiOpenapiArgs {
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Args)]
struct DaemonArgs {
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 8765)]
    port: u16,
    #[arg(long)]
    once: bool,
    #[arg(long)]
    max_requests: Option<usize>,
    #[arg(long)]
    auth_token: Option<String>,
    #[arg(long)]
    auth_token_file: Option<PathBuf>,
    #[arg(long)]
    no_auth: bool,
}

#[derive(Subcommand)]
enum IndexSubcommand {
    /// Rebuild all derived indexes from current workspace state.
    Rebuild,
}

#[derive(Args)]
struct IndexCommand {
    #[command(subcommand)]
    command: IndexSubcommand,
}

#[derive(Args)]
struct GcArgs {
    #[arg(long)]
    dry_run: bool,
}

#[derive(Subcommand)]
enum BackupSubcommand {
    /// Create a portable workspace backup file.
    Create(BackupCreateArgs),
    /// Verify backup integrity before restore.
    Verify(BackupVerifyArgs),
    /// Restore workspace data from a backup archive.
    Restore(BackupRestoreArgs),
}

#[derive(Args)]
struct BackupCommand {
    #[command(subcommand)]
    command: BackupSubcommand,
}

#[derive(Args)]
struct BackupCreateArgs {
    output: PathBuf,
    #[arg(long)]
    overwrite: bool,
}

#[derive(Args)]
struct BackupVerifyArgs {
    path: PathBuf,
}

#[derive(Args)]
struct BackupRestoreArgs {
    path: PathBuf,
    #[arg(long)]
    force: bool,
}
