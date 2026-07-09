use std::path::PathBuf;

use clap::{Parser, Subcommand};

mod acp_args;
mod agent_args;
mod collaboration_args;
mod handler;
mod inspect_args;
mod lane_args;
mod maintenance_args;
mod render;
mod workspace_args;
mod worktree_args;

use acp_args::*;
use agent_args::*;
use collaboration_args::*;
use inspect_args::*;
use lane_args::*;
use maintenance_args::*;
use workspace_args::*;
use worktree_args::*;

pub(crate) fn run() {
    handler::run_cli();
}

#[derive(Parser)]
#[command(name = "trail")]
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
    #[arg(long, global = true, value_name = "URL")]
    daemon_url: Option<String>,
    #[arg(long, global = true, value_name = "TOKEN")]
    daemon_token: Option<String>,
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
    /// Initialize a new Trail workspace and default branch state.
    /// Use this once per repository to create `.trail`, default config,
    /// `.trailignore`, and baseline root metadata.
    Init(InitArgs),
    /// Inspect and edit workspace configuration values.
    /// Use `get` and `set` to read typed keys and adjust behavior safely.
    Config(ConfigCommand),
    /// Manage `.trailignore` rules that shield files from recording.
    /// Add/remove patterns or check whether a path is currently ignored.
    Ignore(IgnoreCommand),
    /// Run policy checks for proposed lane actions.
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
    /// List recent operations across workspace, branch, session, or lane scope.
    Timeline(TimelineArgs),
    /// Print details for an operation, message, ref, or object id.
    /// Supports human and JSON output.
    Show(ShowArgs),
    /// Inspect generic Trail object metadata and summarize known object kinds.
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
    /// Find source operations and changed paths from a message, session, or lane.
    CodeFrom(CodeFromArgs),
    /// Manage lane branches, metadata, sessions, patches, tests, and traces.
    /// This command group covers the full CLI-facing lane workflow.
    Lane(LaneCommand),
    /// Run Agent Client Protocol relay integrations for coding agents.
    Acp(AcpCommand),
    /// Run high-level agent task workflows without managing lanes directly.
    Agent(AgentCommand),
    /// Show a readable transcript for a lane, session, or ACP session.
    Transcript(TranscriptArgs),
    /// Work with durable turns using a short top-level alias.
    Turn(TopTurnCommand),
    /// Create and manage lane sessions, context packets, and lifecycle.
    Session(SessionCommand),
    /// Handle sensitive action approvals and reviewer decisions.
    Approvals(ApprovalsCommand),
    /// Merge a lane branch into a standard branch with readiness checks.
    /// Applies the same conflict rules as queue/manual merges.
    MergeLane(MergeLaneArgs),
    /// Schedule and run controlled serialized merges using a queue.
    /// Merge items pause on first conflict and keep audit history.
    MergeQueue(MergeQueueCommand),
    /// Inspect and resolve conflict sets opened by merge operations.
    Conflicts(ConflictsCommand),
    /// Create and resolve stable anchors that survive nearby line churn.
    Anchor(AnchorCommand),
    /// Manage advisory read/write leases used for path coordination.
    /// Helps prevent overlapping lane writes across workdirs.
    Lease(LeaseCommand),
    /// Move data between Trail refs and Git, and inspect mapping metadata.
    Git(GitCommand),
    /// Inspect and render local API schemas for integrations.
    Api(ApiCommand),
    /// Run the JSON HTTP API daemon for editor and lane integrations.
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
