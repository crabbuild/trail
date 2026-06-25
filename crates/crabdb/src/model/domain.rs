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
