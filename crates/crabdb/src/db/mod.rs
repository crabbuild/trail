use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::fs::OpenOptions;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::{symlink as symlink_file, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ignore::WalkBuilder;
use prolly::{BatchBuilder, Cid, Config, Diff, Encoding, Prolly, SqliteStore, Tree};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use similar::{ChangeTag, TextDiff};

use crate::error::{cbor, from_cbor, Error, Result};
use crate::ids::{
    sha256_hex, AnchorId, ChangeId, FileId, LineId, MessageId, ObjectId, WorkspaceId,
};
use crate::model::*;

const CONFIG_FILE: &str = "config.toml";
const HEAD_FILE: &str = "HEAD";
const DB_RELATIVE_PATH: &str = "index/crabdb.sqlite";
const CRABDB_SCHEMA_VERSION: i64 = 1;
const SCHEMA_META_VERSION_KEY: &str = "schema.version";
const SCHEMA_META_APP_VERSION_KEY: &str = "app.version";
const MAIN_REF_PREFIX: &str = "refs/branches/";
const AGENT_REF_PREFIX: &str = "refs/agents/";
const ROOT_OBJECT_VERSION: u16 = 1;
const TEXT_OBJECT_VERSION: u16 = 1;
const OP_OBJECT_VERSION: u16 = 1;
const BLOB_OBJECT_VERSION: u16 = 1;
const MESSAGE_OBJECT_VERSION: u16 = 1;
const ANCHOR_OBJECT_VERSION: u16 = 1;
const ORDER_KEY_STEP: u64 = 1024;
const AGENT_TEST_OUTPUT_PREVIEW_BYTES: usize = 64 * 1024;
const DEFAULT_CRABIGNORE_PATTERNS: &[&str] = &[
    ".crabdb/",
    ".git/",
    ".env",
    ".env.*",
    "*.pem",
    "*.key",
    "*.p12",
    "*.pfx",
    "id_rsa",
    "id_ed25519",
    "node_modules/",
    "target/",
    "dist/",
    "build/",
    "coverage/",
];

pub struct CrabDb {
    workspace_root: PathBuf,
    db_dir: PathBuf,
    conn: Connection,
    store: Arc<SqliteStore>,
    prolly: Prolly<Arc<SqliteStore>>,
    config: CrabConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitImportMode {
    Empty,
    GitTracked,
    WorkingTree,
}

#[derive(Debug, Clone)]
pub(crate) struct DiskFile {
    path: String,
    bytes: Vec<u8>,
    executable: bool,
}

#[derive(Debug)]
pub(crate) struct RootBuildResult {
    root_id: ObjectId,
    files: BTreeMap<String, FileEntry>,
    stats: ImportStats,
}

#[derive(Debug)]
pub(crate) struct FileBuildResult {
    entry: FileEntry,
    line_changes: Vec<LineChange>,
}

#[derive(Debug)]
pub(crate) struct TextBuildResult {
    object_id: ObjectId,
    line_changes: Vec<LineChange>,
}

#[derive(Debug, Clone)]
pub(crate) struct RootDiff {
    changes: Vec<FileChange>,
    summaries: Vec<FileDiffSummary>,
}

#[derive(Debug)]
pub(crate) struct CommandRunResult {
    success: bool,
    exit_code: Option<i32>,
    timed_out: bool,
    duration_ms: u64,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct AgentTraceSpanBuilder {
    span_id: String,
    trace_id: String,
    agent_id: String,
    session_id: Option<String>,
    turn_id: Option<String>,
    parent_span_id: Option<String>,
    span_type: String,
    name: String,
    started_event_id: String,
    started_at: i64,
    attributes: Option<serde_json::Value>,
    ended_event_id: Option<String>,
    ended_at: Option<i64>,
    status: Option<String>,
    result: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, serde::Deserialize)]
pub(crate) struct BackupManifest {
    format_version: u16,
    crabdb_version: String,
    created_at: i64,
    source_workspace: String,
    source_db_dir: String,
    workspace_id: WorkspaceId,
    branch: String,
    ref_count: u64,
    operation_count: u64,
    sqlite_bytes: u64,
    sqlite_sha256: String,
    worktree_bytes: u64,
}

#[derive(Debug)]
pub(crate) struct PendingLineMerge {
    path: String,
    target_entry: FileEntry,
    lines: Vec<LineEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct LineGap {
    previous: Option<String>,
    next: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct OperationObject {
    object_id: ObjectId,
    operation: Operation,
}

#[derive(Debug, Clone)]
pub(crate) struct DiskManifest {
    kind: FileKind,
    executable: bool,
    content_hash: String,
}

#[derive(Debug, Clone)]
pub(crate) struct MergeContext {
    base_change: ChangeId,
    left_change: ChangeId,
    right_change: ChangeId,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingConflictMerge {
    merge_id: String,
    queue_id: Option<String>,
    source_ref: String,
    target_ref: String,
    base_change: ChangeId,
    left_change: ChangeId,
    right_change: ChangeId,
}

#[derive(Debug, Clone)]
pub(crate) struct GitState {
    head: Option<String>,
    dirty: bool,
}

#[derive(Debug, Default)]
pub(crate) struct GitTreeNode {
    blobs: BTreeMap<String, GitBlobEntry>,
    dirs: BTreeMap<String, GitTreeNode>,
}

#[derive(Debug)]
pub(crate) struct GitBlobEntry {
    mode: &'static str,
    oid: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConflictTake {
    Source,
    Target,
}

#[derive(Debug)]
pub(crate) enum ConflictResolution {
    Take(ConflictTake),
    Manual(ConflictManualResolution),
}

#[derive(Debug)]
pub(crate) struct WorkspaceLock {
    path: PathBuf,
}

impl Drop for WorkspaceLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

mod agent;
mod core;
mod merge;
mod record;
mod storage;
mod util;

#[cfg(test)]
mod tests {
    use super::util::*;
    use super::*;

    #[test]
    fn case_fold_collision_validation_rejects_ambiguous_paths() {
        let paths = [
            "src/Foo.rs".to_string(),
            "src/foo.rs".to_string(),
            "src/bar.rs".to_string(),
        ];
        let err = validate_no_case_fold_collisions(paths.iter()).unwrap_err();
        match err {
            Error::InvalidPath { path, reason } => {
                assert_eq!(path, "src/foo.rs");
                assert!(reason.contains("src/Foo.rs"));
            }
            other => panic!("expected invalid path error, got {other:?}"),
        }
    }

    #[test]
    fn case_fold_collision_validation_allows_distinct_paths() {
        let paths = ["src/foo.rs".to_string(), "src/bar.rs".to_string()];
        validate_no_case_fold_collisions(paths.iter()).unwrap();
    }
}
