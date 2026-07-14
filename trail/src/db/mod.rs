use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::{symlink as symlink_file, MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ignore::WalkBuilder;
use prolly::{
    BatchBuilder, BatchOp, Cid, Config, Diff, Encoding, Prolly, SortedBatchBuilder, Store, Tree,
};
use prolly_store_slatedb::SlateDbStore;
use prolly_store_sqlite::SqliteStore;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use similar::{ChangeTag, TextDiff};
use slatedb::object_store::aws::AmazonS3Builder;
use slatedb::object_store::ObjectStore;

use crate::error::{cbor, from_cbor, Error, Result};
use crate::ids::{
    sha256_hex, AnchorId, ChangeId, FileId, LineId, MessageId, ObjectId, WorkspaceId,
};
use crate::model::*;

const CONFIG_FILE: &str = "config.toml";
const HEAD_FILE: &str = "HEAD";
const DB_RELATIVE_PATH: &str = "index/trail.sqlite";
const TRAIL_SCHEMA_VERSION: i64 = 18;
const SCHEMA_META_VERSION_KEY: &str = "schema.version";
const SCHEMA_META_APP_VERSION_KEY: &str = "app.version";
const MAIN_REF_PREFIX: &str = "refs/branches/";
const LANE_REF_PREFIX: &str = "refs/lanes/";
const ROOT_OBJECT_VERSION: u16 = 1;
const TEXT_OBJECT_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SchemaOpenMode {
    FreshCreate,
    Existing,
}

pub(crate) fn preflight_existing_schema(db_path: &Path, prolly_backend: &str) -> Result<()> {
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
        | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
        | rusqlite::OpenFlags::SQLITE_OPEN_URI;
    let uri = immutable_sqlite_uri(db_path);
    let conn =
        rusqlite::Connection::open_with_flags(uri, flags).map_err(schema_reinitialize_error)?;
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(schema_reinitialize_error)?;
    Trail::validate_schema_v18(&conn).map_err(schema_reinitialize_error)?;
    match prolly_backend {
        "sqlite" => {
            storage::validate_prolly_sqlite_schema_v18(&conn).map_err(schema_reinitialize_error)
        }
        "slatedb" => {
            storage::validate_no_prolly_sqlite_schema_v18(&conn).map_err(schema_reinitialize_error)
        }
        other => Err(Error::InvalidInput(format!(
            "storage.prolly_backend must be sqlite or slatedb, got `{other}`"
        ))),
    }
}

#[cfg(unix)]
fn immutable_sqlite_uri(db_path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;

    let mut uri = String::from("file:");
    for byte in db_path.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(char::from(*byte));
            }
            _ => uri.push_str(&format!("%{byte:02x}")),
        }
    }
    uri.push_str("?immutable=1");
    uri
}

#[cfg(not(unix))]
fn immutable_sqlite_uri(db_path: &Path) -> String {
    let encoded_path = db_path
        .to_string_lossy()
        .replace('%', "%25")
        .replace('?', "%3f")
        .replace('#', "%23");
    format!("file:{encoded_path}?immutable=1")
}

fn schema_reinitialize_error(err: impl std::fmt::Display) -> Error {
    Error::SchemaReinitializeRequired {
        found: err.to_string(),
        guidance: "back up this workspace, then run `trail init --force` to create schema v18"
            .into(),
    }
}

#[cfg(all(test, unix))]
mod immutable_uri_tests {
    use super::*;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn immutable_uri_percent_encodes_non_utf8_path_bytes_losslessly() {
        let path = Path::new(OsStr::from_bytes(b"/tmp/trail-\xff/%?#.sqlite"));

        assert_eq!(
            immutable_sqlite_uri(path),
            "file:/tmp/trail-%ff/%25%3f%23.sqlite?immutable=1"
        );
    }
}

thread_local! {
    static WRITE_LOCK_WAIT_DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) };
}
const OP_OBJECT_VERSION: u16 = 1;
const BLOB_OBJECT_VERSION: u16 = 1;
const MESSAGE_OBJECT_VERSION: u16 = 1;
const ANCHOR_OBJECT_VERSION: u16 = 1;
const WORKSPACE_LAYER_MANIFEST_KIND: &str = "workspace_layer_manifest";
const WORKSPACE_LAYER_MANIFEST_VERSION: u16 = 1;
const OBJECT_CACHE_MAX_ENTRIES: usize = 4096;
const OBJECT_CACHE_MAX_BYTES: usize = 64 * 1024 * 1024;
const ORDER_KEY_STEP: u64 = 1024;
const LANE_TEST_OUTPUT_PREVIEW_BYTES: usize = 64 * 1024;
const DEFAULT_CRABIGNORE_PATTERNS: &[&str] = &[
    ".trail/",
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RootDirectoryChild {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) entry: Option<FileEntry>,
}

