use std::path::PathBuf;

/// Trail result type.
pub type Result<T> = std::result::Result<T, Error>;

/// Stable error categories used by both the library and CLI.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("workspace not found from {0}")]
    WorkspaceNotFound(PathBuf),
    #[error("workspace already initialized at {0}")]
    WorkspaceExists(PathBuf),
    #[error("invalid path `{path}`: {reason}")]
    InvalidPath { path: String, reason: String },
    #[error("path invariant index is required: {0}")]
    PathIndexRequired(String),
    #[error("workspace schema {found} cannot be opened; {guidance}")]
    SchemaReinitializeRequired { found: String, guidance: String },
    #[error("changed-path ledger reconciliation required for {scope}: {reason}; run `{command}`")]
    ChangeLedgerReconcileRequired {
        scope: String,
        state: String,
        reason: String,
        command: String,
    },
    #[error("operation {operation} committed but {repair} repair is required: {reason}")]
    OperationCommittedRepairRequired {
        operation: String,
        repair: String,
        reason: String,
    },
    #[error("lane `{lane}` initialization {initialization_id} committed in phase {phase:?}; run `{repair}`: {reason}")]
    CommittedRepairRequired {
        lane: String,
        initialization_id: String,
        request_fingerprint: String,
        operation_id: String,
        phase: crate::model::LaneInitializationPhase,
        committed: bool,
        repair: String,
        reason: String,
    },
    #[error(
        "lane `{lane}` is already reserved by request {existing_fingerprint}; requested {requested_fingerprint}"
    )]
    LaneInitializationConflict {
        lane: String,
        existing_fingerprint: String,
        requested_fingerprint: String,
    },
    #[error("ignored path `{0}`")]
    IgnoredPath(String),
    #[error("ref not found: {0}")]
    RefNotFound(String),
    #[error("operation not found: {0}")]
    OperationNotFound(String),
    #[error("root not found: {0}")]
    RootNotFound(String),
    #[error("object not found: {kind} {id}")]
    ObjectNotFound { kind: &'static str, id: String },
    #[error("dirty worktree; record changes or pass --force")]
    DirtyWorktree,
    #[error("dirty worktree: {0}")]
    DirtyWorktreeWithMessage(String),
    #[error("workspace is locked by another writer: {0}")]
    WorkspaceLocked(String),
    #[error("merge conflict: {0}")]
    Conflict(String),
    #[error("patch rejected: {0}")]
    PatchRejected(String),
    #[error("stale branch `{0}`")]
    StaleBranch(String),
    #[error("database corrupt: {0}")]
    Corrupt(String),
    #[error("git interop failed: {0}")]
    Git(String),
    #[error("Git baseline mapping is required: {0}")]
    GitMappingRequired(String),
    #[error("Git HEAD changed during handoff: {0}")]
    GitHeadChanged(String),
    #[error("Git tracked worktree is dirty: {0}")]
    GitWorktreeDirty(String),
    #[error("mapped Git delta export is required: {0}")]
    GitDeltaExportRequired(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("native COW is unsupported for this source and destination")]
    CloneUnsupported,
    #[error("native COW source and destination are on different filesystems")]
    CloneCrossDevice,
    #[error("no complete validated filesystem source is available for native COW")]
    NativeCowSourceUnavailable,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("prolly error: {0}")]
    Prolly(#[from] prolly::Error),
    #[error("prolly SQLite error: {0}")]
    ProllySqlite(#[from] prolly_store_sqlite::SqliteStoreError),
    #[error("prolly SlateDB error: {0}")]
    ProllySlateDb(#[from] prolly_store_slatedb::SlateDbStoreError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("TOML error: {0}")]
    TomlSer(#[from] toml::ser::Error),
    #[error("TOML error: {0}")]
    TomlDe(#[from] toml::de::Error),
    #[error("daemon unavailable: {0}")]
    DaemonUnavailable(String),
    #[error("{message}")]
    DaemonError { message: String, exit_code: i32 },
}

impl Error {
    /// Stable machine-readable category used by CLI JSON errors.
    pub fn code(&self) -> &'static str {
        match self {
            Error::WorkspaceNotFound(_) => "WORKSPACE_NOT_FOUND",
            Error::WorkspaceExists(_) => "WORKSPACE_EXISTS",
            Error::InvalidPath { .. } => "INVALID_PATH",
            Error::PathIndexRequired(_) => "PATH_INDEX_REQUIRED",
            Error::SchemaReinitializeRequired { .. } => "SCHEMA_REINITIALIZE_REQUIRED",
            Error::ChangeLedgerReconcileRequired { .. } => "CHANGE_LEDGER_RECONCILE_REQUIRED",
            Error::CommittedRepairRequired { .. }
            | Error::OperationCommittedRepairRequired { .. } => "COMMITTED_REPAIR_REQUIRED",
            Error::LaneInitializationConflict { .. } => "LANE_INITIALIZATION_CONFLICT",
            Error::IgnoredPath(_) => "IGNORED_PATH",
            Error::RefNotFound(_) => "REF_NOT_FOUND",
            Error::OperationNotFound(_) => "OPERATION_NOT_FOUND",
            Error::RootNotFound(_) => "ROOT_NOT_FOUND",
            Error::ObjectNotFound { .. } => "OBJECT_NOT_FOUND",
            Error::DirtyWorktree | Error::DirtyWorktreeWithMessage(_) => "DIRTY_WORKTREE",
            Error::WorkspaceLocked(_) => "WORKSPACE_LOCKED",
            Error::Conflict(_) => "MERGE_CONFLICT",
            Error::PatchRejected(_) => "PATCH_REJECTED",
            Error::StaleBranch(_) => "STALE_BRANCH",
            Error::Corrupt(_) => "DATABASE_CORRUPT",
            Error::Git(_) => "GIT_ERROR",
            Error::GitMappingRequired(_) => "GIT_MAPPING_REQUIRED",
            Error::GitHeadChanged(_) => "GIT_HEAD_CHANGED",
            Error::GitWorktreeDirty(_) => "GIT_WORKTREE_DIRTY",
            Error::GitDeltaExportRequired(_) => "GIT_DELTA_EXPORT_REQUIRED",
            Error::InvalidInput(_) => "INVALID_INPUT",
            Error::CloneUnsupported => "CLONE_UNSUPPORTED",
            Error::CloneCrossDevice => "CLONE_CROSS_DEVICE",
            Error::NativeCowSourceUnavailable => "NATIVE_COW_SOURCE_UNAVAILABLE",
            Error::Io(_) => "IO_ERROR",
            Error::Sqlite(_) => "SQLITE_ERROR",
            Error::Serialization(_) => "SERIALIZATION_ERROR",
            Error::Prolly(_) => "PROLLY_ERROR",
            Error::ProllySqlite(_) => "PROLLY_SQLITE_ERROR",
            Error::ProllySlateDb(_) => "PROLLY_SLATEDB_ERROR",
            Error::Json(_) => "JSON_ERROR",
            Error::TomlSer(_) | Error::TomlDe(_) => "TOML_ERROR",
            Error::DaemonUnavailable(_) => "DAEMON_UNAVAILABLE",
            Error::DaemonError { .. } => "DAEMON_ERROR",
        }
    }

    /// Exit code contract from the CLI design document.
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::WorkspaceNotFound(_) => 3,
            Error::Corrupt(_) => 4,
            Error::DirtyWorktree | Error::DirtyWorktreeWithMessage(_) => 5,
            Error::Conflict(_) => 6,
            Error::PatchRejected(_) => 7,
            Error::StaleBranch(_) | Error::WorkspaceLocked(_) => 8,
            Error::InvalidPath { .. } | Error::PathIndexRequired(_) => 9,
            Error::Git(_)
            | Error::GitMappingRequired(_)
            | Error::GitHeadChanged(_)
            | Error::GitWorktreeDirty(_)
            | Error::GitDeltaExportRequired(_) => 10,
            Error::OperationNotFound(_) => 12,
            Error::RefNotFound(_) => 13,
            Error::IgnoredPath(_) => 14,
            Error::SchemaReinitializeRequired { .. } => 15,
            Error::ChangeLedgerReconcileRequired { .. } => 16,
            Error::CommittedRepairRequired { .. }
            | Error::OperationCommittedRepairRequired { .. } => 16,
            Error::LaneInitializationConflict { .. } => 2,
            Error::InvalidInput(_)
            | Error::WorkspaceExists(_)
            | Error::CloneUnsupported
            | Error::CloneCrossDevice
            | Error::NativeCowSourceUnavailable => 2,
            Error::DaemonUnavailable(_) => 11,
            Error::DaemonError { exit_code, .. } => *exit_code,
            _ => 1,
        }
    }
}

pub(crate) fn cbor<T: serde::Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_cbor::to_vec(value).map_err(|err| Error::Serialization(err.to_string()))
}

