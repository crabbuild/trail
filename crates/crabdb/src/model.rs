use serde::{Deserialize, Serialize};

use crate::ids::{AnchorId, ChangeId, FileId, LineId, MessageId, ObjectId, WorkspaceId};

pub const WORKTREE_ROOT_KIND: &str = "WorktreeRoot";
pub const TEXT_CONTENT_KIND: &str = "TextContent";
pub const OPERATION_KIND: &str = "Operation";
pub const BLOB_KIND: &str = "Blob";
pub const MESSAGE_KIND: &str = "Message";
pub const CONFLICT_SET_KIND: &str = "ConflictSet";
pub const ANCHOR_KIND: &str = "Anchor";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrabConfig {
    pub workspace: WorkspaceConfig,
    pub recording: RecordingConfig,
    pub text: TextConfig,
    pub agent: AgentConfig,
    pub git: GitConfig,
    #[serde(default = "default_guardrails_config")]
    pub guardrails: GuardrailsConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub id: WorkspaceId,
    pub default_branch: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordingConfig {
    pub mode: String,
    pub debounce_ms: u64,
    pub ignore_gitignored: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextConfig {
    pub small_text_max_bytes: u64,
    pub tree_text_min_bytes: u64,
    pub opaque_text_max_bytes: u64,
    pub max_line_bytes: u64,
    pub preserve_similarity: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentConfig {
    pub default_materialize: bool,
    #[serde(default)]
    pub require_test_gate: bool,
    #[serde(default)]
    pub require_eval_gate: bool,
    #[serde(default)]
    pub required_test_suites: Vec<String>,
    #[serde(default)]
    pub required_eval_suites: Vec<String>,
    pub worktrees_dir: String,
    pub merge_strategy: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitConfig {
    pub export_trailers: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuardrailsConfig {
    pub policy: String,
}

fn default_guardrails_config() -> GuardrailsConfig {
    GuardrailsConfig {
        policy: String::new(),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigEntry {
    pub key: String,
    pub value: String,
    pub value_type: String,
    pub read_only: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigSetReport {
    pub key: String,
    pub old_value: String,
    pub new_value: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IgnorePattern {
    pub line: usize,
    pub pattern: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IgnoreListReport {
    pub path: String,
    pub patterns: Vec<IgnorePattern>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IgnoreAddReport {
    pub path: String,
    pub pattern: String,
    pub added: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IgnoreRemoveReport {
    pub path: String,
    pub pattern: String,
    pub removed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IgnoreCheckReport {
    pub path: String,
    pub ignored: bool,
    pub source: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuardrailCheckReport {
    pub agent: Option<AgentDetails>,
    pub action: String,
    pub summary: Option<String>,
    pub decision: String,
    pub reasons: Vec<GuardrailReason>,
    pub path_checks: Vec<IgnoreCheckReport>,
    pub pending_approvals: Vec<AgentApproval>,
    #[serde(default)]
    pub satisfied_approvals: Vec<AgentApproval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_request: Option<GuardrailApprovalRequest>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuardrailReason {
    pub code: String,
    pub severity: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuardrailApprovalRequest {
    pub agent: Option<String>,
    pub action: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

impl CrabConfig {
    pub fn new(workspace_id: WorkspaceId, default_branch: impl Into<String>) -> Self {
        Self {
            workspace: WorkspaceConfig {
                id: workspace_id,
                default_branch: default_branch.into(),
            },
            recording: RecordingConfig {
                mode: "save".to_string(),
                debounce_ms: 500,
                ignore_gitignored: true,
            },
            text: TextConfig {
                small_text_max_bytes: 32 * 1024,
                tree_text_min_bytes: 32 * 1024 + 1,
                opaque_text_max_bytes: 10 * 1024 * 1024,
                max_line_bytes: 1024 * 1024,
                preserve_similarity: 0.45,
            },
            agent: AgentConfig {
                default_materialize: true,
                require_test_gate: false,
                require_eval_gate: false,
                required_test_suites: Vec::new(),
                required_eval_suites: Vec::new(),
                worktrees_dir: ".crabdb/worktrees".to_string(),
                merge_strategy: "conservative".to_string(),
            },
            git: GitConfig {
                export_trailers: true,
            },
            guardrails: default_guardrails_config(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredObject {
    pub id: ObjectId,
    pub kind: String,
    pub version: u16,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorktreeRoot {
    pub version: u16,
    pub path_map_root: Option<String>,
    pub file_index_map_root: Option<String>,
    pub file_count: u64,
    pub total_text_bytes: u64,
    pub created_by: ChangeId,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileEntry {
    pub file_id: FileId,
    pub kind: FileKind,
    pub mode: u32,
    pub executable: bool,
    pub content: FileContentRef,
    pub size_bytes: u64,
    pub content_hash: String,
    pub created_by: ChangeId,
    pub last_content_change: ChangeId,
    pub last_path_change: Option<ChangeId>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileKind {
    Text,
    OpaqueText,
    Binary,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileContentRef {
    Text(ObjectId),
    Opaque(ObjectId),
    Binary(ObjectId),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TextContent {
    pub version: u16,
    pub content_hash: String,
    pub line_count: u64,
    pub byte_count: u64,
    pub order_map_root: Option<String>,
    pub line_index_map_root: Option<String>,
    pub representation: TextRepresentation,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum TextRepresentation {
    TreeText,
    OpaqueText {
        blob_id: ObjectId,
        reason: OpaqueReason,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum OpaqueReason {
    TooLarge,
    LineTooLong,
    InvalidUtf8,
    BinaryLike,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineEntry {
    pub line_id: LineId,
    pub text: Vec<u8>,
    pub newline: NewlineKind,
    pub text_hash: String,
    pub introduced_by: ChangeId,
    pub last_content_change: ChangeId,
    pub last_move_change: Option<ChangeId>,
    pub flags: LineFlags,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum NewlineKind {
    None,
    Lf,
    Crlf,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineFlags {
    pub generated: bool,
    pub redacted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Blob {
    pub version: u16,
    pub content_hash: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Operation {
    pub version: u16,
    pub change_id: ChangeId,
    pub kind: OperationKind,
    pub parents: Vec<ChangeId>,
    pub before_root: Option<ObjectId>,
    pub after_root: ObjectId,
    pub branch: String,
    pub actor: Actor,
    pub session_id: Option<String>,
    pub message: Option<String>,
    pub changes: Vec<FileChange>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum OperationKind {
    Init,
    GitImport,
    FileEdit,
    MultiFileEdit,
    Format,
    ManualCheckpoint,
    ManualRecord,
    WatchRecord,
    Checkout,
    Branch,
    Merge,
    AgentSpawn,
    AgentPatch,
    AgentRecord,
    AgentMerge,
    GitExport,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Actor {
    pub kind: ActorKind,
    pub id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ActorKind {
    Human,
    Agent,
    System,
}

impl Actor {
    pub fn human() -> Self {
        Self {
            kind: ActorKind::Human,
            id: std::env::var("USER").unwrap_or_else(|_| "human".to_string()),
        }
    }

    pub fn system() -> Self {
        Self {
            kind: ActorKind::System,
            id: "crabdb".to_string(),
        }
    }

    pub fn agent(id: impl Into<String>) -> Self {
        Self {
            kind: ActorKind::Agent,
            id: id.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub old_path: Option<String>,
    pub file_id: Option<FileId>,
    pub kind: FileChangeKind,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub line_changes: Vec<LineChange>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    TypeChanged,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LineChange {
    pub line_id: LineId,
    pub kind: LineChangeKind,
    pub old_line_number: Option<u64>,
    pub new_line_number: Option<u64>,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum LineChangeKind {
    Added,
    Modified,
    Deleted,
    Moved,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RefRecord {
    pub name: String,
    pub change_id: ChangeId,
    pub root_id: ObjectId,
    pub operation_id: ObjectId,
    pub generation: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub version: u16,
    pub id: MessageId,
    pub role: String,
    pub body: String,
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub change_id: Option<ChangeId>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRecord {
    pub agent_id: String,
    pub name: String,
    pub kind: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub created_at: i64,
    pub metadata_json: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentBranch {
    pub agent_id: String,
    pub ref_name: String,
    pub base_change: ChangeId,
    pub head_change: ChangeId,
    pub base_root: ObjectId,
    pub head_root: ObjectId,
    pub session_id: Option<String>,
    pub workdir: Option<String>,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentDetails {
    pub record: AgentRecord,
    pub branch: AgentBranch,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentStatusReport {
    pub agent: AgentDetails,
    pub changed_paths: Vec<FileDiffSummary>,
    pub queued_merges: u64,
    pub workdir_state: Option<WorktreeState>,
    pub workdir_changed_paths: Vec<FileDiffSummary>,
    pub latest_test: Option<AgentTestSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_eval: Option<AgentTestSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentContributionReport {
    pub status: AgentStatusReport,
    pub operations: Vec<TimelineEntry>,
    pub sessions: Vec<AgentSession>,
    pub recent_events: Vec<AgentEventRecord>,
    pub approvals: Vec<AgentApproval>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentReadinessReport {
    pub agent: AgentDetails,
    pub ready: bool,
    pub status: String,
    pub blockers: Vec<AgentReadinessIssue>,
    pub warnings: Vec<AgentReadinessIssue>,
    pub changed_paths: Vec<FileDiffSummary>,
    pub workdir_state: Option<WorktreeState>,
    pub workdir_changed_paths: Vec<FileDiffSummary>,
    pub queued_merges: u64,
    pub pending_approvals: Vec<AgentApproval>,
    pub conflicts: Vec<ConflictSetSummary>,
    pub latest_test: Option<AgentTestSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_eval: Option<AgentTestSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentReadinessIssue {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentHandoffReport {
    pub agent: AgentDetails,
    pub readiness: AgentReadinessReport,
    pub current_session: Option<AgentSessionDetails>,
    pub recent_sessions: Vec<AgentSession>,
    pub recent_events: Vec<AgentEventRecord>,
    pub recent_spans: Vec<AgentTraceSpan>,
    pub recent_operations: Vec<TimelineEntry>,
    pub next_steps: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentMessageReport {
    pub agent_id: String,
    pub message_id: MessageId,
    pub role: String,
    pub session_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentSession {
    pub session_id: String,
    pub agent_id: String,
    pub title: Option<String>,
    pub status: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub metadata_json: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentSessionContextReport {
    pub session: AgentSession,
    pub message_count: u64,
    pub event_count: u64,
    pub turn_count: u64,
    pub operation_count: u64,
    pub recent_messages: Vec<Message>,
    pub recent_events: Vec<AgentEventRecord>,
    pub recent_turns: Vec<AgentTurn>,
    pub recent_operations: Vec<TimelineEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTurn {
    pub turn_id: String,
    pub agent_id: String,
    pub session_id: Option<String>,
    pub base_change: ChangeId,
    pub before_change: ChangeId,
    pub after_change: Option<ChangeId>,
    pub status: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub metadata_json: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTurnStartReport {
    pub turn: AgentTurn,
    pub session: AgentSession,
    pub base_root: ObjectId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTurnDetails {
    pub turn: AgentTurn,
    pub session: Option<AgentSession>,
    pub messages: Vec<Message>,
    pub events: Vec<AgentEventRecord>,
    pub operations: Vec<TimelineEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTurnEventReport {
    pub event: AgentEventRecord,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTurnEndReport {
    pub turn: AgentTurn,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentEventRecord {
    pub event_id: String,
    pub agent_id: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub event_type: String,
    pub change_id: Option<ChangeId>,
    pub message_id: Option<MessageId>,
    pub payload: Option<serde_json::Value>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTraceSpan {
    pub span_id: String,
    pub trace_id: String,
    pub agent_id: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub parent_span_id: Option<String>,
    pub span_type: String,
    pub name: String,
    pub status: String,
    pub started_event_id: String,
    pub ended_event_id: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub duration_ms: Option<u64>,
    pub attributes: Option<serde_json::Value>,
    pub result: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTraceSummaryReport {
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub trace_id: Option<String>,
    pub span_count: u64,
    pub open_span_count: u64,
    pub ended_span_count: u64,
    pub failed_span_count: u64,
    pub total_duration_ms: u64,
    pub max_duration_ms: u64,
    pub average_duration_ms: Option<f64>,
    pub status_counts: Vec<NamedCount>,
    pub span_type_counts: Vec<NamedCount>,
    pub trace_counts: Vec<NamedCount>,
    pub slowest_spans: Vec<AgentTraceSpan>,
    pub open_spans: Vec<AgentTraceSpan>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NamedCount {
    pub name: String,
    pub count: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTraceSpanStartReport {
    pub span: AgentTraceSpan,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTraceSpanEndReport {
    pub span: AgentTraceSpan,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentApproval {
    pub approval_id: String,
    pub agent_id: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub action: String,
    pub summary: String,
    pub payload: Option<serde_json::Value>,
    pub status: String,
    pub requested_at: i64,
    pub decided_at: Option<i64>,
    pub reviewer: Option<String>,
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentApprovalRequestReport {
    pub approval: AgentApproval,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentApprovalDecisionReport {
    pub approval: AgentApproval,
    pub decision: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentSessionStartReport {
    pub session: AgentSession,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentSessionCurrentReport {
    pub agent_id: String,
    pub agent_name: String,
    pub ref_name: String,
    pub session: Option<AgentSession>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentSessionEndReport {
    pub session: AgentSession,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentSessionDetails {
    pub session: AgentSession,
    pub turns: Vec<AgentTurn>,
    pub messages: Vec<Message>,
    pub events: Vec<AgentEventRecord>,
    pub operations: Vec<TimelineEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRemoveReport {
    pub agent_id: String,
    pub ref_name: String,
    pub removed_workdir: Option<String>,
    pub forced: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LeaseRecord {
    pub lease_id: String,
    pub agent_id: String,
    pub ref_name: String,
    pub path: Option<String>,
    pub file_id: Option<String>,
    pub mode: String,
    pub expires_at: i64,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentClaimReport {
    pub agent_id: String,
    pub ref_name: String,
    pub path: String,
    pub mode: String,
    pub ttl_secs: u64,
    pub claimed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<LeaseRecord>,
    #[serde(default)]
    pub conflicts: Vec<LeaseRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LeaseAcquireReport {
    pub lease: LeaseRecord,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LeaseReleaseReport {
    pub lease_id: String,
    pub released: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PatchDocument {
    pub base_change: Option<String>,
    pub message: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub allow_ignored: bool,
    pub edits: Vec<PatchEdit>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PatchEdit {
    Write {
        path: String,
        content: String,
        #[serde(default)]
        executable: bool,
    },
    WriteBytes {
        path: String,
        bytes_hex: String,
        #[serde(default)]
        executable: bool,
    },
    ReplaceLine {
        path: String,
        line_id: String,
        #[serde(default)]
        expected_text: Option<String>,
        new_text: String,
    },
    Delete {
        path: String,
    },
    Rename {
        from: String,
        to: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiffSummary {
    pub from: String,
    pub to: String,
    pub files: Vec<FileDiffSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileDiffSummary {
    pub path: String,
    pub old_path: Option<String>,
    pub kind: FileChangeKind,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub additions: u64,
    pub deletions: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub line_changes: Vec<LineChange>,
    pub patch: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StatusReport {
    pub branch: String,
    pub head: RefRecord,
    pub worktree_state: WorktreeState,
    pub changed_paths: Vec<FileDiffSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorktreeState {
    Clean,
    DirtyTracked,
    DirtyUntracked,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub change_id: ChangeId,
    pub kind: OperationKind,
    pub branch: String,
    pub actor_id: String,
    pub message: Option<String>,
    pub created_at: i64,
    pub path_count: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WhyResult {
    pub path: String,
    pub line_number: u64,
    pub file_id: FileId,
    pub line_id: LineId,
    pub current_text: String,
    pub introduced_by: ChangeId,
    pub last_content_change: ChangeId,
    pub last_move_change: Option<ChangeId>,
    pub history: Vec<LineHistoryEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LineHistoryEntry {
    pub change_id: ChangeId,
    pub path: String,
    pub line_number: Option<u64>,
    pub kind: LineChangeKind,
    pub text_hash: Option<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileHistoryEntry {
    pub file_id: String,
    pub change_id: ChangeId,
    pub path: String,
    pub old_path: Option<String>,
    pub kind: FileChangeKind,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryResult {
    pub selector: String,
    pub file_history: Vec<FileHistoryEntry>,
    pub line_history: Vec<LineHistoryEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ObjectInfo {
    pub object_id: ObjectId,
    pub kind: String,
    pub version: u16,
    pub size_bytes: u64,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ObjectInspectReport {
    pub info: ObjectInfo,
    pub summary: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RootInspectReport {
    pub root_id: ObjectId,
    pub root: WorktreeRoot,
    pub files: Vec<RootFileInspect>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RootFileInspect {
    pub path: String,
    pub file_id: String,
    pub kind: FileKind,
    pub mode: u32,
    pub executable: bool,
    pub size_bytes: u64,
    pub content_hash: String,
    pub content_object: ObjectId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextInspectReport {
    pub text_id: ObjectId,
    pub content: TextContent,
    pub lines: Vec<TextLineInspect>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextLineInspect {
    pub line_number: u64,
    pub line_id: String,
    pub text_hash: String,
    pub text: String,
    pub newline: NewlineKind,
    pub introduced_by: ChangeId,
    pub last_content_change: ChangeId,
    pub last_move_change: Option<ChangeId>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapRangeReport {
    pub map_id: String,
    pub map_type: String,
    pub start: Option<String>,
    pub end: Option<String>,
    pub entries: Vec<MapEntryInspect>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapDiffReport {
    pub left_map_id: String,
    pub right_map_id: String,
    pub map_type: String,
    pub start: Option<String>,
    pub end: Option<String>,
    pub changes: Vec<MapDiffInspect>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapEntryInspect {
    pub key: MapKeyInspect,
    pub value: MapValueInspect,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapDiffInspect {
    pub kind: String,
    pub key: MapKeyInspect,
    pub old_value: Option<MapValueInspect>,
    pub new_value: Option<MapValueInspect>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapKeyInspect {
    pub hex: String,
    pub text: Option<String>,
    pub summary: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MapValueInspect {
    pub bytes: usize,
    pub hex_preview: String,
    pub truncated: bool,
    pub text: Option<String>,
    pub summary: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperationShow {
    pub operation: Operation,
    pub changed_paths: Vec<FileDiffSummary>,
    pub messages: Vec<Message>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShowResult {
    Operation { value: OperationShow },
    Message { value: Message },
    Ref { value: RefRecord },
    Agent { value: AgentBranch },
    Object { value: ObjectInfo },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodeFromResult {
    pub selector: String,
    pub operations: Vec<CodeFromOperation>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodeFromOperation {
    pub change_id: ChangeId,
    pub kind: OperationKind,
    pub branch: String,
    pub actor_id: String,
    pub session_id: Option<String>,
    pub message: Option<String>,
    pub changed_paths: Vec<FileDiffSummary>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Anchor {
    pub version: u16,
    pub id: AnchorId,
    pub label: String,
    pub file_id: FileId,
    pub line_id: LineId,
    pub created_path: String,
    pub created_line: u64,
    pub created_change: ChangeId,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnchorCreateReport {
    pub anchor: Anchor,
    pub object_id: ObjectId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnchorResolveReport {
    pub anchor: Anchor,
    pub branch: String,
    pub status: String,
    pub path: Option<String>,
    pub line_number: Option<u64>,
    pub text: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnchorDeleteReport {
    pub anchor_id: AnchorId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DoctorReport {
    pub status: String,
    pub checks: Vec<DoctorCheck>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DoctorCheck {
    pub name: String,
    pub status: String,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FsckReport {
    pub checked_refs: u64,
    pub checked_roots: u64,
    pub checked_texts: u64,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IndexRebuildReport {
    pub operations: u64,
    pub operation_parents: u64,
    pub file_history_rows: u64,
    pub line_history_rows: u64,
    pub messages: u64,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GcReport {
    pub dry_run: bool,
    pub total_known_objects: u64,
    pub reachable_objects: u64,
    pub prunable_objects: u64,
    pub pruned_objects: u64,
    pub preserved_unknown_objects: u64,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupCreateReport {
    pub path: String,
    pub manifest_path: String,
    pub sqlite_path: String,
    pub workspace_id: WorkspaceId,
    pub branch: String,
    pub ref_count: u64,
    pub operation_count: u64,
    pub sqlite_bytes: u64,
    pub sqlite_sha256: String,
    pub worktree_bytes: u64,
    pub fsck_errors: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupVerifyReport {
    pub path: String,
    pub valid: bool,
    pub workspace_id: Option<WorkspaceId>,
    pub branch: Option<String>,
    pub checked_refs: u64,
    pub checked_roots: u64,
    pub checked_texts: u64,
    pub sqlite_bytes: Option<u64>,
    pub sqlite_sha256: Option<String>,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupRestoreReport {
    pub workspace: String,
    pub db_dir: String,
    pub backup_path: String,
    pub workspace_id: WorkspaceId,
    pub branch: String,
    pub replaced_existing: bool,
    pub restored_crabignore: bool,
    pub rewritten_workdirs: u64,
    pub checked_refs: u64,
    pub checked_roots: u64,
    pub checked_texts: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InitReport {
    pub workspace_id: WorkspaceId,
    pub branch: String,
    pub operation: ChangeId,
    pub root_id: ObjectId,
    pub imported: ImportStats,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ImportStats {
    pub files: u64,
    pub text: u64,
    pub opaque: u64,
    pub binary: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordReport {
    pub branch: String,
    pub operation: Option<ChangeId>,
    pub root_id: ObjectId,
    pub changed_paths: Vec<FileDiffSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitImportReport {
    pub branch: String,
    pub operation: Option<ChangeId>,
    pub root_id: ObjectId,
    pub imported: ImportStats,
    pub changed_paths: Vec<FileDiffSummary>,
    pub mapping: Option<GitMapping>,
}

#[derive(Clone, Debug, Default)]
pub struct RecordOptions {
    pub paths: Vec<String>,
    pub kind: Option<OperationKind>,
    pub session_id: Option<String>,
    pub allow_ignored: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitExportReport {
    pub range: String,
    pub branch: String,
    pub operation: ChangeId,
    pub root_id: ObjectId,
    pub commit: String,
    pub parent: Option<String>,
    pub mapping: Option<GitMapping>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitMapping {
    pub mapping_id: String,
    pub direction: String,
    pub branch: String,
    pub git_head: Option<String>,
    pub git_dirty: bool,
    pub crab_change: ChangeId,
    pub crab_root: ObjectId,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BranchReport {
    pub name: String,
    pub from: ChangeId,
    pub root_id: ObjectId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BranchListEntry {
    pub name: String,
    pub ref_name: String,
    pub change_id: ChangeId,
    pub root_id: ObjectId,
    pub generation: i64,
    pub is_current: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BranchDeleteReport {
    pub name: String,
    pub ref_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BranchRenameReport {
    pub old_name: String,
    pub new_name: String,
    pub change_id: ChangeId,
    pub root_id: ObjectId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckoutReport {
    pub change_id: ChangeId,
    pub root_id: ObjectId,
    pub written_files: u64,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_dirty: Option<ChangeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_root: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed_paths: Vec<FileDiffSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentSpawnReport {
    pub agent_id: String,
    pub ref_name: String,
    pub base_change: ChangeId,
    pub workdir: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentPatchReport {
    pub agent_id: String,
    pub operation: ChangeId,
    pub root_id: ObjectId,
    pub changed_paths: Vec<FileDiffSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRecordReport {
    pub agent_id: String,
    pub operation: Option<ChangeId>,
    pub root_id: ObjectId,
    pub changed_paths: Vec<FileDiffSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentWorkdirReport {
    pub agent_id: String,
    pub workdir: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentWorkdirSyncReport {
    pub agent_id: String,
    pub workdir: String,
    pub head_change: ChangeId,
    pub root_id: ObjectId,
    pub forced: bool,
    pub changed_paths: Vec<FileDiffSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentWatchReport {
    pub agent_id: String,
    pub iterations: u64,
    pub recorded_operations: Vec<ChangeId>,
    pub changed_paths: Vec<FileDiffSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTestReport {
    pub agent_id: String,
    pub turn_id: String,
    pub session_id: Option<String>,
    pub workdir: String,
    pub command: Vec<String>,
    #[serde(default = "default_agent_gate_kind")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    pub status: String,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub duration_ms: u64,
    pub stdout_object: ObjectId,
    pub stderr_object: ObjectId,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub stdout_preview: String,
    pub stderr_preview: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub started_event_id: String,
    pub finished_event_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTestSummary {
    pub event_id: String,
    pub turn_id: Option<String>,
    #[serde(default = "default_agent_gate_kind")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    pub status: String,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub duration_ms: u64,
    pub command: Vec<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentGateHistoryReport {
    pub agent: AgentDetails,
    pub kind: String,
    pub limit: usize,
    pub gates: Vec<AgentTestSummary>,
}

fn default_agent_gate_kind() -> String {
    "test".to_string()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AgentGateOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeReport {
    pub operation: ChangeId,
    pub source_ref: String,
    pub target_ref: String,
    pub root_id: ObjectId,
    #[serde(default)]
    pub dry_run: bool,
    pub changed_paths: Vec<FileDiffSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeQueueEntry {
    pub queue_id: String,
    pub source_ref: String,
    pub target_ref: String,
    pub status: String,
    pub priority: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeQueueAddReport {
    pub entry: MergeQueueEntry,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeQueueRemoveReport {
    pub entry: MergeQueueEntry,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeQueueRunReport {
    pub processed: Vec<MergeQueueRunItem>,
    pub stopped_on_conflict: bool,
    pub stopped_on_failure: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeQueueRunItem {
    pub queue_id: String,
    pub source_ref: String,
    pub target_ref: String,
    pub status: String,
    pub operation: Option<ChangeId>,
    pub changed_paths: Vec<FileDiffSummary>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictSetSummary {
    pub conflict_set_id: String,
    pub merge_id: Option<String>,
    pub source_ref: Option<String>,
    pub target_ref: Option<String>,
    pub status: String,
    pub details: Vec<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConflictManualResolution {
    #[serde(default)]
    pub files: std::collections::BTreeMap<String, ConflictManualFile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConflictManualFile {
    Text(String),
    Spec(ConflictManualFileSpec),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConflictManualFileSpec {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub delete: bool,
    #[serde(default)]
    pub executable: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConflictResolveReport {
    pub conflict_set_id: String,
    pub resolution: String,
    pub operation: ChangeId,
    pub target_ref: String,
    pub root_id: ObjectId,
    pub changed_paths: Vec<FileDiffSummary>,
}
