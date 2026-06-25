
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
