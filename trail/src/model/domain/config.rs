pub const WORKTREE_ROOT_KIND: &str = "WorktreeRoot";
pub const TEXT_CONTENT_KIND: &str = "TextContent";
pub const OPERATION_KIND: &str = "Operation";
pub const BLOB_KIND: &str = "Blob";
pub const MESSAGE_KIND: &str = "Message";
pub const CONFLICT_SET_KIND: &str = "ConflictSet";
pub const ANCHOR_KIND: &str = "Anchor";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrailConfig {
    pub workspace: WorkspaceConfig,
    #[serde(default)]
    pub agent: AgentConfig,
    pub recording: RecordingConfig,
    pub text: TextConfig,
    pub lane: LaneConfig,
    pub git: GitConfig,
    #[serde(default = "default_storage_config")]
    pub storage: StorageConfig,
    #[serde(default = "default_guardrails_config")]
    pub guardrails: GuardrailsConfig,
    #[serde(default = "default_workspace_views_config")]
    pub workspace_views: WorkspaceViewsConfig,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(default)]
    pub default_provider: Option<String>,
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
pub struct LaneConfig {
    pub default_materialize: bool,
    #[serde(default)]
    pub require_test_gate: bool,
    #[serde(default)]
    pub require_eval_gate: bool,
    #[serde(default)]
    pub required_test_suites: Vec<String>,
    #[serde(default)]
    pub required_eval_suites: Vec<String>,
    #[serde(default = "default_lane_claim_enforcement")]
    pub claim_enforcement: String,
    #[serde(default)]
    pub enforce_sparse_paths: bool,
    #[serde(default)]
    pub max_patch_bytes: u64,
    #[serde(default)]
    pub max_patch_file_bytes: u64,
    #[serde(default)]
    pub max_changed_paths: u64,
    #[serde(default)]
    pub max_event_payload_bytes: u64,
    #[serde(default)]
    pub max_trace_payload_bytes: u64,
    pub worktrees_dir: String,
    pub merge_strategy: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitConfig {
    pub export_trailers: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_prolly_backend")]
    pub prolly_backend: String,
    #[serde(default = "default_slatedb_path")]
    pub slatedb_path: String,
    #[serde(default = "default_slatedb_s3_endpoint")]
    pub slatedb_s3_endpoint: String,
    #[serde(default = "default_slatedb_s3_bucket")]
    pub slatedb_s3_bucket: String,
    #[serde(default = "default_slatedb_s3_region")]
    pub slatedb_s3_region: String,
    #[serde(default = "default_slatedb_s3_access_key_id")]
    pub slatedb_s3_access_key_id: String,
    #[serde(default = "default_slatedb_s3_secret_access_key")]
    pub slatedb_s3_secret_access_key: String,
    #[serde(default = "default_slatedb_s3_allow_http")]
    pub slatedb_s3_allow_http: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuardrailsConfig {
    pub policy: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceViewsConfig {
    /// Zero disables the corresponding limit.
    pub upper_logical_bytes: u64,
    pub upper_file_count: u64,
    pub single_file_bytes: u64,
    pub journal_bytes: u64,
    pub cache_build_bytes: u64,
    pub concurrent_cache_builders: u64,
    pub cache_retention_secs: u64,
    pub cache_max_bytes: u64,
}

fn default_storage_config() -> StorageConfig {
    StorageConfig {
        prolly_backend: default_prolly_backend(),
        slatedb_path: default_slatedb_path(),
        slatedb_s3_endpoint: default_slatedb_s3_endpoint(),
        slatedb_s3_bucket: default_slatedb_s3_bucket(),
        slatedb_s3_region: default_slatedb_s3_region(),
        slatedb_s3_access_key_id: default_slatedb_s3_access_key_id(),
        slatedb_s3_secret_access_key: default_slatedb_s3_secret_access_key(),
        slatedb_s3_allow_http: default_slatedb_s3_allow_http(),
    }
}

fn default_prolly_backend() -> String {
    "sqlite".to_string()
}

fn default_slatedb_path() -> String {
    "trail/prolly".to_string()
}

fn default_slatedb_s3_endpoint() -> String {
    "http://localhost:9000".to_string()
}

fn default_slatedb_s3_bucket() -> String {
    "crab".to_string()
}

fn default_slatedb_s3_region() -> String {
    "us-east-1".to_string()
}

fn default_slatedb_s3_access_key_id() -> String {
    "crab".to_string()
}

fn default_slatedb_s3_secret_access_key() -> String {
    "crab".to_string()
}

fn default_slatedb_s3_allow_http() -> bool {
    true
}

fn default_guardrails_config() -> GuardrailsConfig {
    GuardrailsConfig {
        policy: String::new(),
    }
}

fn default_workspace_views_config() -> WorkspaceViewsConfig {
    WorkspaceViewsConfig {
        upper_logical_bytes: 0,
        upper_file_count: 0,
        single_file_bytes: 0,
        journal_bytes: 0,
        cache_build_bytes: 0,
        concurrent_cache_builders: 4,
        cache_retention_secs: 7 * 24 * 60 * 60,
        cache_max_bytes: 0,
    }
}

fn default_lane_claim_enforcement() -> String {
    "off".to_string()
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
    pub lane: Option<LaneDetails>,
    pub action: String,
    pub summary: Option<String>,
    pub decision: String,
    pub reasons: Vec<GuardrailReason>,
    pub path_checks: Vec<IgnoreCheckReport>,
    pub pending_approvals: Vec<LaneApproval>,
    #[serde(default)]
    pub satisfied_approvals: Vec<LaneApproval>,
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
    pub lane: Option<String>,
    pub action: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

impl TrailConfig {
    pub fn new(workspace_id: WorkspaceId, default_branch: impl Into<String>) -> Self {
        Self {
            workspace: WorkspaceConfig {
                id: workspace_id,
                default_branch: default_branch.into(),
            },
            agent: AgentConfig::default(),
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
            lane: LaneConfig {
                default_materialize: false,
                require_test_gate: false,
                require_eval_gate: false,
                required_test_suites: Vec::new(),
                required_eval_suites: Vec::new(),
                claim_enforcement: "off".to_string(),
                enforce_sparse_paths: false,
                max_patch_bytes: 0,
                max_patch_file_bytes: 0,
                max_changed_paths: 0,
                max_event_payload_bytes: 0,
                max_trace_payload_bytes: 0,
                worktrees_dir: ".trail/worktrees".to_string(),
                merge_strategy: "conservative".to_string(),
            },
            git: GitConfig {
                export_trailers: true,
            },
            storage: default_storage_config(),
            guardrails: default_guardrails_config(),
            workspace_views: default_workspace_views_config(),
        }
    }
}
