use clap::{Args, Subcommand, ValueEnum};

#[derive(Subcommand)]
pub(super) enum AgentSubcommand {
    /// Internal native-agent hook ingress. Use `agent hooks` to manage integrations.
    #[command(hide = true)]
    Hook(AgentHookCommand),
    /// Install, inspect, diagnose, and remove native agent lifecycle hooks.
    Hooks(AgentHooksCommand),
    /// Declare and manage an explicit ACP/native/terminal capture ownership run.
    Capture(AgentCaptureCommand),
    /// List immutable native transcripts, exports, and other captured artifacts.
    Artifacts(AgentArtifactsArgs),
    /// Inspect the deterministic causal provenance graph for a session.
    Provenance(AgentProvenanceArgs),
    /// Create and verify exact session evidence attestations.
    Attest(AgentAttestCommand),
    /// Review captured, evidence-anchored learnings.
    Learnings(AgentLearningsCommand),
    /// Export a portable, canonical agent trace.
    Export(AgentTraceExportArgs),
    /// Link an exact Git commit to enumerated Trail session/change evidence.
    GitLink(AgentGitLinkCommand),
    /// Configure, diagnose, and run Agent Client Protocol integrations.
    Acp(AgentAcpCommand),
    /// Create a fresh task lane and launch a terminal agent.
    Start(AgentStartArgs),
    /// Continue from an existing agent task checkpoint in a fresh task lane.
    #[command(visible_alias = "follow-up")]
    Continue(AgentContinueArgs),
    /// Show the shortest state-aware guide for using Trail with agent tasks.
    #[command(visible_alias = "help-me")]
    Guide(AgentSelectorArgs),
    /// Route a plain-language question to the right agent view.
    Ask(AgentAskArgs),
    /// Show the one next useful action for an agent task.
    #[command(visible_alias = "todo")]
    Next(AgentSelectorArgs),
    /// Show the latest agent task and the next useful action.
    Status,
    /// Show one compact task dashboard with next action, focus, validation, and apply readiness.
    #[command(visible_alias = "dash")]
    Dashboard(AgentSelectorArgs),
    /// Show one structured review-data packet for editor panels and integrations.
    #[command(hide = true)]
    #[command(visible_alias = "cockpit", visible_alias = "side-panel")]
    ReviewData(AgentSelectorArgs),
    /// Run or print one advertised agent action.
    #[command(visible_alias = "do")]
    Action(AgentActionArgs),
    /// Walk one task through review, validation, and finish as a guided checklist.
    #[command(visible_alias = "walkthrough", visible_alias = "review-loop")]
    ReviewFlow(AgentSelectorArgs),
    /// Show all agent tasks grouped by what needs attention.
    #[command(visible_alias = "home")]
    Inbox(AgentInboxArgs),
    /// Show a multi-agent board with low-noise next actions.
    #[command(visible_alias = "tasks")]
    Board(AgentInboxArgs),
    /// Show overlap and a safe apply order across multiple agent tasks.
    #[command(visible_alias = "order")]
    Stack(AgentInboxArgs),
    /// Show one compact review brief for an agent task.
    #[command(hide = true)]
    Brief(AgentSelectorArgs),
    /// Show the one-page post-run summary for an agent task.
    #[command(hide = true)]
    Summary(AgentSelectorArgs),
    /// Suggest validation commands and show latest test/eval gates.
    #[command(visible_alias = "tests")]
    Validate(AgentSelectorArgs),
    /// Plan the exact test/eval checks to run for an agent task.
    #[command(hide = true)]
    #[command(visible_alias = "validation-plan", visible_alias = "test-checklist")]
    TestPlan(AgentSelectorArgs),
    /// Create a copyable review report for an agent task.
    #[command(hide = true)]
    Report(AgentReportArgs),
    /// Print a copyable handoff packet for another human or agent.
    #[command(hide = true)]
    #[command(visible_alias = "share")]
    Handoff(AgentSelectorArgs),
    /// Print a copyable receipt for what an agent task changed and how to land it.
    #[command(hide = true)]
    Receipt(AgentSelectorArgs),
    /// Print a pull request draft title and body for an agent task.
    #[command(hide = true)]
    Pr(AgentPrArgs),
    /// Explain what happened in one plain-language task story.
    #[command(hide = true)]
    Story(AgentSelectorArgs),
    /// Show tool calls, available commands, and the turns/checkpoints around them.
    #[command(hide = true)]
    Tools(AgentSelectorArgs),
    /// Show touched areas, blast radius, and recommended review/test checks.
    #[command(hide = true)]
    Impact(AgentSelectorArgs),
    /// Show a file-by-file review map grouped by changed area.
    #[command(hide = true)]
    #[command(visible_alias = "review-files", visible_alias = "file-checklist")]
    ReviewMap(AgentSelectorArgs),
    /// Summarize apply risk and concrete mitigation steps.
    #[command(hide = true)]
    Risk(AgentSelectorArgs),
    /// Show one go/no-go confidence verdict across review, validation, risk, and apply preflight.
    #[command(hide = true)]
    #[command(visible_alias = "go", visible_alias = "go-no-go")]
    Confidence(AgentSelectorArgs),
    /// Check whether an agent task is ready to apply without mutating Git.
    #[command(hide = true)]
    #[command(visible_alias = "can-land")]
    Ready(AgentSelectorArgs),
    /// Diagnose a stuck or sideways agent task and show safe recovery options.
    #[command(hide = true)]
    #[command(visible_alias = "recover")]
    Diagnose(AgentSelectorArgs),
    /// Compare two agent tasks and highlight overlap, risk, and apply order.
    #[command(hide = true)]
    Compare(AgentCompareArgs),
    /// Run a command in the agent task workdir and record a test gate.
    #[command(hide = true)]
    Test(AgentGateArgs),
    /// Run a command in the agent task workdir and record an eval gate.
    #[command(hide = true)]
    Eval(AgentGateArgs),
    /// Print the filesystem workdir for an agent task.
    #[command(hide = true)]
    Workdir(AgentSelectorArgs),
    /// List recorded agent tasks.
    List(AgentListArgs),
    /// Inspect one agent task transcript, tools, changes, and checkpoint.
    View(AgentSelectorArgs),
    /// Show changes grouped by turn, operation, or file without chasing ids.
    Changes(AgentChangesArgs),
    /// Show the newest completed turn or operation delta.
    #[command(visible_alias = "last")]
    Delta(AgentDeltaArgs),
    /// Show what changed since this task was last marked reviewed.
    #[command(visible_alias = "what-changed")]
    New(AgentNewArgs),
    /// Mark the current task checkpoint as reviewed.
    #[command(hide = true)]
    #[command(visible_alias = "done")]
    MarkReviewed(AgentMarkReviewedArgs),
    /// Mark one changed file as reviewed at the current task checkpoint.
    #[command(hide = true)]
    #[command(visible_alias = "done-file", visible_alias = "reviewed-file")]
    MarkFileReviewed(AgentMarkFileReviewedArgs),
    /// Hide a finished or irrelevant task from the default agent inbox.
    #[command(hide = true)]
    #[command(visible_alias = "close")]
    Archive(AgentArchiveArgs),
    /// Restore an archived task to the default agent inbox.
    #[command(hide = true)]
    Unarchive(AgentArchiveArgs),
    /// Inspect one high-level change card as a focused change set.
    #[command(hide = true)]
    Change(AgentChangeArgs),
    /// Show changed files with the turns, prompts, and commands behind each one.
    #[command(hide = true)]
    #[command(visible_alias = "changed-files")]
    Files(AgentSelectorArgs),
    /// Inspect one changed file with provenance, change cards, and optional patch.
    #[command(hide = true)]
    #[command(visible_alias = "inspect")]
    File(AgentFileArgs),
    /// List friendly rewind targets and checkpoint ids for an agent task.
    #[command(hide = true)]
    #[command(visible_alias = "rewind-points")]
    Checkpoints(AgentSelectorArgs),
    /// Show the prompt-to-checkpoint timeline for an agent task.
    #[command(hide = true)]
    Timeline(AgentTimelineArgs),
    /// Inspect one agent turn with prompt, tools, checkpoint, files, and optional patch.
    #[command(hide = true)]
    Turn(AgentTurnArgs),
    /// Show the latest or selected turn diff without spelling out diff flags.
    #[command(hide = true)]
    TurnDiff(AgentTurnDiffArgs),
    /// Explain why a file changed in an agent task.
    #[command(hide = true)]
    #[command(visible_alias = "explain")]
    Why(AgentWhyArgs),
    /// Show the whole task diff or a single turn/checkpoint/operation diff.
    #[command(hide = true)]
    Diff(AgentDiffArgs),
    /// Review readiness, transcript, changes, and next steps for an agent task.
    #[command(hide = true)]
    #[command(visible_alias = "review-plan")]
    Review(AgentSelectorArgs),
    /// Focus the next file to inspect by combining review, why, and diff.
    #[command(hide = true)]
    Focus(AgentFocusArgs),
    /// Open the focused file in the configured editor.
    #[command(hide = true)]
    #[command(visible_alias = "edit")]
    Open(AgentOpenArgs),
    /// Safely apply one agent task back to the current Git branch.
    #[command(visible_alias = "land")]
    Apply(AgentApplyArgs),
    /// Apply one agent task and hide it from the default inbox when successful.
    #[command(visible_alias = "ship")]
    Finish(AgentFinishArgs),
    /// Undo the latest agent turn or a selected prompt/turn.
    #[command(visible_alias = "undo-last")]
    Undo(AgentUndoArgs),
    /// Rewind one agent task to a checkpoint or friendly turn label.
    Rewind(AgentRewindArgs),
    /// Check provider and workspace readiness for agent tasks.
    Doctor(AgentDoctorArgs),
}

