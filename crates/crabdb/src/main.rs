use std::collections::BTreeMap;
use std::fs;
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use clap::error::ErrorKind as ClapErrorKind;
use clap::{Args, Parser, Subcommand};
use crabdb::model::{
    AgentApproval, AgentApprovalDecisionReport, AgentApprovalRequestReport, AgentClaimReport,
    AgentContributionReport, AgentDetails, AgentEventRecord, AgentGateHistoryReport,
    AgentGateOptions, AgentHandoffReport, AgentMessageReport, AgentPatchReport,
    AgentReadinessReport, AgentRecordReport, AgentRemoveReport, AgentRunPauseReport,
    AgentRunResumeReport, AgentRunState, AgentSession, AgentSessionContextReport,
    AgentSessionCurrentReport, AgentSessionDetails, AgentSessionEndReport, AgentSessionStartReport,
    AgentSpawnReport, AgentStatusReport, AgentTestReport, AgentTraceSpan, AgentTraceSpanEndReport,
    AgentTraceSpanStartReport, AgentTraceSummaryReport, AgentWatchReport, AgentWorkdirReport,
    AgentWorkdirSyncReport, Anchor, AnchorCreateReport, AnchorDeleteReport, AnchorResolveReport,
    BackupCreateReport, BackupRestoreReport, BackupVerifyReport, BranchDeleteReport,
    BranchListEntry, BranchRenameReport, BranchReport, CheckoutReport, CodeFromResult, ConfigEntry,
    ConfigSetReport, ConflictManualFile, ConflictManualResolution, ConflictResolveReport,
    ConflictSetSummary, DiffSummary, DoctorReport, FsckReport, GcReport, GitExportReport,
    GitImportReport, GitMapping, GuardrailCheckReport, HistoryResult, IgnoreAddReport,
    IgnoreCheckReport, IgnoreListReport, IgnoreRemoveReport, IndexRebuildReport, InitReport,
    LeaseAcquireReport, LeaseRecord, LeaseReleaseReport, MapDiffReport, MapRangeReport,
    MergeQueueAddReport, MergeQueueEntry, MergeQueueRemoveReport, MergeQueueRunReport, MergeReport,
    ObjectInspectReport, OperationKind, RecordOptions, RecordReport, RootInspectReport, ShowResult,
    StatusReport, TextInspectReport, TimelineEntry, WhyResult,
};
use crabdb::{Actor, CrabDb, Error, InitImportMode, PatchDocument, Result, WorktreeState};

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

fn main() {
    let json_errors =
        args_request_json_errors(std::env::args_os().skip(1)) || env_requests_json_errors();
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => handle_cli_parse_error(err, json_errors),
    };
    let json_errors = cli.json
        || matches!(cli.format.as_ref(), Some(OutputFormat::Json))
        || env_requests_json_errors();
    if let Err(err) = run(cli) {
        render_error(&err, json_errors);
        std::process::exit(err.exit_code());
    }
}

fn args_request_json_errors<I>(args: I) -> bool
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    let mut expect_format = false;
    for arg in args {
        let arg = arg.to_string_lossy();
        if expect_format {
            if arg == "json" {
                return true;
            }
            expect_format = false;
            continue;
        }
        if arg == "--json" || arg == "--format=json" {
            return true;
        }
        if arg == "--format" {
            expect_format = true;
        }
    }
    false
}

fn env_requests_json_errors() -> bool {
    std::env::var("CRABDB_FORMAT")
        .map(|value| value.eq_ignore_ascii_case("json"))
        .unwrap_or(false)
}

fn resolve_output_format(cli_format: Option<OutputFormat>) -> Result<OutputFormat> {
    if let Some(format) = cli_format {
        return Ok(format);
    }
    let Some(value) = std::env::var("CRABDB_FORMAT").ok() else {
        return Ok(OutputFormat::Human);
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "human" => Ok(OutputFormat::Human),
        "json" => Ok(OutputFormat::Json),
        "ndjson" => Ok(OutputFormat::Ndjson),
        other => Err(Error::InvalidInput(format!(
            "CRABDB_FORMAT must be human, json, or ndjson, got `{other}`"
        ))),
    }
}

fn handle_cli_parse_error(err: clap::Error, json: bool) -> ! {
    match err.kind() {
        ClapErrorKind::DisplayHelp | ClapErrorKind::DisplayVersion => err.exit(),
        _ if json => {
            let exit_code = err.exit_code();
            render_cli_parse_error(&err, exit_code);
            std::process::exit(exit_code);
        }
        _ => err.exit(),
    }
}

fn render_cli_parse_error(err: &clap::Error, exit_code: i32) {
    let message = err.to_string();
    let value = serde_json::json!({
        "error": {
            "code": "INVALID_INPUT",
            "message": message.trim(),
            "exit_code": exit_code
        }
    });
    eprintln!(
        "{}",
        serde_json::to_string(&value).unwrap_or_else(|_| {
            r#"{"error":{"code":"INVALID_INPUT","message":"invalid CLI input","exit_code":2}}"#
                .to_string()
        })
    );
}

