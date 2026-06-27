use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::fs::OpenOptions;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::{symlink as symlink_file, MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ignore::WalkBuilder;
use prolly::{
    BatchBuilder, Cid, Config, Diff, Encoding, Prolly, SortedBatchBuilder, SqliteStore, Tree,
};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
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
const CRABDB_SCHEMA_VERSION: i64 = 2;
const SCHEMA_META_VERSION_KEY: &str = "schema.version";
const SCHEMA_META_APP_VERSION_KEY: &str = "app.version";
const MAIN_REF_PREFIX: &str = "refs/branches/";
const LANE_REF_PREFIX: &str = "refs/lanes/";
const ROOT_OBJECT_VERSION: u16 = 1;
const TEXT_OBJECT_VERSION: u16 = 1;

thread_local! {
    static WRITE_LOCK_WAIT_DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) };
}
const OP_OBJECT_VERSION: u16 = 1;
const BLOB_OBJECT_VERSION: u16 = 1;
const MESSAGE_OBJECT_VERSION: u16 = 1;
const ANCHOR_OBJECT_VERSION: u16 = 1;
const OBJECT_CACHE_MAX_ENTRIES: usize = 4096;
const OBJECT_CACHE_MAX_BYTES: usize = 64 * 1024 * 1024;
const ORDER_KEY_STEP: u64 = 1024;
const LANE_TEST_OUTPUT_PREVIEW_BYTES: usize = 64 * 1024;
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
    root_prolly: Prolly<Arc<SqliteStore>>,
    config: CrabConfig,
    object_cache: Mutex<ObjectCache>,
    daemon_worktree_cache: Option<DaemonWorktreeCache>,
}

#[derive(Debug, Default)]
struct ObjectCache {
    entries: HashMap<String, ObjectCacheEntry>,
    order: VecDeque<String>,
    total_bytes: usize,
}

#[derive(Debug)]
struct ObjectCacheEntry {
    kind: String,
    bytes: Vec<u8>,
}

impl ObjectCache {
    fn get(&self, kind: &str, object_id: &ObjectId) -> Option<Vec<u8>> {
        self.entries.get(&object_id.0).and_then(|entry| {
            if entry.kind == kind {
                Some(entry.bytes.clone())
            } else {
                None
            }
        })
    }

    fn insert(&mut self, object_id: &ObjectId, kind: &str, bytes: &[u8]) {
        if bytes.len() > OBJECT_CACHE_MAX_BYTES {
            return;
        }
        if self.entries.contains_key(&object_id.0) {
            return;
        }
        self.entries.insert(
            object_id.0.clone(),
            ObjectCacheEntry {
                kind: kind.to_string(),
                bytes: bytes.to_vec(),
            },
        );
        self.order.push_back(object_id.0.clone());
        self.total_bytes = self.total_bytes.saturating_add(bytes.len());
        while self.entries.len() > OBJECT_CACHE_MAX_ENTRIES
            || self.total_bytes > OBJECT_CACHE_MAX_BYTES
        {
            let Some(evicted) = self.order.pop_front() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&evicted) {
                self.total_bytes = self.total_bytes.saturating_sub(entry.bytes.len());
            }
        }
    }
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
pub(crate) struct WorktreePathScan {
    paths: Vec<String>,
    total_bytes: u64,
}

#[derive(Debug)]
pub(crate) struct RootBuildResult {
    root_id: ObjectId,
    files: BTreeMap<String, FileEntry>,
    disk_manifest: BTreeMap<String, DiskManifest>,
    stats: ImportStats,
}

#[derive(Debug)]
pub(crate) struct IncrementalRootBuildResult {
    root_id: ObjectId,
}

#[derive(Debug)]
pub(crate) struct GitTrackedRootBuildResult {
    root_id: ObjectId,
    disk_manifest: BTreeMap<String, DiskManifest>,
    stats: ImportStats,
}