#[derive(Args)]
pub(super) struct AgentHookCommand {
    #[command(subcommand)]
    pub(super) command: AgentHookSubcommand,
}

#[derive(Subcommand)]
pub(super) enum AgentHookSubcommand {
    /// Durably journal one provider hook payload from stdin.
    Receive(AgentHookReceiveArgs),
}

#[derive(Args)]
pub(super) struct AgentHookReceiveArgs {
    pub(super) provider: String,
    pub(super) native_event: String,
    #[arg(long)]
    pub(super) installation: Option<String>,
    #[arg(long, hide = true)]
    pub(super) dedupe_key: Option<String>,
}

#[derive(Args)]
pub(super) struct AgentHooksCommand {
    #[command(subcommand)]
    pub(super) command: AgentHooksSubcommand,
}

#[derive(Subcommand)]
pub(super) enum AgentHooksSubcommand {
    /// Install or update Trail-owned hooks for a provider.
    Setup(AgentHooksSetupArgs),
    /// Remove only the entries or file owned by a Trail installation.
    Remove(AgentHooksRemoveArgs),
    /// List supported providers and their recorded installations.
    List(AgentHooksListArgs),
    /// Inspect one provider's configured installation and drift state.
    Status(AgentHooksProviderArgs),
    /// Diagnose provider support, configuration, and recent delivery.
    Doctor(AgentHooksDoctorArgs),
    /// List durable native receipts for a provider.
    Events(AgentHooksEventsArgs),
    /// Replay one receipt or the pending durable receipt queue.
    Replay(AgentHooksReplayArgs),
    /// Move a retry or quarantined receipt back to the replay queue.
    Retry(AgentHookReceiptArgs),
    /// Explicitly discard a received, retrying, or quarantined receipt.
    Discard(AgentHookReceiptArgs),
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(super) enum AgentHooksScopeArg {
    Project,
    User,
}

#[derive(Args)]
pub(super) struct AgentHooksSetupArgs {
    #[arg(value_name = "PROVIDER", conflicts_with = "provider_flag")]
    pub(super) provider: Option<String>,
    #[arg(
        long = "provider",
        value_name = "PROVIDER",
        conflicts_with = "provider"
    )]
    pub(super) provider_flag: Option<String>,
    #[arg(long, value_enum, default_value = "project")]
    pub(super) scope: AgentHooksScopeArg,
    #[arg(long)]
    pub(super) lane: Option<String>,
    #[arg(long = "print")]
    pub(super) print_only: bool,
    #[arg(long)]
    pub(super) yes: bool,
    #[arg(long)]
    pub(super) force: bool,
}