fn render_error(err: &Error, json: bool) {
    if json {
        let value = serde_json::json!({
            "error": {
                "code": err.code(),
                "message": err.to_string(),
                "exit_code": err.exit_code()
            }
        });
        eprintln!(
            "{}",
            serde_json::to_string(&value)
                .unwrap_or_else(|_| format!(r#"{{"error":{{"message":"{err}"}}}}"#))
        );
    } else {
        eprintln!("crabdb: {err}");
    }
}

fn run(cli: Cli) -> Result<()> {
    let format = resolve_output_format(cli.format)?;
    let json = cli.json || matches!(format, OutputFormat::Json);
    let workspace = cli
        .workspace
        .clone()
        .or_else(|| std::env::var_os("CRABDB_WORKSPACE").map(PathBuf::from));
    let db_dir = cli
        .db
        .clone()
        .or_else(|| std::env::var_os("CRABDB_DIR").map(PathBuf::from));
    let branch = cli
        .branch
        .clone()
        .or_else(|| std::env::var("CRABDB_BRANCH").ok());
    let ctx = RuntimeContext {
        workspace,
        db_dir,
        branch,
        json,
        quiet: cli.quiet,
        format,
    };
    match cli.command {
        Command::Init(args) => {
            let workspace = ctx
                .workspace
                .clone()
                .unwrap_or(std::env::current_dir().map_err(Error::from)?);
            let mode = if args.from_git {
                InitImportMode::GitTracked
            } else if args.working_tree {
                InitImportMode::WorkingTree
            } else {
                InitImportMode::Empty
            };
            let report = CrabDb::init_with_text_policy(
                workspace,
                args.branch,
                mode,
                args.force,
                args.text_policy.as_ref().map(TextPolicyArg::as_str),
            )?;
            render_init(&report, ctx.json, ctx.quiet)
        }
        Command::Config(config) => match config.command {
            ConfigSubcommand::List => {
                let db = open_db(&ctx)?;
                let entries = db.config_entries();
                render_config_list(&entries, ctx.json, ctx.quiet)
            }
            ConfigSubcommand::Get(args) => {
                let db = open_db(&ctx)?;
                let entry = db.config_get(&args.key)?;
                render_config_entry(&entry, ctx.json, ctx.quiet)
            }
            ConfigSubcommand::Set(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.config_set(&args.key, &args.value)?;
                render_config_set(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Ignore(ignore) => match ignore.command {
            IgnoreSubcommand::List => {
                let db = open_db(&ctx)?;
                let report = db.ignore_list()?;
                render_ignore_list(&report, ctx.json, ctx.quiet)
            }
            IgnoreSubcommand::Add(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.ignore_add(&args.pattern)?;
                render_ignore_add(&report, ctx.json, ctx.quiet)
            }
            IgnoreSubcommand::Remove(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.ignore_remove(&args.pattern)?;
                render_ignore_remove(&report, ctx.json, ctx.quiet)
            }
            IgnoreSubcommand::Check(args) => {
                let db = open_db(&ctx)?;
                let report = db.ignore_check(&args.path)?;
                render_ignore_check(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Guardrails(guardrails) => match guardrails.command {
            GuardrailsSubcommand::Check(args) => {
                let db = open_db(&ctx)?;
                let payload = parse_optional_json(args.payload_json.as_deref())?;
                let report = db.guardrail_check(
                    args.agent.as_deref(),
                    &args.action,
                    args.summary.as_deref(),
                    payload,
                    &args.paths,
                )?;
                render_guardrail_check(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Status(args) => {
            let db = open_db(&ctx)?;
            let branch = args.branch.as_deref().or(ctx.branch.as_deref());
            let report = db.status(branch)?;
            render_status(&report, ctx.json, ctx.quiet)
        }
        Command::Record(args) => {
            let mut db = open_db(&ctx)?;
            let kind = args
                .kind
                .as_deref()
                .map(parse_record_kind_arg)
                .transpose()?;
            let report = db.record_with_options(
                ctx.branch.as_deref(),
                args.message,
                Actor::human(),
                RecordOptions {
                    paths: args.paths,
                    kind,
                    session_id: args.session,
                    allow_ignored: args.allow_ignored,
                },
            )?;
            render_record(&report, ctx.json, ctx.quiet)
        }
        Command::Watch(args) => {
            let mut db = open_db(&ctx)?;
            let interval = watch_interval(args.interval_secs, args.debounce_ms)?;
            let _include_untracked = args.include_untracked;
            loop {
                let report = db.record_with_options(
                    ctx.branch.as_deref(),
                    args.message.clone(),
                    Actor::human(),
                    RecordOptions {
                        kind: Some(OperationKind::WatchRecord),
                        session_id: args.session.clone(),
                        ..RecordOptions::default()
                    },
                )?;
                if matches!(ctx.format, OutputFormat::Ndjson) {
                    println!("{}", serde_json::to_string(&report)?);
                } else if report.operation.is_some() {
                    render_record(&report, ctx.json, ctx.quiet)?;
                }
                if args.once {
                    break;
                }
                thread::sleep(interval);
            }
            Ok(())
        }
        Command::Timeline(args) => {
            let db = open_db(&ctx)?;
            let branch = args.branch.as_deref().or(ctx.branch.as_deref());
            let entries = db.timeline_query(
                branch,
                args.session.as_deref(),
                args.agent.as_deref(),
                args.limit,
            )?;
            render_timeline(&entries, ctx.json, ctx.quiet)
        }
        Command::Show(args) => {
            let db = open_db(&ctx)?;
            let result = db.show(&args.selector)?;
            render_show(&result, ctx.json, ctx.quiet)
        }
        Command::Object(object) => match object.command {
            ObjectSubcommand::Show(args) => {
                let db = open_db(&ctx)?;
                let report = db.inspect_object(&args.object_id)?;
                render_object_inspect(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Root(root) => match root.command {
            RootSubcommand::Show(args) => {
                let db = open_db(&ctx)?;
                let report = db.inspect_root(&args.root_id)?;
                render_root_inspect(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Text(text) => match text.command {
            TextSubcommand::Show(args) => {
                let db = open_db(&ctx)?;
                let report = db.inspect_text(&args.text_id, args.limit)?;
                render_text_inspect(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Map(map) => match map.command {
            MapSubcommand::Range(args) => {
                let db = open_db(&ctx)?;
                let report = db.inspect_map_range(
                    &args.map_id,
                    args.map_type.as_str(),
                    args.start.as_deref(),
                    args.end.as_deref(),
                    args.limit,
                )?;
                render_map_range(&report, ctx.json, ctx.quiet)
            }
            MapSubcommand::Diff(args) => {
                let db = open_db(&ctx)?;
                let report = db.inspect_map_diff(
                    &args.left_map_id,
                    &args.right_map_id,
                    args.map_type.as_str(),
                    args.start.as_deref(),
                    args.end.as_deref(),
                    args.limit,
                )?;
                render_map_diff(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Diff(args) => {
            let mut db = open_db(&ctx)?;
            let summary = diff_from_args(&mut db, &args)?;
            render_diff(&summary, ctx.json, ctx.quiet, args.stat)
        }
        Command::Checkout(args) => {
            let mut db = open_db(&ctx)?;
            let report = db.checkout_with_options(
                &args.target,
                args.force,
                args.dry_run,
                args.workdir.as_deref(),
                args.record_dirty,
            )?;
            render_checkout(&report, ctx.json, ctx.quiet)
        }
        Command::Branch(args) => {
            let mut db = open_db(&ctx)?;
            match (
                args.name.as_deref(),
                args.delete.as_deref(),
                args.rename.as_deref(),
                args.to.as_deref(),
            ) {
                (Some(name), None, None, None) => {
                    let report = db.create_branch(name, args.from.as_deref())?;
                    render_branch(&report, ctx.json, ctx.quiet)
                }
                (None, Some(name), None, None) => {
                    let report = db.delete_branch(name)?;
                    render_branch_delete(&report, ctx.json, ctx.quiet)
                }
                (None, None, Some(old_name), Some(new_name)) => {
                    let report = db.rename_branch(old_name, new_name)?;
                    render_branch_rename(&report, ctx.json, ctx.quiet)
                }
                (None, None, None, None) => {
                    let entries = db.list_branches()?;
                    render_branch_list(&entries, ctx.json, ctx.quiet)
                }
                _ => Err(Error::InvalidInput(
                    "branch accepts either NAME [--from REF], --delete NAME, --rename OLD --to NEW, or no arguments".to_string(),
                )),
            }
        }
        Command::Merge(args) => {
            let mut db = open_db(&ctx)?;
            validate_merge_strategy(args.strategy.as_deref())?;
            let report = db.merge_branches_with_options(&args.source, &args.into, args.dry_run)?;
            render_merge(&report, ctx.json, ctx.quiet)
        }
        Command::Why(args) => {
            let db = open_db(&ctx)?;
            let at = args.at.as_deref().or(ctx.branch.as_deref());
            let result = match (args.path_line.as_deref(), args.line_id.as_deref()) {
                (Some(path_line), None) => db.why(path_line, at)?,
                (None, Some(line_id)) => db.why_line_id(line_id, at)?,
                (Some(_), Some(_)) => {
                    return Err(Error::InvalidInput(
                        "why accepts either PATH:LINE or --line-id, not both".to_string(),
                    ));
                }
                (None, None) => {
                    return Err(Error::InvalidInput(
                        "why requires PATH:LINE or --line-id".to_string(),
                    ));
                }
            };
            render_why(&result, ctx.json, ctx.quiet)
        }
        Command::History(args) => {
            let db = open_db(&ctx)?;
            let result = match (
                args.selector.as_deref(),
                args.file_id.as_deref(),
                args.line_id.as_deref(),
            ) {
                (Some(_), Some(_), _) | (Some(_), _, Some(_)) | (_, Some(_), Some(_)) => {
                    return Err(Error::InvalidInput(
                        "history accepts one path, --file-id, or --line-id selector".to_string(),
                    ));
                }
                (_, Some(file_id), None) => db.history_for_file_id(file_id)?,
                (_, None, Some(line_id)) => db.history_for_line_id(line_id)?,
                (Some(path), None, None) => db.history_for_path(path)?,
                (None, None, None) => {
                    return Err(Error::InvalidInput(
                        "history requires a path, --file-id, or --line-id".to_string(),
                    ));
                }
            };
            render_history(&result, ctx.json, ctx.quiet)
        }
        Command::CodeFrom(args) => {
            let db = open_db(&ctx)?;
            let result = db.code_from(&args.selector)?;
            render_code_from(&result, ctx.json, ctx.quiet)
        }
        Command::Agent(agent) => match agent.command {
            AgentSubcommand::Spawn(args) => {
                let mut db = open_db(&ctx)?;
                let materialize = args.materialize && !args.no_materialize;
                let report = db.spawn_agent_with_workdir(
                    &args.name,
                    args.from.as_deref(),
                    materialize,
                    args.provider,
                    args.model,
                    args.workdir,
                )?;
                render_agent_spawn(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::List => {
                let db = open_db(&ctx)?;
                let agents = db.list_agents()?;
                render_agent_list(&agents, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Show(args) => {
                let db = open_db(&ctx)?;
                let details = db.agent_details(&args.name)?;
                render_agent_details(&details, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Status(args) => {
                let db = open_db(&ctx)?;
                let report = db.agent_status(&args.name)?;
                render_agent_status(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Contribution(args) => {
                let db = open_db(&ctx)?;
                let report = db.agent_contribution(&args.name, args.limit)?;
                render_agent_contribution(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Gates(args) => {
                let db = open_db(&ctx)?;
                let report = db.agent_gate_history(&args.name, args.kind.as_deref(), args.limit)?;
                render_agent_gate_history(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Readiness(args) => {
                let db = open_db(&ctx)?;
                let report = db.agent_readiness(&args.name)?;
                render_agent_readiness(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Handoff(args) => {
                let db = open_db(&ctx)?;
                let report = db.agent_handoff(&args.name, args.limit)?;
                render_agent_handoff(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Claim(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.claim_agent_path(&args.name, &args.path, args.ttl_secs)?;
                render_agent_claim(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Message(args) => {
                let mut db = open_db(&ctx)?;
                let report =
                    db.add_agent_message(&args.name, &args.role, &args.text, args.session)?;
                render_agent_message(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Turn(turn) => match turn.command {
                AgentTurnSubcommand::Start(args) => {
                    let mut db = open_db(&ctx)?;
                    let report = db.begin_agent_turn(
                        &args.name,
                        args.from.as_deref(),
                        args.title,
                        args.base_change.as_deref(),
                    )?;
                    render_agent_turn_start(&report, ctx.json, ctx.quiet)
                }
                AgentTurnSubcommand::Show(args) => {
                    let db = open_db(&ctx)?;
                    let details = db.show_agent_turn(&args.turn_id)?;
                    render_agent_turn_details(&details, ctx.json, ctx.quiet)
                }
                AgentTurnSubcommand::Message(args) => {
                    let mut db = open_db(&ctx)?;
                    let report =
                        db.add_agent_turn_message(&args.turn_id, &args.role, &args.text)?;
                    render_agent_message(&report, ctx.json, ctx.quiet)
                }
                AgentTurnSubcommand::Event(args) => {
                    let mut db = open_db(&ctx)?;
                    let payload = parse_optional_json(args.payload_json.as_deref())?;
                    let report = db.add_agent_turn_event(
                        &args.turn_id,
                        &args.event_type,
                        payload,
                        args.change.as_deref(),
                        args.message.as_deref(),
                    )?;
                    render_agent_turn_event(&report, ctx.json, ctx.quiet)
                }
                AgentTurnSubcommand::ApplyPatch(args) => {
                    let mut db = open_db(&ctx)?;
                    let mut patch: PatchDocument =
                        serde_json::from_slice(&std::fs::read(&args.patch).map_err(Error::from)?)?;
                    if args.allow_ignored {
                        patch.allow_ignored = true;
                    }
                    let report = db.apply_agent_turn_patch(&args.turn_id, patch)?;
                    render_agent_patch(&report, ctx.json, ctx.quiet)
                }
                AgentTurnSubcommand::End(args) => {
                    let mut db = open_db(&ctx)?;
                    let report = db.end_agent_turn(&args.turn_id, &args.status)?;
                    render_agent_turn_end(&report, ctx.json, ctx.quiet)
                }
            },
            AgentSubcommand::Run(run) => match run.command {
                AgentRunSubcommand::Pause(args) => {
                    let mut db = open_db(&ctx)?;
                    let state = parse_optional_json(args.state_json.as_deref())?;
                    let interruption = parse_optional_json(args.interruption_json.as_deref())?;
                    let report = db.pause_agent_run(
                        &args.name,
                        &args.reason,
                        &args.summary,
                        state,
                        interruption,
                        args.session.as_deref(),
                        args.turn.as_deref(),
                    )?;
                    render_agent_run_pause(&report, ctx.json, ctx.quiet)
                }
                AgentRunSubcommand::List(args) => {
                    let db = open_db(&ctx)?;
                    let run_states =
                        db.list_agent_run_states(args.agent.as_deref(), args.status.as_deref())?;
                    render_agent_run_list(&run_states, ctx.json, ctx.quiet)
                }
                AgentRunSubcommand::Show(args) => {
                    let db = open_db(&ctx)?;
                    let run_state = db.show_agent_run_state(&args.run_id)?;
                    render_agent_run_state(&run_state, ctx.json, ctx.quiet)
                }
                AgentRunSubcommand::Resume(args) => {
                    let mut db = open_db(&ctx)?;
                    let report = db.resume_agent_run(&args.run_id, args.reviewer, args.note)?;
                    render_agent_run_resume(&report, ctx.json, ctx.quiet)
                }
            },
            AgentSubcommand::Events(args) => {
                let db = open_db(&ctx)?;
                let events = db.list_agent_events(
                    args.agent.as_deref(),
                    args.session.as_deref(),
                    args.turn.as_deref(),
                    args.event_type.as_deref(),
                    args.limit,
                )?;
                render_agent_events(&events, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Trace(trace) => match trace.command {
                AgentTraceSubcommand::Start(args) => {
                    let mut db = open_db(&ctx)?;
                    let attributes = parse_optional_json(args.attributes_json.as_deref())?;
                    let report = db.start_agent_trace_span(
                        &args.turn_id,
                        &args.span_type,
                        &args.name,
                        args.parent.as_deref(),
                        args.trace_id.as_deref(),
                        attributes,
                    )?;
                    render_agent_trace_span_start(&report, ctx.json, ctx.quiet)
                }
                AgentTraceSubcommand::End(args) => {
                    let mut db = open_db(&ctx)?;
                    let result = parse_optional_json(args.result_json.as_deref())?;
                    let report = db.end_agent_trace_span(&args.span_id, &args.status, result)?;
                    render_agent_trace_span_end(&report, ctx.json, ctx.quiet)
                }
                AgentTraceSubcommand::List(args) => {
                    let db = open_db(&ctx)?;
                    let spans = db.list_agent_trace_spans(
                        args.agent.as_deref(),
                        args.session.as_deref(),
                        args.turn.as_deref(),
                        args.trace_id.as_deref(),
                        args.limit,
                    )?;
                    render_agent_trace_spans(&spans, ctx.json, ctx.quiet)
                }
                AgentTraceSubcommand::Summary(args) => {
                    let db = open_db(&ctx)?;
                    let report = db.summarize_agent_trace_spans(
                        args.agent.as_deref(),
                        args.session.as_deref(),
                        args.turn.as_deref(),
                        args.trace_id.as_deref(),
                        args.slowest_limit,
                    )?;
                    render_agent_trace_summary(&report, ctx.json, ctx.quiet)
                }
                AgentTraceSubcommand::Show(args) => {
                    let db = open_db(&ctx)?;
                    let span = db.show_agent_trace_span(&args.span_id)?;
                    render_agent_trace_span(&span, ctx.json, ctx.quiet)
                }
            },
            AgentSubcommand::Record(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.record_agent_workdir(&args.name, args.message)?;
                render_agent_record(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Watch(args) => {
                let mut db = open_db(&ctx)?;
                let interval = watch_interval(args.interval_secs, args.debounce_ms)?;
                let _include_untracked = args.include_untracked;
                if args.once {
                    let report =
                        db.watch_agent_workdir(&args.name, args.message, interval, Some(1))?;
                    render_agent_watch(&report, ctx.json, ctx.quiet)
                } else {
                    loop {
                        let report = db.record_agent_workdir(&args.name, args.message.clone())?;
                        if matches!(ctx.format, OutputFormat::Ndjson) {
                            println!("{}", serde_json::to_string(&report)?);
                        } else if report.operation.is_some() {
                            render_agent_record(&report, ctx.json, ctx.quiet)?;
                        }
                        thread::sleep(interval);
                    }
                }
            }
            AgentSubcommand::Test(args) => {
                let mut db = open_db(&ctx)?;
                let options = AgentGateOptions {
                    suite: args.suite,
                    score: args.score,
                    threshold: args.threshold,
                };
                let report = db.run_agent_test_with_options(
                    &args.name,
                    args.command,
                    args.turn.as_deref(),
                    args.timeout_secs,
                    options,
                )?;
                let render_result = render_agent_test(&report, ctx.json, ctx.quiet);
                if render_result.is_ok() && !report.success {
                    std::process::exit(command_failure_exit_code(report.exit_code));
                }
                render_result
            }
            AgentSubcommand::Eval(args) => {
                let mut db = open_db(&ctx)?;
                let options = AgentGateOptions {
                    suite: args.suite,
                    score: args.score,
                    threshold: args.threshold,
                };
                let report = db.run_agent_eval_with_options(
                    &args.name,
                    args.command,
                    args.turn.as_deref(),
                    args.timeout_secs,
                    options,
                )?;
                let render_result = render_agent_test(&report, ctx.json, ctx.quiet);
                if render_result.is_ok() && !report.success {
                    std::process::exit(command_failure_exit_code(report.exit_code));
                }
                render_result
            }
            AgentSubcommand::Workdir(args) => {
                let db = open_db(&ctx)?;
                let report = db.agent_workdir(&args.name)?;
                render_agent_workdir(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::SyncWorkdir(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.sync_agent_workdir(&args.name, args.force)?;
                render_agent_workdir_sync(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::ApplyPatch(args) => {
                let mut db = open_db(&ctx)?;
                let mut patch: PatchDocument =
                    serde_json::from_slice(&std::fs::read(&args.patch).map_err(Error::from)?)?;
                if args.allow_ignored {
                    patch.allow_ignored = true;
                }
                let report = db.apply_agent_patch(&args.name, patch)?;
                render_agent_patch(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Diff(args) => {
                let db = open_db(&ctx)?;
                let summary =
                    db.diff_agent_with_options(&args.name, args.patch, args.show_line_ids)?;
                render_diff(&summary, ctx.json, ctx.quiet, false)
            }
            AgentSubcommand::Timeline(args) => {
                let db = open_db(&ctx)?;
                let entries = db.agent_timeline(&args.name, args.limit)?;
                render_timeline(&entries, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Checkout(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.checkout_agent_with_options(
                    &args.name,
                    args.force,
                    args.dry_run,
                    args.workdir.as_deref(),
                )?;
                render_checkout(&report, ctx.json, ctx.quiet)
            }
            AgentSubcommand::Rm(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.remove_agent(&args.name, args.force)?;
                render_agent_remove(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Session(session) => match session.command {
            SessionSubcommand::Start(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.start_agent_session(&args.agent, args.title, args.id)?;
                render_session_start(&report, ctx.json, ctx.quiet)
            }
            SessionSubcommand::Current(args) => {
                let db = open_db(&ctx)?;
                let reports = db.current_agent_sessions(args.agent.as_deref())?;
                render_session_current(&reports, ctx.json, ctx.quiet)
            }
            SessionSubcommand::List(args) => {
                let db = open_db(&ctx)?;
                let sessions = db.list_agent_sessions(args.agent.as_deref())?;
                render_session_list(&sessions, ctx.json, ctx.quiet)
            }
            SessionSubcommand::Show(args) => {
                let db = open_db(&ctx)?;
                let details = db.show_agent_session(&args.session_id)?;
                render_session_details(&details, ctx.json, ctx.quiet)
            }
            SessionSubcommand::Context(args) => {
                let db = open_db(&ctx)?;
                let report = db.agent_session_context(&args.session_id, args.limit)?;
                render_session_context(&report, ctx.json, ctx.quiet)
            }
            SessionSubcommand::End(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.end_agent_session(&args.session_id, &args.status)?;
                render_session_end(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Approvals(approvals) => match approvals.command {
            ApprovalsSubcommand::Request(args) => {
                let mut db = open_db(&ctx)?;
                let payload = args
                    .payload_json
                    .map(|payload| serde_json::from_str::<serde_json::Value>(&payload))
                    .transpose()?;
                let report = db.request_agent_approval(
                    &args.agent,
                    &args.action,
                    &args.summary,
                    payload,
                    args.session.as_deref(),
                    args.turn.as_deref(),
                )?;
                render_approval_request(&report, ctx.json, ctx.quiet)
            }
            ApprovalsSubcommand::List(args) => {
                let db = open_db(&ctx)?;
                let approvals =
                    db.list_agent_approvals(args.agent.as_deref(), args.status.as_deref())?;
                render_approval_list(&approvals, ctx.json, ctx.quiet)
            }
            ApprovalsSubcommand::Show(args) => {
                let db = open_db(&ctx)?;
                let approval = db.show_agent_approval(&args.approval_id)?;
                render_approval(&approval, ctx.json, ctx.quiet)
            }
            ApprovalsSubcommand::Decide(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.decide_agent_approval(
                    &args.approval_id,
                    args.decision.as_str(),
                    args.reviewer,
                    args.note,
                )?;
                render_approval_decision(&report, ctx.json, ctx.quiet)
            }
        },
        Command::MergeAgent(args) => {
            let mut db = open_db(&ctx)?;
            validate_merge_strategy(args.strategy.as_deref())?;
            let report = db.merge_agent_with_options(&args.name, &args.into, args.dry_run)?;
            render_merge(&report, ctx.json, ctx.quiet)
        }
        Command::MergeQueue(queue) => match queue.command {
            MergeQueueSubcommand::Add(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.enqueue_merge(&args.source, &args.into, args.priority)?;
                render_merge_queue_add(&report, ctx.json, ctx.quiet)
            }
            MergeQueueSubcommand::List => {
                let db = open_db(&ctx)?;
                let entries = db.list_merge_queue()?;
                render_merge_queue_list(&entries, ctx.json, ctx.quiet)
            }
            MergeQueueSubcommand::Run(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.run_merge_queue(args.limit)?;
                render_merge_queue_run(&report, ctx.json, ctx.quiet)
            }
            MergeQueueSubcommand::Remove(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.remove_merge_queue(&args.selector)?;
                render_merge_queue_remove(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Conflicts(conflicts) => match conflicts.command {
            ConflictsSubcommand::List => {
                let db = open_db(&ctx)?;
                let conflicts = db.list_conflicts()?;
                render_conflicts(&conflicts, ctx.json, ctx.quiet)
            }
            ConflictsSubcommand::Show(args) => {
                let db = open_db(&ctx)?;
                let conflict = db.show_conflict(&args.conflict_set_id)?;
                render_conflict(&conflict, ctx.json, ctx.quiet)
            }
            ConflictsSubcommand::Resolve(args) => {
                let mut db = open_db(&ctx)?;
                let report = if let Some(manual_path) = args.manual {
                    let manual = read_manual_conflict_resolution(&manual_path)?;
                    db.resolve_conflict_manual(&args.conflict_set_id, manual)?
                } else if let Some(take) = args.take {
                    db.resolve_conflict(&args.conflict_set_id, take.as_str())?
                } else {
                    return Err(Error::InvalidInput(
                        "conflicts resolve requires `--take` or `--manual`".to_string(),
                    ));
                };
                render_conflict_resolve(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Anchor(anchor) => match anchor.command {
            AnchorSubcommand::Create(args) => {
                let mut db = open_db(&ctx)?;
                let report =
                    db.create_anchor(&args.path_line, args.label, ctx.branch.as_deref())?;
                render_anchor_create(&report, ctx.json, ctx.quiet)
            }
            AnchorSubcommand::Resolve(args) => {
                let db = open_db(&ctx)?;
                let report = db.resolve_anchor(&args.anchor_id, ctx.branch.as_deref())?;
                render_anchor_resolve(&report, ctx.json, ctx.quiet)
            }
            AnchorSubcommand::List => {
                let db = open_db(&ctx)?;
                let anchors = db.list_anchors()?;
                render_anchor_list(&anchors, ctx.json, ctx.quiet)
            }
            AnchorSubcommand::Delete(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.delete_anchor(&args.anchor_id)?;
                render_anchor_delete(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Lease(lease) => match lease.command {
            LeaseSubcommand::Acquire(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.acquire_lease(
                    &args.agent,
                    Some(&args.path),
                    args.mode.as_str(),
                    args.ttl_secs,
                )?;
                render_lease_acquire(&report, ctx.json, ctx.quiet)
            }
            LeaseSubcommand::List(args) => {
                let db = open_db(&ctx)?;
                let leases = db.list_leases(args.all)?;
                render_lease_list(&leases, ctx.json, ctx.quiet)
            }
            LeaseSubcommand::Release(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.release_lease(&args.lease_id)?;
                render_lease_release(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Git(git) => match git.command {
            GitSubcommand::Export(args) => {
                if let Some(message) = args.message {
                    if args.output.is_some() {
                        return Err(Error::InvalidInput(
                            "git export -m cannot be combined with --output".to_string(),
                        ));
                    }
                    let mut db = open_db(&ctx)?;
                    let report = db.git_export_commit(&args.range, &message)?;
                    render_git_export(&report, ctx.json, ctx.quiet)
                } else if let Some(output) = args.output {
                    let db = open_db(&ctx)?;
                    db.write_patch_to(&args.range, &output)?;
                    if !ctx.quiet {
                        println!("Wrote patch: {}", output.display());
                    }
                    Ok(())
                } else {
                    let db = open_db(&ctx)?;
                    let patch = db.export_patch(&args.range)?;
                    print!("{patch}");
                    Ok(())
                }
            }
            GitSubcommand::ImportUpdate(args) => {
                let mut db = open_db(&ctx)?;
                let report = db.git_import_update(ctx.branch.as_deref(), args.message)?;
                render_git_import_update(&report, ctx.json, ctx.quiet)
            }
            GitSubcommand::Mappings(args) => {
                let db = open_db(&ctx)?;
                let mappings = db.git_mappings(args.limit)?;
                render_git_mappings(&mappings, ctx.json, ctx.quiet)
            }
        },
        Command::Api(api) => match api.command {
            ApiSubcommand::Openapi(args) => {
                let spec = crabdb::server::openapi_spec();
                let json = serde_json::to_string_pretty(&spec)?;
                if let Some(output) = args.output {
                    fs::write(output, json)?;
                } else {
                    println!("{json}");
                }
                Ok(())
            }
        },
        Command::Daemon(args) => {
            let mut db = open_db(&ctx)?;
            let (auth, token_file) = daemon_auth(&db, &args)?;
            let addr: SocketAddr = format!("{}:{}", args.host, args.port)
                .parse()
                .map_err(|err| Error::InvalidInput(format!("invalid listen address: {err}")))?;
            let listener = TcpListener::bind(addr)?;
            let local_addr = listener.local_addr()?;
            if !ctx.quiet {
                println!("CrabDB API listening on http://{local_addr}");
                if args.no_auth {
                    println!("Daemon auth disabled");
                } else if let Some(path) = token_file {
                    println!("Daemon token file: {}", path.display());
                    println!(
                        "Send requests with: Authorization: Bearer $(cat {})",
                        path.display()
                    );
                } else {
                    println!("Daemon token configured from flag or CRABDB_DAEMON_TOKEN");
                }
            }
            let max_requests = if args.once {
                Some(1)
            } else {
                args.max_requests
            };
            crabdb::server::serve_listener_with_auth(&mut db, listener, max_requests, auth)
        }
        Command::Mcp => {
            let mut db = open_db(&ctx)?;
            let stdin = std::io::stdin();
            let mut stdout = std::io::stdout();
            crabdb::mcp::serve_stdio(&mut db, stdin.lock(), &mut stdout)
        }
        Command::Doctor => {
            let db = open_db(&ctx)?;
            let report = db.doctor()?;
            render_doctor(&report, ctx.json, ctx.quiet)
        }
        Command::Backup(backup) => match backup.command {
            BackupSubcommand::Create(args) => {
                let db = open_db(&ctx)?;
                let report = db.create_backup(args.output, args.overwrite)?;
                render_backup_create(&report, ctx.json, ctx.quiet)
            }
            BackupSubcommand::Verify(args) => {
                let report = CrabDb::verify_backup(args.path)?;
                render_backup_verify(&report, ctx.json, ctx.quiet)
            }
            BackupSubcommand::Restore(args) => {
                let workspace = ctx
                    .workspace
                    .clone()
                    .unwrap_or(std::env::current_dir().map_err(Error::from)?);
                let report = CrabDb::restore_backup(workspace, args.path, args.force)?;
                render_backup_restore(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Fsck => {
            let db = open_db(&ctx)?;
            let report = db.fsck()?;
            render_fsck(&report, ctx.json, ctx.quiet)
        }
        Command::Index(index) => match index.command {
            IndexSubcommand::Rebuild => {
                let mut db = open_db(&ctx)?;
                let report = db.rebuild_indexes()?;
                render_index_rebuild(&report, ctx.json, ctx.quiet)
            }
        },
        Command::Gc(args) => {
            let mut db = open_db(&ctx)?;
            let report = db.gc(args.dry_run)?;
            render_gc(&report, ctx.json, ctx.quiet)
        }
    }
}

struct RuntimeContext {
    workspace: Option<PathBuf>,
    db_dir: Option<PathBuf>,
    branch: Option<String>,
    json: bool,
    quiet: bool,
    format: OutputFormat,
}

fn open_db(ctx: &RuntimeContext) -> Result<CrabDb> {
    match (&ctx.workspace, &ctx.db_dir) {
        (Some(workspace), Some(db_dir)) => CrabDb::open_with_db_dir(workspace, db_dir),
        (Some(workspace), None) => CrabDb::open(workspace),
        (None, Some(db_dir)) => {
            let workspace = db_dir
                .parent()
                .ok_or_else(|| Error::WorkspaceNotFound(db_dir.clone()))?;
            CrabDb::open_with_db_dir(workspace, db_dir)
        }
        (None, None) => CrabDb::discover(std::env::current_dir().map_err(Error::from)?),
    }
}

fn daemon_auth(
    db: &CrabDb,
    args: &DaemonArgs,
) -> Result<(crabdb::server::ServerAuth, Option<PathBuf>)> {
    if args.no_auth {
        if args.auth_token.is_some() || args.auth_token_file.is_some() {
            return Err(Error::InvalidInput(
                "--no-auth cannot be combined with --auth-token or --auth-token-file".to_string(),
            ));
        }
        return Ok((crabdb::server::ServerAuth::disabled(), None));
    }

    if let Some(token) = &args.auth_token {
        return Ok((crabdb::server::ServerAuth::bearer(token.clone())?, None));
    }
    if let Ok(token) = std::env::var("CRABDB_DAEMON_TOKEN") {
        return Ok((crabdb::server::ServerAuth::bearer(token)?, None));
    }

    let token_path = args
        .auth_token_file
        .clone()
        .unwrap_or_else(|| db.db_dir().join("daemon.token"));
    let token = if token_path.exists() {
        fs::read_to_string(&token_path)?.trim().to_string()
    } else {
        let token = generate_daemon_token()?;
        fs::write(&token_path, format!("{token}\n"))?;
        restrict_secret_file(&token_path)?;
        token
    };
    Ok((crabdb::server::ServerAuth::bearer(token)?, Some(token_path)))
}

fn generate_daemon_token() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|err| {
        Error::InvalidInput(format!("failed to generate daemon auth token: {err}"))
    })?;
    Ok(hex::encode(bytes))
}

fn restrict_secret_file(path: &std::path::Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(path, permissions)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn parse_optional_json(value: Option<&str>) -> Result<Option<serde_json::Value>> {
    value
        .map(serde_json::from_str)
        .transpose()
        .map_err(Error::from)
}

fn read_manual_conflict_resolution(path: &PathBuf) -> Result<ConflictManualResolution> {
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    if value.get("files").is_some() {
        return serde_json::from_value(value).map_err(Error::from);
    }
    let files: BTreeMap<String, ConflictManualFile> =
        serde_json::from_value(value).map_err(Error::from)?;
    Ok(ConflictManualResolution { files })
}

fn parse_record_kind_arg(value: &str) -> Result<OperationKind> {
    match value {
        "file-edit" => Ok(OperationKind::FileEdit),
        "multi-file-edit" => Ok(OperationKind::MultiFileEdit),
        "format" => Ok(OperationKind::Format),
        "manual-checkpoint" => Ok(OperationKind::ManualCheckpoint),
        "manual-record" => Ok(OperationKind::ManualRecord),
        other => Err(Error::InvalidInput(format!(
            "record kind must be file-edit, multi-file-edit, format, manual-checkpoint, or manual-record, got `{other}`"
        ))),
    }
}

fn validate_merge_strategy(value: Option<&str>) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    match value {
        "conservative" | "line-id-aware" | "line_id_aware" => Ok(()),
        other => Err(Error::InvalidInput(format!(
            "merge strategy must be conservative, line-id-aware, or line_id_aware, got `{other}`"
        ))),
    }
}

fn command_failure_exit_code(exit_code: Option<i32>) -> i32 {
    exit_code
        .filter(|code| *code != 0)
        .unwrap_or(1)
        .clamp(1, 255)
}

fn render_json<T: serde::Serialize + ?Sized>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn render_init(report: &InitReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Initialized CrabDB workspace");
        println!("Workspace: {}", report.workspace_id.0);
        println!("Branch: {}", report.branch);
        println!("Initial operation: {}", report.operation.0);
        println!(
            "Imported: {} files ({} text, {} opaque, {} binary)",
            report.imported.files,
            report.imported.text,
            report.imported.opaque,
            report.imported.binary
        );
    }
    Ok(())
}

fn render_config_list(entries: &[ConfigEntry], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if !quiet {
        for entry in entries {
            let read_only = if entry.read_only { " (read-only)" } else { "" };
            println!(
                "{} = {} [{}]{}",
                entry.key, entry.value, entry.value_type, read_only
            );
        }
    }
    Ok(())
}

fn render_config_entry(entry: &ConfigEntry, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(entry);
    }
    if !quiet {
        println!("{}", entry.value);
    }
    Ok(())
}

fn render_config_set(report: &ConfigSetReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "{}: {} -> {}",
            report.key, report.old_value, report.new_value
        );
    }
    Ok(())
}

fn render_ignore_list(report: &IgnoreListReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Ignore file: {}", report.path);
        if report.patterns.is_empty() {
            println!("No ignore patterns");
        } else {
            for pattern in &report.patterns {
                println!("{}: {}", pattern.line, pattern.pattern);
            }
        }
    }
    Ok(())
}

fn render_ignore_add(report: &IgnoreAddReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.added {
            println!("Added ignore pattern: {}", report.pattern);
        } else {
            println!("Ignore pattern already present: {}", report.pattern);
        }
    }
    Ok(())
}

fn render_ignore_remove(report: &IgnoreRemoveReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.removed {
            println!("Removed ignore pattern: {}", report.pattern);
        } else {
            println!("Ignore pattern not present: {}", report.pattern);
        }
    }
    Ok(())
}

fn render_ignore_check(report: &IgnoreCheckReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        match (&report.ignored, &report.source) {
            (true, Some(source)) => println!("{}: ignored ({})", report.path, source),
            (true, None) => println!("{}: ignored", report.path),
            (false, _) => println!("{}: not ignored", report.path),
        }
    }
    Ok(())
}

fn render_guardrail_check(report: &GuardrailCheckReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Guardrail decision: {}", report.decision);
        println!("Action: {}", report.action);
        if let Some(agent) = &report.agent {
            println!("Agent: {}", agent.record.name);
        }
        if !report.reasons.is_empty() {
            println!("Reasons:");
            for reason in &report.reasons {
                println!(
                    "  {} [{}]: {}",
                    reason.code, reason.severity, reason.message
                );
            }
        }
        if !report.path_checks.is_empty() {
            println!("Paths:");
            for check in &report.path_checks {
                let status = if check.ignored { "ignored" } else { "allowed" };
                match &check.source {
                    Some(source) => println!("  {}: {} ({})", check.path, status, source),
                    None => println!("  {}: {}", check.path, status),
                }
            }
        }
        if let Some(approval) = &report.approval_request {
            println!("Approval suggested: {}", approval.summary);
        }
        if !report.satisfied_approvals.is_empty() {
            println!("Satisfied approvals:");
            for approval in &report.satisfied_approvals {
                println!("  {} {}", approval.approval_id, approval.action);
            }
        }
    }
    Ok(())
}

fn render_status(report: &StatusReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Branch: {}", report.branch);
        println!("Head: {}", report.head.change_id.0);
        println!("Root: {}", report.head.root_id.0);
        println!(
            "Worktree: {}",
            match report.worktree_state {
                WorktreeState::Clean => "clean",
                WorktreeState::DirtyTracked => "dirty",
                WorktreeState::DirtyUntracked => "dirty with untracked paths",
            }
        );
        for path in &report.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
    }
    Ok(())
}

fn render_record(report: &RecordReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        match &report.operation {
            Some(change) => {
                println!("Recorded {}", change.0);
                for path in &report.changed_paths {
                    println!("  {:?} {}", path.kind, path.path);
                }
            }
            None => println!("No changes to record"),
        }
    }
    Ok(())
}

fn render_git_import_update(report: &GitImportReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        match &report.operation {
            Some(change) => {
                println!("Imported Git update {}", change.0);
                println!(
                    "Imported: {} files ({} text, {} opaque, {} binary)",
                    report.imported.files,
                    report.imported.text,
                    report.imported.opaque,
                    report.imported.binary
                );
                for path in &report.changed_paths {
                    println!("  {:?} {}", path.kind, path.path);
                }
            }
            None => println!("No Git-tracked changes to import"),
        }
    }
    Ok(())
}

fn render_git_export(report: &GitExportReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Created Git commit: {}", report.commit);
        println!("Range: {}", report.range);
        println!("CrabDB operation: {}", report.operation.0);
        println!("Root: {}", report.root_id.0);
        if let Some(parent) = &report.parent {
            println!("Parent: {parent}");
        }
        if let Some(mapping) = &report.mapping {
            println!("Mapping: {}", mapping.mapping_id);
        }
    }
    Ok(())
}

fn render_git_mappings(entries: &[GitMapping], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if !quiet {
        if entries.is_empty() {
            println!("No Git mappings");
        }
        for entry in entries {
            let git_head = entry
                .git_head
                .as_deref()
                .map(|head| head.get(..12).unwrap_or(head))
                .unwrap_or("unborn");
            let dirty = if entry.git_dirty { " dirty" } else { "" };
            println!(
                "{} {}{} {} {} {}",
                entry.direction,
                git_head,
                dirty,
                entry.branch,
                entry.crab_change.0,
                entry.crab_root.0
            );
        }
    }
    Ok(())
}

fn render_timeline(entries: &[TimelineEntry], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if !quiet {
        for entry in entries {
            let message = entry.message.as_deref().unwrap_or("");
            println!(
                "{} {:?} {} {}",
                entry.change_id.0, entry.kind, entry.branch, message
            );
        }
    }
    Ok(())
}

fn render_show(result: &ShowResult, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(result);
    }
    if quiet {
        return Ok(());
    }
    match result {
        ShowResult::Operation { value } => {
            let op = &value.operation;
            println!("Operation: {}", op.change_id.0);
            println!("Kind: {:?}", op.kind);
            println!("Branch: {}", op.branch);
            println!("Actor: {}", op.actor.id);
            if let Some(message) = &op.message {
                println!("Message: {message}");
            }
            if !op.parents.is_empty() {
                println!("Parents:");
                for parent in &op.parents {
                    println!("  {}", parent.0);
                }
            }
            if let Some(before) = &op.before_root {
                println!("Before root: {}", before.0);
            }
            println!("After root: {}", op.after_root.0);
            for path in &value.changed_paths {
                println!(
                    "  {:?} {} (+{} -{})",
                    path.kind, path.path, path.additions, path.deletions
                );
            }
            for message in &value.messages {
                println!("Message object: {} {}", message.id.0, message.body);
            }
        }
        ShowResult::Message { value } => {
            println!("Message: {}", value.id.0);
            println!("Role: {}", value.role);
            if let Some(agent_id) = &value.agent_id {
                println!("Agent: {agent_id}");
            }
            if let Some(session_id) = &value.session_id {
                println!("Session: {session_id}");
            }
            if let Some(change_id) = &value.change_id {
                println!("Change: {}", change_id.0);
            }
            println!("{}", value.body);
        }
        ShowResult::Ref { value } => {
            println!("Ref: {}", value.name);
            println!("Change: {}", value.change_id.0);
            println!("Root: {}", value.root_id.0);
            println!("Generation: {}", value.generation);
        }
        ShowResult::Agent { value } => {
            println!("Agent: {}", value.agent_id);
            println!("Ref: {}", value.ref_name);
            println!("Status: {}", value.status);
            println!("Base: {}", value.base_change.0);
            println!("Head: {}", value.head_change.0);
            if let Some(workdir) = &value.workdir {
                println!("Workdir: {workdir}");
            }
        }
        ShowResult::Object { value } => {
            println!("Object: {}", value.object_id.0);
            println!("Kind: {}", value.kind);
            println!("Version: {}", value.version);
            println!("Size: {}", value.size_bytes);
        }
    }
    Ok(())
}

fn render_object_inspect(report: &ObjectInspectReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if quiet {
        return Ok(());
    }
    println!("Object: {}", report.info.object_id.0);
    println!("Kind: {}", report.info.kind);
    println!("Version: {}", report.info.version);
    println!("Size: {}", report.info.size_bytes);
    println!("Created at: {}", report.info.created_at);
    if report
        .summary
        .as_object()
        .map(|summary| !summary.is_empty())
        .unwrap_or(true)
    {
        println!("Summary:");
        let rendered = serde_json::to_string_pretty(&report.summary)?;
        for line in rendered.lines() {
            println!("  {line}");
        }
    }
    Ok(())
}

fn render_root_inspect(report: &RootInspectReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if quiet {
        return Ok(());
    }
    println!("Root: {}", report.root_id.0);
    println!("Created by: {}", report.root.created_by.0);
    println!("Files: {}", report.root.file_count);
    println!("Total text bytes: {}", report.root.total_text_bytes);
    if let Some(path_root) = &report.root.path_map_root {
        println!("Path map: {path_root}");
    }
    if let Some(file_root) = &report.root.file_index_map_root {
        println!("File index: {file_root}");
    }
    for file in &report.files {
        println!(
            "  {:?} {} {} -> {} ({} bytes)",
            file.kind, file.path, file.file_id, file.content_object.0, file.size_bytes
        );
    }
    Ok(())
}

fn render_text_inspect(report: &TextInspectReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if quiet {
        return Ok(());
    }
    println!("Text: {}", report.text_id.0);
    println!("Content hash: {}", report.content.content_hash);
    println!(
        "Lines: {} (showing {})",
        report.content.line_count,
        report.lines.len()
    );
    println!("Bytes: {}", report.content.byte_count);
    for line in &report.lines {
        let text = serde_json::to_string(&line.text)?;
        println!(
            "  {} {} {:?} {}",
            line.line_number, line.line_id, line.newline, text
        );
    }
    if report.truncated {
        println!("  ... truncated; pass --limit 0 to show all lines");
    }
    Ok(())
}

fn render_map_range(report: &MapRangeReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if quiet {
        return Ok(());
    }
    println!("Map: {}", report.map_id);
    println!("Type: {}", report.map_type);
    println!("Entries: {}", report.entries.len());
    for entry in &report.entries {
        let key = render_map_key(&entry.key);
        let value = render_map_value_summary(&entry.value)?;
        println!("  {key} -> {value}");
    }
    if report.truncated {
        println!("  ... truncated; pass --limit 0 to show all entries");
    }
    Ok(())
}

fn render_map_diff(report: &MapDiffReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if quiet {
        return Ok(());
    }
    println!("Map diff {}..{}", report.left_map_id, report.right_map_id);
    println!("Type: {}", report.map_type);
    println!("Changes: {}", report.changes.len());
    for change in &report.changes {
        let key = render_map_key(&change.key);
        println!("  {} {key}", change.kind);
        if let Some(old_value) = &change.old_value {
            println!("    old: {}", render_map_value_summary(old_value)?);
        }
        if let Some(new_value) = &change.new_value {
            println!("    new: {}", render_map_value_summary(new_value)?);
        }
    }
    if report.truncated {
        println!("  ... truncated; pass --limit 0 to show all changes");
    }
    Ok(())
}

fn render_map_key(key: &crabdb::model::MapKeyInspect) -> String {
    key.text
        .clone()
        .unwrap_or_else(|| format!("hex:{}", key.hex))
}

fn render_map_value_summary(value: &crabdb::model::MapValueInspect) -> Result<String> {
    if let Some(text) = &value.text {
        if value.summary == serde_json::json!({ "bytes": value.bytes }) {
            return Ok(format!("{text:?}"));
        }
    }
    let summary = serde_json::to_string(&value.summary)?;
    if value.truncated {
        Ok(format!(
            "{summary} ({} bytes, hex preview truncated)",
            value.bytes
        ))
    } else {
        Ok(format!("{summary} ({} bytes)", value.bytes))
    }
}

fn diff_from_args(db: &mut CrabDb, args: &DiffArgs) -> Result<DiffSummary> {
    let forms = usize::from(args.range.is_some())
        + usize::from(args.root.is_some())
        + usize::from(args.dirty);
    if forms != 1 {
        return Err(Error::InvalidInput(
            "diff requires exactly one of RANGE, --root ROOT..ROOT, or --dirty".to_string(),
        ));
    }
    if let Some(range) = &args.range {
        db.diff_range_with_options(range, args.patch, args.show_line_ids)
    } else if let Some(root_range) = &args.root {
        db.diff_roots(root_range, args.patch, args.show_line_ids)
    } else {
        db.diff_dirty(args.patch, args.show_line_ids)
    }
}

fn watch_interval(interval_secs: u64, debounce_ms: Option<u64>) -> Result<Duration> {
    if let Some(ms) = debounce_ms {
        if ms == 0 {
            return Err(Error::InvalidInput(
                "watch debounce must be greater than 0ms".to_string(),
            ));
        }
        return Ok(Duration::from_millis(ms));
    }
    if interval_secs == 0 {
        return Err(Error::InvalidInput(
            "watch interval must be greater than 0 seconds".to_string(),
        ));
    }
    Ok(Duration::from_secs(interval_secs))
}

fn render_diff(summary: &DiffSummary, json: bool, quiet: bool, stat: bool) -> Result<()> {
    if json {
        return render_json(summary);
    }
    if !quiet {
        println!("Diff {}..{}", summary.from, summary.to);
        let mut total_additions = 0;
        let mut total_deletions = 0;
        for file in &summary.files {
            total_additions += file.additions;
            total_deletions += file.deletions;
            println!(
                "  {:?} {} (+{} -{})",
                file.kind, file.path, file.additions, file.deletions
            );
            for line in &file.line_changes {
                println!(
                    "    {:?} {} old={} new={}",
                    line.kind,
                    format_line_id(&line.line_id),
                    format_optional_line_number(line.old_line_number),
                    format_optional_line_number(line.new_line_number)
                );
            }
            if let Some(patch) = &file.patch {
                print!("{patch}");
            }
        }
        if stat {
            println!(
                "{} files changed, {} additions, {} deletions",
                summary.files.len(),
                total_additions,
                total_deletions
            );
        }
    }
    Ok(())
}

fn format_line_id(line_id: &crabdb::LineId) -> String {
    format!("{}:{}", line_id.origin_change.0, line_id.local_seq)
}

fn format_optional_line_number(line: Option<u64>) -> String {
    line.map(|line| line.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn render_history(result: &HistoryResult, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(result);
    }
    if !quiet {
        println!("{}", result.selector);
        for entry in &result.file_history {
            let old_path = entry
                .old_path
                .as_ref()
                .map(|path| format!(" from {path}"))
                .unwrap_or_default();
            println!(
                "{} {:?} {}{}",
                entry.change_id.0, entry.kind, entry.path, old_path
            );
        }
        for entry in &result.line_history {
            let line = entry
                .line_number
                .map(|line| format!(":{line}"))
                .unwrap_or_default();
            println!(
                "{} {:?} {}{}",
                entry.change_id.0, entry.kind, entry.path, line
            );
        }
    }
    Ok(())
}

fn render_code_from(result: &CodeFromResult, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(result);
    }
    if !quiet {
        println!("{}", result.selector);
        if result.operations.is_empty() {
            println!("No operations found");
        }
        for operation in &result.operations {
            let message = operation.message.as_deref().unwrap_or("");
            println!(
                "{} {:?} {} {}",
                operation.change_id.0, operation.kind, operation.branch, message
            );
            for path in &operation.changed_paths {
                println!("  {:?} {}", path.kind, path.path);
            }
        }
    }
    Ok(())
}

fn render_checkout(report: &CheckoutReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.dry_run {
            println!(
                "Would check out {} ({} changed paths)",
                report.change_id.0,
                report.changed_paths.len()
            );
        } else {
            println!(
                "Checked out {} ({} files)",
                report.change_id.0, report.written_files
            );
        }
        if let Some(output_root) = &report.output_root {
            println!("Output: {output_root}");
        }
        if let Some(recorded) = &report.recorded_dirty {
            println!("Recorded dirty worktree: {}", recorded.0);
        }
        for path in &report.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
    }
    Ok(())
}

fn render_branch(report: &BranchReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Created branch {} from {}", report.name, report.from.0);
    }
    Ok(())
}

fn render_branch_list(entries: &[BranchListEntry], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if !quiet {
        for entry in entries {
            let marker = if entry.is_current { "*" } else { " " };
            println!("{marker} {} {}", entry.name, entry.change_id.0);
        }
    }
    Ok(())
}

fn render_branch_delete(report: &BranchDeleteReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Deleted branch {}", report.name);
    }
    Ok(())
}

fn render_branch_rename(report: &BranchRenameReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Renamed branch {} to {}", report.old_name, report.new_name);
    }
    Ok(())
}

fn render_merge(report: &MergeReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.dry_run {
            println!(
                "Would merge {} into {} as {}",
                report.source_ref, report.target_ref, report.operation.0
            );
        } else {
            println!(
                "Merged {} into {} as {}",
                report.source_ref, report.target_ref, report.operation.0
            );
        }
        for conflict in &report.conflicts {
            println!("  conflict {conflict}");
        }
        for path in &report.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
    }
    Ok(())
}

fn render_merge_queue_add(report: &MergeQueueAddReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Queued {} into {} as {}",
            report.entry.source_ref, report.entry.target_ref, report.entry.queue_id
        );
    }
    Ok(())
}

fn render_merge_queue_list(entries: &[MergeQueueEntry], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if !quiet {
        for entry in entries {
            println!(
                "{} {} priority={} {} -> {}",
                entry.queue_id, entry.status, entry.priority, entry.source_ref, entry.target_ref
            );
        }
    }
    Ok(())
}

fn render_merge_queue_run(report: &MergeQueueRunReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.processed.is_empty() {
            println!("Merge queue is empty");
        }
        for item in &report.processed {
            match (&item.operation, &item.error) {
                (Some(operation), _) => println!(
                    "{} {} as {} {} -> {}",
                    item.queue_id, item.status, operation.0, item.source_ref, item.target_ref
                ),
                (None, Some(error)) => println!(
                    "{} {} {} -> {}: {}",
                    item.queue_id, item.status, item.source_ref, item.target_ref, error
                ),
                (None, None) => println!(
                    "{} {} {} -> {}",
                    item.queue_id, item.status, item.source_ref, item.target_ref
                ),
            }
        }
        if report.stopped_on_conflict {
            println!("Paused on conflict");
        } else if report.stopped_on_failure {
            println!("Paused on failure");
        }
    }
    Ok(())
}

fn render_merge_queue_remove(
    report: &MergeQueueRemoveReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Cancelled {}", report.entry.queue_id);
    }
    Ok(())
}

fn render_conflicts(entries: &[ConflictSetSummary], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if !quiet {
        if entries.is_empty() {
            println!("No conflicts");
        }
        for entry in entries {
            println!(
                "{} {} {} -> {}",
                entry.conflict_set_id,
                entry.status,
                entry.source_ref.as_deref().unwrap_or("-"),
                entry.target_ref.as_deref().unwrap_or("-")
            );
            for detail in &entry.details {
                println!("  {detail}");
            }
        }
    }
    Ok(())
}

fn render_conflict(entry: &ConflictSetSummary, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(entry);
    }
    if !quiet {
        println!("Conflict: {}", entry.conflict_set_id);
        println!("Status: {}", entry.status);
        if let Some(merge_id) = &entry.merge_id {
            println!("Merge: {merge_id}");
        }
        if let Some(source) = &entry.source_ref {
            println!("Source: {source}");
        }
        if let Some(target) = &entry.target_ref {
            println!("Target: {target}");
        }
        for detail in &entry.details {
            println!("  {detail}");
        }
    }
    Ok(())
}

fn render_conflict_resolve(report: &ConflictResolveReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.resolution == "manual" {
            println!(
                "Resolved {} manually as {}",
                report.conflict_set_id, report.operation.0
            );
        } else {
            println!(
                "Resolved {} by taking {} as {}",
                report.conflict_set_id, report.resolution, report.operation.0
            );
        }
        for path in &report.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
    }
    Ok(())
}

fn render_anchor_create(report: &AnchorCreateReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Created anchor {} at {}:{}",
            report.anchor.id.0, report.anchor.created_path, report.anchor.created_line
        );
    }
    Ok(())
}

fn render_anchor_resolve(report: &AnchorResolveReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Anchor: {}", report.anchor.id.0);
        println!("Label: {}", report.anchor.label);
        println!("Status: {}", report.status);
        if let (Some(path), Some(line_number)) = (&report.path, report.line_number) {
            println!("Location: {path}:{line_number}");
        } else if let Some(path) = &report.path {
            println!("Path: {path}");
        }
        if let Some(text) = &report.text {
            println!("{text}");
        }
    }
    Ok(())
}

fn render_anchor_list(anchors: &[Anchor], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(&anchors);
    }
    if !quiet {
        for anchor in anchors {
            println!(
                "{} {} {}:{}",
                anchor.id.0, anchor.label, anchor.created_path, anchor.created_line
            );
        }
    }
    Ok(())
}

fn render_anchor_delete(report: &AnchorDeleteReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Deleted anchor {}", report.anchor_id.0);
    }
    Ok(())
}

fn render_lease_acquire(report: &LeaseAcquireReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Acquired lease {} {} {} {}",
            report.lease.lease_id,
            report.lease.mode,
            report.lease.agent_id,
            report.lease.path.as_deref().unwrap_or("<workspace>")
        );
    }
    Ok(())
}

fn render_agent_claim(report: &AgentClaimReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.claimed {
            if let Some(lease) = &report.lease {
                println!(
                    "Claimed {} for {} until {} ({})",
                    report.path, report.agent_id, lease.expires_at, lease.lease_id
                );
            } else {
                println!("Claimed {} for {}", report.path, report.agent_id);
            }
        } else if let Some(warning) = &report.warning {
            println!("Warning: {warning}");
        } else {
            println!("Path {} is already claimed", report.path);
        }
    }
    Ok(())
}

fn render_lease_list(leases: &[LeaseRecord], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(&leases);
    }
    if !quiet {
        if leases.is_empty() {
            println!("No active leases");
        }
        for lease in leases {
            println!(
                "{} {} {} {} expires_at={}",
                lease.lease_id,
                lease.mode,
                lease.agent_id,
                lease.path.as_deref().unwrap_or("<workspace>"),
                lease.expires_at
            );
        }
    }
    Ok(())
}

fn render_lease_release(report: &LeaseReleaseReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Released lease {}", report.lease_id);
    }
    Ok(())
}

fn render_why(result: &WhyResult, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(result);
    }
    if !quiet {
        println!(
            "{}:{} {}",
            result.path, result.line_number, result.current_text
        );
        println!(
            "Line ID: {}:{}",
            result.line_id.origin_change.0, result.line_id.local_seq
        );
        println!("Introduced by: {}", result.introduced_by.0);
        println!("Last content change: {}", result.last_content_change.0);
        for item in &result.history {
            println!("  {:?} {} {}", item.kind, item.change_id.0, item.path);
        }
    }
    Ok(())
}

fn render_agent_spawn(report: &AgentSpawnReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Spawned {} at {}", report.agent_id, report.base_change.0);
        if let Some(workdir) = &report.workdir {
            println!("Workdir: {workdir}");
        }
    }
    Ok(())
}

fn render_agent_list(entries: &[AgentDetails], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if !quiet {
        for entry in entries {
            println!(
                "{} {} {} {}",
                entry.record.name,
                entry.branch.status,
                entry.branch.head_change.0,
                entry.branch.ref_name
            );
        }
    }
    Ok(())
}

fn render_agent_details(details: &AgentDetails, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(details);
    }
    if !quiet {
        println!("Agent: {}", details.record.name);
        println!("ID: {}", details.record.agent_id);
        println!("Ref: {}", details.branch.ref_name);
        println!("Status: {}", details.branch.status);
        println!("Base: {}", details.branch.base_change.0);
        println!("Head: {}", details.branch.head_change.0);
        if let Some(provider) = &details.record.provider {
            println!("Provider: {provider}");
        }
        if let Some(model) = &details.record.model {
            println!("Model: {model}");
        }
        if let Some(session_id) = &details.branch.session_id {
            println!("Session: {session_id}");
        }
        if let Some(workdir) = &details.branch.workdir {
            println!("Workdir: {workdir}");
        }
    }
    Ok(())
}

fn render_agent_status(report: &AgentStatusReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "{} {} ({} changed paths, {} queued merges)",
            report.agent.record.name,
            report.agent.branch.status,
            report.changed_paths.len(),
            report.queued_merges
        );
        for path in &report.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
        if let Some(state) = &report.workdir_state {
            println!("Workdir: {:?}", state);
            for path in &report.workdir_changed_paths {
                println!("  workdir {:?} {}", path.kind, path.path);
            }
        }
        if let Some(test) = &report.latest_test {
            let command = if test.command.is_empty() {
                String::new()
            } else {
                format!(" {}", test.command.join(" "))
            };
            println!(
                "Latest test: {}{} ({} ms)",
                test.status, command, test.duration_ms
            );
        }
        if let Some(eval) = &report.latest_eval {
            let command = if eval.command.is_empty() {
                String::new()
            } else {
                format!(" {}", eval.command.join(" "))
            };
            println!(
                "Latest eval: {}{} ({} ms)",
                eval.status, command, eval.duration_ms
            );
        }
    }
    Ok(())
}

fn render_agent_contribution(
    report: &AgentContributionReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        let status = &report.status;
        println!(
            "Agent contribution: {} ({})",
            status.agent.record.name, status.agent.branch.status
        );
        println!("Ref: {}", status.agent.branch.ref_name);
        println!(
            "Base: {}  Head: {}",
            status.agent.branch.base_change.0, status.agent.branch.head_change.0
        );
        println!(
            "Changed paths: {}  Operations: {}  Sessions: {}  Events: {}  Approvals: {}",
            status.changed_paths.len(),
            report.operations.len(),
            report.sessions.len(),
            report.recent_events.len(),
            report.approvals.len()
        );
        for path in &status.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
        if let Some(test) = &status.latest_test {
            println!("Latest test: {} ({})", test.status, test.command.join(" "));
        }
        if let Some(eval) = &status.latest_eval {
            println!("Latest eval: {} ({})", eval.status, eval.command.join(" "));
        }
        if !report.operations.is_empty() {
            println!("Recent operations:");
            for operation in &report.operations {
                println!(
                    "  {} {:?} {} path(s) {}",
                    operation.change_id.0,
                    operation.kind,
                    operation.path_count,
                    operation.message.as_deref().unwrap_or("")
                );
            }
        }
        let pending_approvals = report
            .approvals
            .iter()
            .filter(|approval| approval.status == "pending")
            .count();
        if pending_approvals > 0 {
            println!("Pending approvals: {pending_approvals}");
        }
    }
    Ok(())
}

fn render_agent_gate_history(
    report: &AgentGateHistoryReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Agent gates for {} ({}, limit {})",
            report.agent.record.name, report.kind, report.limit
        );
        for gate in &report.gates {
            let suite = gate.suite.as_deref().unwrap_or("-");
            let score = gate
                .score
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string());
            let threshold = gate
                .threshold
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string());
            println!(
                "  {} {} {} suite={} score={} threshold={} {}",
                gate.created_at,
                gate.kind,
                gate.status,
                suite,
                score,
                threshold,
                gate.command.join(" ")
            );
        }
    }
    Ok(())
}

fn render_agent_readiness(report: &AgentReadinessReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Agent readiness: {} ({})",
            report.agent.record.name, report.status
        );
        println!("Ref: {}", report.agent.branch.ref_name);
        println!(
            "Ready: {}  Changed paths: {}  Blockers: {}  Warnings: {}",
            report.ready,
            report.changed_paths.len(),
            report.blockers.len(),
            report.warnings.len()
        );
        if !report.blockers.is_empty() {
            println!("Blockers:");
            for blocker in &report.blockers {
                println!("  {}: {}", blocker.code, blocker.message);
            }
        }
        if !report.warnings.is_empty() {
            println!("Warnings:");
            for warning in &report.warnings {
                println!("  {}: {}", warning.code, warning.message);
            }
        }
        if let Some(test) = &report.latest_test {
            println!("Latest test: {} ({})", test.status, test.command.join(" "));
        }
        if let Some(eval) = &report.latest_eval {
            println!("Latest eval: {} ({})", eval.status, eval.command.join(" "));
        }
    }
    Ok(())
}

fn render_agent_handoff(report: &AgentHandoffReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Agent handoff: {} ({})",
            report.agent.record.name, report.readiness.status
        );
        println!("Ref: {}", report.agent.branch.ref_name);
        println!(
            "Ready: {}  Sessions: {}  Events: {}  Spans: {}  Operations: {}",
            report.readiness.ready,
            report.recent_sessions.len(),
            report.recent_events.len(),
            report.recent_spans.len(),
            report.recent_operations.len()
        );
        if let Some(session) = &report.current_session {
            println!(
                "Current session: {} ({})",
                session.session.session_id, session.session.status
            );
            println!(
                "Session context: {} turn(s), {} message(s), {} event(s), {} operation(s)",
                session.turns.len(),
                session.messages.len(),
                session.events.len(),
                session.operations.len()
            );
        }
        if !report.readiness.blockers.is_empty() {
            println!("Blockers:");
            for blocker in &report.readiness.blockers {
                println!("  {}: {}", blocker.code, blocker.message);
            }
        }
        if !report.next_steps.is_empty() {
            println!("Next steps:");
            for step in &report.next_steps {
                println!("  {step}");
            }
        }
    }
    Ok(())
}

fn render_agent_message(report: &AgentMessageReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Added message {} ({})", report.message_id.0, report.role);
    }
    Ok(())
}

fn render_agent_turn_start(
    report: &crabdb::AgentTurnStartReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Started turn {} for {}",
            report.turn.turn_id, report.turn.agent_id
        );
        println!("Session: {}", report.session.session_id);
        println!("Base: {}", report.turn.before_change.0);
        println!("Root: {}", report.base_root.0);
    }
    Ok(())
}

fn render_agent_turn_details(
    details: &crabdb::AgentTurnDetails,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(details);
    }
    if !quiet {
        println!("Turn: {}", details.turn.turn_id);
        println!("Agent: {}", details.turn.agent_id);
        println!("Status: {}", details.turn.status);
        if let Some(session) = &details.session {
            println!("Session: {}", session.session_id);
        }
        println!("Base: {}", details.turn.before_change.0);
        if let Some(after) = &details.turn.after_change {
            println!("After: {}", after.0);
        }
        println!("Messages: {}", details.messages.len());
        println!("Events: {}", details.events.len());
        println!("Operations: {}", details.operations.len());
        for event in &details.events {
            println!("  event {} {}", event.event_id, event.event_type);
        }
        for operation in &details.operations {
            let message = operation.message.as_deref().unwrap_or("");
            println!(
                "  op {} {:?} {}",
                operation.change_id.0, operation.kind, message
            );
        }
    }
    Ok(())
}

fn render_agent_turn_event(
    report: &crabdb::AgentTurnEventReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Added event {} {}",
            report.event.event_id, report.event.event_type
        );
    }
    Ok(())
}

fn render_agent_events(events: &[AgentEventRecord], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(events);
    }
    if !quiet {
        for event in events {
            let session = event.session_id.as_deref().unwrap_or("-");
            let turn = event.turn_id.as_deref().unwrap_or("-");
            println!(
                "{} {} agent={} session={} turn={}",
                event.event_id, event.event_type, event.agent_id, session, turn
            );
        }
    }
    Ok(())
}

fn render_agent_trace_span_start(
    report: &AgentTraceSpanStartReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Started span {} {} {}",
            report.span.span_id, report.span.span_type, report.span.name
        );
        println!("Trace: {}", report.span.trace_id);
    }
    Ok(())
}

fn render_agent_trace_span_end(
    report: &AgentTraceSpanEndReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Ended span {} {}", report.span.span_id, report.span.status);
        if let Some(duration_ms) = report.span.duration_ms {
            println!("Duration: {duration_ms} ms");
        }
    }
    Ok(())
}

fn render_agent_trace_spans(spans: &[AgentTraceSpan], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(spans);
    }
    if !quiet {
        for span in spans {
            let parent = span.parent_span_id.as_deref().unwrap_or("-");
            let turn = span.turn_id.as_deref().unwrap_or("-");
            let duration = span
                .duration_ms
                .map(|duration_ms| format!("{duration_ms}ms"))
                .unwrap_or_else(|| "-".to_string());
            println!(
                "{} {} {} status={} trace={} parent={} turn={} duration={}",
                span.span_id,
                span.span_type,
                span.name,
                span.status,
                span.trace_id,
                parent,
                turn,
                duration
            );
        }
    }
    Ok(())
}

fn render_agent_trace_summary(
    report: &AgentTraceSummaryReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Trace summary: {} spans ({} open, {} ended, {} failed)",
            report.span_count,
            report.open_span_count,
            report.ended_span_count,
            report.failed_span_count
        );
        if let Some(trace_id) = &report.trace_id {
            println!("Trace: {trace_id}");
        }
        if let Some(agent_id) = &report.agent_id {
            println!("Agent: {agent_id}");
        }
        if let Some(turn_id) = &report.turn_id {
            println!("Turn: {turn_id}");
        }
        if report.total_duration_ms > 0 {
            let average = report
                .average_duration_ms
                .map(|duration| format!("{duration:.1}"))
                .unwrap_or_else(|| "-".to_string());
            println!(
                "Duration: total={}ms max={}ms avg={}ms",
                report.total_duration_ms, report.max_duration_ms, average
            );
        }
        println!("Statuses: {}", render_named_counts(&report.status_counts));
        println!("Types: {}", render_named_counts(&report.span_type_counts));
        println!("Traces: {}", render_named_counts(&report.trace_counts));
        if !report.slowest_spans.is_empty() {
            println!("Slowest spans:");
            for span in &report.slowest_spans {
                println!(
                    "  {} {} {} {}ms",
                    span.span_id,
                    span.span_type,
                    span.status,
                    span.duration_ms.unwrap_or(0)
                );
            }
        }
        if !report.open_spans.is_empty() {
            println!("Open spans:");
            for span in &report.open_spans {
                println!("  {} {} {}", span.span_id, span.span_type, span.name);
            }
        }
    }
    Ok(())
}

fn render_named_counts(counts: &[crabdb::model::NamedCount]) -> String {
    if counts.is_empty() {
        return "-".to_string();
    }
    counts
        .iter()
        .map(|count| format!("{}={}", count.name, count.count))
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_agent_trace_span(span: &AgentTraceSpan, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(span);
    }
    if !quiet {
        println!("Span: {}", span.span_id);
        println!("Trace: {}", span.trace_id);
        println!("Type: {}", span.span_type);
        println!("Name: {}", span.name);
        println!("Status: {}", span.status);
        if let Some(parent) = &span.parent_span_id {
            println!("Parent: {parent}");
        }
        if let Some(turn) = &span.turn_id {
            println!("Turn: {turn}");
        }
        if let Some(duration_ms) = span.duration_ms {
            println!("Duration: {duration_ms} ms");
        }
    }
    Ok(())
}

fn render_agent_turn_end(
    report: &crabdb::AgentTurnEndReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Ended turn {} as {}",
            report.turn.turn_id, report.turn.status
        );
    }
    Ok(())
}

fn render_agent_run_pause(report: &AgentRunPauseReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Paused run {} for {}",
            report.run_state.run_id, report.run_state.agent_id
        );
        println!("Reason: {}", report.run_state.reason);
        println!("Summary: {}", report.run_state.summary);
        if let Some(approval_id) = &report.run_state.approval_id {
            println!("Approval: {approval_id}");
        }
    }
    Ok(())
}

fn render_agent_run_resume(report: &AgentRunResumeReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Resumed run {} for {}",
            report.run_state.run_id, report.run_state.agent_id
        );
        if let Some(resumed_at) = report.run_state.resumed_at {
            println!("Resumed at: {resumed_at}");
        }
    }
    Ok(())
}

fn render_agent_run_list(run_states: &[AgentRunState], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(run_states);
    }
    if !quiet {
        if run_states.is_empty() {
            println!("No agent run states");
        }
        for run_state in run_states {
            let approval = run_state.approval_id.as_deref().unwrap_or("-");
            println!(
                "{} {} agent={} reason={} approval={}",
                run_state.run_id, run_state.status, run_state.agent_id, run_state.reason, approval
            );
            println!("  {}", run_state.summary);
        }
    }
    Ok(())
}

fn render_agent_run_state(run_state: &AgentRunState, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(run_state);
    }
    if !quiet {
        println!("Agent run: {}", run_state.run_id);
        println!("Agent: {}", run_state.agent_id);
        println!("Status: {}", run_state.status);
        println!("Reason: {}", run_state.reason);
        println!("Summary: {}", run_state.summary);
        if let Some(session_id) = &run_state.session_id {
            println!("Session: {session_id}");
        }
        if let Some(turn_id) = &run_state.turn_id {
            println!("Turn: {turn_id}");
        }
        if let Some(approval_id) = &run_state.approval_id {
            println!("Approval: {approval_id}");
        }
        if let Some(reviewer) = &run_state.reviewer {
            println!("Reviewer: {reviewer}");
        }
        if let Some(note) = &run_state.note {
            println!("Note: {note}");
        }
    }
    Ok(())
}

fn render_session_start(report: &AgentSessionStartReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Started session {} for {}",
            report.session.session_id, report.session.agent_id
        );
        if let Some(title) = &report.session.title {
            println!("Title: {title}");
        }
    }
    Ok(())
}

fn render_session_current(
    reports: &[AgentSessionCurrentReport],
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(&reports);
    }
    if !quiet {
        if reports.is_empty() {
            println!("No active sessions");
        }
        for report in reports {
            match &report.session {
                Some(session) => {
                    let title = session.title.as_deref().unwrap_or("");
                    println!(
                        "{} {} {} {}",
                        report.agent_name, session.session_id, session.status, title
                    );
                }
                None => println!("{} has no active session", report.agent_name),
            }
        }
    }
    Ok(())
}

fn render_session_list(sessions: &[AgentSession], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(&sessions);
    }
    if !quiet {
        if sessions.is_empty() {
            println!("No sessions");
        }
        for session in sessions {
            let title = session.title.as_deref().unwrap_or("");
            println!(
                "{} {} {} {}",
                session.session_id, session.status, session.agent_id, title
            );
        }
    }
    Ok(())
}

