use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::{symlink as symlink_file, FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Condvar, Mutex, OnceLock,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ignore::WalkBuilder;
use prolly::{
    BatchBuilder, BatchOp, Cid, Config, Diff, Encoding, Prolly, SortedBatchBuilder, Store, Tree,
};
use prolly_store_slatedb::SlateDbStore;
#[cfg(unix)]
use prolly_store_sqlite::sqlite_main_file_identity;
use prolly_store_sqlite::{SqliteMainFileIdentity, SqliteStore};
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
const SCHEMA_EXCLUSION_FILE: &str = "schema-exclusion.lock";
const SCHEMA_VALIDATION_LEADER_FILE: &str = "schema-validation.lock";
const TRAIL_SCHEMA_VERSION: i64 = 19;
const SCHEMA_V18_VERSION: i64 = 18;
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct SchemaFileGeneration {
    suffix: &'static str,
    present: bool,
    device: u64,
    inode: u64,
    length: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SchemaGeneration(Vec<SchemaFileGeneration>);

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Debug)]
enum CrossProcessSchemaValidationOutcome {
    Success(String),
    Failure(String),
}

#[derive(Default)]
struct SchemaValidationState {
    validating: bool,
    active_handoffs: u64,
    round: u64,
    validated: Option<(SchemaGeneration, String, String)>,
    failed: Option<(u64, String)>,
    validation_count: u64,
}

#[derive(Default)]
struct SchemaValidationEntry {
    state: Mutex<SchemaValidationState>,
    changed: Condvar,
}

static SCHEMA_VALIDATIONS: OnceLock<Mutex<HashMap<PathBuf, Arc<SchemaValidationEntry>>>> =
    OnceLock::new();

#[cfg(any(target_os = "linux", target_os = "macos"))]
static SCHEMA_VALIDATION_SERVERS: OnceLock<Mutex<HashMap<PathBuf, ActiveSchemaValidationServer>>> =
    OnceLock::new();

#[cfg(any(target_os = "linux", target_os = "macos"))]
static NEXT_SCHEMA_VALIDATION_SERVER_ID: AtomicU64 = AtomicU64::new(1);

#[cfg(test)]
static SCHEMA_VALIDATION_FAILURES: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
#[cfg(test)]
type SchemaSnapshotGenerationHook = Box<dyn FnOnce(&Path) + Send>;
#[cfg(test)]
static NEXT_SCHEMA_SNAPSHOT_GENERATION_HOOKS: OnceLock<
    Mutex<HashMap<PathBuf, SchemaSnapshotGenerationHook>>,
> = OnceLock::new();

#[cfg(test)]
static NEXT_SCHEMA_VALIDATION_SERVER_DELAYS: OnceLock<Mutex<HashMap<PathBuf, (u64, u64)>>> =
    OnceLock::new();

#[cfg(all(test, target_os = "linux"))]
type SchemaSocketAuthorityHook = Box<dyn FnOnce(&Path) + Send>;

#[cfg(all(test, target_os = "linux"))]
static NEXT_SCHEMA_SOCKET_AUTHORITY_HOOKS: OnceLock<
    Mutex<HashMap<PathBuf, SchemaSocketAuthorityHook>>,
> = OnceLock::new();

#[cfg(all(test, unix))]
type SchemaWalDigestHook = Box<dyn FnOnce(&Path) + Send>;

#[cfg(all(test, unix))]
static NEXT_SCHEMA_WAL_DIGEST_HOOKS: OnceLock<Mutex<HashMap<PathBuf, SchemaWalDigestHook>>> =
    OnceLock::new();

#[cfg(all(test, unix))]
type SchemaWalDigestAttemptHook = Box<dyn FnMut(&Path) + Send>;

#[cfg(all(test, unix))]
static SCHEMA_WAL_DIGEST_ATTEMPT_HOOKS: OnceLock<
    Mutex<HashMap<PathBuf, SchemaWalDigestAttemptHook>>,
> = OnceLock::new();

#[cfg(all(test, target_os = "linux"))]
fn install_schema_socket_authority_hook(path: &Path, hook: SchemaSocketAuthorityHook) {
    NEXT_SCHEMA_SOCKET_AUTHORITY_HOOKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(path.to_path_buf(), hook);
}

#[cfg(all(test, unix))]
fn install_schema_wal_digest_hook(path: &Path, hook: SchemaWalDigestHook) {
    NEXT_SCHEMA_WAL_DIGEST_HOOKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(path.to_path_buf(), hook);
}

#[cfg(all(test, unix))]
fn install_schema_wal_digest_attempt_hook(path: &Path, hook: SchemaWalDigestAttemptHook) {
    SCHEMA_WAL_DIGEST_ATTEMPT_HOOKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(path.to_path_buf(), hook);
}

#[cfg(all(test, unix))]
fn clear_schema_wal_digest_attempt_hook(path: &Path) {
    SCHEMA_WAL_DIGEST_ATTEMPT_HOOKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(path);
}

#[cfg(test)]
fn fail_next_schema_validation(db_path: &Path) {
    SCHEMA_VALIDATION_FAILURES
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(db_path.to_path_buf());
}

#[cfg(test)]
fn install_schema_snapshot_generation_hook(
    db_path: &Path,
    hook: impl FnOnce(&Path) + Send + 'static,
) {
    NEXT_SCHEMA_SNAPSHOT_GENERATION_HOOKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(db_path.to_path_buf(), Box::new(hook));
}

#[cfg(test)]
fn run_schema_snapshot_generation_hook(db_path: &Path) {
    let hook = NEXT_SCHEMA_SNAPSHOT_GENERATION_HOOKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(db_path);
    if let Some(hook) = hook {
        hook(db_path);
    }
}

#[cfg(test)]
fn delay_next_schema_validation_server_start_for_test(db_path: &Path, delay: Duration) {
    NEXT_SCHEMA_VALIDATION_SERVER_DELAYS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(db_path.to_path_buf())
        .or_default()
        .0 = delay.as_millis() as u64;
}

#[cfg(test)]
fn delay_next_schema_validation_server_shutdown_for_test(db_path: &Path, delay: Duration) {
    NEXT_SCHEMA_VALIDATION_SERVER_DELAYS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(db_path.to_path_buf())
        .or_default()
        .1 = delay.as_millis() as u64;
}

#[cfg(test)]
fn take_schema_validation_server_delays_for_test(db_path: &Path) -> (Duration, Duration) {
    let (start, shutdown) = NEXT_SCHEMA_VALIDATION_SERVER_DELAYS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(db_path)
        .unwrap_or_default();
    (
        Duration::from_millis(start),
        Duration::from_millis(shutdown),
    )
}

#[cfg(test)]
fn schema_validation_count(db_path: &Path) -> u64 {
    SCHEMA_VALIDATIONS
        .get()
        .and_then(|entries| {
            entries
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(db_path)
                .cloned()
        })
        .map(|entry| {
            entry
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .validation_count
        })
        .unwrap_or(0)
}

pub(crate) struct ValidatedSchemaGeneration {
    db_path: PathBuf,
    generation: SchemaGeneration,
    entry: Arc<SchemaValidationEntry>,
    _parent_authority: File,
    main_authority: File,
    _shared_exclusion: SchemaSharedExclusion,
    _leader_exclusion: Option<File>,
    authenticated_wal_digest: String,
}

impl ValidatedSchemaGeneration {
    pub(crate) fn verify_unchanged(&self) -> Result<()> {
        let current = schema_generation(&self.db_path).map_err(schema_reinitialize_error)?;
        let concurrent_handoffs = self
            .entry
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active_handoffs
            > 1;
        if current != self.generation
            && !schema_generation_is_only_volatile_shm_presence_transition(
                &self.generation,
                &current,
                concurrent_handoffs,
            )
            && !(schema_generation_is_only_wal_ctime_transition(&self.generation, &current)
                && schema_wal_digest_allowing_wal_ctime_transition(&self.db_path, &current)
                    .is_ok_and(|digest| digest == self.authenticated_wal_digest))
        {
            return Err(schema_reinitialize_error(
                "schema main/WAL/SHM generation changed during mutable handoff",
            ));
        }
        Ok(())
    }

    pub(crate) fn verify_connection(&self, conn: &Connection) -> Result<()> {
        #[cfg(unix)]
        let identity = sqlite_main_file_identity(conn).map_err(schema_reinitialize_error)?;
        #[cfg(not(unix))]
        let identity = SqliteMainFileIdentity {
            device: 0,
            inode: 0,
            length: 0,
        };
        self.verify_main_identity(identity)
    }

    pub(crate) fn verify_main_identity(&self, identity: SqliteMainFileIdentity) -> Result<()> {
        let expected = self
            .generation
            .0
            .iter()
            .find(|file| file.suffix.is_empty() && file.present)
            .ok_or_else(|| schema_reinitialize_error("validated schema has no main database"))?;
        #[cfg(unix)]
        {
            let retained = self
                .main_authority
                .metadata()
                .map_err(schema_reinitialize_error)?;
            if identity.device != expected.device
                || identity.inode != expected.inode
                || identity.length != expected.length
                || retained.dev() != expected.device
                || retained.ino() != expected.inode
                || retained.len() != expected.length
            {
                return Err(schema_reinitialize_error(
                    "SQLite main-file handle does not match validated schema authority",
                ));
            }
        }
        #[cfg(not(unix))]
        {
            let _ = identity;
            return Err(schema_reinitialize_error(
                "verified SQLite main-file handles are unsupported on this platform",
            ));
        }
        Ok(())
    }
}

fn schema_generation_is_only_wal_ctime_transition(
    expected: &SchemaGeneration,
    current: &SchemaGeneration,
) -> bool {
    expected.0.len() == current.0.len()
        && expected.0.iter().zip(&current.0).all(|(left, right)| {
            if left.suffix != right.suffix {
                return false;
            }
            if left.suffix == "-wal" {
                return left.present
                    && right.present
                    && left.device == right.device
                    && left.inode == right.inode
                    && left.length == right.length
                    && left.modified_seconds == right.modified_seconds
                    && left.modified_nanoseconds == right.modified_nanoseconds;
            }
            left == right
        })
}

fn schema_generation_is_only_volatile_shm_presence_transition(
    expected: &SchemaGeneration,
    current: &SchemaGeneration,
    concurrent_handoffs: bool,
) -> bool {
    expected.0.len() == current.0.len()
        && expected
            .0
            .iter()
            .zip(&current.0)
            .all(|(expected, current)| {
                if expected.suffix != current.suffix {
                    return false;
                }
                if expected.suffix == "-shm" {
                    // SQLite creates, removes, and recreates SHM as its first and
                    // last live connections cross the handoff. SHM is a rebuildable
                    // lock/index file, not durable schema authority. Permit only a
                    // presence transition or a same-device, same-length inode
                    // rotation; in-place byte/length mutation remains a hard failure.
                    return concurrent_handoffs
                        || expected.present != current.present
                        || (expected.present
                            && current.present
                            && expected.device == current.device
                            && expected.inode != current.inode
                            && expected.length == current.length);
                }
                expected == current
            })
}