#[derive(Args)]
pub(super) struct AgentHooksRemoveArgs {
    #[arg(value_name = "PROVIDER", conflicts_with = "provider_flag")]
    pub(super) provider: Option<String>,
    #[arg(
        long = "provider",
        value_name = "PROVIDER",
        conflicts_with = "provider"
    )]
    pub(super) provider_flag: Option<String>,
    #[arg(long, value_enum, default_value = "project")]
    pub(super) scope: AgentHooksScopeArg,
    #[arg(long)]
    pub(super) dry_run: bool,
}

#[derive(Args)]
pub(super) struct AgentHooksListArgs {
    #[arg(long)]
    pub(super) installed: bool,
}

#[derive(Args)]
pub(super) struct AgentHooksProviderArgs {
    #[arg(value_name = "PROVIDER", conflicts_with = "provider_flag")]
    pub(super) provider: Option<String>,
    #[arg(
        long = "provider",
        value_name = "PROVIDER",
        conflicts_with = "provider"
    )]
    pub(super) provider_flag: Option<String>,
}

#[derive(Args)]
pub(super) struct AgentHooksDoctorArgs {
    #[arg(value_name = "PROVIDER", conflicts_with_all = ["provider_flag", "all"])]
    pub(super) provider: Option<String>,
    #[arg(long = "provider", value_name = "PROVIDER", conflicts_with_all = ["provider", "all"])]
    pub(super) provider_flag: Option<String>,
    #[arg(long, conflicts_with_all = ["provider", "provider_flag"])]
    pub(super) all: bool,
    #[arg(long)]
    pub(super) probe: bool,
}

#[derive(Args)]
pub(super) struct AgentHooksEventsArgs {
    #[arg(value_name = "PROVIDER", conflicts_with = "provider_flag")]
    pub(super) provider: Option<String>,
    #[arg(
        long = "provider",
        value_name = "PROVIDER",
        conflicts_with = "provider"
    )]
    pub(super) provider_flag: Option<String>,
    #[arg(long, default_value_t = 20)]
    pub(super) last: usize,
    #[arg(long, default_value_t = 0)]
    pub(super) offset: usize,
    #[arg(long)]
    pub(super) failed: bool,
}

#[derive(Args)]
pub(super) struct AgentHooksReplayArgs {
    #[arg(long, conflicts_with = "pending")]
    pub(super) receipt: Option<String>,
    #[arg(long)]
    pub(super) pending: bool,
    #[arg(long, default_value_t = 100)]
    pub(super) limit: usize,
}

#[derive(Args)]
pub(super) struct AgentHookReceiptArgs {
    pub(super) receipt: String,
}

#[derive(Args)]
pub(super) struct AgentCaptureCommand {
    #[command(subcommand)]
    pub(super) command: AgentCaptureSubcommand,
}