fn render_session_details(details: &AgentSessionDetails, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(details);
    }
    if !quiet {
        println!("Session: {}", details.session.session_id);
        println!("Agent: {}", details.session.agent_id);
        println!("Status: {}", details.session.status);
        if let Some(title) = &details.session.title {
            println!("Title: {title}");
        }
        println!("Turns: {}", details.turns.len());
        println!("Messages: {}", details.messages.len());
        println!("Operations: {}", details.operations.len());
        for turn in &details.turns {
            let after = turn
                .after_change
                .as_ref()
                .map(|change| change.0.as_str())
                .unwrap_or("-");
            println!("  {} {} {}", turn.turn_id, turn.status, after);
        }
        for operation in &details.operations {
            let message = operation.message.as_deref().unwrap_or("");
            println!(
                "  op {} {:?} {}",
                operation.change_id.0, operation.kind, message
            );
        }
    }
    Ok(())
}

fn render_session_context(
    report: &AgentSessionContextReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Session context: {}", report.session.session_id);
        println!("Agent: {}", report.session.agent_id);
        println!("Status: {}", report.session.status);
        if let Some(title) = &report.session.title {
            println!("Title: {title}");
        }
        println!(
            "Totals: {} messages, {} events, {} turns, {} operations",
            report.message_count, report.event_count, report.turn_count, report.operation_count
        );
        if !report.recent_messages.is_empty() {
            println!("Recent messages:");
            for message in &report.recent_messages {
                let preview = single_line_preview(&message.body, 80);
                println!("  {} {} {}", message.id.0, message.role, preview);
            }
        }
        if !report.recent_turns.is_empty() {
            println!("Recent turns:");
            for turn in &report.recent_turns {
                println!("  {} {}", turn.turn_id, turn.status);
            }
        }
        if !report.recent_operations.is_empty() {
            println!("Recent operations:");
            for operation in &report.recent_operations {
                let message = operation.message.as_deref().unwrap_or("");
                println!(
                    "  {} {:?} {}",
                    operation.change_id.0, operation.kind, message
                );
            }
        }
    }
    Ok(())
}

