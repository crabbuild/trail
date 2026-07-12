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
            Error::InvalidPath { .. } => 9,
            Error::Git(_) => 10,
            Error::OperationNotFound(_) => 12,
            Error::RefNotFound(_) => 13,
            Error::IgnoredPath(_) => 14,
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
