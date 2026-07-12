use std::path::PathBuf;

use clap::{Parser, Subcommand};

mod acp_args;
mod agent_args;
mod collaboration_args;
mod environment_args;
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
use environment_args::*;
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
#[command(version)]
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
    /// Build, attach, and inspect reproducible dependency environments.
    Deps(DepsCommand),
    /// Inspect and synchronize adapter-owned workspace environments.
    Env(EnvironmentCommand),
    /// Inspect and verify immutable local workspace cache layers.
    Cache(CacheCommand),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_environment_adapter_catalog() {
        let cli = Cli::try_parse_from(["trail", "env", "adapters"])
            .expect("environment adapter catalog command should parse");
        let Command::Env(EnvironmentCommand {
            command: EnvironmentSubcommand::Adapters,
        }) = cli.command
        else {
            panic!("expected environment adapters command");
        };
    }

    #[test]
    fn parses_environment_status() {
        let cli = Cli::try_parse_from(["trail", "env", "status", "lane-a"])
            .expect("environment status command should parse");

        let Command::Env(EnvironmentCommand {
            command: EnvironmentSubcommand::Status(args),
        }) = cli.command
        else {
            panic!("expected environment status command");
        };
        assert_eq!(args.lane, "lane-a");
    }

    #[test]
    fn parses_environment_discovery_with_component_root() {
        let cli = Cli::try_parse_from(["trail", "env", "discover", "lane-a", "--path", "apps/web"])
            .expect("environment discovery command should parse");
        let Command::Env(EnvironmentCommand {
            command: EnvironmentSubcommand::Discover(args),
        }) = cli.command
        else {
            panic!("expected environment discovery command");
        };
        assert_eq!(args.lane, "lane-a");
        assert_eq!(args.path.as_deref(), Some("apps/web"));
    }

    #[test]
    fn parses_environment_graph_with_component_root() {
        let cli = Cli::try_parse_from([
            "trail", "env", "graph", "lane-a", "--path", "apps", "--offset", "10", "--limit", "20",
        ])
        .expect("environment graph command should parse");
        let Command::Env(EnvironmentCommand {
            command: EnvironmentSubcommand::Graph(args),
        }) = cli.command
        else {
            panic!("expected environment graph command");
        };
        assert_eq!(args.lane, "lane-a");
        assert_eq!(args.path.as_deref(), Some("apps"));
        assert_eq!(args.offset, 10);
        assert_eq!(args.limit, 20);
    }

    #[test]
    fn parses_environment_generation() {
        let cli = Cli::try_parse_from(["trail", "env", "generation", "lane-a"])
            .expect("environment generation command should parse");
        let Command::Env(EnvironmentCommand {
            command: EnvironmentSubcommand::Generation(args),
        }) = cli.command
        else {
            panic!("expected environment generation command");
        };
        assert_eq!(args.lane, "lane-a");
    }

    #[test]
    fn parses_environment_plan_with_adapter_and_root() {
        let cli = Cli::try_parse_from([
            "trail",
            "env",
            "plan",
            "lane-a",
            "--adapter",
            "command",
            "--path",
            "tools/schema",
            "--component",
            "protobuf.generated",
        ])
        .expect("environment plan command should parse");
        let Command::Env(EnvironmentCommand {
            command: EnvironmentSubcommand::Plan(args),
        }) = cli.command
        else {
            panic!("expected environment plan command");
        };
        assert_eq!(args.lane, "lane-a");
        assert_eq!(args.adapter, "command");
        assert_eq!(args.path.as_deref(), Some("tools/schema"));
        assert_eq!(args.component.as_deref(), Some("protobuf.generated"));
    }

    #[test]
    fn environment_sync_defaults_to_auto_detection_and_accepts_a_component_root() {
        let cli = Cli::try_parse_from(["trail", "env", "sync", "lane-a", "--path", "apps/web"])
            .expect("environment sync command should parse");

        let Command::Env(EnvironmentCommand {
            command: EnvironmentSubcommand::Sync(args),
        }) = cli.command
        else {
            panic!("expected environment sync command");
        };
        assert_eq!(args.lane, "lane-a");
        assert_eq!(args.adapter, "auto");
        assert_eq!(args.path.as_deref(), Some("apps/web"));
        assert_eq!(args.component, None);
    }

    #[test]
    fn environment_sync_accepts_an_explicit_adapter() {
        let cli = Cli::try_parse_from([
            "trail",
            "env",
            "sync",
            "lane-a",
            "--adapter",
            "example/python@1",
        ])
        .expect("environment sync command should parse");

        let Command::Env(EnvironmentCommand {
            command: EnvironmentSubcommand::Sync(args),
        }) = cli.command
        else {
            panic!("expected environment sync command");
        };
        assert_eq!(args.adapter, "example/python@1");
        assert_eq!(args.path, None);
        assert_eq!(args.component, None);
    }

    #[test]
    fn environment_sync_all_accepts_a_discovery_root() {
        let cli = Cli::try_parse_from(["trail", "env", "sync-all", "lane-a", "--path", "apps"])
            .expect("environment sync-all command should parse");
        let Command::Env(EnvironmentCommand {
            command: EnvironmentSubcommand::SyncAll(args),
        }) = cli.command
        else {
            panic!("expected environment sync-all command");
        };
        assert_eq!(args.lane, "lane-a");
        assert_eq!(args.path.as_deref(), Some("apps"));
    }

    #[test]
    fn parses_environment_runtime_lifecycle_commands() {
        for (action, expected) in [
            ("status", "status"),
            ("reconcile", "reconcile"),
            ("stop", "stop"),
        ] {
            let cli = Cli::try_parse_from(["trail", "env", "runtime", action, "lane-a"])
                .expect("environment runtime command should parse");
            let Command::Env(EnvironmentCommand {
                command: EnvironmentSubcommand::Runtime(runtime),
            }) = cli.command
            else {
                panic!("expected environment runtime command");
            };
            let (actual, lane) = match runtime.command {
                EnvironmentRuntimeSubcommand::Status(args) => ("status", args.lane),
                EnvironmentRuntimeSubcommand::Reconcile(args) => ("reconcile", args.lane),
                EnvironmentRuntimeSubcommand::Stop(args) => ("stop", args.lane),
            };
            assert_eq!(actual, expected);
            assert_eq!(lane, "lane-a");
        }
    }
}