fn render_session_end(report: &AgentSessionEndReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Ended session {} as {}",
            report.session.session_id, report.session.status
        );
    }
    Ok(())
}

fn single_line_preview(value: &str, limit: usize) -> String {
    let mut preview = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if preview.len() > limit {
        preview.truncate(limit.saturating_sub(3));
        preview.push_str("...");
    }
    preview
}

fn render_approval_request(
    report: &AgentApprovalRequestReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Requested approval {} {}",
            report.approval.approval_id, report.approval.action
        );
        println!("{}", report.approval.summary);
        if let Some(run_state) = &report.run_state {
            println!("Paused run: {}", run_state.run_id);
        }
    }
    Ok(())
}

fn render_approval_list(approvals: &[AgentApproval], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(&approvals);
    }
    if !quiet {
        if approvals.is_empty() {
            println!("No approvals");
        }
        for approval in approvals {
            println!(
                "{} {} {} {}",
                approval.approval_id, approval.status, approval.agent_id, approval.action
            );
            println!("  {}", approval.summary);
        }
    }
    Ok(())
}

fn render_approval(approval: &AgentApproval, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(approval);
    }
    if !quiet {
        println!("Approval: {}", approval.approval_id);
        println!("Agent: {}", approval.agent_id);
        println!("Status: {}", approval.status);
        println!("Action: {}", approval.action);
        println!("Summary: {}", approval.summary);
        if let Some(session_id) = &approval.session_id {
            println!("Session: {session_id}");
        }
        if let Some(turn_id) = &approval.turn_id {
            println!("Turn: {turn_id}");
        }
        if let Some(reviewer) = &approval.reviewer {
            println!("Reviewer: {reviewer}");
        }
        if let Some(note) = &approval.note {
            println!("Note: {note}");
        }
    }
    Ok(())
}