pub struct Trail {
    workspace_root: PathBuf,
    db_dir: PathBuf,
    sqlite_path: PathBuf,
    conn: Connection,
    store: TrailProllyStore,
    prolly: Prolly<TrailProllyStore>,
    root_prolly: Prolly<TrailProllyStore>,
    config: TrailConfig,
    object_cache: Mutex<ObjectCache>,
    daemon_worktree_cache: Option<DaemonWorktreeCache>,
    git_handoff_metrics: Cell<GitHandoffMetrics>,
    case_fold_index_metrics: Cell<CaseFoldIndexMetrics>,
    operation_metrics: Option<Arc<OperationMetricsState>>,
}

pub(crate) struct WorkspaceIgnorePolicySnapshot {
    workspace_root: PathBuf,
    metrics: Option<Arc<OperationMetricsState>>,
    matcher: OnceLock<std::result::Result<::ignore::gitignore::Gitignore, String>>,
}

#[derive(Clone)]
struct TrailProllyStore {
    backend: TrailProllyStoreBackend,
    metrics: Option<Arc<OperationMetricsState>>,
}

#[derive(Clone)]
enum TrailProllyStoreBackend {
    Sqlite(Arc<SqliteStore>),
    SlateDb(Arc<SlateDbStore>),
}

impl TrailProllyStore {
    fn new(backend: TrailProllyStoreBackend, metrics: Option<Arc<OperationMetricsState>>) -> Self {
        Self { backend, metrics }
    }

    fn note_prolly_read_call(&self, key_count: usize) {
        if let Some(metrics) = &self.metrics {
            metrics.note_prolly_read_call(key_count);
        }
    }

    fn note_prolly_read_values<'a, I>(&self, values: I)
    where
        I: IntoIterator<Item = &'a Vec<u8>>,
    {
        if let Some(metrics) = &self.metrics {
            metrics.note_prolly_read_values(values);
        }
    }

    fn note_prolly_write_call(&self, key_count: usize, value_bytes: usize) {
        if let Some(metrics) = &self.metrics {
            metrics.note_prolly_write_call(key_count, value_bytes);
        }
    }
}

#[derive(Debug)]
struct TrailProllyStoreError {
    message: String,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl TrailProllyStoreError {
    fn with_source(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }
}

impl std::fmt::Display for TrailProllyStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Trail prolly store error: {}", self.message)
    }
}

impl std::error::Error for TrailProllyStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl Store for TrailProllyStore {
    type Error = TrailProllyStoreError;

    fn get(&self, key: &[u8]) -> std::result::Result<Option<Vec<u8>>, Self::Error> {
        self.note_prolly_read_call(1);
        let result = match &self.backend {
            TrailProllyStoreBackend::Sqlite(store) => store
                .get(key)
                .map_err(|err| TrailProllyStoreError::with_source("SQLite prolly get failed", err)),
            TrailProllyStoreBackend::SlateDb(store) => store.get(key).map_err(|err| {
                TrailProllyStoreError::with_source("SlateDB prolly get failed", err)
            }),
        };
        if let Ok(Some(value)) = &result {
            self.note_prolly_read_values(std::iter::once(value));
        }
        result
    }

    fn put(&self, key: &[u8], value: &[u8]) -> std::result::Result<(), Self::Error> {
        self.note_prolly_write_call(1, value.len());
        match &self.backend {
            TrailProllyStoreBackend::Sqlite(store) => store
                .put(key, value)
                .map_err(|err| TrailProllyStoreError::with_source("SQLite prolly put failed", err)),
            TrailProllyStoreBackend::SlateDb(store) => store.put(key, value).map_err(|err| {
                TrailProllyStoreError::with_source("SlateDB prolly put failed", err)
            }),
        }
    }

    fn delete(&self, key: &[u8]) -> std::result::Result<(), Self::Error> {
        self.note_prolly_write_call(1, 0);
        match &self.backend {
            TrailProllyStoreBackend::Sqlite(store) => store.delete(key).map_err(|err| {
                TrailProllyStoreError::with_source("SQLite prolly delete failed", err)
            }),
            TrailProllyStoreBackend::SlateDb(store) => store.delete(key).map_err(|err| {
                TrailProllyStoreError::with_source("SlateDB prolly delete failed", err)
            }),
        }
    }

    fn batch(&self, ops: &[BatchOp]) -> std::result::Result<(), Self::Error> {
        let value_bytes = ops
            .iter()
            .map(|op| match op {
                BatchOp::Upsert { value, .. } => value.len(),
                BatchOp::Delete { .. } => 0,
            })
            .fold(0usize, usize::saturating_add);
        self.note_prolly_write_call(ops.len(), value_bytes);
        match &self.backend {
            TrailProllyStoreBackend::Sqlite(store) => store.batch(ops).map_err(|err| {
                TrailProllyStoreError::with_source("SQLite prolly batch failed", err)
            }),
            TrailProllyStoreBackend::SlateDb(store) => store.batch(ops).map_err(|err| {
                TrailProllyStoreError::with_source("SlateDB prolly batch failed", err)
            }),
        }
    }

