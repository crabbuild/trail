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
    #[serde(default)]
    pub rich_text_hydrated: u64,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorktreeIndexReport {
    pub files: u64,
    pub indexed_entries: u64,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExternalMutationAuditRecord {
    pub audit_id: String,
    pub surface: String,
    pub command: String,
    pub target_ref: Option<String>,
    pub lane_id: Option<String>,
    pub status: String,
    pub status_code: Option<i64>,
    pub change_id: Option<ChangeId>,
    pub summary: Option<serde_json::Value>,
    pub created_at: i64,
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