fn render_approval_decision(
    report: &AgentApprovalDecisionReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Decision {} for {}",
            report.decision, report.approval.approval_id
        );
        if !report.run_states.is_empty() {
            println!("Linked run states: {}", report.run_states.len());
        }
    }
    Ok(())
}

fn render_agent_record(report: &AgentRecordReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        match &report.operation {
            Some(operation) => {
                println!("Recorded agent workdir {}", operation.0);
                for path in &report.changed_paths {
                    println!("  {:?} {}", path.kind, path.path);
                }
            }
            None => println!("No agent workdir changes to record"),
        }
    }
    Ok(())
}

fn render_agent_watch(report: &AgentWatchReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Watched {} for {} iteration(s); recorded {} operation(s)",
            report.agent_id,
            report.iterations,
            report.recorded_operations.len()
        );
        for operation in &report.recorded_operations {
            println!("  {operation}");
        }
        for path in &report.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
    }
    Ok(())
}

fn render_agent_test(report: &AgentTestReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Agent {} {} for {}",
            report.kind, report.status, report.agent_id
        );
        println!("Turn: {}", report.turn_id);
        println!("Command: {}", report.command.join(" "));
        if let Some(suite) = &report.suite {
            println!("Suite: {suite}");
        }
        if report.score.is_some() || report.threshold.is_some() {
            let score = report
                .score
                .map(|value| value.to_string())
                .unwrap_or_else(|| "n/a".to_string());
            let threshold = report
                .threshold
                .map(|value| value.to_string())
                .unwrap_or_else(|| "n/a".to_string());
            println!("Score: {score} / threshold {threshold}");
        }
        match report.exit_code {
            Some(code) => println!("Exit: {code}"),
            None if report.timed_out => println!("Exit: timed out"),
            None => println!("Exit: unavailable"),
        }
        println!("Duration: {} ms", report.duration_ms);
        println!("Stdout object: {}", report.stdout_object.0);
        println!("Stderr object: {}", report.stderr_object.0);
        if !report.stdout_preview.is_empty() {
            println!("Stdout:");
            print!("{}", report.stdout_preview);
            if !report.stdout_preview.ends_with('\n') {
                println!();
            }
        }
        if !report.stderr_preview.is_empty() {
            println!("Stderr:");
            eprint!("{}", report.stderr_preview);
            if !report.stderr_preview.ends_with('\n') {
                eprintln!();
            }
        }
    }
    Ok(())
}