    fn batch_get(
        &self,
        keys: &[&[u8]],
    ) -> std::result::Result<HashMap<Vec<u8>, Vec<u8>>, Self::Error> {
        self.note_prolly_read_call(keys.len());
        let result = match &self.backend {
            TrailProllyStoreBackend::Sqlite(store) => store.batch_get(keys).map_err(|err| {
                TrailProllyStoreError::with_source("SQLite prolly batch_get failed", err)
            }),
            TrailProllyStoreBackend::SlateDb(store) => store.batch_get(keys).map_err(|err| {
                TrailProllyStoreError::with_source("SlateDB prolly batch_get failed", err)
            }),
        };
        if let Ok(values) = &result {
            self.note_prolly_read_values(values.values());
        }
        result
    }

    fn batch_get_ordered(
        &self,
        keys: &[&[u8]],
    ) -> std::result::Result<Vec<Option<Vec<u8>>>, Self::Error> {
        self.note_prolly_read_call(keys.len());
        let result = match &self.backend {
            TrailProllyStoreBackend::Sqlite(store) => {
                store.batch_get_ordered(keys).map_err(|err| {
                    TrailProllyStoreError::with_source(
                        "SQLite prolly batch_get_ordered failed",
                        err,
                    )
                })
            }
            TrailProllyStoreBackend::SlateDb(store) => {
                store.batch_get_ordered(keys).map_err(|err| {
                    TrailProllyStoreError::with_source(
                        "SlateDB prolly batch_get_ordered failed",
                        err,
                    )
                })
            }
        };
        if let Ok(values) = &result {
            self.note_prolly_read_values(values.iter().filter_map(Option::as_ref));
        }
        result
    }

    fn batch_put(&self, entries: &[(&[u8], &[u8])]) -> std::result::Result<(), Self::Error> {
        let value_bytes = entries
            .iter()
            .map(|(_, value)| value.len())
            .fold(0usize, usize::saturating_add);
        self.note_prolly_write_call(entries.len(), value_bytes);
        match &self.backend {
            TrailProllyStoreBackend::Sqlite(store) => store.batch_put(entries).map_err(|err| {
                TrailProllyStoreError::with_source("SQLite prolly batch_put failed", err)
            }),
            TrailProllyStoreBackend::SlateDb(store) => store.batch_put(entries).map_err(|err| {
                TrailProllyStoreError::with_source("SlateDB prolly batch_put failed", err)
            }),
        }
    }

    fn supports_hints(&self) -> bool {
        match &self.backend {
            TrailProllyStoreBackend::Sqlite(store) => store.supports_hints(),
            TrailProllyStoreBackend::SlateDb(store) => store.supports_hints(),
        }
    }

    fn get_hint(
        &self,
        namespace: &[u8],
        key: &[u8],
    ) -> std::result::Result<Option<Vec<u8>>, Self::Error> {
        match &self.backend {
            TrailProllyStoreBackend::Sqlite(store) => {
                store.get_hint(namespace, key).map_err(|err| {
                    TrailProllyStoreError::with_source("SQLite prolly get_hint failed", err)
                })
            }
            TrailProllyStoreBackend::SlateDb(store) => {
                store.get_hint(namespace, key).map_err(|err| {
                    TrailProllyStoreError::with_source("SlateDB prolly get_hint failed", err)
                })
            }
        }
    }

    fn put_hint(
        &self,
        namespace: &[u8],
        key: &[u8],
        value: &[u8],
    ) -> std::result::Result<(), Self::Error> {
        match &self.backend {
            TrailProllyStoreBackend::Sqlite(store) => {
                store.put_hint(namespace, key, value).map_err(|err| {
                    TrailProllyStoreError::with_source("SQLite prolly put_hint failed", err)
                })
            }
            TrailProllyStoreBackend::SlateDb(store) => {
                store.put_hint(namespace, key, value).map_err(|err| {
                    TrailProllyStoreError::with_source("SlateDB prolly put_hint failed", err)
                })
            }
        }
    }

    fn batch_put_with_hint(
        &self,
        entries: &[(&[u8], &[u8])],
        namespace: &[u8],
        key: &[u8],
        value: &[u8],
    ) -> std::result::Result<(), Self::Error> {
        let value_bytes = entries
            .iter()
            .map(|(_, value)| value.len())
            .fold(0usize, usize::saturating_add);
        self.note_prolly_write_call(entries.len(), value_bytes);
        match &self.backend {
            TrailProllyStoreBackend::Sqlite(store) => store
                .batch_put_with_hint(entries, namespace, key, value)
                .map_err(|err| {
                    TrailProllyStoreError::with_source(
                        "SQLite prolly batch_put_with_hint failed",
                        err,
                    )
                }),
            TrailProllyStoreBackend::SlateDb(store) => store
                .batch_put_with_hint(entries, namespace, key, value)
                .map_err(|err| {
                    TrailProllyStoreError::with_source(
                        "SlateDB prolly batch_put_with_hint failed",
                        err,
                    )
                }),
        }
    }
}

