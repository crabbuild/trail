#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LaneWorkdirMode {
    Virtual,
    Sparse,
    FullCow,
    OverlayCow,
    NfsCow,
}

impl LaneWorkdirMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            LaneWorkdirMode::Virtual => "virtual",
            LaneWorkdirMode::Sparse => "sparse",
            LaneWorkdirMode::FullCow => "full-cow",
            LaneWorkdirMode::OverlayCow => "overlay-cow",
            LaneWorkdirMode::NfsCow => "nfs-cow",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "virtual" => Some(LaneWorkdirMode::Virtual),
            "sparse" => Some(LaneWorkdirMode::Sparse),
            "full-cow" | "full_cow" => Some(LaneWorkdirMode::FullCow),
            "overlay-cow" | "overlay_cow" => Some(LaneWorkdirMode::OverlayCow),
            "nfs-cow" | "nfs_cow" => Some(LaneWorkdirMode::NfsCow),
            _ => None,
        }
    }

    pub fn materializes(&self) -> bool {
        !matches!(self, LaneWorkdirMode::Virtual)
    }

    pub fn cow_backend(&self) -> Option<&'static str> {
        match self {
            LaneWorkdirMode::Virtual => None,
            LaneWorkdirMode::Sparse | LaneWorkdirMode::FullCow => Some("filesystem-clone"),
            LaneWorkdirMode::OverlayCow => Some("overlay"),
            LaneWorkdirMode::NfsCow => Some("nfs-overlay"),
        }
    }

    pub fn is_transparent_cow(&self) -> bool {
        matches!(self, LaneWorkdirMode::OverlayCow | LaneWorkdirMode::NfsCow)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneSpawnReport {
    pub lane_id: String,
    pub ref_name: String,
    pub base_change: ChangeId,
    pub workdir: Option<String>,
    pub workdir_mode: LaneWorkdirMode,
    pub cow_backend: Option<String>,
    pub sparse_paths: Vec<String>,
    pub overlay_available: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LaneWorkspaceViewReport {
    pub view_id: String,
    pub lane_id: String,
    pub base_change: ChangeId,
    pub base_root: ObjectId,
    pub backend: String,
    pub mountpoint: String,
    pub source_upper: String,
    pub generated_upper: String,
    pub scratch_upper: String,
    pub meta_dir: String,
    pub journal_path: String,
    pub generation: u64,
    pub checkpoint_seq: u64,
    pub checkpoint_root: Option<ObjectId>,
    pub status: String,
    pub owner_pid: Option<u32>,
    pub owner_start_token: Option<String>,
    pub heartbeat_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceLayerReport {
    pub layer_id: String,
    pub kind: String,
    pub cache_key: String,
    pub adapter: String,
    pub state: String,
    pub storage_path: String,
    pub logical_bytes: u64,
    pub physical_bytes: Option<u64>,
    pub entry_count: u64,
    pub portability_scope: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceEnvironmentReport {
    pub view_id: String,
    pub adapter: String,
    pub expected_key: String,
    pub attached_key: Option<String>,
    pub status: String,
    pub reason: Option<String>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceCheckpointReport {
    pub view_id: String,
    pub operation: Option<ChangeId>,
    pub root_id: ObjectId,
    pub journal_sequence: u64,
    pub source_paths: Vec<String>,
    pub generated_dirty_paths: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceSpaceReport {
    pub view_id: String,
    pub logical_visible_bytes: u64,
    pub shared_physical_bytes: u64,
    pub lane_exclusive_physical_bytes: u64,
    pub shared_extent_bytes: Option<u64>,
    pub reclaimable_cache_bytes: u64,
    pub uncheckpointed_source_bytes: u64,
    pub generated_upper_bytes: u64,
    pub scratch_upper_bytes: u64,
    pub physical_accounting: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceMountReport {
    pub view_id: String,
    pub backend: String,
    pub mountpoint: String,
    pub generation: u64,
    pub owner_pid: Option<u32>,
    pub owner_start_token: Option<String>,
    pub healthy: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceExecReport {
    pub view_id: String,
    pub lane_id: String,
    pub source_root: ObjectId,
    pub generation: u64,
    pub backend: String,
    pub command: Vec<String>,
    pub exit_code: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceLayerKeyV1 {
    pub kind: String,
    pub adapter: String,
    pub adapter_version: u32,
    pub inputs: std::collections::BTreeMap<String, String>,
    pub tool_versions: std::collections::BTreeMap<String, String>,
    pub platform: String,
    pub architecture: String,
    pub portability_scope: String,
    pub strategy: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceGitShadowReport {
    pub view_id: String,
    pub git_dir: String,
    pub work_tree: String,
    pub policy: String,
    pub pinned_head: String,
    pub current_head: String,
    pub status: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceQuotaReport {
    pub view_id: String,
    pub upper_logical_bytes: u64,
    pub upper_file_count: u64,
    pub largest_file_bytes: u64,
    pub journal_bytes: u64,
    pub cache_physical_bytes: u64,
    pub exceeded: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceCacheGcEntry {
    pub kind: String,
    pub id: String,
    pub path: String,
    pub physical_bytes: u64,
    pub pinned: bool,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceCacheGcReport {
    pub dry_run: bool,
    pub retention_secs: u64,
    pub cache_physical_bytes_before: u64,
    pub reclaimable_bytes: u64,
    pub reclaimed_bytes: u64,
    pub candidates: Vec<WorkspaceCacheGcEntry>,
    pub deleted: Vec<WorkspaceCacheGcEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LanePatchReport {
    pub lane_id: String,
    pub operation: ChangeId,
    pub root_id: ObjectId,
    pub changed_paths: Vec<FileDiffSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneRecordReport {
    pub lane_id: String,
    pub operation: Option<ChangeId>,
    pub root_id: ObjectId,
    pub changed_paths: Vec<FileDiffSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneRecordPreviewReport {
    pub lane_id: String,
    pub workdir: String,
    pub head_change: ChangeId,
    pub root_id: ObjectId,
    pub clean: bool,
    pub changed_paths: Vec<FileDiffSummary>,
    pub ignored_paths: Vec<LaneWorkdirIgnoredPath>,
    pub risky_paths: Vec<LaneWorkdirRisk>,
    pub oversized_files: Vec<LaneRecordOversizedFile>,
    pub policy: LaneRecordPolicyPreview,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneWorkdirIgnoredPath {
    pub path: String,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneWorkdirRisk {
    pub path: String,
    pub kind: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneRecordOversizedFile {
    pub path: String,
    pub size_bytes: u64,
    pub limit_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneRecordPolicyPreview {
    pub allowed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneRefreshPreviewReport {
    pub lane_id: String,
    pub ref_name: String,
    pub base_change: ChangeId,
    pub lane_head_change: ChangeId,
    pub lane_head_root: ObjectId,
    pub target_ref: String,
    pub target_change: ChangeId,
    pub target_root: ObjectId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operations_behind: Option<u64>,
    pub clean: bool,
    pub conflicted: bool,
    pub changed_paths: Vec<FileDiffSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
    pub next_steps: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneRewindReport {
    pub lane_id: String,
    pub ref_name: String,
    pub target: String,
    pub previous_change: ChangeId,
    pub previous_root: ObjectId,
    pub target_change: ChangeId,
    pub target_root: ObjectId,
    pub operation: ChangeId,
    pub root_id: ObjectId,
    pub changed_paths: Vec<FileDiffSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_current: Option<ChangeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserved_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserved_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
    pub workdir_synced: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneWorkdirReport {
    pub lane_id: String,
    pub workdir: Option<String>,
    pub workdir_mode: LaneWorkdirMode,
    pub cow_backend: Option<String>,
    pub sparse_paths: Vec<String>,
    pub overlay_available: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneWorkdirSyncReport {
    pub lane_id: String,
    pub workdir: String,
    pub head_change: ChangeId,
    pub root_id: ObjectId,
    pub forced: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rescue_workdir: Option<String>,
    pub changed_paths: Vec<FileDiffSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneWatchReport {
    pub lane_id: String,
    pub iterations: u64,
    pub recorded_operations: Vec<ChangeId>,
    pub changed_paths: Vec<FileDiffSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneTestReport {
    pub lane_id: String,
    pub turn_id: String,
    pub session_id: Option<String>,
    pub workdir: String,
    pub source_root: ObjectId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub environment_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layer_ids: Vec<String>,
    pub command: Vec<String>,
    #[serde(default = "default_lane_gate_kind")]
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
pub struct LaneTestSummary {
    pub event_id: String,
    pub turn_id: Option<String>,
    #[serde(default = "default_lane_gate_kind")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_root: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub environment_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layer_ids: Vec<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneGateHistoryReport {
    pub lane: LaneDetails,
    pub kind: String,
    pub limit: usize,
    pub gates: Vec<LaneTestSummary>,
}

fn default_lane_gate_kind() -> String {
    "test".to_string()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LaneGateOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
}