pub(crate) fn from_cbor<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    serde_cbor::from_slice(bytes).map_err(|err| Error::Serialization(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_handoff_errors_have_stable_codes_and_exit_status() {
        let errors = [
            (
                Error::GitMappingRequired("missing mapping".into()),
                "GIT_MAPPING_REQUIRED",
            ),
            (
                Error::GitHeadChanged("head changed".into()),
                "GIT_HEAD_CHANGED",
            ),
            (
                Error::GitWorktreeDirty("tracked changes".into()),
                "GIT_WORKTREE_DIRTY",
            ),
            (
                Error::GitDeltaExportRequired("mapped delta required".into()),
                "GIT_DELTA_EXPORT_REQUIRED",
            ),
        ];
        for (error, code) in errors {
            assert_eq!(error.code(), code);
            assert_eq!(error.exit_code(), 10);
        }
    }

    #[test]
    fn path_index_required_has_stable_code_and_exit_status() {
        let error = Error::PathIndexRequired(
            "legacy root has no case-fold index; run `trail index rebuild`".into(),
        );

        assert_eq!(error.code(), "PATH_INDEX_REQUIRED");
        assert_eq!(error.exit_code(), 9);
    }

    #[test]
    fn schema_and_ledger_recovery_errors_have_stable_contracts() {
        let schema = Error::SchemaReinitializeRequired {
            found: "version 17".into(),
            guidance: "back up this workspace, then run `trail init --force` to create schema v19"
                .into(),
        };
        assert_eq!(schema.code(), "SCHEMA_REINITIALIZE_REQUIRED");
        assert_eq!(schema.exit_code(), 15);
        assert_eq!(
            schema.to_string(),
            "workspace schema version 17 cannot be opened; back up this workspace, then run `trail init --force` to create schema v19"
        );

        let reconcile = Error::ChangeLedgerReconcileRequired {
            scope: "workspace:main".into(),
            state: "untrusted_gap".into(),
            reason: "observer startup failed".into(),
            command: "trail status".into(),
        };
        assert_eq!(reconcile.code(), "CHANGE_LEDGER_RECONCILE_REQUIRED");
        assert_eq!(reconcile.exit_code(), 16);
        assert_eq!(
            reconcile.to_string(),
            "changed-path ledger reconciliation required for workspace:main: observer startup failed; run `trail status`"
        );

        let committed = Error::OperationCommittedRepairRequired {
            operation: "op-1".into(),
            repair: "ref mirror".into(),
            reason: "injected failure".into(),
        };
        assert_eq!(committed.code(), "COMMITTED_REPAIR_REQUIRED");
        assert_eq!(committed.exit_code(), 16);
        assert!(committed.to_string().contains("operation op-1 committed"));
    }

    #[test]
    fn lane_initialization_conflict_has_stable_code_exit_and_fields() {
        let error = Error::LaneInitializationConflict {
            lane: "agent-1".into(),
            existing_fingerprint: "sha256:existing".into(),
            requested_fingerprint: "sha256:requested".into(),
        };

        assert_eq!(error.code(), "LANE_INITIALIZATION_CONFLICT");
        assert_eq!(error.exit_code(), 2);
        assert_eq!(
            error.to_string(),
            "lane `agent-1` is already reserved by request sha256:existing; requested sha256:requested"
        );
    }
}
