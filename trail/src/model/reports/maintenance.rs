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
    /// Immutable roots upgraded with the persistent path-invariant index.
    #[serde(default)]
    pub path_index_repaired_roots: Vec<PathIndexRootRepair>,
    /// Mutable branch/lane refs advanced to equivalent indexed roots.
    #[serde(default)]
    pub path_index_repaired_refs: Vec<PathIndexRefRepair>,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PathIndexRootRepair {
    pub old_root: ObjectId,
    pub new_root: ObjectId,
    pub case_fold_map_root: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PathIndexRefRepair {
    pub name: String,
    pub old_change: ChangeId,
    pub new_change: ChangeId,
    pub old_root: ObjectId,
    pub new_root: ObjectId,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorktreeIndexReport {
    pub files: u64,
    pub indexed_entries: u64,
    pub duration_ms: u64,
}

/// Result of a requested full changed-path ledger reconciliation.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChangeLedgerReconcileReport {
    pub scope_id: String,
    pub scope_kind: String,
    pub previous_state: String,
    pub reason: String,
    pub observed_paths: u64,
    pub candidates: u64,
    pub resulting_epoch: u64,
    pub resulting_state: String,
    #[serde(skip)]
    pub(crate) mode: String,
    #[serde(skip)]
    pub(crate) observed_files: u64,
    #[serde(skip)]
    pub(crate) staged_rows: u64,
    #[serde(skip)]
    pub(crate) observed_candidates: u64,
    #[serde(skip)]
    pub(crate) candidate_rows: u64,
    #[serde(skip)]
    pub(crate) hashed_bytes: u64,
    #[serde(skip)]
    pub(crate) peak_batch_rows: u64,
    #[serde(skip)]
    pub(crate) peak_buffer_bytes: u64,
    #[serde(skip)]
    pub(crate) start_sequence: u64,
    #[serde(skip)]
    pub(crate) end_sequence: u64,
    #[serde(skip)]
    pub(crate) start_durable_offset: u64,
    #[serde(skip)]
    pub(crate) end_durable_offset: u64,
    #[serde(skip)]
    pub(crate) refreshed: bool,
    #[serde(skip)]
    pub(crate) published: bool,
    #[serde(skip)]
    pub(crate) trust_state: String,
    #[serde(skip)]
    pub(crate) retries: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuredRecovery {
    pub command: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuredErrorDetails {
    pub code: String,
    pub status: u16,
    pub exit: i32,
    pub message: String,
    pub scope: Option<String>,
    pub state: Option<String>,
    pub reason: Option<String>,
    pub recovery: Option<StructuredRecovery>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuredErrorEnvelope {
    pub error: StructuredErrorDetails,
}

impl StructuredErrorEnvelope {
    pub fn from_error(error: &crate::Error) -> Self {
        let status = match error {
            crate::Error::RefNotFound(_)
            | crate::Error::OperationNotFound(_)
            | crate::Error::RootNotFound(_) => 404,
            crate::Error::Conflict(_)
            | crate::Error::DirtyWorktree
            | crate::Error::DirtyWorktreeWithMessage(_)
            | crate::Error::PatchRejected(_)
            | crate::Error::StaleBranch(_)
            | crate::Error::WorkspaceLocked(_)
            | crate::Error::SchemaReinitializeRequired { .. }
            | crate::Error::ChangeLedgerReconcileRequired { .. }
            | crate::Error::CommittedRepairRequired { .. }
            | crate::Error::LaneInitializationConflict { .. } => 409,
            crate::Error::InvalidInput(_)
            | crate::Error::InvalidPath { .. }
            | crate::Error::IgnoredPath(_)
            | crate::Error::Json(_) => 400,
            _ => 500,
        };
        let (scope, state, reason, command) = match error {
            crate::Error::ChangeLedgerReconcileRequired {
                scope,
                state,
                reason,
                command,
            } => (
                Some(scope.clone()),
                Some(state.clone()),
                Some(reason.clone()),
                Some(command.clone()),
            ),
            crate::Error::SchemaReinitializeRequired { found, guidance } => (
                None,
                Some("reinitialize_required".to_string()),
                Some(format!("{found}; {guidance}")),
                Some("trail init --force".to_string()),
            ),
            crate::Error::CommittedRepairRequired { reason, repair, .. } => (
                None,
                Some("repair_required".to_string()),
                Some(reason.clone()),
                Some(repair.clone()),
            ),
            _ => (None, None, None, None),
        };
        let details = match error {
            crate::Error::LaneInitializationConflict {
                lane,
                existing_fingerprint,
                requested_fingerprint,
            } => Some(serde_json::json!({
                "lane": lane,
                "existing_fingerprint": existing_fingerprint,
                "requested_fingerprint": requested_fingerprint,
            })),
            crate::Error::CommittedRepairRequired {
                lane,
                initialization_id,
                request_fingerprint,
                operation_id,
                phase,
                committed,
                repair,
                reason,
            } => Some(serde_json::json!({
                "lane": lane,
                "initialization_id": initialization_id,
                "request_fingerprint": request_fingerprint,
                "operation_id": operation_id,
                "phase": phase,
                "committed": committed,
                "repair": repair,
                "reason": reason,
            })),
            _ => None,
        };
        Self {
            error: StructuredErrorDetails {
                code: error.code().to_string(),
                status,
                exit: error.exit_code(),
                message: error.to_string(),
                scope,
                state,
                reason,
                recovery: command.map(|command| StructuredRecovery { command }),
                details,
            },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExternalMutationAuditRecord {
    pub audit_id: String,
    pub actor: String,
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
    pub restored_trailignore: bool,
    pub rewritten_workdirs: u64,
    pub checked_refs: u64,
    pub checked_roots: u64,
    pub checked_texts: u64,
}

#[cfg(test)]
mod maintenance_tests {
    use super::*;

    #[test]
    fn lane_initialization_conflict_has_shared_status_and_identity_details() {
        let error = crate::Error::LaneInitializationConflict {
            lane: "agent-1".into(),
            existing_fingerprint: "sha256:existing".into(),
            requested_fingerprint: "sha256:requested".into(),
        };
        let value = serde_json::to_value(StructuredErrorEnvelope::from_error(&error)).unwrap();

        assert_eq!(value["error"]["code"], "LANE_INITIALIZATION_CONFLICT");
        assert_eq!(value["error"]["status"], 409);
        assert_eq!(value["error"]["exit"], 2);
        assert_eq!(value["error"]["details"]["lane"], "agent-1");
        assert_eq!(
            value["error"]["details"]["existing_fingerprint"],
            "sha256:existing"
        );
        assert_eq!(
            value["error"]["details"]["requested_fingerprint"],
            "sha256:requested"
        );
    }

    #[test]
    fn legacy_index_rebuild_report_defaults_path_index_repairs() {
        let report: IndexRebuildReport = serde_json::from_value(serde_json::json!({
            "operations": 1,
            "operation_parents": 0,
            "file_history_rows": 0,
            "line_history_rows": 0,
            "messages": 0,
            "errors": []
        }))
        .unwrap();

        assert!(report.path_index_repaired_roots.is_empty());
        assert!(report.path_index_repaired_refs.is_empty());
    }
}