#[derive(Subcommand)]
pub(super) enum AgentCaptureSubcommand {
    Begin(AgentCaptureBeginArgs),
    Renew(AgentCaptureRenewArgs),
    End(AgentCaptureEndArgs),
    Status(AgentCaptureStatusArgs),
    Reconcile,
}

#[derive(Args)]
pub(super) struct AgentCaptureBeginArgs {
    #[arg(long)]
    pub(super) owner: String,
    #[arg(long)]
    pub(super) session: String,
    #[arg(long)]
    pub(super) executor: Option<String>,
    #[arg(long)]
    pub(super) lane: Option<String>,
    #[arg(long)]
    pub(super) workdir: Option<std::path::PathBuf>,
    #[arg(long)]
    pub(super) work_item: Option<String>,
    #[arg(long, default_value_t = 300_000)]
    pub(super) ttl_ms: u64,
}

#[derive(Args)]
pub(super) struct AgentCaptureRenewArgs {
    pub(super) run_id: String,
    #[arg(long)]
    pub(super) owner: String,
    #[arg(long)]
    pub(super) session: String,
    #[arg(long, default_value_t = 300_000)]
    pub(super) ttl_ms: u64,
}

#[derive(Args)]
pub(super) struct AgentCaptureEndArgs {
    pub(super) run_id: String,
    #[arg(long)]
    pub(super) owner: String,
    #[arg(long)]
    pub(super) session: String,
}

#[derive(Args)]
pub(super) struct AgentCaptureStatusArgs {
    #[arg(long)]
    pub(super) all: bool,
    #[arg(long, default_value_t = 100)]
    pub(super) limit: usize,
    #[arg(long, default_value_t = 0)]
    pub(super) offset: usize,
}

#[derive(Args)]
pub(super) struct AgentArtifactsArgs {
    pub(super) session: String,
    #[arg(long)]
    pub(super) turn: Option<String>,
    #[arg(long, default_value_t = 100)]
    pub(super) limit: usize,
    #[arg(long, default_value_t = 0)]
    pub(super) offset: usize,
}

#[derive(Args)]
pub(super) struct AgentProvenanceArgs {
    pub(super) session: String,
    #[arg(long, default_value_t = 1_000)]
    pub(super) limit: usize,
    #[arg(long, default_value_t = 0)]
    pub(super) offset: usize,
}

#[derive(Args)]
pub(super) struct AgentAttestCommand {
    #[command(subcommand)]
    pub(super) command: AgentAttestSubcommand,
}

#[derive(Subcommand)]
pub(super) enum AgentAttestSubcommand {
    Create(AgentAttestCreateArgs),
    List(AgentSessionIdArgs),
    Show(AgentAttestationIdArgs),
    Verify(AgentAttestationIdArgs),
}

#[derive(Args)]
pub(super) struct AgentAttestCreateArgs {
    pub(super) session: String,
    #[arg(long, default_value = "manual")]
    pub(super) policy: String,
}

#[derive(Args)]
pub(super) struct AgentSessionIdArgs {
    pub(super) session: String,
    #[arg(long, default_value_t = 100)]
    pub(super) limit: usize,
    #[arg(long, default_value_t = 0)]
    pub(super) offset: usize,
}

#[derive(Args)]
pub(super) struct AgentAttestationIdArgs {
    pub(super) attestation_id: String,
}

#[derive(Args)]
pub(super) struct AgentLearningsCommand {
    #[command(subcommand)]
    pub(super) command: AgentLearningsSubcommand,
}

#[derive(Subcommand)]
pub(super) enum AgentLearningsSubcommand {
    List(AgentLearningsListArgs),
    Accept(AgentLearningReviewArgs),
    Reject(AgentLearningReviewArgs),
}

#[derive(Args)]
pub(super) struct AgentLearningsListArgs {
    #[arg(long)]
    pub(super) session: Option<String>,
    #[arg(long)]
    pub(super) status: Option<String>,
    #[arg(long, default_value_t = 100)]
    pub(super) limit: usize,
    #[arg(long, default_value_t = 0)]
    pub(super) offset: usize,
}

#[derive(Args)]
pub(super) struct AgentLearningReviewArgs {
    pub(super) learning_id: String,
    #[arg(long)]
    pub(super) reviewer: String,
}

#[derive(Args)]
pub(super) struct AgentTraceExportArgs {
    pub(super) session: String,
    #[arg(long)]
    pub(super) output: Option<std::path::PathBuf>,
    #[arg(long)]
    pub(super) attachments: bool,
}

#[derive(Args)]
pub(super) struct AgentGitLinkCommand {
    #[command(subcommand)]
    pub(super) command: AgentGitLinkSubcommand,
}

#[derive(Subcommand)]
pub(super) enum AgentGitLinkSubcommand {
    Link(AgentGitLinkArgs),
    List(AgentSessionIdArgs),
}

#[derive(Args)]
pub(super) struct AgentGitLinkArgs {
    pub(super) session: String,
    pub(super) git_commit: String,
    #[arg(long)]
    pub(super) turn: Option<String>,
    #[arg(long)]
    pub(super) from_change: Option<String>,
    #[arg(long)]
    pub(super) through_change: Option<String>,
    #[arg(long, default_value = "exact-change")]
    pub(super) confidence: String,
    #[arg(long, default_value = "explicit-cli")]
    pub(super) source: String,
}