fn render_agent_workdir(report: &AgentWorkdirReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if let Some(workdir) = &report.workdir {
            println!("{workdir}");
        } else {
            println!("Agent {} has no materialized workdir", report.agent_id);
        }
    }
    Ok(())
}

fn render_agent_workdir_sync(
    report: &AgentWorkdirSyncReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Synced agent workdir: {}", report.workdir);
        println!("Head: {}", report.head_change.0);
        if report.forced {
            println!("Forced: true");
        }
        for path in &report.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
    }
    Ok(())
}

fn render_agent_patch(report: &AgentPatchReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Applied agent patch {}", report.operation.0);
        for path in &report.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
    }
    Ok(())
}

fn render_agent_remove(report: &AgentRemoveReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Removed agent {} ({})", report.agent_id, report.ref_name);
        if let Some(workdir) = &report.removed_workdir {
            println!("Removed workdir: {workdir}");
        }
    }
    Ok(())
}

fn render_doctor(report: &DoctorReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Doctor: {}", report.status);
        for check in &report.checks {
            println!("[{}] {}: {}", check.status, check.name, check.message);
        }
    }
    Ok(())
}

fn render_backup_create(report: &BackupCreateReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Created backup: {}", report.path);
        println!("Branch: {}", report.branch);
        println!("Refs: {}", report.ref_count);
        println!("Operations: {}", report.operation_count);
        println!("SQLite bytes: {}", report.sqlite_bytes);
        println!("SQLite SHA-256: {}", report.sqlite_sha256);
        if !report.fsck_errors.is_empty() {
            println!("FSCK warnings:");
            for error in &report.fsck_errors {
                println!("  {error}");
            }
        }
    }
    Ok(())
}

