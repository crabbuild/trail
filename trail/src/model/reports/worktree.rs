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
    #[serde(default)]
    pub performance: GitHandoffMetricsReport,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GitHandoffMetricsReport {
    pub export_mode: String,
    pub changed_path_count: u64,
    pub blob_write_count: u64,
    #[serde(default)]
    pub git_plumbing_command_count: u64,
    pub tracked_status_count: u64,
    pub full_root_file_count: u64,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_handoff_metrics_defaults_new_plumbing_counter_for_legacy_json() {
        let report: GitHandoffMetricsReport = serde_json::from_value(serde_json::json!({
            "export_mode": "mapped_delta",
            "changed_path_count": 2,
            "blob_write_count": 2,
            "tracked_status_count": 1,
            "full_root_file_count": 0
        }))
        .unwrap();

        assert_eq!(report.git_plumbing_command_count, 0);
    }
}