#[derive(Args)]
#[command(after_help = "Daily path:
  trail agent guide
  trail agent ask what should I do
  trail agent action
  trail agent changes latest
  trail agent apply latest --dry-run

Run `trail agent` with no subcommand to show the current task dashboard, or the grouped inbox when there are multiple tasks. Specialist commands still work; prefer `trail agent ask ...` when you do not remember the exact command.")]
pub(super) struct AgentCommand {
    #[command(subcommand)]
    pub(super) command: Option<AgentSubcommand>,
}

#[derive(Args)]
pub(super) struct AgentAcpCommand {
    #[command(subcommand)]
    pub(super) command: AgentAcpSubcommand,
}

#[derive(Subcommand)]
pub(super) enum AgentAcpSubcommand {
    /// Configure an editor to launch an ACP-backed Trail task.
    Setup(AgentAcpSetupArgs),
    /// Run the editor-facing ACP task entrypoint.
    #[command(hide = true)]
    Run(AgentAcpRunArgs),
    /// List available ACP providers and configured status.
    Status(AgentAcpStatusArgs),
    /// Run ACP integration diagnostics.
    Doctor(AgentAcpDoctorArgs),
    /// List captured ACP sessions.
    Sessions(AgentAcpSessionsArgs),
}

#[derive(Args)]
pub(super) struct AgentAcpSetupArgs {
    #[arg(value_name = "PROVIDER", conflicts_with = "provider_flag")]
    pub(super) provider: Option<String>,
    #[arg(
        long = "provider",
        value_name = "PROVIDER",
        conflicts_with = "provider"
    )]
    pub(super) provider_flag: Option<String>,
    #[arg(long, default_value = "generic")]
    pub(super) editor: String,
    #[arg(long = "print")]
    pub(super) print_only: bool,
    #[arg(long)]
    pub(super) yes: bool,
}

#[derive(Args)]
pub(super) struct AgentAcpRunArgs {
    #[arg(value_name = "PROVIDER", conflicts_with = "provider_flag")]
    pub(super) provider: Option<String>,
    #[arg(
        long = "provider",
        value_name = "PROVIDER",
        conflicts_with = "provider"
    )]
    pub(super) provider_flag: Option<String>,
    #[arg(long)]
    pub(super) name: Option<String>,
    #[arg(long)]
    pub(super) from: Option<String>,
    #[arg(long = "no-mcp")]
    pub(super) no_mcp: bool,
    #[arg(last = true, num_args = 0..)]
    pub(super) command: Vec<String>,
}

#[derive(Args)]
pub(super) struct AgentAcpStatusArgs {
    #[arg(value_name = "PROVIDER", conflicts_with = "provider_flag")]
    pub(super) provider: Option<String>,
    #[arg(
        long = "provider",
        value_name = "PROVIDER",
        conflicts_with = "provider"
    )]
    pub(super) provider_flag: Option<String>,
}

#[derive(Args)]
pub(super) struct AgentAcpDoctorArgs {
    #[arg(value_name = "PROVIDER", conflicts_with = "provider_flag")]
    pub(super) provider: Option<String>,
    #[arg(
        long = "provider",
        value_name = "PROVIDER",
        conflicts_with = "provider"
    )]
    pub(super) provider_flag: Option<String>,
    #[arg(long = "relay-command", num_args = 1.., allow_hyphen_values = true)]
    pub(super) relay_command: Vec<String>,
}

#[derive(Args)]
pub(super) struct AgentAcpSessionsArgs {
    #[arg(long)]
    pub(super) lane: Option<String>,
}

#[derive(Args)]
pub(super) struct AgentStartArgs {
    #[arg(value_name = "PROVIDER", conflicts_with = "provider_flag")]
    pub(super) provider: Option<String>,
    #[arg(
        long = "provider",
        value_name = "PROVIDER",
        conflicts_with = "provider"
    )]
    pub(super) provider_flag: Option<String>,
    #[arg(long)]
    pub(super) name: Option<String>,
    #[arg(
        long,
        help = "Start the fresh task from this Trail ref, task, lane, or checkpoint"
    )]
    pub(super) from: Option<String>,
    #[arg(
        long = "workdir-mode",
        value_parser = ["auto", "native-cow", "portable-copy", "fuse-cow", "nfs-cow", "dokan-cow"],
        default_value = "auto",
        help = "Filesystem view for the terminal agent task"
    )]
    pub(super) workdir_mode: String,
    #[arg(last = true, num_args = 0..)]
    pub(super) command: Vec<String>,
}