fn render_backup_verify(report: &BackupVerifyReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        let status = if report.valid { "valid" } else { "invalid" };
        println!("Backup {status}: {}", report.path);
        if let Some(branch) = &report.branch {
            println!("Branch: {branch}");
        }
        println!(
            "Checked {} refs, {} roots, {} text objects",
            report.checked_refs, report.checked_roots, report.checked_texts
        );
        for error in &report.errors {
            println!("  {error}");
        }
    }
    Ok(())
}

fn render_backup_restore(report: &BackupRestoreReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Restored backup: {}", report.backup_path);
        println!("Workspace: {}", report.workspace);
        println!("Branch: {}", report.branch);
        println!("Replaced existing DB: {}", report.replaced_existing);
        println!("Rewritten agent workdirs: {}", report.rewritten_workdirs);
        println!(
            "Checked {} refs, {} roots, {} text objects",
            report.checked_refs, report.checked_roots, report.checked_texts
        );
    }
    Ok(())
}

fn render_fsck(report: &FsckReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Checked {} refs, {} roots, {} text objects",
            report.checked_refs, report.checked_roots, report.checked_texts
        );
        if report.errors.is_empty() {
            println!("No errors");
        } else {
            for error in &report.errors {
                println!("  {error}");
            }
        }
    }
    Ok(())
}

fn render_index_rebuild(report: &IndexRebuildReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Rebuilt indexes: {} operations, {} parents, {} file rows, {} line rows, {} messages",
            report.operations,
            report.operation_parents,
            report.file_history_rows,
            report.line_history_rows,
            report.messages
        );
        for error in &report.errors {
            println!("  warning: {error}");
        }
    }
    Ok(())
}

fn render_gc(report: &GcReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.dry_run {
            println!(
                "GC dry run: {} prunable of {} known objects ({} reachable, {} unknown preserved)",
                report.prunable_objects,
                report.total_known_objects,
                report.reachable_objects,
                report.preserved_unknown_objects
            );
        } else {
            println!(
                "GC pruned {} objects ({} reachable, {} unknown preserved)",
                report.pruned_objects, report.reachable_objects, report.preserved_unknown_objects
            );
        }
        for error in &report.errors {
            println!("  warning: {error}");
        }
    }
    Ok(())
}