fn open_prolly_store(
    config: &TrailConfig,
    sqlite_path: &Path,
    metrics: Option<Arc<OperationMetricsState>>,
    schema_mode: SchemaOpenMode,
) -> Result<TrailProllyStore> {
    let backend = match config.storage.prolly_backend.as_str() {
        "sqlite" => {
            let store = match schema_mode {
                SchemaOpenMode::FreshCreate => SqliteStore::open(sqlite_path)?,
                SchemaOpenMode::Existing => SqliteStore::open_existing(sqlite_path)?,
            };
            TrailProllyStoreBackend::Sqlite(Arc::new(store))
        }
        "slatedb" => open_slatedb_prolly_store(&config.storage)?,
        other => Err(Error::InvalidInput(format!(
            "storage.prolly_backend must be sqlite or slatedb, got `{other}`"
        )))?,
    };
    Ok(TrailProllyStore::new(backend, metrics))
}

fn open_slatedb_prolly_store(storage: &StorageConfig) -> Result<TrailProllyStoreBackend> {
    let path = storage.slatedb_path.trim().trim_matches('/');
    if path.is_empty() {
        return Err(Error::InvalidInput(
            "storage.slatedb_path must not be empty".to_string(),
        ));
    }

    let object_store = build_slatedb_object_store(storage)?;
    let store = SlateDbStore::open(path, object_store)?;
    Ok(TrailProllyStoreBackend::SlateDb(Arc::new(store)))
}

fn build_slatedb_object_store(storage: &StorageConfig) -> Result<Arc<dyn ObjectStore>> {
    if storage.slatedb_s3_endpoint.trim().is_empty() {
        return Err(Error::InvalidInput(
            "storage.slatedb_s3_endpoint must not be empty".to_string(),
        ));
    }
    if storage.slatedb_s3_bucket.trim().is_empty() {
        return Err(Error::InvalidInput(
            "storage.slatedb_s3_bucket must not be empty".to_string(),
        ));
    }
    if storage.slatedb_s3_region.trim().is_empty() {
        return Err(Error::InvalidInput(
            "storage.slatedb_s3_region must not be empty".to_string(),
        ));
    }

    let store = AmazonS3Builder::new()
        .with_endpoint(storage.slatedb_s3_endpoint.trim_end_matches('/'))
        .with_bucket_name(storage.slatedb_s3_bucket.trim())
        .with_region(storage.slatedb_s3_region.trim())
        .with_access_key_id(&storage.slatedb_s3_access_key_id)
        .with_secret_access_key(&storage.slatedb_s3_secret_access_key)
        .with_allow_http(storage.slatedb_s3_allow_http)
        .with_virtual_hosted_style_request(false)
        .build()
        .map_err(|err| {
            Error::InvalidInput(format!(
                "failed to configure SlateDB S3 object store: {err}"
            ))
        })?;

    Ok(Arc::new(store))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GitExportPolicy {
    RequireMappedDelta,
    AllowFullSnapshot,
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
pub(crate) enum RecordCaseFoldResolutionState {
    Indexed {
        previous_tree: Tree,
        mutations: Vec<prolly::Mutation>,
    },
    LegacyUnavailable,
    Collision {
        path: String,
        previous: String,
    },
}

#[derive(Debug)]
pub(crate) struct RecordCaseFoldResolution {
    selected_paths: Vec<String>,
    expected_final_present_paths: BTreeSet<String>,
    expected_observed_present_paths: BTreeSet<String>,
    expected_absent_paths: BTreeSet<String>,
    state: RecordCaseFoldResolutionState,
}

#[derive(Debug)]
pub(crate) struct RecordCaseFoldPreflight {
    selected_paths: Vec<String>,
    expected_final_present_paths: BTreeSet<String>,
    expected_observed_present_paths: BTreeSet<String>,
    expected_absent_paths: BTreeSet<String>,
    case_fold_tree: Tree,
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
    trail_version: String,
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
    watcher: Option<notify::RecommendedWatcher>,
}

#[derive(Clone, Debug)]
pub(crate) struct DaemonWorktreeCachePersist {
    path: PathBuf,
    workspace_root: PathBuf,
    pid: u32,
    active: Arc<AtomicBool>,
    metrics: Option<Arc<OperationMetricsState>>,
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
    policy_invalidation_index: Option<change_ledger::PolicyInvalidationIndex>,
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
    lane_queue_id: Option<String>,
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

#[derive(Debug, Clone)]
pub(crate) struct GitIdentity {
    head: String,
    branch: Option<String>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GitHandoffMetrics {
    export_mode: GitExportMode,
    changed_path_count: u64,
    blob_write_count: u64,
    git_plumbing_command_count: u64,
    tracked_status_count: u64,
    full_root_file_count: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CaseFoldIndexMetrics {
    mode: CaseFoldIndexMode,
    lookup_count: u64,
    full_root_path_load_count: u64,
    full_filesystem_path_scan_count: u64,
}

#[derive(Clone, Copy, Debug, Default)]
enum CaseFoldIndexMode {
    #[default]
    Unknown,
    Indexed,
}

#[allow(dead_code)] // Reported by Task 5's scale harness; tests use it in this slice.
impl CaseFoldIndexMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Indexed => "indexed",
        }
    }
}

pub(crate) type CaseFoldIndexMetricsReport = PathIndexMetricsReport;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) enum GitExportMode {
    #[default]
    Unknown,
    MappedDelta,
    FullSnapshot,
}

impl GitExportMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::MappedDelta => "mapped_delta",
            Self::FullSnapshot => "full_snapshot",
        }
    }
}