#[derive(Args)]
pub(super) struct AgentContinueArgs {
    #[arg(
        default_value = "latest",
        help = "Agent task, lane, session, ACP session, or latest"
    )]
    pub(super) selector: String,
    #[arg(
        long,
        value_name = "PROVIDER",
        help = "Override the provider for the follow-up task"
    )]
    pub(super) provider: Option<String>,
    #[arg(long, help = "Optional human label for the follow-up task")]
    pub(super) name: Option<String>,
    #[arg(
        long = "workdir-mode",
        value_parser = ["auto", "native-cow", "portable-copy", "fuse-cow", "nfs-cow", "dokan-cow"],
        default_value = "auto",
        help = "Filesystem view for the terminal follow-up task"
    )]
    pub(super) workdir_mode: String,
    #[arg(last = true, num_args = 0..)]
    pub(super) command: Vec<String>,
}

#[derive(Args)]
pub(super) struct AgentSelectorArgs {
    #[arg(
        default_value = "latest",
        help = "Agent task, lane, session, ACP session, or latest"
    )]
    pub(super) selector: String,
}

#[derive(Args)]
pub(super) struct AgentInboxArgs {
    #[arg(long, help = "Include archived agent tasks")]
    pub(super) all: bool,
}

#[derive(Args)]
pub(super) struct AgentListArgs {
    #[arg(long, help = "Include archived agent tasks")]
    pub(super) all: bool,
}

#[derive(Args)]
pub(super) struct AgentAskArgs {
    #[arg(
        long,
        default_value = "latest",
        help = "Agent task, lane, session, ACP session, or latest"
    )]
    pub(super) selector: String,
    #[arg(
        required = true,
        num_args = 1..,
        help = "Plain-language question, for example: what changed, is it safe, explain README.md"
    )]
    pub(super) question: Vec<String>,
}

#[derive(Args)]
pub(super) struct AgentActionArgs {
    #[arg(
        value_name = "SELECTOR_OR_ACTION",
        help = "Action id, or task selector when ACTION is also provided"
    )]
    pub(super) selector_or_action: Option<String>,
    #[arg(
        value_name = "ACTION",
        help = "Action id from `trail agent review-data`"
    )]
    pub(super) action: Option<String>,
    #[arg(long, help = "Print the underlying command without running it")]
    pub(super) print: bool,
    #[arg(
        long,
        help = "Required for actions that need confirmation, such as validation and apply"
    )]
    pub(super) confirm: bool,
    #[arg(short, long, help = "Apply/finish commit message for apply actions")]
    pub(super) message: Option<String>,
    #[arg(long, help = "Optional note for review/archive marker actions")]
    pub(super) note: Option<String>,
}

#[derive(Args)]
pub(super) struct AgentChangesArgs {
    #[arg(
        default_value = "latest",
        help = "Agent task, lane, session, ACP session, or latest"
    )]
    pub(super) selector: String,
    #[arg(
        long = "by-turn",
        conflicts_with_all = ["by_operation", "by_file"],
        help = "Group changes by prompt/response turn"
    )]
    pub(super) by_turn: bool,
    #[arg(
        long = "by-operation",
        conflicts_with = "by_file",
        help = "Group changes by recorded Trail operation"
    )]
    pub(super) by_operation: bool,
    #[arg(long = "by-file", help = "Group review cards by changed file")]
    pub(super) by_file: bool,
}

#[derive(Args)]
pub(super) struct AgentTimelineArgs {
    #[arg(
        default_value = "latest",
        help = "Agent task, lane, session, ACP session, or latest"
    )]
    pub(super) selector: String,
    #[arg(
        long = "by-turn",
        conflicts_with = "by_operation",
        help = "Group timeline items by prompt/response turn"
    )]
    pub(super) by_turn: bool,
    #[arg(
        long = "by-operation",
        help = "Group timeline items by recorded Trail operation"
    )]
    pub(super) by_operation: bool,
}

#[derive(Args)]
pub(super) struct AgentDeltaArgs {
    #[arg(
        default_value = "latest",
        help = "Agent task, lane, session, ACP session, or latest"
    )]
    pub(super) selector: String,
    #[arg(
        long = "by-turn",
        conflicts_with = "by_operation",
        help = "Show the newest prompt/response turn delta"
    )]
    pub(super) by_turn: bool,
    #[arg(
        long = "by-operation",
        help = "Show the newest recorded Trail operation delta"
    )]
    pub(super) by_operation: bool,
    #[arg(
        long,
        value_name = "PATH",
        help = "Limit the newest delta to one changed file"
    )]
    pub(super) file: Option<String>,
    #[arg(long, help = "Include unified patches")]
    pub(super) patch: bool,
}

#[derive(Args)]
pub(super) struct AgentNewArgs {
    #[arg(
        default_value = "latest",
        help = "Agent task, lane, session, ACP session, or latest"
    )]
    pub(super) selector: String,
    #[arg(
        long,
        value_name = "PATH",
        help = "Limit new changes to one changed file"
    )]
    pub(super) file: Option<String>,
    #[arg(long, help = "Include unified patches")]
    pub(super) patch: bool,
}

#[derive(Args)]
pub(super) struct AgentMarkReviewedArgs {
    #[arg(
        default_value = "latest",
        help = "Agent task, lane, session, ACP session, or latest"
    )]
    pub(super) selector: String,
    #[arg(long, help = "Optional note to store with the reviewed marker")]
    pub(super) note: Option<String>,
}