#[derive(Debug)]
pub(crate) struct SelectedWorktreeSnapshot {
    paths: Vec<String>,
    files: Vec<DiskFile>,
    summaries: Vec<FileDiffSummary>,
}

#[derive(Debug)]
pub(crate) struct FileBuildResult {
    entry: FileEntry,
    disk_manifest: DiskManifest,
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
pub(crate) struct PathLocalMergeResult {
    target_files: BTreeMap<String, FileEntry>,
    merged_files: BTreeMap<String, FileEntry>,
    conflicts: Vec<String>,
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
pub(crate) struct ExternalMutationAuditInput {
    pub(crate) actor: String,
    pub(crate) surface: String,
    pub(crate) command: String,
    pub(crate) target_ref: Option<String>,
    pub(crate) lane_id: Option<String>,
    pub(crate) turn_id: Option<String>,
    pub(crate) status: String,
    pub(crate) status_code: Option<i64>,
    pub(crate) change_id: Option<ChangeId>,
    pub(crate) summary: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub(crate) struct HttpIdempotencyEntry {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) request_hash: String,
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct HttpIdempotencyStoreInput {
    pub(crate) key: String,
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) request_hash: String,
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct LaneTraceSpanBuilder {
    span_id: String,
    trace_id: String,
    lane_id: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WorktreeFileStamp {
    size_bytes: u64,
    modified_ns: i64,
    changed_ns: i64,
    device_id: i64,
    inode: i64,
    executable: bool,
}

impl WorktreeFileStamp {
    pub(crate) fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self {
            size_bytes: metadata.len(),
            modified_ns: metadata_modified_ns(metadata),
            changed_ns: metadata_changed_ns(metadata),
            device_id: metadata_device_id(metadata),
            inode: metadata_inode(metadata),
            executable: metadata_executable(metadata),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct WorkdirFileStamp {
    size_bytes: u64,
    modified_ns: i64,
    changed_ns: i64,
    #[serde(default)]
    device_id: i64,
    #[serde(default)]
    inode: i64,
    executable: bool,
}

impl WorkdirFileStamp {
    pub(crate) fn from_metadata(metadata: &fs::Metadata) -> Self {
        Self {
            size_bytes: metadata.len(),
            modified_ns: metadata_modified_ns(metadata),
            changed_ns: metadata_changed_ns(metadata),
            device_id: metadata_device_id(metadata),
            inode: metadata_inode(metadata),
            executable: metadata_executable(metadata),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MaterializedWorkdir {
    files_written: usize,
    stamps: BTreeMap<String, WorkdirFileStamp>,
}

impl MaterializedWorkdir {
    pub(crate) fn insert_stamp(&mut self, path: String, stamp: WorkdirFileStamp) {
        self.files_written += 1;
        self.stamps.insert(path, stamp);
    }

    pub(crate) fn extend(&mut self, other: MaterializedWorkdir) {
        self.files_written += other.files_written;
        self.stamps.extend(other.stamps);
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RootMaterializationReport {
    file_count: u64,
    disk_manifest: BTreeMap<String, DiskManifest>,
    materialized: MaterializedWorkdir,
}

#[derive(Debug, Clone)]
pub(crate) struct IndexedDiskManifest {
    manifest: DiskManifest,
    stamp: WorktreeFileStamp,
}

fn metadata_modified_ns(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(duration_ns)
        .unwrap_or(0)
}

#[cfg(unix)]
fn metadata_changed_ns(metadata: &fs::Metadata) -> i64 {
    metadata
        .ctime()
        .saturating_mul(1_000_000_000)
        .saturating_add(metadata.ctime_nsec())
}

#[cfg(not(unix))]
fn metadata_changed_ns(_metadata: &fs::Metadata) -> i64 {
    0
}

#[cfg(unix)]
fn metadata_device_id(metadata: &fs::Metadata) -> i64 {
    metadata.dev().min(i64::MAX as u64) as i64
}

#[cfg(not(unix))]
fn metadata_device_id(_metadata: &fs::Metadata) -> i64 {
    0
}

#[cfg(unix)]
fn metadata_inode(metadata: &fs::Metadata) -> i64 {
    metadata.ino().min(i64::MAX as u64) as i64
}

#[cfg(not(unix))]
fn metadata_inode(_metadata: &fs::Metadata) -> i64 {
    0
}

#[cfg(unix)]
fn metadata_executable(metadata: &fs::Metadata) -> bool {
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn metadata_executable(_metadata: &fs::Metadata) -> bool {
    false
}

fn duration_ns(duration: Duration) -> i64 {
    let ns = (duration.as_secs() as u128)
        .saturating_mul(1_000_000_000)
        .saturating_add(duration.subsec_nanos() as u128);
    ns.min(i64::MAX as u128) as i64
}

pub(crate) struct DaemonWorktreeCache {
    state: Arc<Mutex<DaemonWorktreeCacheState>>,
    persist: Option<DaemonWorktreeCachePersist>,
    _watcher: notify::RecommendedWatcher,
}

#[derive(Clone, Debug)]
pub(crate) struct DaemonWorktreeCachePersist {
    path: PathBuf,
    workspace_root: PathBuf,
    pid: u32,
}

#[derive(Debug)]
pub struct DaemonWorktreeCacheWarmup {
    workspace_root: PathBuf,
    db_dir: PathBuf,
    state: Arc<Mutex<DaemonWorktreeCacheState>>,
    persist: Option<DaemonWorktreeCachePersist>,
    generation: u64,
}

#[derive(Debug, Default)]
pub(crate) struct DaemonWorktreeCacheState {
    dirty_paths: BTreeSet<String>,
    overflow: bool,
    initialized: bool,
    baseline_root_id: Option<ObjectId>,
    generation: u64,
}

#[derive(Debug)]
pub(crate) enum DaemonWorktreeSnapshot {
    Clean {
        generation: u64,
        root_id: Option<ObjectId>,
    },
    Dirty {
        generation: u64,
        paths: Vec<String>,
    },
    Overflow {
        generation: u64,
    },
}

pub(crate) enum CachedWorkdirManifestStatus {
    Clean,
    Dirty {
        disk_manifest: BTreeMap<String, DiskManifest>,
        candidate_paths: Option<Vec<String>>,
    },
    Missing,
}

#[derive(Debug, Clone)]
pub(crate) struct MergeContext {
    base_change: ChangeId,
    left_change: ChangeId,
    right_change: ChangeId,
    base_root: ObjectId,
    left_root: ObjectId,
    right_root: ObjectId,
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
    base_root: Option<ObjectId>,
    left_root: Option<ObjectId>,
    right_root: Option<ObjectId>,
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

struct WriteLockWaitGuard {
    previous: Option<Instant>,
}

impl Drop for WriteLockWaitGuard {
    fn drop(&mut self) {
        WRITE_LOCK_WAIT_DEADLINE.with(|deadline| deadline.set(self.previous));
    }
}

mod agent;
mod core;
mod lane;
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
    fn case_fold_collision_validation_rejects_unicode_compatibility_aliases() {
        let paths = ["src/Ｋ.rs".to_string(), "src/k.rs".to_string()];
        let err = validate_no_case_fold_collisions(paths.iter()).unwrap_err();
        match err {
            Error::InvalidPath { path, reason } => {
                assert_eq!(path, "src/k.rs");
                assert!(reason.contains("src/Ｋ.rs"));
            }
            other => panic!("expected invalid path error, got {other:?}"),
        }
    }

    #[test]
    fn case_fold_collision_validation_allows_distinct_paths() {
        let paths = ["src/foo.rs".to_string(), "src/bar.rs".to_string()];
        validate_no_case_fold_collisions(paths.iter()).unwrap();
    }

    #[test]
    fn relative_path_normalization_rejects_unicode_aliases() {
        let composed = normalize_relative_path("docs/caf\u{00E9}.md").unwrap();
        assert_eq!(composed, "docs/caf\u{00E9}.md");

        let err = normalize_relative_path("docs/cafe\u{0301}.md").unwrap_err();
        match err {
            Error::InvalidPath { path, reason } => {
                assert_eq!(path, "docs/cafe\u{0301}.md");
                assert!(reason.contains("Unicode NFC"));
            }
            other => panic!("expected invalid path error, got {other:?}"),
        }
    }

    #[test]
    fn relative_path_normalization_rejects_separator_lookalikes() {
        for separator in [
            '\u{2044}', '\u{2215}', '\u{2216}', '\u{29F8}', '\u{29F9}', '\u{FE68}', '\u{FF0F}',
            '\u{FF3C}',
        ] {
            let path = format!("docs{separator}README.md");
            let err = normalize_relative_path(&path).unwrap_err();
            match err {
                Error::InvalidPath { reason, .. } => {
                    assert!(reason.contains("slash lookalike"));
                }
                other => panic!("expected invalid path error, got {other:?}"),
            }
        }
    }

    #[test]
    fn relative_path_normalization_rejects_invisible_format_controls() {
        for control in [
            '\u{200B}', '\u{200C}', '\u{200D}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}',
            '\u{202C}', '\u{202D}', '\u{202E}', '\u{2060}', '\u{2066}', '\u{2067}', '\u{2068}',
            '\u{2069}', '\u{FEFF}',
        ] {
            let path = format!("docs/readme{control}.md");
            let err = normalize_relative_path(&path).unwrap_err();
            match err {
                Error::InvalidPath { reason, .. } => {
                    assert!(reason.contains("invisible Unicode format controls"));
                }
                other => panic!("expected invalid path error, got {other:?}"),
            }
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn relative_path_normalization_rejects_backslash_separators() {
        let err = normalize_relative_path("docs\\README.md").unwrap_err();
        match err {
            Error::InvalidPath { reason, .. } => {
                assert!(reason.contains("backslash"));
                assert!(reason.contains("use `/`"));
            }
            other => panic!("expected invalid path error, got {other:?}"),
        }
    }

    #[test]
    fn relative_path_normalization_rejects_windows_device_aliases() {
        for path in [
            "CONIN$",
            "CONOUT$",
            "COM\u{00B9}.txt",
            "COM\u{00B2}.txt",
            "COM\u{00B3}.txt",
            "LPT\u{00B9}",
            "LPT\u{00B2}",
            "LPT\u{00B3}",
        ] {
            let err = normalize_relative_path(path).unwrap_err();
            match err {
                Error::InvalidPath { reason, .. } => {
                    assert!(reason.contains("reserved on Windows"));
                }
                other => panic!("expected invalid path error for {path}, got {other:?}"),
            }
        }
    }

    #[test]
    fn relative_path_normalization_fuzz_corpus_never_escapes_workspace() {
        for seed in 0..512_u64 {
            let path = generated_path(seed);
            if let Ok(normalized) = normalize_relative_path(&path) {
                assert!(!normalized.is_empty(), "seed {seed} normalized empty");
                assert!(!normalized.starts_with('/'), "seed {seed}: {normalized}");
                assert!(!normalized.contains('\\'), "seed {seed}: {normalized}");
                assert!(!normalized.contains('\0'), "seed {seed}: {normalized}");
                for part in normalized.split('/') {
                    assert!(!part.is_empty(), "seed {seed}: {normalized}");
                    assert_ne!(part, ".", "seed {seed}: {normalized}");
                    assert_ne!(part, "..", "seed {seed}: {normalized}");
                    assert!(!part.contains(':'), "seed {seed}: {normalized}");
                    assert!(!part.ends_with([' ', '.']), "seed {seed}: {normalized}");
                    assert!(
                        !part.chars().any(|ch| matches!(
                            ch,
                            '\u{200B}'
                                | '\u{200C}'
                                | '\u{200D}'
                                | '\u{200E}'
                                | '\u{200F}'
                                | '\u{202A}'
                                | '\u{202B}'
                                | '\u{202C}'
                                | '\u{202D}'
                                | '\u{202E}'
                                | '\u{2060}'
                                | '\u{2066}'
                                | '\u{2067}'
                                | '\u{2068}'
                                | '\u{2069}'
                                | '\u{FEFF}'
                        )),
                        "seed {seed}: {normalized}"
                    );
                }
            }
        }
    }

    #[test]
    fn patch_document_parser_fuzz_corpus_accepts_only_known_shapes() {
        for seed in 0..256_u64 {
            let value = generated_patch_json(seed);
            match serde_json::from_value::<PatchDocument>(value) {
                Ok(document) => {
                    let encoded = serde_json::to_value(&document).unwrap();
                    assert!(encoded.get("edits").is_some());
                }
                Err(err) => {
                    let message = err.to_string();
                    assert!(
                        message.contains("unknown field")
                            || message.contains("unknown variant")
                            || message.contains("missing field")
                            || message.contains("invalid type"),
                        "unexpected parse error for seed {seed}: {message}"
                    );
                }
            }
        }
    }

    fn generated_path(seed: u64) -> String {
        let atoms = [
            "src",
            "lib.rs",
            "..",
            ".",
            "",
            "CON",
            "aux.txt",
            "has:colon",
            "trail.",
            "trail ",
            "nested\\path",
            "normal-name",
            "\u{2215}",
            "\u{29F8}",
            "cafe\u{0301}.md",
            "spoof\u{202E}txt",
            "zero\u{200B}width",
            "emoji",
            ".git",
            ".crabdb",
        ];
        let mut state = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let mut parts = Vec::new();
        for _ in 0..=((seed % 5) as usize) {
            state = state
                .wrapping_mul(2862933555777941757)
                .wrapping_add(3037000493);
            parts.push(atoms[(state as usize) % atoms.len()]);
        }
        let mut path = parts.join(if seed % 7 == 0 { "\\" } else { "/" });
        if seed % 11 == 0 {
            path.insert(0, '/');
        }
        if seed % 13 == 0 {
            path.push('\0');
        }
        path
    }

    fn generated_patch_json(seed: u64) -> serde_json::Value {
        let path = generated_path(seed);
        let op = match seed % 7 {
            0 => "write",
            1 => "write_bytes",
            2 => "replace_line",
            3 => "delete",
            4 => "rename",
            5 => "unknown",
            _ => "write",
        };
        let edit = match op {
            "write" => serde_json::json!({
                "op": op,
                "path": path,
                "content": format!("seed-{seed}\n"),
                "extra": (seed % 3 == 0).then_some(true)
            }),
            "write_bytes" => serde_json::json!({
                "op": op,
                "path": path,
                "bytes_hex": if seed % 2 == 0 { "00ff" } else { "not-hex" }
            }),
            "replace_line" => serde_json::json!({
                "op": op,
                "path": path,
                "line_id": if seed % 2 == 0 {
                    serde_json::json!("change_abc:1")
                } else {
                    serde_json::json!(1)
                },
                "expected_text": "old",
                "new_text": "new"
            }),
            "delete" => serde_json::json!({
                "op": op,
                "path": path
            }),
            "rename" => serde_json::json!({
                "op": op,
                "from": path,
                "to": generated_path(seed.wrapping_add(17))
            }),
            _ => serde_json::json!({
                "op": op,
                "path": path
            }),
        };
        serde_json::json!({
            "message": format!("generated patch {seed}"),
            "allow_stale": seed % 2 == 0,
            "edits": [edit]
        })
    }
}