impl From<GitHandoffMetrics> for GitHandoffMetricsReport {
    fn from(metrics: GitHandoffMetrics) -> Self {
        Self {
            export_mode: metrics.export_mode.as_str().to_string(),
            changed_path_count: metrics.changed_path_count,
            blob_write_count: metrics.blob_write_count,
            git_plumbing_command_count: metrics.git_plumbing_command_count,
            tracked_status_count: metrics.tracked_status_count,
            full_root_file_count: metrics.full_root_file_count,
        }
    }
}

pub(crate) fn validate_git_publication_state(expected_head: &str, state: &GitState) -> Result<()> {
    if state.head.as_deref() != Some(expected_head) {
        return Err(Error::GitHeadChanged(format!(
            "expected Git HEAD `{expected_head}`, found `{}`",
            state.head.as_deref().unwrap_or("<unborn>")
        )));
    }
    if state.dirty {
        return Err(Error::GitWorktreeDirty(
            "current Git worktree has tracked changes; commit, stash, or revert them before `trail agent apply`"
                .to_string(),
        ));
    }
    Ok(())
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
mod change_ledger;
#[cfg(all(debug_assertions, unix))]
pub(crate) use change_ledger::run_non_utf_database_path_mark_recover_and_retire;
#[cfg(debug_assertions)]
pub(crate) use change_ledger::{
    run_acknowledgement_race, run_advanced_prefix_recovery, run_ambiguous_recovery_gate,
    run_backup_overwrite_rollback, run_backup_restore_rotation, run_callback_spool,
    run_crash_matrix, run_deletion_normal_retry_idempotence,
    run_deletion_parent_substitution_rejection,
    run_deletion_post_quarantine_verification_substitution_rejection,
    run_deletion_post_verification_substitution_rejection,
    run_deletion_quiesced_missing_quarantine_rejection,
    run_deletion_quiesced_reappeared_original_rejection,
    run_deletion_retry_hostile_quarantine_replacement_rejection,
    run_exact_interval_bridge_rejection, run_gc_root_lifecycle, run_lane_deletion_retirement,
    run_missing_sidecar_rejection, run_oracle, run_prefix_interval_bridge_rejection,
    run_qualified_proof_revalidation, run_races, run_restored_nullable_provider_lane_deletion,
    run_retirement_barrier, run_valid_prefix_interval_recovery,
};
#[cfg(all(debug_assertions, unix))]
pub(crate) use change_ledger::{
    run_deletion_leaf_substitution_rejection, run_mark_ancestor_substitution_rejection,
    run_recovery_ancestor_substitution_rejection,
};
#[cfg(all(debug_assertions, any(target_os = "linux", target_os = "macos")))]
pub(crate) use change_ledger::{
    run_empty_orphan_quarantine_rejection, run_no_orphan_quarantine_allocation,
    run_orphan_quarantine_substitution_rejection,
};
mod core;
mod lane;
mod merge;
mod performance;
mod record;
mod storage;
use self::performance::*;
pub(crate) use storage::{observed_exact_paths_for_candidates, ObservedPathKind};
mod util;

#[doc(hidden)]
pub use self::util::process_liveness::run_internal_process_watchdog;
pub(crate) use self::util::redact_sensitive_json;

#[cfg(test)]
mod tests {
    use super::util::*;
    use super::*;

    #[test]
    fn operation_metrics_scope_nests_and_resets_after_errors_retries_and_cancellation() {
        let metrics = Arc::new(OperationMetricsState::default());

        let first: Result<()> = metrics.profile(OperationMetricsKind::Status, || {
            metrics.add(OperationMetricsDelta {
                input_path_count: 3,
                ..OperationMetricsDelta::default()
            });
            metrics.profile(OperationMetricsKind::Diff, || {
                metrics.add(OperationMetricsDelta {
                    final_path_count: 2,
                    ..OperationMetricsDelta::default()
                });
                Ok::<(), Error>(())
            })?;
            Err(Error::InvalidInput(
                "expected metric test failure".to_string(),
            ))
        });
        assert!(first.is_err());
        let failed = metrics.last_report();
        assert_eq!(failed.generation, 1);
        assert_eq!(failed.operation, "status");
        assert_eq!(failed.outcome, OperationMetricsOutcome::Error);
        assert_eq!(failed.input_path_count, 3);
        assert_eq!(failed.final_path_count, 2);

        metrics
            .profile(OperationMetricsKind::Status, || {
                metrics.add(OperationMetricsDelta {
                    input_path_count: 1,
                    ..OperationMetricsDelta::default()
                });
                Ok::<(), Error>(())
            })
            .unwrap();
        let retry = metrics.last_report();
        assert_eq!(retry.generation, 2);
        assert_eq!(retry.outcome, OperationMetricsOutcome::Success);
        assert_eq!(retry.input_path_count, 1);
        assert_eq!(retry.final_path_count, 0);

        let cancelled = std::panic::catch_unwind(std::panic::AssertUnwindSafe({
            let metrics = Arc::clone(&metrics);
            move || {
                metrics.profile(OperationMetricsKind::Record, || -> Result<()> {
                    metrics.add(OperationMetricsDelta {
                        expanded_path_count: 7,
                        ..OperationMetricsDelta::default()
                    });
                    panic!("cancel metric scope")
                })
            }
        }));
        assert!(cancelled.is_err());
        let cancelled = metrics.last_report();
        assert_eq!(cancelled.generation, 3);
        assert_eq!(cancelled.operation, "record");
        assert_eq!(
            cancelled.outcome,
            OperationMetricsOutcome::CancelledOrUnclassified
        );
        assert_eq!(cancelled.expanded_path_count, 7);
        assert_eq!(cancelled.input_path_count, 0);
    }

    #[test]
    fn trail_prolly_store_reports_calls_requested_keys_found_values_and_bytes_across_clones() {
        let metrics = Arc::new(OperationMetricsState::default());
        let store = TrailProllyStore::new(
            TrailProllyStoreBackend::Sqlite(Arc::new(SqliteStore::open_in_memory().unwrap())),
            Some(Arc::clone(&metrics)),
        );
        store.put(b"present", b"abc").unwrap();
        let clone = store.clone();

        metrics
            .profile(OperationMetricsKind::Diff, || {
                store.put(b"written", b"xyz").unwrap();
                store
                    .batch(&[
                        BatchOp::Upsert {
                            key: b"batch-written",
                            value: b"de",
                        },
                        BatchOp::Delete {
                            key: b"batch-missing",
                        },
                    ])
                    .unwrap();
                store
                    .batch_put(&[(b"batch-put".as_slice(), b"fgh".as_slice())])
                    .unwrap();
                store.delete(b"delete-missing").unwrap();
                store
                    .batch_put_with_hint(
                        &[(b"hinted-node".as_slice(), b"ijkl".as_slice())],
                        b"test-namespace",
                        b"test-key",
                        b"performance-hint-not-a-node",
                    )
                    .unwrap();
                assert_eq!(store.get(b"present").unwrap(), Some(b"abc".to_vec()));
                assert_eq!(store.get(b"missing").unwrap(), None);
                let unordered = clone.batch_get(&[b"present", b"missing", b"present"])?;
                assert_eq!(unordered.len(), 1);
                let ordered = store.batch_get_ordered(&[b"present", b"missing", b"present"])?;
                assert_eq!(ordered.iter().filter(|value| value.is_some()).count(), 2);
                Ok::<(), TrailProllyStoreError>(())
            })
            .unwrap();

        let report = metrics.last_report();
        assert_eq!(report.prolly_read_call_count, 4);
        assert_eq!(report.prolly_read_key_count, 8);
        assert_eq!(report.prolly_read_value_count, 4);
        assert_eq!(report.prolly_read_value_bytes, 12);
        assert_eq!(report.prolly_write_call_count, 5);
        assert_eq!(report.prolly_write_key_count, 6);
        assert_eq!(report.prolly_write_value_bytes, 12);
    }

    #[test]
    #[ignore = "reproducible release-mode microbenchmark; run explicitly for performance evidence"]
    fn operation_metrics_store_read_overhead_benchmark() {
        const READS_PER_SAMPLE: u64 = 50_000;
        const SAMPLES: usize = 7;

        let raw = SqliteStore::open_in_memory().unwrap();
        raw.put(b"present", b"abc").unwrap();
        let disabled = TrailProllyStore::new(
            TrailProllyStoreBackend::Sqlite(Arc::new(SqliteStore::open_in_memory().unwrap())),
            None,
        );
        disabled.put(b"present", b"abc").unwrap();
        let metrics = Arc::new(OperationMetricsState::default());
        let measured = TrailProllyStore::new(
            TrailProllyStoreBackend::Sqlite(Arc::new(SqliteStore::open_in_memory().unwrap())),
            Some(Arc::clone(&metrics)),
        );
        measured.put(b"present", b"abc").unwrap();

        let mut raw_samples = Vec::with_capacity(SAMPLES);
        let mut disabled_samples = Vec::with_capacity(SAMPLES);
        let mut measured_samples = Vec::with_capacity(SAMPLES);
        for sample in 0..SAMPLES {
            let run_raw = || {
                let started = Instant::now();
                for _ in 0..READS_PER_SAMPLE {
                    std::hint::black_box(raw.get(b"present").unwrap());
                }
                started.elapsed().as_nanos() as u64
            };
            let run_measured = || {
                let started = Instant::now();
                for _ in 0..READS_PER_SAMPLE {
                    std::hint::black_box(measured.get(b"present").unwrap());
                }
                started.elapsed().as_nanos() as u64
            };
            let run_disabled = || {
                let started = Instant::now();
                for _ in 0..READS_PER_SAMPLE {
                    std::hint::black_box(disabled.get(b"present").unwrap());
                }
                started.elapsed().as_nanos() as u64
            };
            match sample % 3 {
                0 => {
                    raw_samples.push(run_raw());
                    disabled_samples.push(run_disabled());
                    measured_samples.push(run_measured());
                }
                1 => {
                    disabled_samples.push(run_disabled());
                    measured_samples.push(run_measured());
                    raw_samples.push(run_raw());
                }
                _ => {
                    measured_samples.push(run_measured());
                    raw_samples.push(run_raw());
                    disabled_samples.push(run_disabled());
                }
            }
        }
        raw_samples.sort_unstable();
        disabled_samples.sort_unstable();
        measured_samples.sort_unstable();
        let raw_ns_per_read = raw_samples[SAMPLES / 2] as f64 / READS_PER_SAMPLE as f64;
        let disabled_ns_per_read = disabled_samples[SAMPLES / 2] as f64 / READS_PER_SAMPLE as f64;
        let measured_ns_per_read = measured_samples[SAMPLES / 2] as f64 / READS_PER_SAMPLE as f64;
        let disabled_overhead_percent =
            ((disabled_ns_per_read / raw_ns_per_read) - 1.0).mul_add(100.0, 0.0);
        let enabled_overhead_percent =
            ((measured_ns_per_read / raw_ns_per_read) - 1.0).mul_add(100.0, 0.0);
        println!(
            "operation_metrics_store_read raw_ns_per_read={raw_ns_per_read:.2} disabled_ns_per_read={disabled_ns_per_read:.2} enabled_ns_per_read={measured_ns_per_read:.2} disabled_overhead_percent={disabled_overhead_percent:.2} enabled_overhead_percent={enabled_overhead_percent:.2} samples={SAMPLES} reads_per_sample={READS_PER_SAMPLE}"
        );
    }

    #[test]
    fn disabled_operation_metrics_skip_scopes_reports_and_store_counters() {
        let disabled = None;
        let result =
            profile_operation_metrics(disabled.as_ref(), OperationMetricsKind::Status, || {
                Ok::<_, Error>("unchanged")
            })
            .unwrap();
        assert_eq!(result, "unchanged");
        assert_eq!(operation_metrics_report(disabled.as_ref()), None);

        let untouched = Arc::new(OperationMetricsState::default());
        let store = TrailProllyStore::new(
            TrailProllyStoreBackend::Sqlite(Arc::new(SqliteStore::open_in_memory().unwrap())),
            None,
        );
        store.put(b"present", b"abc").unwrap();
        assert_eq!(store.get(b"present").unwrap(), Some(b"abc".to_vec()));
        untouched
            .profile(OperationMetricsKind::Diff, || Ok::<(), Error>(()))
            .unwrap();
        let report = untouched.last_report();
        assert_eq!(report.prolly_read_call_count, 0);
        assert_eq!(report.prolly_write_call_count, 0);
    }

    #[test]
    fn operation_metrics_env_parser_accepts_only_documented_truthy_values() {
        for value in ["1", "true", "TRUE", "yes", "YES", "on", "ON"] {
            assert!(operation_metrics_env_value_is_truthy(value), "{value}");
        }
        for value in ["", "0", "false", "enabled", " true", "on ", "2"] {
            assert!(!operation_metrics_env_value_is_truthy(value), "{value}");
        }
    }

    #[test]
    fn operation_metrics_expose_truthful_structural_surface_and_daemon_cumulative_totals() {
        let metrics = Arc::new(OperationMetricsState::default());
        metrics.note_daemon_cumulative_rewrite(11);

        metrics
            .profile(OperationMetricsKind::Record, || {
                metrics.add(OperationMetricsDelta {
                    input_path_count: 1,
                    canonical_path_count: 2,
                    expanded_path_count: 3,
                    final_path_count: 4,
                    full_filesystem_walk_count: 5,
                    bounded_filesystem_walk_count: 6,
                    filesystem_entry_count: 7,
                    filesystem_stat_count: 8,
                    filesystem_read_count: 9,
                    filesystem_read_bytes: 10,
                    filesystem_hash_count: 11,
                    filesystem_hash_bytes: 12,
                    full_root_range_count: 13,
                    bounded_root_range_count: 14,
                    root_range_row_count: 15,
                    root_point_key_count: 16,
                    prolly_tree_batch_call_count: 17,
                    prolly_tree_batch_mutation_count: 18,
                    selected_worktree_index_sqlite_envelope_count: 1,
                    selected_worktree_index_sqlite_full_scan_count: 19,
                    selected_worktree_index_sqlite_row_read_count: 20,
                    selected_worktree_index_sqlite_row_delete_count: 21,
                    selected_worktree_index_sqlite_row_upsert_count: 22,
                    selected_worktree_index_sqlite_statement_count: 23,
                    selected_worktree_index_sqlite_transaction_count: 24,
                    selection_comparison_count: 25,
                    policy_build_count: 26,
                    policy_dependency_bytes: 27,
                    policy_dependency_file_count: 28,
                    git_subprocess_count: 29,
                    git_global_work_count: 30,
                    git_output_bytes: 31,
                    git_output_record_count: 32,
                    daemon_snapshot_bytes: 33,
                    daemon_snapshot_path_count: 34,
                    manifest_bytes: 35,
                    manifest_key_comparison_count: 36,
                    journal_bytes: 37,
                    upper_work_count: 38,
                    ..OperationMetricsDelta::default()
                });
                metrics.note_daemon_cumulative_rewrite(13);
                Ok::<(), Error>(())
            })
            .unwrap();

        let report = metrics.last_report();
        assert_eq!(report.input_path_count, 1);
        assert_eq!(report.canonical_path_count, 2);
        assert_eq!(report.expanded_path_count, 3);
        assert_eq!(report.final_path_count, 4);
        assert_eq!(report.full_filesystem_walk_count, 5);
        assert_eq!(report.bounded_filesystem_walk_count, 6);
        assert_eq!(report.filesystem_entry_count, 7);
        assert_eq!(report.filesystem_stat_count, 8);
        assert_eq!(report.filesystem_read_count, 9);
        assert_eq!(report.filesystem_read_bytes, 10);
        assert_eq!(report.filesystem_hash_count, 11);
        assert_eq!(report.filesystem_hash_bytes, 12);
        assert_eq!(report.full_root_range_count, 13);
        assert_eq!(report.bounded_root_range_count, 14);
        assert_eq!(report.root_range_row_count, 15);
        assert_eq!(report.root_point_key_count, 16);
        assert_eq!(report.prolly_tree_batch_call_count, 17);
        assert_eq!(report.prolly_tree_batch_mutation_count, 18);
        assert!(report.selected_worktree_index_sqlite_accounting_complete);
        assert_eq!(report.selected_worktree_index_sqlite_envelope_count, 1);
        assert_eq!(report.selected_worktree_index_sqlite_full_scan_count, 19);
        assert_eq!(report.selected_worktree_index_sqlite_row_read_count, 20);
        assert_eq!(report.selected_worktree_index_sqlite_row_delete_count, 21);
        assert_eq!(report.selected_worktree_index_sqlite_row_upsert_count, 22);
        assert_eq!(report.selected_worktree_index_sqlite_statement_count, 23);
        assert_eq!(report.selected_worktree_index_sqlite_transaction_count, 24);
        assert_eq!(report.selection_comparison_count, 25);
        assert_eq!(report.policy_build_count, 26);
        assert_eq!(report.policy_dependency_bytes, 27);
        assert_eq!(report.policy_dependency_file_count, 28);
        assert_eq!(report.git_subprocess_count, 29);
        assert_eq!(report.git_global_work_count, 30);
        assert_eq!(report.git_output_bytes, 31);
        assert_eq!(report.git_output_record_count, 32);
        assert_eq!(report.daemon_snapshot_bytes, 33);
        assert_eq!(report.daemon_snapshot_path_count, 34);
        assert_eq!(report.manifest_bytes, 35);
        assert_eq!(report.manifest_key_comparison_count, 36);
        assert_eq!(report.journal_bytes, 37);
        assert_eq!(report.upper_work_count, 38);
        assert_eq!(report.daemon_cumulative_rewrite_count, 1);
        assert_eq!(report.daemon_cumulative_rewrite_bytes, 13);
        assert_eq!(report.daemon_cumulative_rewrite_count_total, 2);
        assert_eq!(report.daemon_cumulative_rewrite_bytes_total, 24);
        assert!(report.wall_time_ns > 0);
        assert!(report.rss_end_bytes <= report.rss_lifetime_high_water_bytes);
        assert!(report.rss_start_bytes <= report.rss_lifetime_high_water_bytes);
    }

    #[test]
    fn daemon_rewrite_count_and_bytes_are_snapshotted_as_one_event() {
        const REWRITES: usize = 20_000;
        const BYTES_PER_REWRITE: u64 = 7;
        let metrics = Arc::new(OperationMetricsState::default());
        let writer_metrics = Arc::clone(&metrics);
        let writer = std::thread::spawn(move || {
            for _ in 0..REWRITES {
                writer_metrics.note_daemon_cumulative_rewrite(BYTES_PER_REWRITE as usize);
            }
        });

        while !writer.is_finished() {
            let snapshot = metrics.snapshot();
            assert_eq!(
                snapshot.daemon_cumulative_rewrite_bytes,
                snapshot
                    .daemon_cumulative_rewrite_count
                    .saturating_mul(BYTES_PER_REWRITE)
            );
        }
        writer.join().unwrap();
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.daemon_cumulative_rewrite_count, REWRITES as u64);
        assert_eq!(
            snapshot.daemon_cumulative_rewrite_bytes,
            (REWRITES as u64).saturating_mul(BYTES_PER_REWRITE)
        );
    }

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
            ".trail",
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
                    serde_json::json!("line_abc:1")
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