#[derive(Args)]
pub(super) struct AgentMarkFileReviewedArgs {
    #[arg(
        value_name = "SELECTOR_OR_PATH",
        help = "Changed file path, or task selector when PATH is also provided"
    )]
    pub(super) selector_or_path: String,
    #[arg(value_name = "PATH", help = "Changed file path to mark reviewed")]
    pub(super) path: Option<String>,
    #[arg(long, help = "Optional note to store with the file reviewed marker")]
    pub(super) note: Option<String>,
}

#[derive(Args)]
pub(super) struct AgentArchiveArgs {
    #[arg(
        default_value = "latest",
        help = "Agent task, lane, session, ACP session, or latest"
    )]
    pub(super) selector: String,
    #[arg(long, help = "Optional note to store with the archive marker")]
    pub(super) note: Option<String>,
}

#[derive(Args)]
pub(super) struct AgentChangeArgs {
    #[arg(
        value_name = "SELECTOR_OR_CHANGE",
        help = "Change card rank/key, or task selector when CHANGE is also provided"
    )]
    pub(super) selector_or_change: Option<String>,
    #[arg(value_name = "CHANGE", help = "Change card rank, key, or title")]
    pub(super) change: Option<String>,
    #[arg(long, help = "Include focused patches for files in this change set")]
    pub(super) patch: bool,
}

#[derive(Args)]
pub(super) struct AgentFileArgs {
    #[arg(
        value_name = "SELECTOR_OR_PATH",
        help = "Path to inspect, or task selector when PATH is also provided"
    )]
    pub(super) selector_or_path: String,
    #[arg(value_name = "PATH", help = "Path to inspect for the selected task")]
    pub(super) path: Option<String>,
    #[arg(long, help = "Include the focused patch for this file")]
    pub(super) patch: bool,
}

#[derive(Args)]
pub(super) struct AgentReportArgs {
    #[arg(
        default_value = "latest",
        help = "Agent task, lane, session, ACP session, or latest"
    )]
    pub(super) selector: String,
    #[arg(long, help = "Print the copyable Markdown review bundle")]
    pub(super) markdown: bool,
}

#[derive(Args)]
pub(super) struct AgentPrArgs {
    #[arg(
        default_value = "latest",
        help = "Agent task, lane, session, ACP session, or latest"
    )]
    pub(super) selector: String,
    #[arg(long, conflicts_with = "body_only", help = "Print only the PR title")]
    pub(super) title_only: bool,
    #[arg(long, help = "Print only the PR body")]
    pub(super) body_only: bool,
}

#[derive(Args)]
pub(super) struct AgentCompareArgs {
    #[arg(help = "Left agent task, lane, session, or ACP session")]
    pub(super) left: String,
    #[arg(help = "Right agent task, lane, session, or ACP session")]
    pub(super) right: String,
}

#[derive(Args)]
pub(super) struct AgentWhyArgs {
    #[arg(
        value_name = "SELECTOR_OR_PATH",
        help = "Path to explain, or task selector when PATH is also provided"
    )]
    pub(super) selector_or_path: String,
    #[arg(value_name = "PATH", help = "Path to explain for the selected task")]
    pub(super) path: Option<String>,
}

#[derive(Args)]
pub(super) struct AgentTurnArgs {
    #[arg(
        value_name = "SELECTOR_OR_TURN",
        help = "Agent task selector, turn index, turn id, or latest/last"
    )]
    pub(super) selector_or_turn: Option<String>,
    #[arg(value_name = "TURN", help = "Turn index, turn id, or latest/last")]
    pub(super) turn: Option<String>,
    #[arg(
        long,
        value_name = "PATH",
        help = "Limit embedded diff output to one changed file"
    )]
    pub(super) file: Option<String>,
    #[arg(long, help = "Include unified patches in the embedded diff")]
    pub(super) patch: bool,
}

#[derive(Args)]
pub(super) struct AgentTurnDiffArgs {
    #[arg(
        default_value = "latest",
        help = "Agent task, lane, session, ACP session, or latest"
    )]
    pub(super) selector: String,
    #[arg(
        long,
        value_name = "N_OR_TURN_ID",
        help = "Diff one 1-based turn index or turn id"
    )]
    pub(super) turn: Option<String>,
    #[arg(
        long,
        value_name = "PATH",
        help = "Limit the turn diff output to one changed file"
    )]
    pub(super) file: Option<String>,
    #[arg(long, help = "Show file-level stats")]
    pub(super) stat: bool,
    #[arg(long, help = "Include unified patches")]
    pub(super) patch: bool,
}

#[derive(Args)]
pub(super) struct AgentGateArgs {
    #[arg(
        default_value = "latest",
        help = "Agent task, lane, session, ACP session, or latest"
    )]
    pub(super) selector: String,
    #[arg(long)]
    pub(super) turn: Option<String>,
    #[arg(long, default_value_t = 600)]
    pub(super) timeout_secs: u64,
    #[arg(long)]
    pub(super) suite: Option<String>,
    #[arg(long)]
    pub(super) score: Option<f64>,
    #[arg(long)]
    pub(super) threshold: Option<f64>,
    #[arg(last = true, num_args = 1.., required = true)]
    pub(super) command: Vec<String>,
}