#[cfg(unix)]
fn open_schema_main_authority(
    db_path: &Path,
    generation: &SchemaGeneration,
) -> Result<(File, File)> {
    use rustix::fs::{openat, Mode, OFlags, CWD};

    let expected = generation
        .0
        .iter()
        .find(|file| file.suffix.is_empty() && file.present)
        .ok_or_else(|| schema_reinitialize_error("schema main database is missing"))?;
    let parent_path = db_path
        .parent()
        .ok_or_else(|| schema_reinitialize_error("schema database has no parent directory"))?;
    let leaf = db_path
        .file_name()
        .ok_or_else(|| schema_reinitialize_error("schema database has no main-file leaf"))?;
    let parent = File::from(
        openat(
            CWD,
            parent_path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(schema_reinitialize_error)?,
    );
    let file = File::from(
        openat(
            &parent,
            Path::new(leaf),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(schema_reinitialize_error)?,
    );
    let metadata = file.metadata().map_err(schema_reinitialize_error)?;
    if !metadata.is_file()
        || metadata.dev() != expected.device
        || metadata.ino() != expected.inode
        || metadata.len() != expected.length
    {
        return Err(schema_reinitialize_error(
            "schema main-file authority changed after validation",
        ));
    }
    Ok((parent, file))
}

#[cfg(not(unix))]
fn open_schema_main_authority(
    _db_path: &Path,
    _generation: &SchemaGeneration,
) -> Result<(File, File)> {
    Err(schema_reinitialize_error(
        "verified SQLite main-file handles are unsupported on this platform",
    ))
}

impl Drop for ValidatedSchemaGeneration {
    fn drop(&mut self) {
        let mut state = self
            .entry
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.active_handoffs = state.active_handoffs.saturating_sub(1);
        self.entry.changed.notify_all();
    }
}

#[derive(Debug)]
struct SchemaSharedExclusion {
    _database: File,
}

pub(crate) fn preflight_existing_schema(
    db_path: &Path,
    prolly_backend: &str,
) -> Result<ValidatedSchemaGeneration> {
    let shared_exclusion = acquire_schema_shared_exclusion(db_path)?;
    let mut generation = schema_generation(db_path).map_err(schema_reinitialize_error)?;
    let key = db_path.to_path_buf();
    let entry = {
        let entries = SCHEMA_VALIDATIONS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut entries = entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        entries
            .entry(key)
            .or_insert_with(|| Arc::new(SchemaValidationEntry::default()))
            .clone()
    };
    let backend = prolly_backend.to_owned();
    let mut volatile_generation_retries = 0_usize;

    loop {
        let mut state = entry
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state
            .validated
            .as_ref()
            .is_some_and(|(validated, validated_backend, _)| {
                validated == &generation && validated_backend == &backend
            })
        {
            let (parent_authority, main_authority) =
                open_schema_main_authority(db_path, &generation)?;
            let authenticated_wal_digest = state.validated.as_ref().unwrap().2.clone();
            state.active_handoffs = state.active_handoffs.saturating_add(1);
            drop(state);
            return Ok(ValidatedSchemaGeneration {
                db_path: db_path.to_path_buf(),
                generation,
                entry,
                _parent_authority: parent_authority,
                main_authority,
                _shared_exclusion: shared_exclusion,
                _leader_exclusion: None,
                authenticated_wal_digest,
            });
        }
        if state.validating {
            let waited_round = state.round;
            while state.validating && state.round == waited_round {
                state = entry
                    .changed
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            if state
                .failed
                .as_ref()
                .is_some_and(|(round, _)| *round == waited_round)
            {
                let message = state.failed.as_ref().unwrap().1.clone();
                return Err(schema_reinitialize_error(message));
            }
            continue;
        }
        if state.active_handoffs != 0 {
            while state.active_handoffs != 0 {
                state = entry
                    .changed
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            drop(state);
            generation = schema_generation(db_path).map_err(schema_reinitialize_error)?;
            continue;
        }
        state.validating = true;
        state.round = state.round.saturating_add(1);
        state.validation_count = state.validation_count.saturating_add(1);
        let round = state.round;
        drop(state);

        let mut authenticated_wal_digest = None;
        let (mut validation, leader_exclusion) = coordinate_schema_snapshot_validation(
            db_path,
            prolly_backend,
            &mut generation,
            &mut authenticated_wal_digest,
        );
        if validation.is_ok() && authenticated_wal_digest.is_none() {
            match schema_wal_digest(db_path, &generation) {
                Ok(digest) => authenticated_wal_digest = Some(digest),
                Err(error) => validation = Err(error),
            }
        }

        // A live writer can advance the same WAL while the private snapshot
        // is being copied. Depending on the exact torn-copy point SQLite may
        // report a low-level snapshot error (including IOERR), not only one of
        // our explicit generation-change diagnostics. Retry only when a fresh
        // authority read proves that the main database is unchanged and the
        // same WAL/SHM objects advanced in place. Stable corruption and every
        // pathname replacement still fail closed on the first round.
        let retry_generation = validation
            .is_err()
            .then(|| schema_generation(db_path).map_err(schema_reinitialize_error))
            .transpose()?
            .filter(|current| {
                schema_snapshot_generation_is_retryable_volatile_change(&generation, current)
            });
        if let Some(current) = retry_generation {
            let mut state = entry
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.validating = false;
            state.validated = None;
            state.failed = None;
            entry.changed.notify_all();
            drop(state);
            drop(leader_exclusion);
            volatile_generation_retries = volatile_generation_retries.saturating_add(1);
            if volatile_generation_retries >= 8 {
                return Err(Error::WorkspaceLocked(
                    "SQLite WAL remained active throughout bounded schema snapshot validation; retry the command"
                        .into(),
                ));
            }
            generation = current;
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }

        let mut state = entry
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.validating = false;
        match validation {
            Ok(()) => {
                state.validated = Some((
                    generation.clone(),
                    backend.clone(),
                    authenticated_wal_digest.clone().unwrap(),
                ));
                state.failed = None;
            }
            Err(error) => {
                state.validated = None;
                state.failed = Some((round, schema_failure_message(&error)));
            }
        }
        entry.changed.notify_all();
        if let Some((failed_round, message)) = &state.failed {
            if *failed_round == round {
                return Err(schema_reinitialize_error(message.clone()));
            }
        }
        let (parent_authority, main_authority) = open_schema_main_authority(db_path, &generation)?;
        state.active_handoffs = state.active_handoffs.saturating_add(1);
        drop(state);
        return Ok(ValidatedSchemaGeneration {
            db_path: db_path.to_path_buf(),
            generation,
            entry,
            _parent_authority: parent_authority,
            main_authority,
            _shared_exclusion: shared_exclusion,
            _leader_exclusion: leader_exclusion,
            authenticated_wal_digest: authenticated_wal_digest.unwrap(),
        });
    }
}

pub(crate) fn inspect_existing_schema_version(db_path: &Path, prolly_backend: &str) -> Result<i64> {
    let _shared_exclusion = acquire_schema_shared_exclusion(db_path)?;
    let before = schema_generation(db_path).map_err(schema_reinitialize_error)?;
    let snapshot = tempfile::Builder::new()
        .prefix("trail-schema-version-")
        .tempdir()
        .map_err(schema_reinitialize_error)?;
    let snapshot_db = snapshot.path().join("trail.sqlite");
    for suffix in ["", "-wal", "-journal"] {
        let mut source = db_path.as_os_str().to_os_string();
        source.push(suffix);
        let source = PathBuf::from(source);
        if source.exists() {
            let mut destination = snapshot_db.as_os_str().to_os_string();
            destination.push(suffix);
            fs::copy(source, PathBuf::from(destination)).map_err(schema_reinitialize_error)?;
        }
    }
    let after = schema_generation(db_path).map_err(schema_reinitialize_error)?;
    if before != after {
        return Err(Error::WorkspaceLocked(
            "schema generation changed during predecessor inspection; retry the command".into(),
        ));
    }
    let conn = Connection::open(&snapshot_db).map_err(schema_reinitialize_error)?;
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(schema_reinitialize_error)?;
    let version = conn
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(schema_reinitialize_error)?;
    if version == SCHEMA_V18_VERSION {
        storage::validate_schema_v18_for_migration(&conn).map_err(schema_reinitialize_error)?;
        match prolly_backend {
            "sqlite" => storage::validate_prolly_sqlite_schema_v18(&conn)
                .map_err(schema_reinitialize_error)?,
            "slatedb" => storage::validate_no_prolly_sqlite_schema_v18(&conn)
                .map_err(schema_reinitialize_error)?,
            other => {
                return Err(Error::InvalidInput(format!(
                    "storage.prolly_backend must be sqlite or slatedb, got `{other}`"
                )))
            }
        }
    }
    Ok(version)
}

fn schema_snapshot_generation_is_retryable_volatile_change(
    before: &SchemaGeneration,
    after: &SchemaGeneration,
) -> bool {
    before != after
        && before.0.len() == after.0.len()
        && before.0.iter().zip(&after.0).all(|(left, right)| {
            if left.suffix != right.suffix {
                return false;
            }
            match left.suffix {
                "-wal" | "-shm" => {
                    left.present == right.present
                        && (!left.present
                            || (left.device == right.device && left.inode == right.inode))
                }
                // The main database and rollback journal are immutable
                // authority for this retry classification. A pathname
                // replacement, checkpoint rewrite, or any other change must
                // fail this validation round rather than authenticate a new
                // object under a stale snapshot.
                _ => left == right,
            }
        })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn schema_generation_key(generation: &SchemaGeneration) -> String {
    let mut digest = Sha256::new();
    digest.update(b"trail-schema-generation-v1\0");
    for file in &generation.0 {
        digest.update((file.suffix.len() as u64).to_le_bytes());
        digest.update(file.suffix.as_bytes());
        digest.update([u8::from(file.present)]);
        digest.update(file.device.to_le_bytes());
        digest.update(file.inode.to_le_bytes());
        digest.update(file.length.to_le_bytes());
        digest.update(file.modified_seconds.to_le_bytes());
        digest.update(file.modified_nanoseconds.to_le_bytes());
        digest.update(file.changed_seconds.to_le_bytes());
        digest.update(file.changed_nanoseconds.to_le_bytes());
    }
    hex::encode(digest.finalize())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn schema_validation_runtime_namespace(db_path: &Path) -> Result<Option<(PathBuf, String)>> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::DirBuilderExt;

    let uid = rustix::process::getuid().as_raw();
    let runtime_dir = PathBuf::from(format!("/tmp/trail-sv-{uid}"));
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    if let Err(error) = builder.create(&runtime_dir) {
        if error.kind() != std::io::ErrorKind::AlreadyExists {
            return Ok(None);
        }
    }
    let metadata = match fs::symlink_metadata(&runtime_dir) {
        Ok(metadata) => metadata,
        Err(_) => return Ok(None),
    };
    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
        return Ok(None);
    }

    let mut digest = Sha256::new();
    digest.update(b"trail-schema-validation-path-v1\0");
    digest.update(db_path.as_os_str().as_bytes());
    Ok(Some((runtime_dir, hex::encode(digest.finalize()))))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct SchemaValidationRuntimeEntry {
    path: PathBuf,
    device: u64,
    inode: u64,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct SchemaValidationSocketAuthority {
    #[cfg(target_os = "linux")]
    _inode: File,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn schema_validation_socket_identity_matches(
    metadata: &fs::Metadata,
    expected: &SchemaValidationRuntimeEntry,
) -> bool {
    metadata.file_type().is_socket()
        && metadata.uid() == rustix::process::getuid().as_raw()
        && metadata.dev() == expected.device
        && metadata.ino() == expected.inode
        && metadata.mode() & 0o777 == 0o600
}

#[cfg(target_os = "linux")]
fn secure_schema_validation_socket(
    path: &Path,
    expected: &SchemaValidationRuntimeEntry,
) -> std::io::Result<SchemaValidationSocketAuthority> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    // Linux does not implement fchmodat(AT_SYMLINK_NOFOLLOW). Pin the bound
    // socket inode with O_PATH|O_NOFOLLOW, then chmod through that descriptor's
    // procfs handle. The pathname can be replaced without redirecting chmod;
    // the final lstat comparison rejects any such substitution.
    let inode = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let before = inode.metadata()?;
    if !before.file_type().is_socket()
        || before.uid() != rustix::process::getuid().as_raw()
        || before.dev() != expected.device
        || before.ino() != expected.inode
    {
        return Err(std::io::Error::other(
            "schema validation socket inode changed before permission binding",
        ));
    }
    let descriptor_path = PathBuf::from(format!("/proc/self/fd/{}", inode.as_raw_fd()));
    fs::set_permissions(&descriptor_path, fs::Permissions::from_mode(0o600))?;
    let pinned = inode.metadata()?;
    let named = fs::symlink_metadata(path)?;
    if !schema_validation_socket_identity_matches(&pinned, expected)
        || !schema_validation_socket_identity_matches(&named, expected)
    {
        return Err(std::io::Error::other(
            "schema validation socket inode changed during permission binding",
        ));
    }
    #[cfg(test)]
    if let Some(hook) = NEXT_SCHEMA_SOCKET_AUTHORITY_HOOKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(path)
    {
        hook(path);
    }
    let pinned = inode.metadata()?;
    let named = fs::symlink_metadata(path)?;
    if !schema_validation_socket_identity_matches(&pinned, expected)
        || !schema_validation_socket_identity_matches(&named, expected)
    {
        return Err(std::io::Error::other(
            "schema validation socket pathname changed after permission binding",
        ));
    }
    Ok(SchemaValidationSocketAuthority { _inode: inode })
}

#[cfg(target_os = "macos")]
fn secure_schema_validation_socket(
    path: &Path,
    expected: &SchemaValidationRuntimeEntry,
) -> std::io::Result<SchemaValidationSocketAuthority> {
    rustix::fs::chmodat(
        rustix::fs::CWD,
        path,
        rustix::fs::Mode::from_raw_mode(0o600),
        rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
    )
    .map_err(std::io::Error::from)?;
    let named = fs::symlink_metadata(path)?;
    if !schema_validation_socket_identity_matches(&named, expected) {
        return Err(std::io::Error::other(
            "schema validation socket inode changed during permission binding",
        ));
    }
    Ok(SchemaValidationSocketAuthority {})
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl SchemaValidationRuntimeEntry {
    fn capture(path: &Path) -> std::io::Result<Self> {
        let metadata = fs::symlink_metadata(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Drop for SchemaValidationRuntimeEntry {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path)
            .is_ok_and(|metadata| metadata.dev() == self.device && metadata.ino() == self.inode)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn cleanup_stale_schema_validation_runtime_entries(runtime_dir: &Path, namespace: &str) {
    let prefix = format!("{}-", &namespace[..24]);
    for path in fs::read_dir(runtime_dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            (name.starts_with(&prefix)
                && (name.ends_with(".socket") || name.ends_with(".announce")))
            .then(|| entry.path())
        })
    {
        if let Ok(entry) = SchemaValidationRuntimeEntry::capture(&path) {
            drop(entry);
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn schema_validation_peer_identity(
    stream: &std::os::unix::net::UnixStream,
) -> std::io::Result<(u32, u32)> {
    use std::os::fd::AsRawFd;

    #[cfg(target_os = "linux")]
    {
        let mut credentials = std::mem::MaybeUninit::<libc::ucred>::uninit();
        let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let status = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                credentials.as_mut_ptr().cast(),
                &mut length,
            )
        };
        if status != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let credentials = unsafe { credentials.assume_init() };
        return Ok((credentials.pid as u32, credentials.uid));
    }
    #[cfg(target_os = "macos")]
    {
        let mut pid = 0_i32;
        let mut length = std::mem::size_of::<i32>() as libc::socklen_t;
        let status = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_LOCAL,
                libc::LOCAL_PEERPID,
                (&mut pid as *mut i32).cast(),
                &mut length,
            )
        };
        if status != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut uid = 0_u32;
        let mut gid = 0_u32;
        if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        return Ok((pid as u32, uid));
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn schema_validation_wire_result(
    nonce: &str,
    generation: &str,
    backend: &str,
    outcome: &CrossProcessSchemaValidationOutcome,
) -> String {
    let (kind, payload) = match outcome {
        CrossProcessSchemaValidationOutcome::Success(wal_digest) => ("success", wal_digest.clone()),
        CrossProcessSchemaValidationOutcome::Failure(message) => {
            ("failure", hex::encode(message.as_bytes()))
        }
    };
    format!(
        "trail-schema-validation-ipc-v1\n{nonce}\n{generation}\n{}\n{kind}\n{payload}\n",
        hex::encode(backend.as_bytes()),
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn parse_schema_validation_wire_result(
    text: &str,
    nonce: &str,
    generation: &str,
    backend: &str,
) -> Option<CrossProcessSchemaValidationOutcome> {
    let mut lines = text.lines();
    if lines.next()? != "trail-schema-validation-ipc-v1"
        || lines.next()? != nonce
        || lines.next()? != generation
        || lines.next()? != hex::encode(backend.as_bytes())
    {
        return None;
    }
    let kind = lines.next()?;
    let payload = lines.next()?;
    if lines.next().is_some() {
        return None;
    }
    match kind {
        "success" if payload.len() == 64 && hex::decode(payload).is_ok() => Some(
            CrossProcessSchemaValidationOutcome::Success(payload.to_owned()),
        ),
        "failure" => String::from_utf8(hex::decode(payload).ok()?)
            .ok()
            .map(CrossProcessSchemaValidationOutcome::Failure),
        _ => None,
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn serve_schema_validation_result(
    listener: &std::os::unix::net::UnixListener,
    nonce: &str,
    generation: &str,
    backend: &str,
    outcome: &CrossProcessSchemaValidationOutcome,
    shutdown: &SchemaValidationServerShutdown,
) {
    use std::net::Shutdown;

    let response = schema_validation_wire_result(nonce, generation, backend, outcome);
    let expected_request = format!(
        "trail-schema-validation-request-v1\n{nonce}\n{generation}\n{}\n",
        hex::encode(backend.as_bytes())
    );
    let hard_deadline = Instant::now() + Duration::from_millis(300);
    let mut idle_deadline = hard_deadline;
    while !shutdown.is_stopping()
        && Instant::now() < hard_deadline
        && Instant::now() < idle_deadline
    {
        match listener.accept() {
            Ok((mut stream, _)) => {
                // Followers can be runnable yet unscheduled while the validating
                // process publishes its result. Keep the detached server alive for
                // the remaining absolute window; the foreground handoff is already
                // free to continue and pays none of this grace period.
                idle_deadline = (Instant::now() + Duration::from_millis(300)).min(hard_deadline);
                if schema_validation_peer_identity(&stream)
                    .is_ok_and(|(_, uid)| uid == rustix::process::getuid().as_raw())
                    && read_schema_validation_request(&mut stream, shutdown)
                        .is_some_and(|request| request == expected_request)
                {
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_write_timeout(Some(Duration::from_millis(10)));
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.shutdown(Shutdown::Both);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                let remaining = hard_deadline.saturating_duration_since(Instant::now());
                if shutdown.wait(remaining.min(Duration::from_millis(1))) {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_schema_validation_request(
    stream: &mut std::os::unix::net::UnixStream,
    shutdown: &SchemaValidationServerShutdown,
) -> Option<String> {
    stream.set_nonblocking(true).ok()?;
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => return String::from_utf8(request).ok(),
            Ok(read) => {
                if request.len().saturating_add(read) > 8192 {
                    return None;
                }
                request.extend_from_slice(&buffer[..read]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() || shutdown.wait(remaining.min(Duration::from_millis(1))) {
                    return None;
                }
            }
            Err(_) => return None,
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Default)]
struct SchemaValidationServerShutdown {
    stopping: AtomicBool,
    mutex: Mutex<()>,
    changed: Condvar,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl SchemaValidationServerShutdown {
    fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }

    fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
        self.changed.notify_all();
    }

    fn wait(&self, timeout: Duration) -> bool {
        if self.is_stopping() {
            return true;
        }
        let guard = self
            .mutex
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = self
            .changed
            .wait_timeout_while(guard, timeout, |_| !self.is_stopping())
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.is_stopping()
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct SchemaValidationServer {
    listener: std::os::unix::net::UnixListener,
    nonce: String,
    generation: String,
    backend: String,
    outcome: CrossProcessSchemaValidationOutcome,
    _leader_exclusion: File,
    _socket_authority: SchemaValidationSocketAuthority,
    _socket_cleanup: SchemaValidationRuntimeEntry,
    _announcement_cleanup: SchemaValidationRuntimeEntry,
    #[cfg(test)]
    panic_on_serve: bool,
    #[cfg(test)]
    start_delay: Duration,
    #[cfg(test)]
    shutdown_delay: Duration,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl SchemaValidationServer {
    #[cfg(test)]
    fn delay_start_for_test(&self) {
        std::thread::sleep(self.start_delay);
    }

    #[cfg(not(test))]
    fn delay_start_for_test(&self) {}

    fn serve(self, shutdown: &SchemaValidationServerShutdown) {
        #[cfg(test)]
        if self.panic_on_serve {
            panic!("injected schema validation server panic");
        }
        serve_schema_validation_result(
            &self.listener,
            &self.nonce,
            &self.generation,
            &self.backend,
            &self.outcome,
            shutdown,
        );
        #[cfg(test)]
        std::thread::sleep(self.shutdown_delay);
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[cfg(test)]
fn spawn_schema_validation_server(
    server: SchemaValidationServer,
    shutdown: Arc<SchemaValidationServerShutdown>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("trail-schema-validation".to_owned())
        .spawn(move || server.serve(&shutdown))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[derive(Clone)]
struct ActiveSchemaValidationServer {
    id: u64,
    shutdown: Arc<SchemaValidationServerShutdown>,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct SchemaValidationServerRegistration {
    db_path: PathBuf,
    id: u64,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Drop for SchemaValidationServerRegistration {
    fn drop(&mut self) {
        let servers = SCHEMA_VALIDATION_SERVERS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut servers = servers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if servers
            .get(&self.db_path)
            .is_some_and(|active| active.id == self.id)
        {
            servers.remove(&self.db_path);
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn stop_schema_validation_server(db_path: &Path) -> bool {
    let active = SCHEMA_VALIDATION_SERVERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(db_path)
        .cloned();
    if let Some(active) = active {
        active.shutdown.stop();
        true
    } else {
        false
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn start_schema_validation_server(db_path: &Path, server: SchemaValidationServer) {
    let id = NEXT_SCHEMA_VALIDATION_SERVER_ID.fetch_add(1, Ordering::Relaxed);
    let shutdown = Arc::new(SchemaValidationServerShutdown::default());
    let registration = SchemaValidationServerRegistration {
        db_path: db_path.to_path_buf(),
        id,
    };
    let servers = SCHEMA_VALIDATION_SERVERS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut servers = servers
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(active) = servers.get(db_path) {
        active.shutdown.stop();
        return;
    }
    servers.insert(
        db_path.to_path_buf(),
        ActiveSchemaValidationServer {
            id,
            shutdown: shutdown.clone(),
        },
    );
    drop(servers);
    let thread_shutdown = shutdown.clone();
    let _ = std::thread::Builder::new()
        .name("trail-schema-validation".to_owned())
        .spawn(move || {
            let _registration = registration;
            server.delay_start_for_test();
            server.serve(&thread_shutdown);
        });
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn request_schema_validation_result(
    socket_path: &Path,
    nonce: &str,
    generation: &str,
    backend: &str,
    leader_pid: u32,
) -> Option<CrossProcessSchemaValidationOutcome> {
    use std::net::Shutdown;
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(socket_path).ok()?;
    let (peer_pid, peer_uid) = schema_validation_peer_identity(&stream).ok()?;
    if peer_pid != leader_pid || peer_uid != rustix::process::getuid().as_raw() {
        return None;
    }
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let request = format!(
        "trail-schema-validation-request-v1\n{nonce}\n{generation}\n{}\n",
        hex::encode(backend.as_bytes())
    );
    stream.write_all(request.as_bytes()).ok()?;
    stream.shutdown(Shutdown::Write).ok()?;
    let mut response = String::new();
    stream
        .take(1024 * 1024)
        .read_to_string(&mut response)
        .ok()?;
    parse_schema_validation_wire_result(&response, nonce, generation, backend)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_schema_validation_announcement(
    path: &Path,
    leader_pid: u32,
    expected_namespace: &str,
) -> Option<(String, PathBuf)> {
    use std::os::unix::fs::OpenOptionsExt;

    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file()
        || metadata.uid() != rustix::process::getuid().as_raw()
        || metadata.mode() & 0o077 != 0
    {
        return None;
    }
    let mut text = String::new();
    file.take(8192).read_to_string(&mut text).ok()?;
    let mut lines = text.lines();
    if lines.next()? != "trail-schema-validation-announce-v1"
        || lines.next()?.parse::<u32>().ok()? != leader_pid
        || lines.next()? != expected_namespace
    {
        return None;
    }
    let nonce = lines.next()?.to_owned();
    use std::os::unix::ffi::OsStringExt;
    let socket = PathBuf::from(std::ffi::OsString::from_vec(
        hex::decode(lines.next()?).ok()?,
    ));
    if nonce.len() != 64 || lines.next().is_some() {
        return None;
    }
    Some((nonce, socket))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn coordinate_schema_snapshot_validation(
    db_path: &Path,
    backend: &str,
    generation: &mut SchemaGeneration,
    authenticated_wal_digest: &mut Option<String>,
) -> (Result<()>, Option<File>) {
    use rustix::process::{Flock, FlockType, Pid};
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::net::UnixListener;

    *authenticated_wal_digest = None;

    // POSIX record locks are process-associated, so never overlap a replacement
    // FD with an existing process-local server. Revoke the old server without
    // waiting and conservatively perform this rare replacement validation
    // locally; later rounds may elect a fresh cross-process authority after the
    // old thread has released its FD.
    if stop_schema_validation_server(db_path) {
        return (
            validate_schema_snapshot_generation(db_path, backend, generation),
            None,
        );
    }

    let Some((runtime_dir, namespace)) =
        schema_validation_runtime_namespace(db_path).ok().flatten()
    else {
        return (
            validate_schema_snapshot_generation(db_path, backend, generation),
            None,
        );
    };
    loop {
        let file = match OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(schema_validation_leader_path(db_path))
        {
            Ok(file) => file,
            Err(error) => return (Err(schema_reinitialize_error(error)), None),
        };
        match rustix::fs::fcntl_lock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => {
                // Exclusive leader authority proves no live cross-process server
                // owns an entry in this workspace namespace. Remove artifacts
                // left by a process that exited without running Rust destructors.
                cleanup_stale_schema_validation_runtime_entries(&runtime_dir, &namespace);
                *generation = match schema_generation(db_path).map_err(schema_reinitialize_error) {
                    Ok(current) => current,
                    Err(error) => return (Err(error), Some(file)),
                };
                let mut nonce = [0_u8; 32];
                if getrandom::getrandom(&mut nonce).is_err() {
                    return (
                        validate_schema_snapshot_generation(db_path, backend, generation),
                        Some(file),
                    );
                }
                let nonce = hex::encode(nonce);
                let leaf = format!("{}-{}", &namespace[..24], &nonce[..24]);
                let socket_path = runtime_dir.join(format!("{leaf}.socket"));
                let announcement_path = runtime_dir.join(format!("{leaf}.announce"));
                if socket_path.as_os_str().as_encoded_bytes().len() >= 100 {
                    return (
                        validate_schema_snapshot_generation(db_path, backend, generation),
                        Some(file),
                    );
                }
                let listener = match UnixListener::bind(&socket_path) {
                    Ok(listener) => listener,
                    Err(_) => {
                        return (
                            validate_schema_snapshot_generation(db_path, backend, generation),
                            Some(file),
                        )
                    }
                };
                let socket_cleanup = match SchemaValidationRuntimeEntry::capture(&socket_path) {
                    Ok(identity) => identity,
                    Err(_) => {
                        return (
                            validate_schema_snapshot_generation(db_path, backend, generation),
                            Some(file),
                        )
                    }
                };
                let socket_authority =
                    match secure_schema_validation_socket(&socket_path, &socket_cleanup) {
                        Ok(authority) => authority,
                        Err(_) => {
                            return (
                                validate_schema_snapshot_generation(db_path, backend, generation),
                                Some(file),
                            )
                        }
                    };
                let _ = listener.set_nonblocking(true);
                let announcement = format!(
                    "trail-schema-validation-announce-v1\n{}\n{}\n{}\n{}\n",
                    std::process::id(),
                    namespace,
                    nonce,
                    hex::encode(socket_path.as_os_str().as_encoded_bytes()),
                );
                let mut announcement_file = match OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    .open(&announcement_path)
                {
                    Ok(file) => file,
                    Err(_) => {
                        return (
                            validate_schema_snapshot_generation(db_path, backend, generation),
                            Some(file),
                        )
                    }
                };
                let announcement_cleanup =
                    match SchemaValidationRuntimeEntry::capture(&announcement_path) {
                        Ok(identity) => identity,
                        Err(_) => {
                            return (
                                validate_schema_snapshot_generation(db_path, backend, generation),
                                Some(file),
                            )
                        }
                    };
                if announcement_file
                    .write_all(announcement.as_bytes())
                    .is_err()
                    || announcement_file.sync_all().is_err()
                {
                    return (
                        validate_schema_snapshot_generation(db_path, backend, generation),
                        Some(file),
                    );
                }
                *generation = match schema_generation(db_path).map_err(schema_reinitialize_error) {
                    Ok(current) => current,
                    Err(error) => return (Err(error), Some(file)),
                };
                let generation_key = schema_generation_key(generation);
                let mut validation =
                    validate_schema_snapshot_generation(db_path, backend, generation);
                let outcome = if validation.is_ok() {
                    match schema_wal_digest(db_path, generation) {
                        Ok(digest) => {
                            *authenticated_wal_digest = Some(digest.clone());
                            CrossProcessSchemaValidationOutcome::Success(digest)
                        }
                        Err(error) => {
                            validation = Err(error);
                            CrossProcessSchemaValidationOutcome::Failure(schema_failure_message(
                                validation.as_ref().unwrap_err(),
                            ))
                        }
                    }
                } else {
                    CrossProcessSchemaValidationOutcome::Failure(schema_failure_message(
                        validation.as_ref().unwrap_err(),
                    ))
                };
                #[cfg(test)]
                let (start_delay, shutdown_delay) =
                    take_schema_validation_server_delays_for_test(db_path);
                let server = SchemaValidationServer {
                    listener,
                    nonce,
                    generation: generation_key,
                    backend: backend.to_owned(),
                    outcome,
                    _leader_exclusion: file,
                    _socket_authority: socket_authority,
                    _socket_cleanup: socket_cleanup,
                    _announcement_cleanup: announcement_cleanup,
                    #[cfg(test)]
                    panic_on_serve: false,
                    #[cfg(test)]
                    start_delay,
                    #[cfg(test)]
                    shutdown_delay,
                };
                start_schema_validation_server(db_path, server);
                return (validation, None);
            }
            Err(error) if error == rustix::io::Errno::AGAIN => {
                let requested = Flock::from(FlockType::WriteLock);
                let leader_pid = rustix::process::fcntl_getlk(&file, &requested)
                    .ok()
                    .flatten()
                    .map(|lock| Pid::as_raw(lock.pid) as u32);
                if let Some(leader_pid) = leader_pid {
                    let generation_key = schema_generation_key(generation);
                    let prefix = format!("{}-", &namespace[..24]);
                    let announcements = fs::read_dir(&runtime_dir)
                        .ok()
                        .into_iter()
                        .flatten()
                        .filter_map(|entry| entry.ok())
                        .filter(|entry| {
                            let name = entry.file_name();
                            let name = name.to_string_lossy();
                            name.starts_with(&prefix) && name.ends_with(".announce")
                        });
                    for announcement in announcements {
                        if let Some((nonce, announced_socket)) = read_schema_validation_announcement(
                            &announcement.path(),
                            leader_pid,
                            &namespace,
                        ) {
                            let expected_socket = runtime_dir.join(format!(
                                "{}-{}.socket",
                                &namespace[..24],
                                &nonce[..24]
                            ));
                            if announced_socket != expected_socket {
                                continue;
                            }
                            if let Some(outcome) = request_schema_validation_result(
                                &announced_socket,
                                &nonce,
                                &generation_key,
                                backend,
                                leader_pid,
                            ) {
                                return (
                                    match outcome {
                                        CrossProcessSchemaValidationOutcome::Success(digest) => {
                                            *authenticated_wal_digest = Some(digest);
                                            Ok(())
                                        }
                                        CrossProcessSchemaValidationOutcome::Failure(message) => {
                                            Err(schema_reinitialize_error(message))
                                        }
                                    },
                                    None,
                                );
                            }
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(error) => return (Err(Error::Io(error.into())), None),
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn coordinate_schema_snapshot_validation(
    db_path: &Path,
    backend: &str,
    generation: &mut SchemaGeneration,
    authenticated_wal_digest: &mut Option<String>,
) -> (Result<()>, Option<File>) {
    *authenticated_wal_digest = None;
    (
        validate_schema_snapshot_generation(db_path, backend, generation),
        None,
    )
}

fn validate_schema_snapshot_generation(
    db_path: &Path,
    prolly_backend: &str,
    generation: &SchemaGeneration,
) -> Result<()> {
    validate_schema_snapshot(db_path, prolly_backend)?;
    #[cfg(test)]
    run_schema_snapshot_generation_hook(db_path);
    let after = schema_generation(db_path).map_err(schema_reinitialize_error)?;
    if &after != generation {
        return Err(schema_reinitialize_error(
            "schema main/WAL/SHM generation changed during snapshot validation",
        ));
    }
    Ok(())
}

fn schema_failure_message(error: &Error) -> String {
    match error {
        Error::SchemaReinitializeRequired { found, .. } => found.clone(),
        other => other.to_string(),
    }
}

fn validate_schema_snapshot(db_path: &Path, prolly_backend: &str) -> Result<()> {
    #[cfg(test)]
    schema_validation_process_test_probe()?;
    #[cfg(test)]
    if SCHEMA_VALIDATION_FAILURES
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(db_path)
    {
        std::thread::sleep(Duration::from_millis(100));
        return Err(schema_reinitialize_error(
            "injected schema validation leader failure",
        ));
    }
    let snapshot = tempfile::Builder::new()
        .prefix("trail-schema-preflight-")
        .tempdir()
        .map_err(schema_reinitialize_error)?;
    let snapshot_db = snapshot.path().join("trail.sqlite");
    // The WAL is part of the durable database image. The SHM file is only a
    // rebuildable shared-memory index and copying it can transplant stale reader
    // marks into the private snapshot.
    for suffix in ["", "-wal", "-journal"] {
        let mut source = db_path.as_os_str().to_os_string();
        source.push(suffix);
        let source = PathBuf::from(source);
        if source.exists() {
            let mut destination = snapshot_db.as_os_str().to_os_string();
            destination.push(suffix);
            let destination = PathBuf::from(destination);
            fs::copy(&source, &destination).map_err(schema_reinitialize_error)?;
        }
    }
    let conn = rusqlite::Connection::open(&snapshot_db).map_err(schema_reinitialize_error)?;
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(schema_reinitialize_error)?;
    Trail::validate_schema_v19(&conn).map_err(schema_reinitialize_error)?;
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

#[cfg(test)]
fn schema_validation_process_test_probe() -> Result<()> {
    use std::io::Write as _;

    if let Some(counter_path) = std::env::var_os("TRAIL_TEST_SCHEMA_VALIDATION_COUNTER") {
        let mut counter = OpenOptions::new()
            .create(true)
            .append(true)
            .open(counter_path)?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        rustix::fs::flock(&counter, rustix::fs::FlockOperation::LockExclusive)
            .map_err(|error| Error::Io(error.into()))?;
        writeln!(counter, "{}", std::process::id())?;
        counter.sync_all()?;
    }
    if let Some(started_path) = std::env::var_os("TRAIL_TEST_SCHEMA_VALIDATION_STARTED") {
        let _ = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(started_path);
    }
    if let Some(crash_path) = std::env::var_os("TRAIL_TEST_SCHEMA_VALIDATION_CRASH_ONCE") {
        if let Ok(mut crash) = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(crash_path)
        {
            writeln!(crash, "{}", std::process::id())?;
            crash.sync_all()?;
            loop {
                std::thread::park();
            }
        }
    }
    if let Some(delay) = std::env::var_os("TRAIL_TEST_SCHEMA_VALIDATION_DELAY_MS") {
        let delay = delay
            .to_string_lossy()
            .parse::<u64>()
            .map_err(|error| Error::InvalidInput(error.to_string()))?;
        std::thread::sleep(Duration::from_millis(delay));
    }
    if let Ok(message) = std::env::var("TRAIL_TEST_SCHEMA_VALIDATION_FAIL") {
        return Err(schema_reinitialize_error(message));
    }
    Ok(())
}

#[cfg(unix)]
fn schema_wal_digest(db_path: &Path, expected: &SchemaGeneration) -> Result<String> {
    schema_wal_digest_inner(db_path, expected, false)?.ok_or_else(|| {
        schema_reinitialize_error("schema WAL unexpectedly lacked a stable digest snapshot")
    })
}

#[cfg(unix)]
fn schema_wal_digest_allowing_wal_ctime_transition(
    db_path: &Path,
    expected: &SchemaGeneration,
) -> Result<String> {
    const MAX_STABLE_SNAPSHOT_ATTEMPTS: usize = 16;

    for attempt in 0..MAX_STABLE_SNAPSHOT_ATTEMPTS {
        if let Some(digest) = schema_wal_digest_inner(db_path, expected, true)? {
            return Ok(digest);
        }
        if attempt + 1 < MAX_STABLE_SNAPSHOT_ATTEMPTS {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    Err(schema_reinitialize_error(
        "schema WAL did not reach a stable descriptor snapshot for digest",
    ))
}

#[cfg(unix)]
fn schema_wal_digest_inner(
    db_path: &Path,
    expected: &SchemaGeneration,
    allow_wal_ctime_transition: bool,
) -> Result<Option<String>> {
    use std::os::unix::fs::OpenOptionsExt;

    let before = schema_generation(db_path).map_err(schema_reinitialize_error)?;
    if &before != expected
        && !(allow_wal_ctime_transition
            && schema_generation_is_only_wal_ctime_transition(expected, &before))
    {
        return Err(schema_reinitialize_error(
            "schema generation changed before WAL digest",
        ));
    }
    let wal = before
        .0
        .iter()
        .find(|file| file.suffix == "-wal")
        .ok_or_else(|| schema_reinitialize_error("schema generation omitted WAL authority"))?;
    let mut digest = Sha256::new();
    digest.update(b"trail-schema-wal-v1\0");
    digest.update([u8::from(wal.present)]);
    if wal.present {
        let mut path = db_path.as_os_str().to_os_string();
        path.push("-wal");
        let path = PathBuf::from(path);
        #[cfg(test)]
        if let Some(hook) = NEXT_SCHEMA_WAL_DIGEST_HOOKS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&path)
        {
            hook(&path);
        }
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
            .map_err(schema_reinitialize_error)?;
        let opened = file.metadata().map_err(schema_reinitialize_error)?;
        let opened = schema_wal_generation_from_metadata(&opened);
        if !schema_wal_generation_matches_authority(&opened, wal) {
            return Err(schema_reinitialize_error(
                "schema WAL descriptor changed before digest",
            ));
        }
        #[cfg(test)]
        if let Some(hook) = SCHEMA_WAL_DIGEST_ATTEMPT_HOOKS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_mut(&path)
        {
            hook(&path);
        }
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).map_err(schema_reinitialize_error)?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
        let hashed = file.metadata().map_err(schema_reinitialize_error)?;
        let hashed = schema_wal_generation_from_metadata(&hashed);
        if hashed != opened {
            if allow_wal_ctime_transition {
                return Ok(None);
            }
            return Err(schema_reinitialize_error(
                "schema WAL descriptor changed during digest",
            ));
        }
    }
    let after = schema_generation(db_path).map_err(schema_reinitialize_error)?;
    if after != before {
        if allow_wal_ctime_transition
            && schema_generation_is_only_wal_ctime_transition(&before, &after)
        {
            return Ok(None);
        }
        return Err(schema_reinitialize_error(
            "schema generation changed during WAL digest",
        ));
    }
    Ok(Some(hex::encode(digest.finalize())))
}

#[cfg(unix)]
fn schema_wal_generation_from_metadata(metadata: &fs::Metadata) -> SchemaFileGeneration {
    SchemaFileGeneration {
        suffix: "-wal",
        present: metadata.file_type().is_file(),
        device: metadata.dev(),
        inode: metadata.ino(),
        length: metadata.len(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
    }
}

#[cfg(unix)]
fn schema_wal_generation_matches_authority(
    observed: &SchemaFileGeneration,
    expected: &SchemaFileGeneration,
) -> bool {
    observed.present
        && observed.device == expected.device
        && observed.inode == expected.inode
        && observed.length == expected.length
        && observed.modified_seconds == expected.modified_seconds
        && observed.modified_nanoseconds == expected.modified_nanoseconds
}

#[cfg(not(unix))]
fn schema_wal_digest(_db_path: &Path, _expected: &SchemaGeneration) -> Result<String> {
    Err(schema_reinitialize_error(
        "schema WAL digest is unsupported on this platform",
    ))
}

#[cfg(unix)]
fn schema_generation(db_path: &Path) -> std::io::Result<SchemaGeneration> {
    let mut files = Vec::with_capacity(4);
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut path = db_path.as_os_str().to_os_string();
        path.push(suffix);
        let path = PathBuf::from(path);
        match fs::metadata(&path) {
            Ok(metadata) => files.push(SchemaFileGeneration {
                suffix,
                present: true,
                device: metadata.dev(),
                inode: metadata.ino(),
                length: metadata.len(),
                modified_seconds: (suffix != "-shm").then_some(metadata.mtime()).unwrap_or(0),
                modified_nanoseconds: (suffix != "-shm")
                    .then_some(metadata.mtime_nsec())
                    .unwrap_or(0),
                changed_seconds: (suffix != "-shm").then_some(metadata.ctime()).unwrap_or(0),
                changed_nanoseconds: (suffix != "-shm")
                    .then_some(metadata.ctime_nsec())
                    .unwrap_or(0),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                files.push(SchemaFileGeneration {
                    suffix,
                    present: false,
                    device: 0,
                    inode: 0,
                    length: 0,
                    modified_seconds: 0,
                    modified_nanoseconds: 0,
                    changed_seconds: 0,
                    changed_nanoseconds: 0,
                });
            }
            Err(error) => return Err(error),
        }
    }
    Ok(SchemaGeneration(files))
}

fn schema_lock_waiting_is_enabled() -> bool {
    WRITE_LOCK_WAIT_DEADLINE
        .with(|deadline| deadline.get())
        .is_some_and(|deadline| Instant::now() < deadline)
}

fn schema_exclusion_path(_db_dir: &Path, db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(SCHEMA_EXCLUSION_FILE)
}

fn schema_validation_leader_path(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(SCHEMA_VALIDATION_LEADER_FILE)
}

fn schema_db_dir(db_path: &Path) -> Result<&Path> {
    let parent = db_path.parent().ok_or_else(|| {
        Error::InvalidInput("schema database path has no workspace directory".into())
    })?;
    if parent.file_name().is_some_and(|name| name == "index") {
        return parent.parent().ok_or_else(|| {
            Error::InvalidInput("schema database index has no workspace directory".into())
        });
    }
    Ok(parent)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn acquire_schema_shared_exclusion(db_path: &Path) -> Result<SchemaSharedExclusion> {
    let lock_path = schema_exclusion_path(schema_db_dir(db_path)?, db_path);
    let database = File::open(lock_path).map_err(schema_reinitialize_error)?;
    let mut delay = Duration::from_millis(2);
    loop {
        match rustix::fs::flock(&database, rustix::fs::FlockOperation::NonBlockingLockShared) {
            Ok(()) => {
                return Ok(SchemaSharedExclusion {
                    _database: database,
                })
            }
            Err(error) if error == rustix::io::Errno::AGAIN => {
                if !schema_lock_waiting_is_enabled() {
                    return Err(Error::WorkspaceLocked(
                        "workspace schema writer is active".into(),
                    ));
                }
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_millis(50));
            }
            Err(error) => return Err(Error::Io(error.into())),
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn acquire_schema_shared_exclusion(db_path: &Path) -> Result<SchemaSharedExclusion> {
    let lock_path = schema_exclusion_path(schema_db_dir(db_path)?, db_path);
    let database = File::open(lock_path).map_err(schema_reinitialize_error)?;
    Ok(SchemaSharedExclusion {
        _database: database,
    })
}

#[cfg(not(unix))]
fn schema_generation(db_path: &Path) -> std::io::Result<SchemaGeneration> {
    let mut files = Vec::with_capacity(4);
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut path = db_path.as_os_str().to_os_string();
        path.push(suffix);
        let path = PathBuf::from(path);
        match fs::metadata(&path) {
            Ok(metadata) => files.push(SchemaFileGeneration {
                suffix,
                present: true,
                device: 0,
                inode: 0,
                length: metadata.len(),
                modified_seconds: if suffix == "-shm" {
                    0
                } else {
                    metadata
                        .modified()?
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                        .min(i64::MAX as u64) as i64
                },
                modified_nanoseconds: if suffix == "-shm" {
                    0
                } else {
                    metadata
                        .modified()?
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .subsec_nanos() as i64
                },
                changed_seconds: 0,
                changed_nanoseconds: 0,
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                files.push(SchemaFileGeneration {
                    suffix,
                    present: false,
                    device: 0,
                    inode: 0,
                    length: 0,
                    modified_seconds: 0,
                    modified_nanoseconds: 0,
                    changed_seconds: 0,
                    changed_nanoseconds: 0,
                });
            }
            Err(error) => return Err(error),
        }
    }
    Ok(SchemaGeneration(files))
}

fn schema_reinitialize_error(err: impl std::fmt::Display) -> Error {
    Error::SchemaReinitializeRequired {
        found: err.to_string(),
        guidance: "back up this workspace, then run `trail init --force` to create schema v19"
            .into(),
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
    store: TrailProllyStore,
    prolly: Prolly<TrailProllyStore>,
    root_prolly: Prolly<TrailProllyStore>,
    // Keep the connection configured with NO_CKPT_ON_CLOSE after every cloned
    // SQLite Prolly store handle so it is the last SQLite connection dropped.
    conn: Connection,
    config: TrailConfig,
    object_cache: Mutex<ObjectCache>,
    daemon_worktree_cache: Option<DaemonWorktreeCache>,
    changed_path_daemon_registry: Mutex<change_ledger::ChangedPathDaemonRegistry>,
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
    validated_schema: Option<&ValidatedSchemaGeneration>,
) -> Result<TrailProllyStore> {
    let backend = match config.storage.prolly_backend.as_str() {
        "sqlite" => {
            let store = match schema_mode {
                SchemaOpenMode::FreshCreate => SqliteStore::open(sqlite_path)?,
                SchemaOpenMode::Existing => {
                    if let Some(validated) = validated_schema {
                        SqliteStore::open_existing_verified(sqlite_path, |identity| {
                            validated.verify_main_identity(identity).map_err(|error| {
                                prolly_store_sqlite::SqliteStoreError::new(error.to_string())
                            })
                        })
                        .map_err(schema_reinitialize_error)?
                    } else {
                        // The only unverified existing-open path is an internal clone made
                        // while the caller already owns the workspace writer exclusion and
                        // a fully validated Trail handle.
                        SqliteStore::open_existing(sqlite_path)?
                    }
                }
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
pub(crate) struct DaemonWorktreeCachePersist;

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
    VerifiedRangeDelta,
    FullSnapshot,
}

impl GitExportMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::MappedDelta => "mapped_delta",
            Self::VerifiedRangeDelta => "verified_range_delta",
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
    owner_record: String,
    candidate_residue: Option<PathBuf>,
    _owner_exclusion: File,
    _schema_exclusion: File,
    observer_coordination: Option<PathBuf>,
    observer_write_exclusion: Option<AuthorizedObserverWriteExclusion>,
}

#[derive(Default)]
struct AuthorizedObserverExclusionEntry {
    active: usize,
    observer_lock_active: bool,
    release_generation: u64,
}

#[derive(Default)]
struct AuthorizedObserverExclusionState {
    entries: HashMap<PathBuf, AuthorizedObserverExclusionEntry>,
}

static AUTHORIZED_OBSERVER_EXCLUSIONS: OnceLock<(
    Mutex<AuthorizedObserverExclusionState>,
    Condvar,
)> = OnceLock::new();

#[derive(Debug)]
pub(crate) struct AuthorizedObserverWriteExclusion {
    db_dir: PathBuf,
}

pub(crate) fn begin_authorized_observer_write_exclusion(
    db_dir: &Path,
) -> AuthorizedObserverWriteExclusion {
    let (state, _) = AUTHORIZED_OBSERVER_EXCLUSIONS.get_or_init(|| {
        (
            Mutex::new(AuthorizedObserverExclusionState::default()),
            Condvar::new(),
        )
    });
    let mut state = state.lock().unwrap_or_else(|poison| poison.into_inner());
    state
        .entries
        .entry(db_dir.to_path_buf())
        .or_default()
        .active += 1;
    AuthorizedObserverWriteExclusion {
        db_dir: db_dir.to_path_buf(),
    }
}

fn wait_for_observer_workspace_lock(db_dir: &Path) {
    let (state, changed) = AUTHORIZED_OBSERVER_EXCLUSIONS.get_or_init(|| {
        (
            Mutex::new(AuthorizedObserverExclusionState::default()),
            Condvar::new(),
        )
    });
    let mut state = state.lock().unwrap_or_else(|poison| poison.into_inner());
    while state
        .entries
        .entry(db_dir.to_path_buf())
        .or_default()
        .observer_lock_active
    {
        state = changed
            .wait(state)
            .unwrap_or_else(|poison| poison.into_inner());
    }
}

/// Active observer durability participates in an explicit same-process
/// handoff with observed-record publication. It waits without an arbitrary
/// timeout only while that exact authorized handoff is live; unrelated writer
/// contention and every durability/integrity failure remain terminal.
pub(crate) fn acquire_workspace_lock_for_observer(
    db_dir: &Path,
    schema_path: &Path,
    operation_id: &str,
) -> Result<WorkspaceLock> {
    let coordination = AUTHORIZED_OBSERVER_EXCLUSIONS.get_or_init(|| {
        (
            Mutex::new(AuthorizedObserverExclusionState::default()),
            Condvar::new(),
        )
    });
    loop {
        let (generation, mut reservation) = {
            let (state, changed) = coordination;
            let mut state = state.lock().unwrap_or_else(|poison| poison.into_inner());
            loop {
                let entry = state.entries.entry(db_dir.to_path_buf()).or_default();
                if entry.active == 0 && !entry.observer_lock_active {
                    entry.observer_lock_active = true;
                    break (
                        entry.release_generation,
                        ObserverWorkspaceLockReservation {
                            db_dir: db_dir.to_path_buf(),
                            armed: true,
                        },
                    );
                }
                state = changed
                    .wait(state)
                    .unwrap_or_else(|poison| poison.into_inner());
            }
        };
        match acquire_workspace_lock_with_admission(
            db_dir,
            schema_path,
            WorkspaceLockAdmission {
                purpose: WorkspaceLockPurpose::ObserverPublication,
                operation_id: Some(operation_id),
                deadline: default_workspace_lock_admission_deadline()?,
                retry_command: "retry observer publication",
            },
        ) {
            Ok(mut lock) => {
                lock.observer_coordination = Some(reservation.disarm());
                return Ok(lock);
            }
            Err(error @ Error::WorkspaceLocked(_)) => {
                drop(reservation);
                let (state, _) = coordination;
                let mut state = state.lock().unwrap_or_else(|poison| poison.into_inner());
                let entry = state.entries.entry(db_dir.to_path_buf()).or_default();
                if entry.active > 0 || entry.release_generation != generation {
                    continue;
                }
                return Err(error);
            }
            Err(error) => {
                drop(reservation);
                return Err(error);
            }
        }
    }
}

struct ObserverWorkspaceLockReservation {
    db_dir: PathBuf,
    armed: bool,
}

impl ObserverWorkspaceLockReservation {
    fn disarm(&mut self) -> PathBuf {
        self.armed = false;
        self.db_dir.clone()
    }
}

impl Drop for ObserverWorkspaceLockReservation {
    fn drop(&mut self) {
        if self.armed {
            release_observer_workspace_lock(&self.db_dir);
        }
    }
}

fn release_observer_workspace_lock(db_dir: &Path) {
    let Some((state, changed)) = AUTHORIZED_OBSERVER_EXCLUSIONS.get() else {
        return;
    };
    let mut state = state.lock().unwrap_or_else(|poison| poison.into_inner());
    let entry = state.entries.entry(db_dir.to_path_buf()).or_default();
    debug_assert!(entry.observer_lock_active);
    entry.observer_lock_active = false;
    changed.notify_all();
}

pub(crate) fn acquire_workspace_lock(db_dir: &Path) -> Result<WorkspaceLock> {
    acquire_workspace_lock_for_database(db_dir, &db_dir.join(DB_RELATIVE_PATH))
}

pub(crate) fn acquire_workspace_lock_for_database(
    db_dir: &Path,
    schema_path: &Path,
) -> Result<WorkspaceLock> {
    let deadline = WRITE_LOCK_WAIT_DEADLINE.with(|configured| {
        configured
            .get()
            .and_then(|deadline| deadline.checked_duration_since(Instant::now()))
            .unwrap_or_default()
    });
    acquire_workspace_lock_with_admission(
        db_dir,
        schema_path,
        WorkspaceLockAdmission {
            purpose: WorkspaceLockPurpose::CommandMutation,
            operation_id: None,
            deadline,
            retry_command: "retry the command",
        },
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkspaceLockPurpose {
    CommandMutation,
    LaneAssociation,
    ObserverStartup,
    ObserverPublication,
    SchemaTransition,
    Maintenance,
}

impl WorkspaceLockPurpose {
    fn as_str(self) -> &'static str {
        match self {
            Self::CommandMutation => "command_mutation",
            Self::LaneAssociation => "lane_association",
            Self::ObserverStartup => "observer_startup",
            Self::ObserverPublication => "observer_publication",
            Self::SchemaTransition => "schema_transition",
            Self::Maintenance => "maintenance",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "command_mutation" => Self::CommandMutation,
            "lane_association" => Self::LaneAssociation,
            "observer_startup" => Self::ObserverStartup,
            "observer_publication" => Self::ObserverPublication,
            "schema_transition" => Self::SchemaTransition,
            "maintenance" => Self::Maintenance,
            _ => return None,
        })
    }
}

pub(crate) struct WorkspaceLockAdmission<'a> {
    pub purpose: WorkspaceLockPurpose,
    pub operation_id: Option<&'a str>,
    pub deadline: Duration,
    pub retry_command: &'a str,
}

fn configured_workspace_lock_wait(requested: Duration) -> Result<Duration> {
    let configured = match std::env::var("TRAIL_WORKSPACE_LOCK_WAIT_MS") {
        Ok(value) => {
            let millis = value.parse::<u64>().map_err(|_| {
                Error::InvalidInput(
                    "TRAIL_WORKSPACE_LOCK_WAIT_MS must be a non-negative integer".into(),
                )
            })?;
            Duration::from_millis(millis)
        }
        Err(std::env::VarError::NotPresent) => requested,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(Error::InvalidInput(
                "TRAIL_WORKSPACE_LOCK_WAIT_MS must be valid Unicode digits".into(),
            ));
        }
    };
    Ok(configured.min(Duration::from_secs(120)))
}

pub(crate) fn default_workspace_lock_admission_deadline() -> Result<Duration> {
    configured_workspace_lock_wait(Duration::from_secs(30))
}

pub(crate) fn acquire_workspace_lock_for_lane_association(
    db_dir: &Path,
    schema_path: &Path,
    operation_id: &str,
    retry_command: &str,
) -> Result<WorkspaceLock> {
    let exclusion = begin_authorized_observer_write_exclusion(db_dir);
    wait_for_observer_workspace_lock(db_dir);
    let mut lock = acquire_workspace_lock_with_admission(
        db_dir,
        schema_path,
        WorkspaceLockAdmission {
            purpose: WorkspaceLockPurpose::LaneAssociation,
            operation_id: Some(operation_id),
            deadline: default_workspace_lock_admission_deadline()?,
            retry_command,
        },
    )?;
    lock.observer_write_exclusion = Some(exclusion);
    Ok(lock)
}

pub(crate) fn acquire_workspace_lock_with_admission(
    db_dir: &Path,
    schema_path: &Path,
    admission: WorkspaceLockAdmission<'_>,
) -> Result<WorkspaceLock> {
    let path = db_dir.join("lock");
    let mut delay = Duration::from_millis(2);
    let wait_for = configured_workspace_lock_wait(admission.deadline)?;
    let wait_deadline = Instant::now() + wait_for;
    cleanup_abandoned_workspace_lock_candidates(db_dir)?;
    let owner = WorkspaceLockOwner::current(admission.purpose, admission.operation_id)?;
    let publication;
    loop {
        match publish_workspace_lock(db_dir, &path, &owner) {
            Ok(value) => {
                publication = value;
                break;
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                match inspect_existing_workspace_lock(&path)? {
                    WorkspaceLockInspection::Reaped => continue,
                    WorkspaceLockInspection::Terminal(holder) => {
                        return Err(Error::WorkspaceLocked(holder.trim().to_string()));
                    }
                    WorkspaceLockInspection::Turnover(holder) => {
                        if !workspace_lock_admission_can_wait(&admission, &holder) {
                            return Err(Error::WorkspaceLocked(holder.encode().trim().to_string()));
                        }
                        if Instant::now() >= wait_deadline {
                            return Err(workspace_lock_timeout(&holder, &admission));
                        }
                        continue;
                    }
                    WorkspaceLockInspection::Held(holder) => {
                        if !workspace_lock_admission_can_wait(&admission, &holder) {
                            return Err(Error::WorkspaceLocked(holder.encode().trim().to_string()));
                        }
                        if Instant::now() >= wait_deadline {
                            return Err(workspace_lock_timeout(&holder, &admission));
                        }
                        let remaining = wait_deadline.saturating_duration_since(Instant::now());
                        let jittered =
                            workspace_lock_retry_delay(delay, &owner.nonce, &holder.nonce)
                                .min(remaining);
                        wait_for_workspace_lock_release(
                            db_dir,
                            jittered,
                            holder.pid == std::process::id(),
                        );
                        delay = (delay * 2).min(Duration::from_millis(50));
                    }
                }
            }
            Err(err) => return Err(Error::Io(err)),
        }
    }
    let WorkspaceLockPublication {
        file: owner_exclusion,
        candidate_residue,
    } = publication;
    let exclusion_path = schema_exclusion_path(db_dir, schema_path);
    let schema_exclusion = match OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(exclusion_path)
    {
        Ok(file) => file,
        Err(error) => {
            cleanup_owned_workspace_lock(
                &path,
                candidate_residue.as_deref(),
                &owner.encode(),
                &owner_exclusion,
            );
            return Err(Error::Io(error));
        }
    };
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let mut delay = Duration::from_millis(2);
        let reader_drain_deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match rustix::fs::flock(
                &schema_exclusion,
                rustix::fs::FlockOperation::NonBlockingLockExclusive,
            ) {
                Ok(()) => break,
                Err(error)
                    if error == rustix::io::Errno::AGAIN
                        && (schema_lock_waiting_is_enabled()
                            || Instant::now() < reader_drain_deadline) =>
                {
                    std::thread::sleep(delay);
                    delay = (delay * 2).min(Duration::from_millis(50));
                }
                Err(error) if error == rustix::io::Errno::AGAIN => {
                    cleanup_owned_workspace_lock(
                        &path,
                        candidate_residue.as_deref(),
                        &owner.encode(),
                        &owner_exclusion,
                    );
                    return Err(Error::WorkspaceLocked(
                        "workspace schema reader is active".into(),
                    ));
                }
                Err(error) => {
                    cleanup_owned_workspace_lock(
                        &path,
                        candidate_residue.as_deref(),
                        &owner.encode(),
                        &owner_exclusion,
                    );
                    return Err(Error::Io(error.into()));
                }
            }
        }
    }
    Ok(WorkspaceLock {
        path,
        owner_record: owner.encode(),
        candidate_residue,
        _owner_exclusion: owner_exclusion,
        _schema_exclusion: schema_exclusion,
        observer_coordination: None,
        observer_write_exclusion: None,
    })
}

fn workspace_lock_admission_can_wait(
    admission: &WorkspaceLockAdmission<'_>,
    holder: &WorkspaceLockOwner,
) -> bool {
    match (admission.purpose, holder.purpose) {
        (
            WorkspaceLockPurpose::ObserverStartup,
            WorkspaceLockPurpose::LaneAssociation | WorkspaceLockPurpose::ObserverPublication,
        ) => true,
        (WorkspaceLockPurpose::ObserverStartup, WorkspaceLockPurpose::ObserverStartup) => {
            matches!((admission.operation_id, holder.operation_id.as_deref()),
                (Some(requested), Some(held)) if requested != held)
        }
        (
            WorkspaceLockPurpose::LaneAssociation,
            WorkspaceLockPurpose::CommandMutation | WorkspaceLockPurpose::ObserverPublication,
        ) => true,
        (WorkspaceLockPurpose::LaneAssociation, WorkspaceLockPurpose::LaneAssociation) => {
            matches!((admission.operation_id, holder.operation_id.as_deref()),
                (Some(requested), Some(held)) if requested != held)
        }
        (
            WorkspaceLockPurpose::ObserverPublication,
            WorkspaceLockPurpose::LaneAssociation | WorkspaceLockPurpose::ObserverStartup,
        ) => true,
        (WorkspaceLockPurpose::ObserverPublication, WorkspaceLockPurpose::ObserverPublication) => {
            matches!((admission.operation_id, holder.operation_id.as_deref()),
            (Some(requested), Some(held)) if requested != held)
        }
        _ => false,
    }
}

fn workspace_lock_timeout(
    holder: &WorkspaceLockOwner,
    admission: &WorkspaceLockAdmission<'_>,
) -> Error {
    let age_seconds = self::util::now_ts().saturating_sub(holder.created_at);
    Error::WorkspaceLockTimeout {
        holder_purpose: holder.purpose.as_str().to_string(),
        holder_age_ms: u64::try_from(age_seconds)
            .unwrap_or(u64::MAX)
            .saturating_mul(1_000),
        operation_id: holder.operation_id.clone(),
        retry_command: admission.retry_command.to_string(),
    }
}

fn workspace_lock_retry_delay(
    base: Duration,
    contender_nonce: &str,
    holder_nonce: &str,
) -> Duration {
    let mut digest = Sha256::new();
    digest.update(contender_nonce.as_bytes());
    digest.update([0]);
    digest.update(holder_nonce.as_bytes());
    let bytes = digest.finalize();
    let jitter_window = (base.as_micros() / 2).max(1);
    let sample = u16::from_le_bytes([bytes[0], bytes[1]]) as u128;
    let jitter = sample % jitter_window;
    Duration::from_micros(
        u64::try_from(base.as_micros().saturating_add(jitter))
            .unwrap_or(u64::MAX)
            .min(50_000),
    )
}

fn wait_for_workspace_lock_release(db_dir: &Path, delay: Duration, same_process: bool) {
    if !same_process {
        std::thread::sleep(delay);
        return;
    }
    let (state, changed) = AUTHORIZED_OBSERVER_EXCLUSIONS.get_or_init(|| {
        (
            Mutex::new(AuthorizedObserverExclusionState::default()),
            Condvar::new(),
        )
    });
    let guard = state.lock().unwrap_or_else(|poison| poison.into_inner());
    let _ = changed
        .wait_timeout(guard, delay)
        .unwrap_or_else(|poison| poison.into_inner());
    let _ = db_dir;
}

impl Drop for WorkspaceLock {
    fn drop(&mut self) {
        cleanup_owned_workspace_lock(
            &self.path,
            self.candidate_residue.as_deref(),
            &self.owner_record,
            &self._owner_exclusion,
        );
        if let Some(db_dir) = self.observer_coordination.take() {
            release_observer_workspace_lock(&db_dir);
        }
        if let Some((_, changed)) = AUTHORIZED_OBSERVER_EXCLUSIONS.get() {
            changed.notify_all();
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkspaceLockOwner {
    version: u8,
    pid: u32,
    process_start_identity: String,
    executable_identity: String,
    nonce: String,
    purpose: WorkspaceLockPurpose,
    operation_id: Option<String>,
    created_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LegacyWorkspaceLockOwner {
    pid: u32,
    process_start_identity: String,
    nonce: String,
    created_at: i64,
}

enum ParsedWorkspaceLockOwner {
    V1(LegacyWorkspaceLockOwner),
    V2(WorkspaceLockOwner),
}

enum WorkspaceLockInspection {
    Reaped,
    Held(WorkspaceLockOwner),
    Turnover(WorkspaceLockOwner),
    Terminal(String),
}

#[cfg(test)]
type WorkspaceLockInspectionHook = Box<dyn FnOnce() + Send>;
#[cfg(test)]
static WORKSPACE_LOCK_INSPECTION_HOOKS: OnceLock<
    Mutex<HashMap<PathBuf, WorkspaceLockInspectionHook>>,
> = OnceLock::new();

#[cfg(test)]
fn install_workspace_lock_inspection_hook(lock_path: &Path, hook: impl FnOnce() + Send + 'static) {
    WORKSPACE_LOCK_INSPECTION_HOOKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .insert(lock_path.to_path_buf(), Box::new(hook));
}

#[cfg(test)]
fn run_workspace_lock_inspection_hook(lock_path: &Path) {
    let hook = WORKSPACE_LOCK_INSPECTION_HOOKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .remove(lock_path);
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkspaceLockPublishPhase {
    BeforePublish,
    AfterPublish,
}

#[cfg(test)]
static WORKSPACE_LOCK_PUBLISH_CRASHES: OnceLock<
    Mutex<HashMap<PathBuf, WorkspaceLockPublishPhase>>,
> = OnceLock::new();
#[cfg(test)]
static WORKSPACE_LOCK_CANDIDATE_UNLINK_FAILURES: OnceLock<Mutex<HashSet<PathBuf>>> =
    OnceLock::new();
#[cfg(test)]
static WORKSPACE_LOCK_ROLLBACK_UNLINK_FAILURES: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

#[cfg(test)]
fn crash_next_workspace_lock_publish(lock_path: &Path, phase: WorkspaceLockPublishPhase) {
    WORKSPACE_LOCK_PUBLISH_CRASHES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .insert(lock_path.to_path_buf(), phase);
}

#[cfg(test)]
fn run_workspace_lock_publish_crash_hook(lock_path: &Path, phase: WorkspaceLockPublishPhase) {
    let crashes = WORKSPACE_LOCK_PUBLISH_CRASHES.get_or_init(|| Mutex::new(HashMap::new()));
    let should_crash = {
        let mut crashes = crashes.lock().unwrap_or_else(|poison| poison.into_inner());
        if crashes.get(lock_path) == Some(&phase) {
            crashes.remove(lock_path);
            true
        } else {
            false
        }
    };
    if should_crash {
        panic!("simulated workspace-lock crash at {phase:?}");
    }
}

#[cfg(test)]
fn fail_next_workspace_lock_candidate_unlink(lock_path: &Path) {
    WORKSPACE_LOCK_CANDIDATE_UNLINK_FAILURES
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .insert(lock_path.to_path_buf());
}

#[cfg(test)]
fn fail_next_workspace_lock_rollback_unlink(lock_path: &Path) {
    WORKSPACE_LOCK_ROLLBACK_UNLINK_FAILURES
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .insert(lock_path.to_path_buf());
}

#[cfg(test)]
fn workspace_lock_rollback_unlink_result(lock_path: &Path) -> std::io::Result<()> {
    let fail = WORKSPACE_LOCK_ROLLBACK_UNLINK_FAILURES
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .remove(lock_path);
    if fail {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "injected workspace-lock rollback unlink failure",
        ))
    } else {
        fs::remove_file(lock_path)
    }
}

#[cfg(not(test))]
fn workspace_lock_rollback_unlink_result(lock_path: &Path) -> std::io::Result<()> {
    fs::remove_file(lock_path)
}

#[cfg(test)]
fn workspace_lock_candidate_unlink_result(
    lock_path: &Path,
    candidate: &Path,
) -> std::io::Result<()> {
    let fail = WORKSPACE_LOCK_CANDIDATE_UNLINK_FAILURES
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .remove(lock_path);
    if fail {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "injected workspace-lock candidate unlink failure",
        ))
    } else {
        fs::remove_file(candidate)
    }
}

#[cfg(not(test))]
fn workspace_lock_candidate_unlink_result(
    _lock_path: &Path,
    candidate: &Path,
) -> std::io::Result<()> {
    fs::remove_file(candidate)
}

impl WorkspaceLockOwner {
    fn current(purpose: WorkspaceLockPurpose, operation_id: Option<&str>) -> std::io::Result<Self> {
        let mut nonce = [0_u8; 32];
        getrandom::getrandom(&mut nonce).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("failed to generate workspace-lock nonce: {error}"),
            )
        })?;
        Ok(Self {
            version: 2,
            pid: std::process::id(),
            process_start_identity: self::util::current_process_start_token(),
            executable_identity: current_executable_identity()?,
            nonce: hex::encode(nonce),
            purpose,
            operation_id: operation_id.map(str::to_owned),
            created_at: self::util::now_ts(),
        })
    }

    fn encode(&self) -> String {
        let operation_id = self
            .operation_id
            .as_deref()
            .map(|value| hex::encode(value.as_bytes()))
            .unwrap_or_default();
        format!(
            "trail-workspace-lock-v2\npid={}\nprocess_start_identity={}\nexecutable_identity={}\nnonce={}\npurpose={}\noperation_id={}\ncreated_at={}\n",
            self.pid,
            hex::encode(self.process_start_identity.as_bytes()),
            self.executable_identity,
            self.nonce,
            self.purpose.as_str(),
            operation_id,
            self.created_at
        )
    }

    fn parse(record: &str) -> Option<Self> {
        let mut lines = record.lines();
        if lines.next()? != "trail-workspace-lock-v2" {
            return None;
        }
        let pid = lines.next()?.strip_prefix("pid=")?.parse().ok()?;
        let process_start_identity = String::from_utf8(
            hex::decode(lines.next()?.strip_prefix("process_start_identity=")?).ok()?,
        )
        .ok()?;
        let executable_identity = lines
            .next()?
            .strip_prefix("executable_identity=")?
            .to_owned();
        let nonce = lines.next()?.strip_prefix("nonce=")?.to_owned();
        let purpose = WorkspaceLockPurpose::parse(lines.next()?.strip_prefix("purpose=")?)?;
        let operation_id = lines.next()?.strip_prefix("operation_id=")?;
        let operation_id = if operation_id.is_empty() {
            None
        } else {
            Some(String::from_utf8(hex::decode(operation_id).ok()?).ok()?)
        };
        let created_at = lines.next()?.strip_prefix("created_at=")?.parse().ok()?;
        if pid == 0
            || process_start_identity.is_empty()
            || executable_identity.len() != 64
            || !executable_identity
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || nonce.len() != 64
            || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit())
            || operation_id.as_ref().is_some_and(|id| id.is_empty())
            || lines.next().is_some()
        {
            return None;
        }
        Some(Self {
            version: 2,
            pid,
            process_start_identity,
            executable_identity,
            nonce,
            purpose,
            operation_id,
            created_at,
        })
    }
}

impl LegacyWorkspaceLockOwner {
    fn parse(record: &str) -> Option<Self> {
        let mut lines = record.lines();
        if lines.next()? != "trail-workspace-lock-v1" {
            return None;
        }
        let pid = lines.next()?.strip_prefix("pid=")?.parse().ok()?;
        let process_start_identity = String::from_utf8(
            hex::decode(lines.next()?.strip_prefix("process_start_identity=")?).ok()?,
        )
        .ok()?;
        let nonce = lines.next()?.strip_prefix("nonce=")?.to_owned();
        let created_at = lines.next()?.strip_prefix("created_at=")?.parse().ok()?;
        if pid == 0
            || process_start_identity.is_empty()
            || nonce.len() != 64
            || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit())
            || lines.next().is_some()
        {
            return None;
        }
        Some(Self {
            pid,
            process_start_identity,
            nonce,
            created_at,
        })
    }
}

fn parse_workspace_lock_owner(record: &str) -> Option<ParsedWorkspaceLockOwner> {
    WorkspaceLockOwner::parse(record)
        .map(ParsedWorkspaceLockOwner::V2)
        .or_else(|| LegacyWorkspaceLockOwner::parse(record).map(ParsedWorkspaceLockOwner::V1))
}

fn current_executable_identity() -> std::io::Result<String> {
    let executable = std::env::current_exe()?;
    let canonical = fs::canonicalize(&executable)?;
    let metadata = fs::symlink_metadata(&canonical)?;
    if !metadata.file_type().is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "current executable is not a regular file",
        ));
    }
    let mut digest = Sha256::new();
    digest.update(b"trail-executable-identity-v1\0");
    digest.update(canonical.as_os_str().to_string_lossy().as_bytes());
    digest.update([0]);
    #[cfg(unix)]
    {
        digest.update(metadata.dev().to_le_bytes());
        digest.update(metadata.ino().to_le_bytes());
        digest.update(metadata.len().to_le_bytes());
        digest.update(metadata.mtime().to_le_bytes());
        digest.update(metadata.mtime_nsec().to_le_bytes());
    }
    #[cfg(not(unix))]
    {
        digest.update(metadata.len().to_le_bytes());
        let modified = metadata
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "current executable modification time precedes the Unix epoch",
                )
            })?;
        digest.update(modified.as_secs().to_le_bytes());
        digest.update(modified.subsec_nanos().to_le_bytes());
    }
    Ok(hex::encode(digest.finalize()))
}

fn authenticated_live_workspace_lock_owner(
    record: &str,
) -> std::result::Result<Option<WorkspaceLockOwner>, String> {
    match parse_workspace_lock_owner(record) {
        Some(ParsedWorkspaceLockOwner::V1(owner)) => {
            if self::util::process_matches_start_token(owner.pid, &owner.process_start_identity) {
                Err(
                    "live trail-workspace-lock-v1 owner is incompatible with role-aware admission"
                        .into(),
                )
            } else {
                Ok(None)
            }
        }
        Some(ParsedWorkspaceLockOwner::V2(owner)) => {
            if !self::util::process_matches_start_token(owner.pid, &owner.process_start_identity) {
                return Ok(None);
            }
            match current_executable_identity() {
                Ok(identity) if identity == owner.executable_identity => Ok(Some(owner)),
                Ok(_) => Err(
                    "workspace lock executable identity changed while its owner remained live"
                        .into(),
                ),
                Err(error) => Err(format!(
                    "workspace lock executable identity could not be authenticated: {error}"
                )),
            }
        }
        None => Err(record.trim().to_string()),
    }
}

fn create_workspace_lock_candidate(
    db_dir: &Path,
    owner: &WorkspaceLockOwner,
) -> std::io::Result<(PathBuf, File)> {
    let candidate = db_dir.join(format!(".workspace-lock-candidate-{}", owner.nonce));
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&candidate)?;
        file.write_all(owner.encode().as_bytes())?;
        // The record must be complete before its inode becomes authoritative.
        // The directory is intentionally not fsynced: this lock is ephemeral,
        // so losing either a publication or an unlink across a machine crash is
        // safe (no owner survives, and a reappearing record is reaped). Avoiding
        // directory barriers also keeps every workspace command off that hot
        // durability path.
        file.sync_all()?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive)
            .map_err(std::io::Error::from)?;
        Ok(file)
    })();
    match result {
        Ok(file) => Ok((candidate, file)),
        Err(error) => {
            let _ = fs::remove_file(&candidate);
            Err(error)
        }
    }
}

struct WorkspaceLockPublication {
    file: File,
    candidate_residue: Option<PathBuf>,
}

fn publish_workspace_lock(
    db_dir: &Path,
    lock_path: &Path,
    owner: &WorkspaceLockOwner,
) -> std::io::Result<WorkspaceLockPublication> {
    let (candidate, file) = create_workspace_lock_candidate(db_dir, owner)?;
    #[cfg(test)]
    run_workspace_lock_publish_crash_hook(lock_path, WorkspaceLockPublishPhase::BeforePublish);
    match fs::hard_link(&candidate, lock_path) {
        Ok(()) => {
            #[cfg(test)]
            run_workspace_lock_publish_crash_hook(
                lock_path,
                WorkspaceLockPublishPhase::AfterPublish,
            );
            match workspace_lock_candidate_unlink_result(lock_path, &candidate) {
                Ok(()) => Ok(WorkspaceLockPublication {
                    file,
                    candidate_residue: None,
                }),
                Err(error) => {
                    let link_count = file.metadata().map(|metadata| {
                        #[cfg(unix)]
                        {
                            metadata.nlink()
                        }
                        #[cfg(not(unix))]
                        {
                            2
                        }
                    });
                    if matches!(link_count, Ok(1)) {
                        // The original candidate name disappeared despite the
                        // unlink error (for example, a concurrent same-uid
                        // substitution). The published inode is already in its
                        // valid one-link state.
                        return Ok(WorkspaceLockPublication {
                            file,
                            candidate_residue: None,
                        });
                    }
                    if rollback_workspace_lock_publication(
                        lock_path,
                        &candidate,
                        &owner.encode(),
                        &file,
                    ) {
                        Err(error)
                    } else {
                        // Returning an ordinary error here would drop the only
                        // flock capability while leaving a current-live-PID
                        // record. Keep ownership explicit so mutual exclusion
                        // remains fail-closed and Drop can retry exact cleanup.
                        Ok(WorkspaceLockPublication {
                            file,
                            candidate_residue: Some(candidate),
                        })
                    }
                }
            }
        }
        Err(error) => {
            let _ = fs::remove_file(&candidate);
            Err(error)
        }
    }
}

const MAX_WORKSPACE_LOCK_RECORD_BYTES: u64 = 4096;

fn read_workspace_lock_record(file: &File) -> std::io::Result<Option<String>> {
    let before = file.metadata()?;
    if before.len() > MAX_WORKSPACE_LOCK_RECORD_BYTES {
        return Ok(None);
    }
    let mut bytes = vec![0_u8; before.len() as usize];
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        let mut offset = 0;
        while offset < bytes.len() {
            let read = file.read_at(&mut bytes[offset..], offset as u64)?;
            if read == 0 {
                return Ok(None);
            }
            offset += read;
        }
    }
    #[cfg(not(unix))]
    {
        let mut clone = file.try_clone()?;
        clone.seek(SeekFrom::Start(0))?;
        clone.read_exact(&mut bytes)?;
    }
    let after = file.metadata()?;
    if after.len() != before.len() {
        return Ok(None);
    }
    #[cfg(unix)]
    if after.dev() != before.dev() || after.ino() != before.ino() {
        return Ok(None);
    }
    Ok(String::from_utf8(bytes).ok())
}

fn rollback_workspace_lock_publication(
    lock_path: &Path,
    candidate_path: &Path,
    owner_record: &str,
    owner_file: &File,
) -> bool {
    #[cfg(unix)]
    {
        let Ok(descriptor) = owner_file.metadata() else {
            return false;
        };
        let Ok(lock) = fs::symlink_metadata(lock_path) else {
            return false;
        };
        let Ok(candidate) = fs::symlink_metadata(candidate_path) else {
            return false;
        };
        if !workspace_lock_metadata_is_private_regular(&descriptor)
            || !workspace_lock_metadata_is_private_regular(&lock)
            || !workspace_lock_metadata_is_private_regular(&candidate)
            || descriptor.nlink() != 2
            || lock.nlink() != 2
            || candidate.nlink() != 2
            || lock.dev() != descriptor.dev()
            || lock.ino() != descriptor.ino()
            || candidate.dev() != descriptor.dev()
            || candidate.ino() != descriptor.ino()
        {
            return false;
        }
    }
    if read_workspace_lock_record(owner_file)
        .ok()
        .flatten()
        .as_deref()
        != Some(owner_record)
    {
        return false;
    }
    if workspace_lock_rollback_unlink_result(lock_path).is_ok() {
        remove_workspace_lock_candidate_if_matches(candidate_path, owner_record, owner_file);
        true
    } else {
        false
    }
}

fn cleanup_owned_workspace_lock(
    lock_path: &Path,
    candidate_residue: Option<&Path>,
    owner_record: &str,
    owner_file: &File,
) {
    if let Some(candidate) = candidate_residue {
        if rollback_workspace_lock_publication(lock_path, candidate, owner_record, owner_file) {
            return;
        }
    }
    remove_owned_workspace_lock(lock_path, owner_record, owner_file);
}

fn remove_owned_workspace_lock(lock_path: &Path, owner_record: &str, owner_file: &File) {
    #[cfg(unix)]
    {
        let Ok(descriptor) = owner_file.metadata() else {
            return;
        };
        let Ok(named) = fs::symlink_metadata(lock_path) else {
            return;
        };
        if !workspace_lock_metadata_is_private_regular(&descriptor)
            || !workspace_lock_metadata_is_private_regular(&named)
            || descriptor.nlink() != 1
            || named.nlink() != 1
            || descriptor.dev() != named.dev()
            || descriptor.ino() != named.ino()
            || read_workspace_lock_record(owner_file)
                .ok()
                .flatten()
                .as_deref()
                != Some(owner_record)
        {
            return;
        }
    }
    #[cfg(not(unix))]
    if read_workspace_lock_record(owner_file)
        .ok()
        .flatten()
        .as_deref()
        != Some(owner_record)
    {
        return;
    }
    // POSIX has no unlink-if-inode-matches primitive. The retained flock makes
    // this conditional unlink safe among Trail processes; a hostile same-uid
    // process could still swap the name in the final syscall window, but such a
    // process can already delete or rewrite Trail's private state and is outside
    // the workspace-lock threat boundary.
    let _ = fs::remove_file(lock_path);
}

fn inspect_existing_workspace_lock(lock_path: &Path) -> std::io::Result<WorkspaceLockInspection> {
    #[cfg(unix)]
    {
        let metadata = match fs::symlink_metadata(lock_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(WorkspaceLockInspection::Reaped);
            }
            Err(error) => return Err(error),
        };
        if !workspace_lock_metadata_is_private_regular(&metadata) {
            return Ok(WorkspaceLockInspection::Terminal(
                "workspace lock is not a private regular file owned by this user".into(),
            ));
        }
        if metadata.len() > MAX_WORKSPACE_LOCK_RECORD_BYTES {
            return Ok(WorkspaceLockInspection::Terminal(
                "workspace lock record exceeds the safe size limit".into(),
            ));
        }
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = match options.open(lock_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(WorkspaceLockInspection::Reaped);
        }
        Err(error) => {
            return Ok(WorkspaceLockInspection::Terminal(format!(
                "unreadable workspace lock: {error}"
            )));
        }
    };

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let descriptor_exclusive =
        match rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => true,
            Err(error) if error == rustix::io::Errno::AGAIN => false,
            Err(error) => return Err(error.into()),
        };
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let descriptor_exclusive = true;

    let Some(holder) = read_workspace_lock_record(&file)? else {
        return Ok(WorkspaceLockInspection::Terminal(
            "workspace lock record is invalid or exceeds the safe size limit".into(),
        ));
    };
    let authenticated_predecessor = authenticated_live_workspace_lock_owner(&holder);
    #[cfg(test)]
    run_workspace_lock_inspection_hook(lock_path);

    #[cfg(unix)]
    {
        let descriptor = file.metadata()?;
        let named = match fs::symlink_metadata(lock_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(match authenticated_predecessor {
                    Ok(Some(owner)) => WorkspaceLockInspection::Turnover(owner),
                    Ok(None) => WorkspaceLockInspection::Reaped,
                    Err(reason) => WorkspaceLockInspection::Terminal(reason),
                });
            }
            Err(error) => return Err(error),
        };
        if descriptor.dev() != named.dev() || descriptor.ino() != named.ino() {
            return Ok(match authenticated_predecessor {
                Ok(Some(owner)) => WorkspaceLockInspection::Turnover(owner),
                Ok(None) => WorkspaceLockInspection::Terminal(
                    "workspace lock pathname changed during validation".into(),
                ),
                Err(reason) => WorkspaceLockInspection::Terminal(reason),
            });
        }
        if !workspace_lock_metadata_is_private_regular(&descriptor) {
            return Ok(WorkspaceLockInspection::Terminal(
                "workspace lock inode is not a private regular file owned by this user".into(),
            ));
        }
    }
    if read_workspace_lock_record(&file)?.as_deref() != Some(&holder) {
        return Ok(WorkspaceLockInspection::Terminal(
            "workspace lock changed during validation".into(),
        ));
    }

    let Some(parsed) = parse_workspace_lock_owner(&holder) else {
        return Ok(WorkspaceLockInspection::Terminal(holder));
    };
    let (pid, process_start_identity, nonce) = match &parsed {
        ParsedWorkspaceLockOwner::V1(owner) => (
            owner.pid,
            owner.process_start_identity.as_str(),
            owner.nonce.as_str(),
        ),
        ParsedWorkspaceLockOwner::V2(owner) => (
            owner.pid,
            owner.process_start_identity.as_str(),
            owner.nonce.as_str(),
        ),
    };
    #[cfg(unix)]
    {
        let mut topology_valid = false;
        let mut topology_delay = Duration::from_micros(50);
        for attempt in 0..16 {
            let descriptor = file.metadata()?;
            if workspace_lock_link_topology_is_valid(lock_path, &descriptor, nonce) {
                topology_valid = true;
                break;
            }
            if attempt + 1 < 16 {
                std::thread::sleep(topology_delay);
                topology_delay = (topology_delay * 2).min(Duration::from_millis(1));
            }
        }
        if !topology_valid {
            let named_changed = fs::symlink_metadata(lock_path).is_ok_and(|named| {
                file.metadata().is_ok_and(|descriptor| {
                    descriptor.dev() != named.dev() || descriptor.ino() != named.ino()
                })
            });
            if named_changed {
                return Ok(match authenticated_predecessor {
                    Ok(Some(owner)) => WorkspaceLockInspection::Turnover(owner),
                    Ok(None) => WorkspaceLockInspection::Terminal(
                        "workspace lock pathname changed during validation".into(),
                    ),
                    Err(reason) => WorkspaceLockInspection::Terminal(reason),
                });
            }
            return Ok(WorkspaceLockInspection::Terminal(
                "workspace lock has an untrusted hard-link topology".into(),
            ));
        }
    }

    if self::util::process_matches_start_token(pid, process_start_identity) {
        return match parsed {
            ParsedWorkspaceLockOwner::V1(_) => Ok(WorkspaceLockInspection::Terminal(
                "live trail-workspace-lock-v1 owner is incompatible with role-aware admission"
                    .into(),
            )),
            ParsedWorkspaceLockOwner::V2(owner) => match current_executable_identity() {
                Ok(identity) if identity == owner.executable_identity => {
                    Ok(WorkspaceLockInspection::Held(owner))
                }
                Ok(_) => Ok(WorkspaceLockInspection::Terminal(
                    "workspace lock executable identity changed while its owner remained live"
                        .into(),
                )),
                Err(error) => Ok(WorkspaceLockInspection::Terminal(format!(
                    "workspace lock executable identity could not be authenticated: {error}"
                ))),
            },
        };
    }
    if !descriptor_exclusive {
        return Ok(WorkspaceLockInspection::Terminal(
            "workspace lock owner is not live but its inode remains held".into(),
        ));
    }

    match fs::remove_file(lock_path) {
        Ok(()) => {
            remove_workspace_lock_candidate_if_matches(
                &lock_path.with_file_name(format!(".workspace-lock-candidate-{nonce}")),
                &holder,
                &file,
            );
            Ok(WorkspaceLockInspection::Reaped)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(WorkspaceLockInspection::Reaped)
        }
        Err(error) => Err(error),
    }
}

fn remove_workspace_lock_candidate_if_matches(
    candidate_path: &Path,
    owner_record: &str,
    owner_file: &File,
) {
    #[cfg(unix)]
    {
        let Ok(descriptor) = owner_file.metadata() else {
            return;
        };
        let Ok(named) = fs::symlink_metadata(candidate_path) else {
            return;
        };
        if !workspace_lock_metadata_is_private_regular(&descriptor)
            || !workspace_lock_metadata_is_private_regular(&named)
            || descriptor.nlink() != 1
            || named.nlink() != 1
            || descriptor.dev() != named.dev()
            || descriptor.ino() != named.ino()
        {
            return;
        }
    }
    if read_workspace_lock_record(owner_file)
        .ok()
        .flatten()
        .as_deref()
        != Some(owner_record)
    {
        return;
    }
    let _ = fs::remove_file(candidate_path);
}

#[cfg(unix)]
fn workspace_lock_metadata_is_private_regular(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_file()
        && metadata.mode() & 0o777 == 0o600
        // SAFETY: geteuid has no preconditions and does not dereference memory.
        && metadata.uid() == unsafe { libc::geteuid() }
}

#[cfg(unix)]
fn workspace_lock_link_topology_is_valid(
    lock_path: &Path,
    metadata: &fs::Metadata,
    nonce: &str,
) -> bool {
    match metadata.nlink() {
        1 => true,
        2 => {
            let Some(parent) = lock_path.parent() else {
                return false;
            };
            let candidate = parent.join(format!(".workspace-lock-candidate-{nonce}"));
            fs::symlink_metadata(candidate).is_ok_and(|candidate| {
                workspace_lock_metadata_is_private_regular(&candidate)
                    && candidate.dev() == metadata.dev()
                    && candidate.ino() == metadata.ino()
                    && candidate.nlink() == 2
            })
        }
        _ => false,
    }
}

fn cleanup_abandoned_workspace_lock_candidates(db_dir: &Path) -> std::io::Result<()> {
    const MAX_DIRECTORY_ENTRIES: usize = 4096;
    let entries = fs::read_dir(db_dir)?;
    for (index, entry) in entries.enumerate() {
        if index >= MAX_DIRECTORY_ENTRIES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "workspace lock directory exceeds the bounded cleanup scan; remove unexpected entries and retry",
            ));
        }
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(nonce) = name.strip_prefix(".workspace-lock-candidate-") else {
            continue;
        };
        if nonce.len() != 64 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            continue;
        }
        let path = entry.path();
        #[cfg(unix)]
        {
            let Ok(named) = fs::symlink_metadata(&path) else {
                continue;
            };
            if !workspace_lock_metadata_is_private_regular(&named) || named.nlink() != 1 {
                continue;
            }
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let Ok(file) = options.open(&path) else {
            continue;
        };
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let mut locked = false;
            for _ in 0..3 {
                if rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive)
                    .is_ok()
                {
                    locked = true;
                    break;
                }
                std::thread::yield_now();
            }
            if !locked {
                continue;
            }
        }
        let Some(record) = read_workspace_lock_record(&file)? else {
            continue;
        };
        let Some(owner) = parse_workspace_lock_owner(&record) else {
            continue;
        };
        let (owner_pid, owner_start, owner_nonce) = match &owner {
            ParsedWorkspaceLockOwner::V1(owner) => (
                owner.pid,
                owner.process_start_identity.as_str(),
                owner.nonce.as_str(),
            ),
            ParsedWorkspaceLockOwner::V2(owner) => (
                owner.pid,
                owner.process_start_identity.as_str(),
                owner.nonce.as_str(),
            ),
        };
        if owner_nonce != nonce || self::util::process_matches_start_token(owner_pid, owner_start) {
            continue;
        }
        #[cfg(unix)]
        {
            let Ok(metadata) = file.metadata() else {
                continue;
            };
            let Ok(named) = fs::symlink_metadata(&path) else {
                continue;
            };
            if !workspace_lock_metadata_is_private_regular(&metadata)
                || metadata.nlink() != 1
                || named.dev() != metadata.dev()
                || named.ino() != metadata.ino()
            {
                continue;
            }
        }
        if read_workspace_lock_record(&file)?.as_deref() != Some(&record) {
            continue;
        }
        let _ = fs::remove_file(path);
    }
    Ok(())
}

impl Drop for AuthorizedObserverWriteExclusion {
    fn drop(&mut self) {
        let Some((state, changed)) = AUTHORIZED_OBSERVER_EXCLUSIONS.get() else {
            return;
        };
        let mut state = state.lock().unwrap_or_else(|poison| poison.into_inner());
        let entry = state.entries.entry(self.db_dir.clone()).or_default();
        debug_assert!(entry.active > 0);
        entry.active = entry.active.saturating_sub(1);
        if entry.active == 0 {
            entry.release_generation = entry.release_generation.wrapping_add(1);
            changed.notify_all();
        }
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
#[cfg(debug_assertions)]
pub(crate) use change_ledger::run_command_flow;
#[cfg(debug_assertions)]
pub(crate) use change_ledger::run_command_long_lock_flow;
#[cfg(debug_assertions)]
pub(crate) use change_ledger::run_materialized_candidate_lifecycle_flow;
#[cfg(debug_assertions)]
pub(crate) use change_ledger::run_materialized_lane_snapshot_flow;
#[cfg(all(debug_assertions, unix))]
pub(crate) use change_ledger::run_non_utf_database_path_mark_recover_and_retire;
#[cfg(debug_assertions)]
pub(crate) use change_ledger::run_tracked_ignored_candidate_flow;
#[cfg(debug_assertions)]
#[allow(unused_imports)]
pub(crate) use change_ledger::set_command_authority_override;
pub(crate) use change_ledger::{
    command_authority_enabled, prepare_workspace_daemon_launch,
    verified_stale_workspace_owner_for_launch, workspace_daemon_fence,
    workspace_daemon_ready_proof, workspace_daemon_reconcile, VerifiedStaleWorkspaceOwner,
    WorkspaceDaemonLaunchIdentity, WorkspaceDaemonProof,
};
#[cfg(debug_assertions)]
pub(crate) use change_ledger::{
    ledger_authority_enabled_for, prepare_workspace_daemon, ActivationEvidence,
    LEDGER_AUTHORITY_ENABLED,
};
pub(crate) use change_ledger::{
    materialized_lane_daemon_full_reconcile, workspace_daemon_full_reconcile,
};
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
    run_retained_writer_quiescence, run_retirement_barrier, run_valid_prefix_interval_recovery,
};
#[cfg(all(debug_assertions, target_os = "linux"))]
pub(crate) use change_ledger::{
    run_authenticated_fence_rejections, run_complete_prefix_publication_races,
    run_content_mode_create_delete, run_controlled_fence_queue_ordering, run_delayed_backlog,
    run_fault_revocation_matrix, run_fence_ordering, run_owner_death_and_root_replacement,
    run_policy_dependency_observation, run_process_owner_child, run_raw_decoder_faults,
    run_reconciliation_interval_qualification, run_recursive_coverage, run_rename_matrix,
    run_rename_storm_and_cookie_expiry, run_segment_writer_reconcile_publication,
    run_unsupported_filesystem_rejection,
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
#[cfg(all(debug_assertions, target_os = "macos"))]
pub(crate) use change_ledger::{
    run_macos_continuity_fault_matrix, run_macos_fence_ordering, run_macos_gap_flag_matrix,
    run_macos_history_authority, run_macos_malformed_callbacks, run_macos_null_context_generation,
    run_macos_paused_callback_fence, run_macos_real_apfs_file_events,
    run_macos_root_revalidation_failures, run_macos_startup_cancellation,
    run_macos_unsupported_filesystem_rejection, run_macos_uuid_revalidation,
};
mod core;
mod lane;
#[cfg(debug_assertions)]
pub(crate) use lane::{
    clear_schema_v19_backfill_times, install_lane_record_after_c2_write_for_current_thread,
    install_schema_v18_authenticated_lane_evidence, install_schema_v19_backfill_times,
    run_changed_path_view_flow, schema_v19_backfill_times_remaining,
    set_lane_association_failure_for_current_thread,
    set_lane_initialization_io_failure_for_current_thread,
    set_lane_record_postcommit_failure_for_current_thread,
    set_sparse_selection_write_failure_for_current_thread,
};
mod merge;
mod performance;
mod record;
#[cfg(debug_assertions)]
pub(crate) use record::{
    install_observed_record_after_compare_hook, install_observed_record_with_lock_hook,
};
mod storage;
use self::performance::*;
#[cfg(any(test, debug_assertions))]
pub(crate) use storage::{
    clear_schema_v19_migration_failure, create_schema_v18_fixture_for_test,
    install_schema_v19_migration_failure, SchemaV19MigrationBoundary,
};
#[cfg(debug_assertions)]
pub(crate) use storage::{
    install_git_qualification_after_c2_hook, install_git_qualification_after_porcelain_hook,
};
pub(crate) use storage::{observed_exact_paths_for_candidates, ObservedPathKind};
mod util;

#[doc(hidden)]
pub use self::util::process_liveness::run_internal_process_watchdog;
pub(crate) use self::util::{redact_sensitive_json, redact_sensitive_text};

#[cfg(test)]
mod tests {
    use super::util::*;
    use super::*;

    fn dead_workspace_lock_owner(nonce_byte: u8) -> WorkspaceLockOwner {
        WorkspaceLockOwner {
            version: 2,
            pid: u32::MAX,
            process_start_identity: "definitely-not-a-live-process".into(),
            executable_identity: "ee".repeat(32),
            nonce: format!("{nonce_byte:02x}").repeat(32),
            purpose: WorkspaceLockPurpose::Maintenance,
            operation_id: None,
            created_at: 1,
        }
    }

    fn legacy_workspace_lock_record(owner: &LegacyWorkspaceLockOwner) -> String {
        format!(
            "trail-workspace-lock-v1\npid={}\nprocess_start_identity={}\nnonce={}\ncreated_at={}\n",
            owner.pid,
            hex::encode(owner.process_start_identity.as_bytes()),
            owner.nonce,
            owner.created_at,
        )
    }

    #[test]
    fn workspace_lock_v2_owner_round_trips_and_rejects_unknown_purpose() {
        let owner = WorkspaceLockOwner::current(
            WorkspaceLockPurpose::ObserverStartup,
            Some("init_round_trip"),
        )
        .unwrap();
        assert_eq!(WorkspaceLockOwner::parse(&owner.encode()), Some(owner));

        let unknown = WorkspaceLockOwner::current(
            WorkspaceLockPurpose::ObserverStartup,
            Some("init_unknown"),
        )
        .unwrap()
        .encode()
        .replace("purpose=observer_startup", "purpose=future_purpose");
        assert!(WorkspaceLockOwner::parse(&unknown).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn workspace_lock_v1_dead_owner_is_reaped_but_live_owner_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let lock_path = temp.path().join("lock");
        let dead = LegacyWorkspaceLockOwner {
            pid: u32::MAX,
            process_start_identity: "dead-v1-owner".into(),
            nonce: "71".repeat(32),
            created_at: 1,
        };
        fs::write(&lock_path, legacy_workspace_lock_record(&dead)).unwrap();
        fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o600)).unwrap();
        let replacement =
            acquire_workspace_lock_for_database(temp.path(), &temp.path().join("trail.sqlite"))
                .unwrap();
        drop(replacement);

        let live = LegacyWorkspaceLockOwner {
            pid: std::process::id(),
            process_start_identity: self::util::current_process_start_token(),
            nonce: "72".repeat(32),
            created_at: self::util::now_ts(),
        };
        fs::write(&lock_path, legacy_workspace_lock_record(&live)).unwrap();
        fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o600)).unwrap();
        let result = acquire_workspace_lock_with_admission(
            temp.path(),
            &temp.path().join("trail.sqlite"),
            WorkspaceLockAdmission {
                purpose: WorkspaceLockPurpose::ObserverStartup,
                operation_id: Some("init-v2"),
                deadline: Duration::from_millis(25),
                retry_command: "trail lane repair-initialization lane-v2",
            },
        );
        assert!(matches!(
            result,
            Err(Error::WorkspaceLocked(message)) if message.contains("v1")
        ));
        assert!(lock_path.exists());
    }

    #[test]
    fn workspace_lock_timeout_is_structured_and_never_revokes_live_owner() {
        let temp = tempfile::tempdir().unwrap();
        let schema = temp.path().join("trail.sqlite");
        let held = acquire_workspace_lock_with_admission(
            temp.path(),
            &schema,
            WorkspaceLockAdmission {
                purpose: WorkspaceLockPurpose::LaneAssociation,
                operation_id: Some("init_holder"),
                deadline: Duration::ZERO,
                retry_command: "holder",
            },
        )
        .unwrap();
        let result = acquire_workspace_lock_with_admission(
            temp.path(),
            &schema,
            WorkspaceLockAdmission {
                purpose: WorkspaceLockPurpose::ObserverStartup,
                operation_id: Some("init_waiter"),
                deadline: Duration::from_millis(20),
                retry_command: "trail lane repair-initialization waiter",
            },
        );
        assert!(matches!(
            result,
            Err(Error::WorkspaceLockTimeout {
                holder_purpose,
                operation_id: Some(operation_id),
                retry_command,
                ..
            }) if holder_purpose == "lane_association"
                && operation_id == "init_holder"
                && retry_command == "trail lane repair-initialization waiter"
        ));
        assert!(temp.path().join("lock").exists());
        drop(held);
        assert!(!temp.path().join("lock").exists());
    }

    #[test]
    fn workspace_lock_observer_startup_same_scope_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let schema = temp.path().join("trail.sqlite");
        let held = acquire_workspace_lock_with_admission(
            temp.path(),
            &schema,
            WorkspaceLockAdmission {
                purpose: WorkspaceLockPurpose::ObserverStartup,
                operation_id: Some("scope-same"),
                deadline: Duration::ZERO,
                retry_command: "holder",
            },
        )
        .unwrap();
        let result = acquire_workspace_lock_with_admission(
            temp.path(),
            &schema,
            WorkspaceLockAdmission {
                purpose: WorkspaceLockPurpose::ObserverStartup,
                operation_id: Some("scope-same"),
                deadline: Duration::from_secs(1),
                retry_command: "retry observer startup",
            },
        );
        assert!(matches!(result, Err(Error::WorkspaceLocked(_))));
        drop(held);
    }

    #[test]
    fn workspace_lock_generation_turnover_retries_only_for_compatible_admission() {
        let temp = tempfile::tempdir().unwrap();
        let db_dir = temp.path().to_path_buf();
        let schema = db_dir.join("trail.sqlite");
        let predecessor = acquire_workspace_lock_with_admission(
            &db_dir,
            &schema,
            WorkspaceLockAdmission {
                purpose: WorkspaceLockPurpose::LaneAssociation,
                operation_id: Some("init-predecessor"),
                deadline: Duration::ZERO,
                retry_command: "predecessor",
            },
        )
        .unwrap();
        let predecessor = Arc::new(Mutex::new(Some(predecessor)));
        let predecessor_for_hook = Arc::clone(&predecessor);
        let successor_dir = db_dir.clone();
        let successor_schema = schema.clone();
        install_workspace_lock_inspection_hook(&db_dir.join("lock"), move || {
            drop(
                predecessor_for_hook
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .take(),
            );
            let successor = acquire_workspace_lock_with_admission(
                &successor_dir,
                &successor_schema,
                WorkspaceLockAdmission {
                    purpose: WorkspaceLockPurpose::ObserverPublication,
                    operation_id: Some("publication-successor"),
                    deadline: Duration::ZERO,
                    retry_command: "successor",
                },
            )
            .unwrap();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(20));
                drop(successor);
            });
        });

        let acquired = acquire_workspace_lock_with_admission(
            &db_dir,
            &schema,
            WorkspaceLockAdmission {
                purpose: WorkspaceLockPurpose::ObserverStartup,
                operation_id: Some("scope-distinct"),
                deadline: Duration::from_secs(1),
                retry_command: "retry observer startup",
            },
        )
        .unwrap();
        drop(acquired);
        assert!(!db_dir.join("lock").exists());
    }

    #[test]
    fn workspace_lock_distinct_observer_startups_wait_for_same_process_release() {
        let temp = tempfile::tempdir().unwrap();
        let db_dir = temp.path().to_path_buf();
        let schema = db_dir.join("trail.sqlite");
        let held = acquire_workspace_lock_with_admission(
            &db_dir,
            &schema,
            WorkspaceLockAdmission {
                purpose: WorkspaceLockPurpose::ObserverStartup,
                operation_id: Some("scope-a"),
                deadline: Duration::ZERO,
                retry_command: "holder",
            },
        )
        .unwrap();
        let waiter_dir = db_dir.clone();
        let waiter_schema = schema.clone();
        let waiter = std::thread::spawn(move || {
            acquire_workspace_lock_with_admission(
                &waiter_dir,
                &waiter_schema,
                WorkspaceLockAdmission {
                    purpose: WorkspaceLockPurpose::ObserverStartup,
                    operation_id: Some("scope-b"),
                    deadline: Duration::from_secs(1),
                    retry_command: "retry observer startup",
                },
            )
        });
        std::thread::sleep(Duration::from_millis(25));
        drop(held);
        let acquired = waiter.join().unwrap().unwrap();
        drop(acquired);
        assert!(!db_dir.join("lock").exists());
    }

    #[test]
    fn workspace_lock_live_pid_reuse_and_executable_replacement_fail_closed() {
        for replacement in ["process_start", "executable"] {
            let temp = tempfile::tempdir().unwrap();
            let lock_path = temp.path().join("lock");
            let mut owner = WorkspaceLockOwner::current(
                WorkspaceLockPurpose::LaneAssociation,
                Some("init_identity"),
            )
            .unwrap();
            match replacement {
                "process_start" => owner.process_start_identity = "reused-pid".into(),
                "executable" => owner.executable_identity = "ff".repeat(32),
                _ => unreachable!(),
            }
            let publication = publish_workspace_lock(temp.path(), &lock_path, &owner).unwrap();
            let result = acquire_workspace_lock_with_admission(
                temp.path(),
                &temp.path().join("trail.sqlite"),
                WorkspaceLockAdmission {
                    purpose: WorkspaceLockPurpose::ObserverStartup,
                    operation_id: Some("init_waiter"),
                    deadline: Duration::from_millis(20),
                    retry_command: "retry observer startup",
                },
            );
            assert!(matches!(result, Err(Error::WorkspaceLocked(_))));
            assert!(lock_path.exists());
            remove_owned_workspace_lock(&lock_path, &owner.encode(), &publication.file);
            assert!(!lock_path.exists());
        }
    }

    #[test]
    fn workspace_lock_crash_before_atomic_publish_leaves_no_lock_name() {
        let temp = tempfile::tempdir().unwrap();
        let owner = dead_workspace_lock_owner(0x11);
        let lock_path = temp.path().join("lock");
        crash_next_workspace_lock_publish(&lock_path, WorkspaceLockPublishPhase::BeforePublish);

        let crashed = std::panic::catch_unwind(|| {
            publish_workspace_lock(temp.path(), &lock_path, &owner).unwrap();
        });

        assert!(crashed.is_err());
        assert!(!lock_path.exists());
        let candidate = temp
            .path()
            .join(format!(".workspace-lock-candidate-{}", owner.nonce));
        assert_eq!(
            WorkspaceLockOwner::parse(&fs::read_to_string(&candidate).unwrap()),
            Some(owner.clone())
        );
        #[cfg(unix)]
        {
            let metadata = fs::symlink_metadata(&candidate).unwrap();
            assert!(workspace_lock_metadata_is_private_regular(&metadata));
            assert_eq!(metadata.nlink(), 1);
        }
        assert!(!self::util::process_matches_start_token(
            owner.pid,
            &owner.process_start_identity
        ));
        let lock =
            acquire_workspace_lock_for_database(temp.path(), &temp.path().join("trail.sqlite"))
                .unwrap();
        assert!(!candidate.exists());
        drop(lock);
    }

    #[test]
    fn workspace_lock_crash_after_atomic_publish_is_reaped_by_verified_identity() {
        let temp = tempfile::tempdir().unwrap();
        let lock_path = temp.path().join("lock");
        let owner = dead_workspace_lock_owner(0x22);
        crash_next_workspace_lock_publish(&lock_path, WorkspaceLockPublishPhase::AfterPublish);
        let crashed = std::panic::catch_unwind(|| {
            publish_workspace_lock(temp.path(), &lock_path, &owner).unwrap();
        });
        assert!(crashed.is_err());
        let candidate = temp
            .path()
            .join(format!(".workspace-lock-candidate-{}", owner.nonce));
        assert!(candidate.exists());
        assert_eq!(fs::read_to_string(&lock_path).unwrap(), owner.encode());

        let lock =
            acquire_workspace_lock_for_database(temp.path(), &temp.path().join("trail.sqlite"))
                .unwrap();

        let replacement = WorkspaceLockOwner::parse(&fs::read_to_string(&lock_path).unwrap())
            .expect("replacement lock has an authenticated owner record");
        assert_ne!(replacement.nonce, owner.nonce);
        assert_eq!(replacement.pid, std::process::id());
        assert!(!candidate.exists());
        drop(lock);
        assert!(!lock_path.exists());
    }

    #[test]
    fn workspace_lock_atomic_publication_has_one_winner_under_parallel_contention() {
        use std::sync::atomic::AtomicUsize;
        use std::sync::Barrier;

        const CONTENDERS: usize = 16;
        let temp = tempfile::tempdir().unwrap();
        let db_dir = Arc::new(temp.path().to_path_buf());
        let start = Arc::new(Barrier::new(CONTENDERS + 1));
        let acquired = Arc::new(Barrier::new(CONTENDERS + 1));
        let release = Arc::new(Barrier::new(CONTENDERS + 1));
        let winners = Arc::new(AtomicUsize::new(0));
        let mut threads = Vec::new();
        for _ in 0..CONTENDERS {
            let db_dir = Arc::clone(&db_dir);
            let start = Arc::clone(&start);
            let acquired = Arc::clone(&acquired);
            let release = Arc::clone(&release);
            let winners = Arc::clone(&winners);
            threads.push(std::thread::spawn(move || {
                start.wait();
                let result =
                    acquire_workspace_lock_for_database(&db_dir, &db_dir.join("trail.sqlite"));
                if result.is_ok() {
                    winners.fetch_add(1, Ordering::SeqCst);
                } else {
                    assert!(matches!(result, Err(Error::WorkspaceLocked(_))));
                }
                acquired.wait();
                release.wait();
                drop(result);
            }));
        }

        start.wait();
        acquired.wait();
        assert_eq!(winners.load(Ordering::SeqCst), 1);
        release.wait();
        for thread in threads {
            thread.join().unwrap();
        }
        assert!(!temp.path().join("lock").exists());
    }

    #[test]
    fn legacy_empty_workspace_lock_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let lock_path = temp.path().join("lock");
        File::create(&lock_path).unwrap();

        let result =
            acquire_workspace_lock_for_database(temp.path(), &temp.path().join("trail.sqlite"));

        assert!(matches!(result, Err(Error::WorkspaceLocked(_))));
        assert!(lock_path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn workspace_lock_candidate_cleanup_retains_hostile_symlink_type_mode_and_hardlink() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let temp = tempfile::tempdir().unwrap();

        let symlink_owner = dead_workspace_lock_owner(0x31);
        let symlink_victim = temp.path().join("symlink-victim");
        fs::write(&symlink_victim, symlink_owner.encode()).unwrap();
        let symlink_candidate = temp
            .path()
            .join(format!(".workspace-lock-candidate-{}", symlink_owner.nonce));
        symlink(&symlink_victim, &symlink_candidate).unwrap();

        let mode_owner = dead_workspace_lock_owner(0x32);
        let mode_candidate = temp
            .path()
            .join(format!(".workspace-lock-candidate-{}", mode_owner.nonce));
        fs::write(&mode_candidate, mode_owner.encode()).unwrap();
        fs::set_permissions(&mode_candidate, fs::Permissions::from_mode(0o644)).unwrap();

        let type_owner = dead_workspace_lock_owner(0x33);
        let type_candidate = temp
            .path()
            .join(format!(".workspace-lock-candidate-{}", type_owner.nonce));
        fs::create_dir(&type_candidate).unwrap();

        let hardlink_owner = dead_workspace_lock_owner(0x34);
        let hardlink_victim = temp.path().join("hardlink-victim");
        fs::write(&hardlink_victim, hardlink_owner.encode()).unwrap();
        fs::set_permissions(&hardlink_victim, fs::Permissions::from_mode(0o600)).unwrap();
        let hardlink_candidate = temp.path().join(format!(
            ".workspace-lock-candidate-{}",
            hardlink_owner.nonce
        ));
        fs::hard_link(&hardlink_victim, &hardlink_candidate).unwrap();

        let lock =
            acquire_workspace_lock_for_database(temp.path(), &temp.path().join("trail.sqlite"))
                .unwrap();

        assert!(symlink_candidate
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(symlink_victim.exists());
        assert!(mode_candidate.exists());
        assert!(type_candidate.is_dir());
        assert!(hardlink_candidate.exists());
        assert!(hardlink_victim.exists());
        drop(lock);
    }

    #[cfg(unix)]
    #[test]
    fn workspace_lock_hostile_symlink_type_mode_and_hardlink_fail_closed() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        for kind in ["symlink", "directory", "mode", "hardlink"] {
            let temp = tempfile::tempdir().unwrap();
            let lock_path = temp.path().join("lock");
            let owner = dead_workspace_lock_owner(match kind {
                "symlink" => 0x41,
                "directory" => 0x42,
                "mode" => 0x43,
                _ => 0x44,
            });
            match kind {
                "symlink" => {
                    let victim = temp.path().join("victim");
                    fs::write(&victim, owner.encode()).unwrap();
                    symlink(victim, &lock_path).unwrap();
                }
                "directory" => fs::create_dir(&lock_path).unwrap(),
                "mode" => {
                    fs::write(&lock_path, owner.encode()).unwrap();
                    fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o644)).unwrap();
                }
                "hardlink" => {
                    let victim = temp.path().join("victim");
                    fs::write(&victim, owner.encode()).unwrap();
                    fs::set_permissions(&victim, fs::Permissions::from_mode(0o600)).unwrap();
                    fs::hard_link(victim, &lock_path).unwrap();
                }
                _ => unreachable!(),
            }

            let result =
                acquire_workspace_lock_for_database(temp.path(), &temp.path().join("trail.sqlite"));

            assert!(
                matches!(result, Err(Error::WorkspaceLocked(_))),
                "hostile {kind} lock must fail closed: {result:?}"
            );
            assert!(lock_path.symlink_metadata().is_ok());
        }
    }

    #[cfg(unix)]
    #[test]
    fn workspace_lock_oversized_record_fails_closed_without_unbounded_read() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let lock_path = temp.path().join("lock");
        fs::write(
            &lock_path,
            vec![b'x'; MAX_WORKSPACE_LOCK_RECORD_BYTES as usize + 1],
        )
        .unwrap();
        fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o600)).unwrap();

        let result =
            acquire_workspace_lock_for_database(temp.path(), &temp.path().join("trail.sqlite"));

        assert!(
            matches!(result, Err(Error::WorkspaceLocked(message)) if message.contains("size limit"))
        );
        assert_eq!(
            fs::metadata(lock_path).unwrap().len(),
            MAX_WORKSPACE_LOCK_RECORD_BYTES + 1
        );
    }

    #[test]
    fn workspace_lock_owner_unlink_does_not_remove_substituted_successor() {
        let temp = tempfile::tempdir().unwrap();
        let lock_path = temp.path().join("lock");
        let owner =
            WorkspaceLockOwner::current(WorkspaceLockPurpose::CommandMutation, None).unwrap();
        let publication = publish_workspace_lock(temp.path(), &lock_path, &owner).unwrap();
        fs::remove_file(&lock_path).unwrap();
        let successor =
            WorkspaceLockOwner::current(WorkspaceLockPurpose::CommandMutation, None).unwrap();
        let successor_publication =
            publish_workspace_lock(temp.path(), &lock_path, &successor).unwrap();

        remove_owned_workspace_lock(&lock_path, &owner.encode(), &publication.file);

        assert_eq!(fs::read_to_string(&lock_path).unwrap(), successor.encode());
        remove_owned_workspace_lock(&lock_path, &successor.encode(), &successor_publication.file);
        assert!(!lock_path.exists());
    }

    #[test]
    fn workspace_lock_candidate_unlink_failure_rolls_back_and_allows_same_process_reacquire() {
        let temp = tempfile::tempdir().unwrap();
        let lock_path = temp.path().join("lock");
        fail_next_workspace_lock_candidate_unlink(&lock_path);

        let failed =
            acquire_workspace_lock_for_database(temp.path(), &temp.path().join("trail.sqlite"));

        assert!(
            matches!(failed, Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::PermissionDenied)
        );
        assert!(!lock_path.exists());
        assert!(!fs::read_dir(temp.path()).unwrap().any(|entry| {
            entry
                .ok()
                .and_then(|entry| entry.file_name().into_string().ok())
                .is_some_and(|name| name.starts_with(".workspace-lock-candidate-"))
        }));

        let lock =
            acquire_workspace_lock_for_database(temp.path(), &temp.path().join("trail.sqlite"))
                .unwrap();
        drop(lock);
        assert!(!lock_path.exists());
    }

    #[test]
    fn workspace_lock_double_unlink_failure_retains_raii_owner_until_drop_then_reacquires() {
        let temp = tempfile::tempdir().unwrap();
        let lock_path = temp.path().join("lock");
        fail_next_workspace_lock_candidate_unlink(&lock_path);
        fail_next_workspace_lock_rollback_unlink(&lock_path);

        let retained =
            acquire_workspace_lock_for_database(temp.path(), &temp.path().join("trail.sqlite"))
                .unwrap();

        assert!(lock_path.exists());
        assert!(retained
            .candidate_residue
            .as_ref()
            .is_some_and(|path| path.exists()));
        let contended =
            acquire_workspace_lock_for_database(temp.path(), &temp.path().join("trail.sqlite"));
        assert!(matches!(contended, Err(Error::WorkspaceLocked(_))));

        drop(retained);
        assert!(!lock_path.exists());
        let reacquired =
            acquire_workspace_lock_for_database(temp.path(), &temp.path().join("trail.sqlite"))
                .unwrap();
        drop(reacquired);
        assert!(!lock_path.exists());
    }

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
                    policy_dependency_full_discovery: 39,
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
        assert_eq!(
            report.selected_worktree_index_sqlite_accounting_disposition,
            "complete"
        );
        assert_eq!(report.selected_worktree_index_sqlite_envelope_count, 1);
        assert_eq!(
            report.selected_worktree_index_sqlite_not_applicable_count,
            0
        );
        assert_eq!(report.selected_worktree_index_sqlite_full_scan_count, 19);
        assert_eq!(report.selected_worktree_index_sqlite_row_read_count, 20);
        assert_eq!(report.selected_worktree_index_sqlite_row_delete_count, 21);
        assert_eq!(report.selected_worktree_index_sqlite_row_upsert_count, 22);
        assert_eq!(report.selected_worktree_index_sqlite_statement_count, 23);
        assert_eq!(report.selected_worktree_index_sqlite_transaction_count, 24);
        assert_eq!(report.selection_comparison_count, 25);
        assert_eq!(report.policy_build_count, 26);
        assert_eq!(report.policy_dependency_full_discovery, 39);
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
    fn operation_metrics_distinguish_not_applicable_from_ambiguous_sqlite_accounting() {
        let metrics = Arc::new(OperationMetricsState::default());
        metrics
            .profile(OperationMetricsKind::Status, || {
                metrics.add(OperationMetricsDelta {
                    selected_worktree_index_sqlite_not_applicable_count: 1,
                    ..OperationMetricsDelta::default()
                });
                Ok::<(), Error>(())
            })
            .unwrap();
        let not_applicable = metrics.last_report();
        assert!(!not_applicable.selected_worktree_index_sqlite_accounting_complete);
        assert_eq!(
            not_applicable.selected_worktree_index_sqlite_accounting_disposition,
            "not_applicable"
        );
        assert_eq!(
            not_applicable.selected_worktree_index_sqlite_not_applicable_count,
            1
        );

        metrics
            .profile(OperationMetricsKind::Diff, || {
                metrics.add(OperationMetricsDelta {
                    selected_worktree_index_sqlite_envelope_count: 1,
                    selected_worktree_index_sqlite_not_applicable_count: 1,
                    ..OperationMetricsDelta::default()
                });
                Ok::<(), Error>(())
            })
            .unwrap();
        let ambiguous = metrics.last_report();
        assert!(!ambiguous.selected_worktree_index_sqlite_accounting_complete);
        assert_eq!(
            ambiguous.selected_worktree_index_sqlite_accounting_disposition,
            "ambiguous"
        );
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