#[derive(Args)]
pub(super) struct AgentDiffArgs {
    #[arg(
        default_value = "latest",
        help = "Agent task, lane, session, ACP session, or latest"
    )]
    pub(super) selector: String,
    #[arg(long, conflicts_with_all = ["operation", "checkpoint", "last_turn"], help = "Diff one 1-based turn index or turn id")]
    pub(super) turn: Option<String>,
    #[arg(long, conflicts_with_all = ["turn", "checkpoint", "last_turn"], help = "Diff one recorded operation/change id")]
    pub(super) operation: Option<String>,
    #[arg(long, conflicts_with_all = ["turn", "operation", "last_turn"], help = "Diff one checkpoint/change id")]
    pub(super) checkpoint: Option<String>,
    #[arg(long = "last-turn", conflicts_with_all = ["turn", "operation", "checkpoint"], help = "Diff the latest completed agent turn")]
    pub(super) last_turn: bool,
    #[arg(
        long,
        value_name = "PATH",
        help = "Limit the diff output to one changed file"
    )]
    pub(super) file: Option<String>,
    #[arg(long, help = "Show file-level stats")]
    pub(super) stat: bool,
    #[arg(long, help = "Include unified patches")]
    pub(super) patch: bool,
}

#[derive(Args)]
pub(super) struct AgentFocusArgs {
    #[arg(
        default_value = "latest",
        help = "Agent task, lane, session, ACP session, or latest"
    )]
    pub(super) selector: String,
    #[arg(
        long,
        value_name = "PATH",
        help = "Focus a specific changed file instead of the top review-priority file"
    )]
    pub(super) file: Option<String>,
    #[arg(long, help = "Include the focused unified patch")]
    pub(super) patch: bool,
}

#[derive(Args)]
pub(super) struct AgentOpenArgs {
    #[arg(
        default_value = "latest",
        help = "Agent task, lane, session, ACP session, or latest"
    )]
    pub(super) selector: String,
    #[arg(
        long,
        value_name = "PATH",
        help = "Open a specific changed file instead of the top review-priority file"
    )]
    pub(super) file: Option<String>,
    #[arg(long, help = "Print the editor command without launching it")]
    pub(super) print: bool,
}

#[derive(Args)]
pub(super) struct AgentApplyArgs {
    #[arg(default_value = "latest")]
    pub(super) selector: String,
    #[arg(long = "into-current-git-branch")]
    pub(super) into_current_git_branch: bool,
    #[arg(long)]
    pub(super) dry_run: bool,
    #[arg(short, long)]
    pub(super) message: Option<String>,
}

#[derive(Args)]
pub(super) struct AgentFinishArgs {
    #[arg(default_value = "latest")]
    pub(super) selector: String,
    #[arg(long = "into-current-git-branch")]
    pub(super) into_current_git_branch: bool,
    #[arg(long)]
    pub(super) dry_run: bool,
    #[arg(short, long)]
    pub(super) message: Option<String>,
    #[arg(long, help = "Optional note to store with the archive marker")]
    pub(super) note: Option<String>,
}

#[derive(Args)]
pub(super) struct AgentRewindArgs {
    #[arg(
        default_value = "latest",
        help = "Agent task, lane, session, ACP session, or latest"
    )]
    pub(super) selector: String,
    #[arg(
        long = "to",
        value_name = "CHECKPOINT_OR_LABEL",
        help = "Checkpoint/change id or friendly label such as before-last-turn, turn:2, before-turn:2, before-prompt:<text>, or before-last-operation"
    )]
    pub(super) target: String,
}

#[derive(Args)]
pub(super) struct AgentUndoArgs {
    #[arg(
        default_value = "latest",
        help = "Agent task, lane, session, ACP session, or latest"
    )]
    pub(super) selector: String,
    #[arg(
        long = "last-turn",
        conflicts_with_all = ["turn", "prompt", "last_operation"],
        help = "Undo the latest completed turn; this is the default when no target flag is provided"
    )]
    pub(super) last_turn: bool,
    #[arg(
        long,
        value_name = "N_OR_TURN_ID",
        conflicts_with_all = ["last_turn", "prompt", "last_operation"],
        help = "Undo one 1-based turn index or turn id"
    )]
    pub(super) turn: Option<String>,
    #[arg(
        long,
        value_name = "TEXT",
        conflicts_with_all = ["last_turn", "turn", "last_operation"],
        help = "Undo the latest prompt containing this text"
    )]
    pub(super) prompt: Option<String>,
    #[arg(
        long = "last-operation",
        conflicts_with_all = ["last_turn", "turn", "prompt"],
        help = "Undo the latest recorded operation when no turn transcript exists"
    )]
    pub(super) last_operation: bool,
}

#[derive(Args)]
pub(super) struct AgentDoctorArgs {
    #[arg(value_name = "PROVIDER", conflicts_with = "provider_flag")]
    pub(super) provider: Option<String>,
    #[arg(
        long = "provider",
        value_name = "PROVIDER",
        conflicts_with = "provider"
    )]
    pub(super) provider_flag: Option<String>,
}
