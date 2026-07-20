use super::*;
use crate::db::change_ledger::secure_fs::SecureDirectory;

const MAX_MATERIALIZATION_OPERATION_BYTES: u64 = 64 * 1024;
// Native COW cloning is metadata-heavy: every file is cloned, authenticated,
// and flushed. Agent processes are already independently concurrent, so a
// workspace-wide bound prevents a burst of lane spawns from saturating APFS
// (or an equivalent POSIX filesystem) with clone and xattr operations.
const MAX_CONCURRENT_NATIVE_MATERIALIZATIONS: usize = 16;
const NATIVE_MATERIALIZATION_ADMISSION_WAIT: std::time::Duration =
    std::time::Duration::from_secs(120);
const MATERIALIZATION_COORDINATION_DIRECTORY: &str = "materialization-coordination";
const MATERIALIZATION_RECOVERY_LOCK: &str = "recovery";

fn materialization_coordination_directory(db_dir: &Path) -> Result<SecureDirectory> {
    let db_dir = db_dir.canonicalize()?;
    let authority = SecureDirectory::open_absolute(&db_dir)?;
    match authority.open_private_dir(MATERIALIZATION_COORDINATION_DIRECTORY) {
        Ok(directory) => Ok(directory),
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            match authority.create_private_dir(MATERIALIZATION_COORDINATION_DIRECTORY) {
                Ok(directory) => Ok(directory),
                Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    authority.open_private_dir(MATERIALIZATION_COORDINATION_DIRECTORY)
                }
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

struct MaterializationRecoveryGuard {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    _lock: std::fs::File,
}

impl MaterializationRecoveryGuard {
    fn acquire_shared(db_dir: &Path) -> Result<Self> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let coordination = materialization_coordination_directory(db_dir)?;
            let file =
                coordination.open_or_create_private_regular(MATERIALIZATION_RECOVERY_LOCK)?;
            let deadline = std::time::Instant::now() + NATIVE_MATERIALIZATION_ADMISSION_WAIT;
            let mut delay = std::time::Duration::from_millis(2);
            loop {
                match rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockShared) {
                    Ok(()) => return Ok(Self { _lock: file }),
                    Err(error) if error == rustix::io::Errno::AGAIN => {}
                    Err(error) => return Err(Error::Io(error.into())),
                }
                if std::time::Instant::now() >= deadline {
                    return Err(Error::WorkspaceLockTimeout {
                        holder_purpose: "materialization_recovery".to_string(),
                        holder_age_ms: NATIVE_MATERIALIZATION_ADMISSION_WAIT
                            .as_millis()
                            .try_into()
                            .unwrap_or(u64::MAX),
                        operation_id: None,
                        retry_command: "repeat the lane spawn command".to_string(),
                    });
                }
                std::thread::sleep(delay);
                delay = (delay * 2).min(std::time::Duration::from_millis(50));
            }
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = db_dir;
            Ok(Self {})
        }
    }

    fn try_acquire_exclusive(db_dir: &Path) -> Result<Self> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let coordination = materialization_coordination_directory(db_dir)?;
            let file =
                coordination.open_or_create_private_regular(MATERIALIZATION_RECOVERY_LOCK)?;
            match rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive)
            {
                Ok(()) => Ok(Self { _lock: file }),
                Err(error) if error == rustix::io::Errno::AGAIN => Err(Error::WorkspaceLocked(
                    "materialization recovery is deferred while an active materialization owns the workspace".into(),
                )),
                Err(error) => Err(Error::Io(error.into())),
            }
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = db_dir;
            Ok(Self {})
        }
    }
}

struct NativeMaterializationAdmission {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    _slot: std::fs::File,
}

impl NativeMaterializationAdmission {
    fn acquire(db_dir: &Path) -> Result<Self> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let slots = materialization_coordination_directory(db_dir)?;
            let deadline = std::time::Instant::now() + NATIVE_MATERIALIZATION_ADMISSION_WAIT;
            let mut delay = std::time::Duration::from_millis(2);
            let first_slot = (now_nanos() as usize) % MAX_CONCURRENT_NATIVE_MATERIALIZATIONS;
            loop {
                for offset in 0..MAX_CONCURRENT_NATIVE_MATERIALIZATIONS {
                    let slot = (first_slot + offset) % MAX_CONCURRENT_NATIVE_MATERIALIZATIONS;
                    let file = slots.open_or_create_private_regular(&format!("slot-{slot:02}"))?;
                    match rustix::fs::flock(
                        &file,
                        rustix::fs::FlockOperation::NonBlockingLockExclusive,
                    ) {
                        Ok(()) => return Ok(Self { _slot: file }),
                        Err(error) if error == rustix::io::Errno::AGAIN => {}
                        Err(error) => return Err(Error::Io(error.into())),
                    }
                }
                if std::time::Instant::now() >= deadline {
                    return Err(Error::WorkspaceLockTimeout {
                        holder_purpose: "native_cow_materialization".to_string(),
                        holder_age_ms: NATIVE_MATERIALIZATION_ADMISSION_WAIT
                            .as_millis()
                            .try_into()
                            .unwrap_or(u64::MAX),
                        operation_id: None,
                        retry_command: "repeat the lane spawn command".to_string(),
                    });
                }
                std::thread::sleep(delay);
                delay = (delay * 2).min(std::time::Duration::from_millis(50));
            }
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = db_dir;
            Ok(Self {})
        }
    }
}

/// Holds one workspace-wide pre-open slot for a materializing CLI lane spawn.
/// This deliberately uses a different slot namespace from the native-COW
/// stage admission: it prevents a burst from overwhelming SQLite preflight,
/// while the latter continues to bound filesystem clone work.
pub(crate) struct PreOpenMaterializationAdmission {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    _slot: std::fs::File,
}

impl PreOpenMaterializationAdmission {
    pub(crate) fn acquire_for_workspace(workspace: &Path) -> Result<Self> {
        let workspace = workspace.canonicalize()?;
        Self::acquire(&workspace.join(".trail/index"))
    }

    fn acquire(db_dir: &Path) -> Result<Self> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let slots = materialization_coordination_directory(db_dir)?;
            let deadline = std::time::Instant::now() + NATIVE_MATERIALIZATION_ADMISSION_WAIT;
            let mut delay = std::time::Duration::from_millis(2);
            let first_slot = (now_nanos() as usize) % MAX_CONCURRENT_NATIVE_MATERIALIZATIONS;
            loop {
                for offset in 0..MAX_CONCURRENT_NATIVE_MATERIALIZATIONS {
                    let slot = (first_slot + offset) % MAX_CONCURRENT_NATIVE_MATERIALIZATIONS;
                    let file =
                        slots.open_or_create_private_regular(&format!("spawn-slot-{slot:02}"))?;
                    match rustix::fs::flock(
                        &file,
                        rustix::fs::FlockOperation::NonBlockingLockExclusive,
                    ) {
                        Ok(()) => return Ok(Self { _slot: file }),
                        Err(error) if error == rustix::io::Errno::AGAIN => {}
                        Err(error) => return Err(Error::Io(error.into())),
                    }
                }
                if std::time::Instant::now() >= deadline {
                    return Err(Error::WorkspaceLockTimeout {
                        holder_purpose: "native_cow_spawn_admission".to_string(),
                        holder_age_ms: NATIVE_MATERIALIZATION_ADMISSION_WAIT
                            .as_millis()
                            .try_into()
                            .unwrap_or(u64::MAX),
                        operation_id: None,
                        retry_command: "repeat the lane spawn command".to_string(),
                    });
                }
                std::thread::sleep(delay);
                delay = (delay * 2).min(std::time::Duration::from_millis(50));
            }
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = db_dir;
            Ok(Self {})
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MaterializationPolicy {
    StrictNative,
    Portable,
    Auto,
}

#[derive(Clone, Debug)]
pub(crate) struct MaterializationOutcome {
    pub(crate) resolved_mode: LaneWorkdirMode,
    pub(crate) backend: WorkdirBackend,
    pub(crate) report: MaterializationReport,
    pub(crate) materialization_operation_id: String,
}

#[derive(Debug)]
enum NativeAttemptError {
    Unavailable(MaterializationFallbackReason),
    Hard(Error),
}

struct NativeSource {
    root: PathBuf,
    stamps: BTreeMap<String, WorktreeFileStamp>,
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum MaterializationOperationState {
    Preparing,
    Materializing,
    Verified,
    Published,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct MaterializationOperationRecord {
    version: u16,
    operation_id: String,
    parent: String,
    parent_device: u64,
    parent_inode: u64,
    destination_leaf: String,
    stage_leaf: String,
    owned_device: u64,
    owned_inode: u64,
    root_id: String,
    state: MaterializationOperationState,
    owner_pid: u32,
    owner_start_token: String,
}

struct RegisteredMaterializationStage {
    record_path: PathBuf,
    stage_path: PathBuf,
    parent_authority: SecureDirectory,
    stage_authority: SecureDirectory,
    _recovery_guard: MaterializationRecoveryGuard,
    record: MaterializationOperationRecord,
}

fn new_materialization_operation_id() -> Result<String> {
    let mut nonce = [0_u8; 16];
    getrandom::getrandom(&mut nonce)
        .map_err(|error| Error::Io(std::io::Error::other(error.to_string())))?;
    Ok(format!(
        "materialize-{}-{}",
        now_nanos(),
        hex::encode(nonce)
    ))
}

impl RegisteredMaterializationStage {
    fn path(&self) -> &Path {
        &self.stage_path
    }

    fn set_state(&mut self, state: MaterializationOperationState) -> Result<()> {
        self.record.state = state;
        write_materialization_record(&self.record_path, &self.record)
    }

    fn publish(&mut self) -> Result<()> {
        let expected_parent = (self.record.parent_device, self.record.parent_inode);
        let expected_stage = (self.record.owned_device, self.record.owned_inode);
        // Retain the originally authenticated descriptor for publication, and
        // also require the path binding to still name that same parent.
        SecureDirectory::open_absolute(Path::new(&self.record.parent))?
            .verify_identity(expected_parent)?;
        self.parent_authority.verify_identity(expected_parent)?;
        self.stage_authority.verify_identity(expected_stage)?;
        self.parent_authority
            .open_dir(&self.record.stage_leaf)?
            .verify_identity(expected_stage)?;
        self.parent_authority
            .rename_leaf_noreplace(&self.record.stage_leaf, &self.record.destination_leaf)?;
        self.parent_authority.sync()?;
        self.parent_authority
            .open_dir(&self.record.destination_leaf)?
            .verify_identity(expected_stage)?;
        Ok(())
    }

    fn abort(self) {
        if remove_owned_materialization_tree(&self.record, true).is_ok() {
            let _ = remove_materialization_record(&self.record_path);
        }
    }
}

impl From<Error> for NativeAttemptError {
    fn from(error: Error) -> Self {
        Self::Hard(error)
    }
}

impl From<std::io::Error> for NativeAttemptError {
    fn from(error: std::io::Error) -> Self {
        Self::Hard(Error::Io(error))
    }
}

impl Trail {
    pub(crate) fn materialize_lane_root_staged(
        &self,
        root_id: &ObjectId,
        destination: &Path,
        custom_workdir: bool,
        policy: MaterializationPolicy,
    ) -> Result<MaterializationOutcome> {
        prepare_staged_destination(destination, custom_workdir)?;
        match policy {
            MaterializationPolicy::StrictNative => self
                .materialize_strict_native_attempt(root_id, destination)
                .map_err(native_attempt_error),
            MaterializationPolicy::Portable => {
                self.materialize_portable_attempt(root_id, destination, None)
            }
            MaterializationPolicy::Auto => {
                match self.materialize_strict_native_attempt(root_id, destination) {
                    Ok(outcome) => Ok(outcome),
                    Err(NativeAttemptError::Unavailable(reason)) => {
                        self.materialize_portable_attempt(root_id, destination, Some(reason))
                    }
                    Err(NativeAttemptError::Hard(error)) => Err(error),
                }
            }
        }
    }

    pub(crate) fn materialize_sparse_lane_root_staged(
        &self,
        root_id: &ObjectId,
        destination: &Path,
        custom_workdir: bool,
        files: &BTreeMap<String, FileEntry>,
    ) -> Result<MaterializationOutcome> {
        prepare_staged_destination(destination, custom_workdir)?;
        let mut operation = self.create_materialization_stage(destination, root_id)?;
        let operation_id = operation.record.operation_id.clone();
        let stage = operation.path().to_path_buf();
        let result = (|| {
            let empty = BTreeMap::new();
            let mut stamps = BTreeMap::new();
            let mut report = MaterializationReport::default();
            for (path, entry) in files {
                let mut cloned = false;
                match materialize_workspace_file_cow_status_if_matching_with_durability(
                    &self.workspace_root,
                    &stage,
                    path,
                    entry,
                    true,
                )? {
                    WorkspaceCowMaterializeStatus::Cloned(stamp) => {
                        stamps.insert(path.clone(), stamp);
                        report.cloned_files += 1;
                        report.cloned_bytes += entry.size_bytes;
                        cloned = true;
                    }
                    WorkspaceCowMaterializeStatus::Skipped
                    | WorkspaceCowMaterializeStatus::Unavailable(_) => {}
                }
                if !cloned {
                    let one = BTreeMap::from([(path.clone(), entry.clone())]);
                    let materialized = self.materialize_files_at_report(&stage, &empty, &one)?;
                    stamps.extend(materialized.stamps);
                    report.copied_files += 1;
                    report.copied_bytes += entry.size_bytes;
                }
            }

            self.write_sparse_workdir_manifest(&stage, files.keys())?;
            self.write_clean_workdir_manifest_from_stamps(
                &stage,
                root_id,
                files,
                files.keys(),
                stamps,
            )?;
            ensure_staged_manifest_is_clean(self, &stage, root_id)?;
            let backend = report.backend();
            operation.set_state(MaterializationOperationState::Verified)?;
            operation.publish()?;
            ensure_staged_manifest_is_clean(self, destination, root_id)?;
            operation.set_state(MaterializationOperationState::Published)?;
            Ok(MaterializationOutcome {
                resolved_mode: LaneWorkdirMode::Sparse,
                backend,
                report,
                materialization_operation_id: operation_id,
            })
        })();
        if result.is_err() {
            operation.abort();
        }
        result
    }

    fn materialize_strict_native_attempt(
        &self,
        root_id: &ObjectId,
        destination: &Path,
    ) -> std::result::Result<MaterializationOutcome, NativeAttemptError> {
        // Hold this through source discovery, cloning, verification, and
        // publication. Releasing it earlier merely moves the APFS saturation
        // to source metadata scans or durable sync barriers.
        let _admission = NativeMaterializationAdmission::acquire(&self.db_dir)?;
        let files = self.load_root_files(root_id)?;
        let source = self
            .resolve_native_materialization_source(root_id, &files)?
            .ok_or(NativeAttemptError::Unavailable(
                MaterializationFallbackReason::NativeSourceUnavailable,
            ))?;

        let mut operation = self.create_materialization_stage(destination, root_id)?;
        let operation_id = operation.record.operation_id.clone();
        let stage = operation.path().to_path_buf();
        let result = (|| {
            verify_same_native_filesystem(&source.root, &stage)?;
            probe_native_clone(&stage)?;
            // Pin this while the stage is still allowed to change. Case
            // sensitivity detection creates and unlinks a probe file and must
            // never run during either post-barrier verification.
            let case_insensitive = is_case_insensitive_filesystem(&stage)?;

            let mut stamps = BTreeMap::new();
            let mut report = MaterializationReport::default();
            // Lane spawning is already parallel across CLI/daemon processes.
            // A Rayon pool inside every materializing lane multiplies that
            // parallelism (for example, 64 lane spawns each create a full
            // worker pool) and overwhelms filesystem clone/xattr operations.
            // Keep a lane's native-COW work ordered; the outer lane fan-out
            // remains the concurrency boundary.
            let results = files
                .iter()
                .map(|(path, entry)| {
                    let source_stamp =
                        source
                            .stamps
                            .get(path)
                            .ok_or(NativeAttemptError::Unavailable(
                                MaterializationFallbackReason::NativeSourceUnavailable,
                            ))?;
                    let status = materialize_workspace_file_cow_status_if_stamp_matches(
                        &source.root,
                        &stage,
                        path,
                        entry,
                        *source_stamp,
                        false,
                    )?;
                    Ok((path.clone(), entry.size_bytes, status))
                })
                .collect::<Vec<std::result::Result<_, NativeAttemptError>>>();
            for result in results {
                let (path, size_bytes, status) = result?;
                match status {
                    WorkspaceCowMaterializeStatus::Cloned(stamp) => {
                        stamps.insert(path, stamp);
                        report.cloned_files += 1;
                        report.cloned_bytes += size_bytes;
                    }
                    WorkspaceCowMaterializeStatus::Skipped => {
                        return Err(NativeAttemptError::Unavailable(
                            MaterializationFallbackReason::NativeSourceUnavailable,
                        ));
                    }
                    WorkspaceCowMaterializeStatus::Unavailable(reason) => {
                        return Err(NativeAttemptError::Unavailable(fallback_reason_for_clone(
                            reason,
                        )));
                    }
                }
            }

            self.write_clean_workdir_manifest_from_stamps(
                &stage,
                root_id,
                &files,
                files.keys(),
                stamps,
            )?;
            // The stage is private and cannot be published until this batch
            // durability barrier succeeds. Flush every source and cloned
            // inode with ordinary fsync, every created directory ancestor
            // bottom-up, and finally one full volume/filesystem barrier. The
            // manifest is deliberately written first so `.trail`, its file,
            // and the stage-parent binding are included in the same cut.
            sync_native_materialization_stage(&source.root, &stage, files.keys())?;
            ensure_staged_manifest_is_clean_read_only(self, &stage, root_id, case_insensitive)?;
            operation.set_state(MaterializationOperationState::Verified)?;
            operation.publish()?;
            ensure_staged_manifest_is_clean_read_only(
                self,
                destination,
                root_id,
                case_insensitive,
            )?;
            operation.set_state(MaterializationOperationState::Published)?;
            Ok(MaterializationOutcome {
                resolved_mode: LaneWorkdirMode::NativeCow,
                backend: WorkdirBackend::Clone,
                report,
                materialization_operation_id: operation_id,
            })
        })();
        if result.is_err() {
            operation.abort();
        }
        result
    }

    fn materialize_portable_attempt(
        &self,
        root_id: &ObjectId,
        destination: &Path,
        fallback_reason: Option<MaterializationFallbackReason>,
    ) -> Result<MaterializationOutcome> {
        let files = self.load_root_files(root_id)?;
        let source = self.resolve_native_materialization_source(root_id, &files)?;
        let mut operation = self.create_materialization_stage(destination, root_id)?;
        let operation_id = operation.record.operation_id.clone();
        let stage = operation.path().to_path_buf();
        let result = (|| {
            let mut stamps = BTreeMap::new();
            let mut report = MaterializationReport {
                fallback_reason,
                ..MaterializationReport::default()
            };
            let empty = BTreeMap::new();
            for (path, entry) in &files {
                let mut cloned = false;
                let clone_status = if let Some(native_source) = source.as_ref() {
                    if let Some(source_stamp) = native_source.stamps.get(path) {
                        Some(materialize_workspace_file_cow_status_if_stamp_matches(
                            &native_source.root,
                            &stage,
                            path,
                            entry,
                            *source_stamp,
                            true,
                        )?)
                    } else {
                        None
                    }
                } else {
                    Some(
                        materialize_workspace_file_cow_status_if_matching_with_durability(
                            &self.workspace_root,
                            &stage,
                            path,
                            entry,
                            true,
                        )?,
                    )
                };
                if let Some(status) = clone_status {
                    match status {
                        WorkspaceCowMaterializeStatus::Cloned(stamp) => {
                            stamps.insert(path.clone(), stamp);
                            report.cloned_files += 1;
                            report.cloned_bytes += entry.size_bytes;
                            cloned = true;
                        }
                        WorkspaceCowMaterializeStatus::Skipped
                        | WorkspaceCowMaterializeStatus::Unavailable(_) => {}
                    }
                }
                if !cloned {
                    let one = BTreeMap::from([(path.clone(), entry.clone())]);
                    let materialized = self.materialize_files_at_report(&stage, &empty, &one)?;
                    stamps.extend(materialized.stamps);
                    report.copied_files += 1;
                    report.copied_bytes += entry.size_bytes;
                }
            }

            self.write_clean_workdir_manifest_from_stamps(
                &stage,
                root_id,
                &files,
                files.keys(),
                stamps,
            )?;
            ensure_staged_manifest_is_clean(self, &stage, root_id)?;
            let backend = report.backend();
            operation.set_state(MaterializationOperationState::Verified)?;
            operation.publish()?;
            ensure_staged_manifest_is_clean(self, destination, root_id)?;
            operation.set_state(MaterializationOperationState::Published)?;
            Ok(MaterializationOutcome {
                resolved_mode: LaneWorkdirMode::PortableCopy,
                backend,
                report,
                materialization_operation_id: operation_id,
            })
        })();
        if result.is_err() {
            operation.abort();
        }
        result
    }

    fn resolve_native_materialization_source(
        &self,
        root_id: &ObjectId,
        files: &BTreeMap<String, FileEntry>,
    ) -> Result<Option<NativeSource>> {
        // Strict source selection must not trust a possibly stale daemon/index
        // snapshot. Hash the current workspace entries before choosing it.
        let workspace_stamps = self.workspace_file_stamps_if_entries_match(files)?;
        if let Some(stamps) = workspace_stamps {
            return Ok(Some(NativeSource {
                root: self.workspace_root.clone(),
                stamps,
            }));
        }

        let mut statement = self.conn.prepare(
            "SELECT workdir FROM lane_branches \
             WHERE head_root = ?1 AND workdir IS NOT NULL ORDER BY updated_at DESC",
        )?;
        let candidates = statement
            .query_map(params![root_id.0], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for candidate in candidates {
            let root = PathBuf::from(candidate);
            if !root.is_dir()
                || !matches!(
                    self.cached_workdir_manifest_status(&root, root_id)?,
                    CachedWorkdirManifestStatus::Clean
                )
            {
                continue;
            }
            let mut stamps = BTreeMap::new();
            let mut complete = true;
            for path in files.keys() {
                let source = safe_join(&root, path)?;
                let metadata = match fs::symlink_metadata(&source) {
                    Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                        metadata
                    }
                    Ok(_) => {
                        complete = false;
                        break;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        complete = false;
                        break;
                    }
                    Err(error) => return Err(Error::Io(error)),
                };
                stamps.insert(path.clone(), WorktreeFileStamp::from_metadata(&metadata));
            }
            if complete {
                return Ok(Some(NativeSource { root, stamps }));
            }
        }
        Ok(None)
    }

    fn create_materialization_stage(
        &self,
        destination: &Path,
        root_id: &ObjectId,
    ) -> Result<RegisteredMaterializationStage> {
        // Startup recovery holds this gate exclusively. A materialization
        // holds it shared from journal publication through publish/abort, so
        // a newly opened CLI cannot mistake an active stage for an abandoned
        // one while it holds the workspace write lock for recovery.
        let recovery_guard = MaterializationRecoveryGuard::acquire_shared(&self.db_dir)?;
        let parent = destination
            .parent()
            .ok_or_else(|| Error::InvalidPath {
                path: destination.to_string_lossy().to_string(),
                reason: "lane workdir has no parent".to_string(),
            })?
            .canonicalize()?;
        let leaf = destination
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| Error::InvalidPath {
                path: destination.to_string_lossy().to_string(),
                reason: "lane workdir leaf is not UTF-8".into(),
            })?;
        let (parent_device, parent_inode) = SecureDirectory::open_absolute(&parent)?.identity()?;
        let journal_dir = self.db_dir.join("materialization-operations");
        create_dir_all_durable(&journal_dir)?;
        let journal = SecureDirectory::open_absolute(&journal_dir.canonicalize()?)?;
        for _ in 0..32 {
            let parent_authority = SecureDirectory::open_absolute(&parent)?;
            parent_authority.verify_identity((parent_device, parent_inode))?;
            let operation_id = new_materialization_operation_id()?;
            let stage_leaf = format!(".{leaf}.trail-{operation_id}");
            let stage = parent.join(&stage_leaf);
            let record_path = journal_dir.join(format!("{operation_id}.json"));
            let record = MaterializationOperationRecord {
                version: 2,
                operation_id,
                parent: parent.to_string_lossy().to_string(),
                parent_device,
                parent_inode,
                destination_leaf: leaf.to_string(),
                stage_leaf,
                owned_device: 0,
                owned_inode: 0,
                root_id: root_id.0.clone(),
                state: MaterializationOperationState::Preparing,
                owner_pid: std::process::id(),
                owner_start_token: current_process_start_token(),
            };
            let record_leaf = record_path
                .file_name()
                .and_then(|leaf| leaf.to_str())
                .ok_or_else(|| Error::Corrupt("invalid materialization record leaf".into()))?;
            if journal
                .read_regular_optional_bounded(record_leaf, MAX_MATERIALIZATION_OPERATION_BYTES)?
                .is_some()
            {
                continue;
            }
            journal.write_atomic_regular(record_leaf, &serde_json::to_vec(&record)?)?;
            match parent_authority.create_private_dir(&record.stage_leaf) {
                Ok(stage_authority) => {
                    let mut registered = RegisteredMaterializationStage {
                        record_path,
                        stage_path: stage,
                        parent_authority,
                        stage_authority,
                        _recovery_guard: recovery_guard,
                        record,
                    };
                    let initialized = (|| {
                        registered.stage_authority.sync()?;
                        registered.parent_authority.sync()?;
                        let owned = registered.stage_authority.identity()?;
                        registered.record.owned_device = owned.0;
                        registered.record.owned_inode = owned.1;
                        registered.set_state(MaterializationOperationState::Materializing)
                    })();
                    match initialized {
                        Ok(()) => return Ok(registered),
                        Err(error) => {
                            registered.abort();
                            return Err(error);
                        }
                    }
                }
                Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let _ = journal.remove_leaf(record_leaf);
                }
                Err(error) => {
                    let _ = journal.remove_leaf(record_leaf);
                    return Err(error);
                }
            }
        }
        Err(Error::InvalidInput(
            "could not create a unique workdir materialization stage".to_string(),
        ))
    }

    pub(crate) fn recover_materialization_stages(&self) -> Result<()> {
        let journal_dir = self.db_dir.join("materialization-operations");
        if !journal_dir.is_dir() {
            return Ok(());
        }
        // This recovery path already holds the workspace write lock. Do not
        // wait for a materializer which will later need that write lock to
        // associate its completed stage; defer recovery to the next open.
        let _recovery_guard = MaterializationRecoveryGuard::try_acquire_exclusive(&self.db_dir)?;
        let journal = SecureDirectory::open_absolute(&journal_dir)?;
        for entry in journal.entry_names()? {
            let Some(entry) = entry.to_str() else {
                return Err(Error::Corrupt(
                    "materialization journal has a non-UTF8 entry".into(),
                ));
            };
            if !entry.ends_with(".json") {
                continue;
            }
            let Some(bytes) = journal
                .read_regular_optional_bounded(entry, MAX_MATERIALIZATION_OPERATION_BYTES)?
            else {
                // A prior-version owner may finish or abort between directory
                // enumeration and the descriptor-relative open. It is no
                // longer recovery work; a later open will inspect any record
                // that remains.
                continue;
            };
            let record: MaterializationOperationRecord = serde_json::from_slice(&bytes)?;
            if record.version != 2
                || entry.strip_suffix(".json") != Some(record.operation_id.as_str())
            {
                return Err(Error::Corrupt(format!(
                    "invalid materialization operation record `{entry}`"
                )));
            }
            if process_matches_start_token(record.owner_pid, &record.owner_start_token) {
                continue;
            }
            let associated = self.materialization_record_has_lane_association(&record)?;
            if !associated {
                if self.materialization_record_has_pending_initialization(&record)? {
                    continue;
                }
                remove_owned_materialization_tree(&record, true)?;
            }
            journal.remove_leaf(entry)?;
        }
        Ok(())
    }

    pub(crate) fn complete_materialization_operation(&self, operation_id: &str) -> Result<()> {
        self.complete_materialization_operation_with_missing_policy(operation_id, false)
    }

    pub(crate) fn complete_materialization_operation_for_ownerless_repair(
        &self,
        operation_id: &str,
    ) -> Result<()> {
        self.complete_materialization_operation_with_missing_policy(operation_id, true)
    }

    fn complete_materialization_operation_with_missing_policy(
        &self,
        operation_id: &str,
        missing_is_complete: bool,
    ) -> Result<()> {
        let journal_dir = self.db_dir.join("materialization-operations");
        let journal = SecureDirectory::open_absolute(&journal_dir)?;
        let name = format!("{operation_id}.json");
        let bytes = match journal
            .read_regular_optional_bounded(&name, MAX_MATERIALIZATION_OPERATION_BYTES)?
        {
            Some(bytes) => bytes,
            None if missing_is_complete => return Ok(()),
            None => {
                return Err(Error::Corrupt(
                    "materialization operation record disappeared".into(),
                ));
            }
        };
        let record: MaterializationOperationRecord = serde_json::from_slice(&bytes)?;
        let associated = self.materialization_record_has_lane_association(&record)?;
        if !associated {
            return Err(Error::Corrupt(
                "materialized destination was not atomically associated with a lane row".into(),
            ));
        }
        journal.remove_leaf(&name)
    }

    pub(crate) fn abort_materialization_operation(&self, operation_id: &str) -> Result<()> {
        let journal_dir = self.db_dir.join("materialization-operations");
        let journal = SecureDirectory::open_absolute(&journal_dir)?;
        let name = format!("{operation_id}.json");
        let bytes = journal
            .read_regular_optional_bounded(&name, MAX_MATERIALIZATION_OPERATION_BYTES)?
            .ok_or_else(|| Error::Corrupt("materialization operation record disappeared".into()))?;
        let record: MaterializationOperationRecord = serde_json::from_slice(&bytes)?;
        if record.version != 2 || record.operation_id != operation_id {
            return Err(Error::Corrupt(format!(
                "invalid materialization operation record `{name}`"
            )));
        }
        if self.materialization_record_has_lane_association(&record)? {
            return Err(Error::Corrupt(
                "refusing to abort materialization associated with a lane row".into(),
            ));
        }
        remove_owned_materialization_tree(&record, true)?;
        journal.remove_leaf(&name)
    }

    fn materialization_record_has_lane_association(
        &self,
        record: &MaterializationOperationRecord,
    ) -> Result<bool> {
        let mut statement = self.conn.prepare(
            "SELECT workdir FROM lane_branches
             WHERE workdir IS NOT NULL AND status<>'removed'",
        )?;
        let candidates = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let parent = SecureDirectory::open_absolute(Path::new(&record.parent))?;
        parent.verify_identity((record.parent_device, record.parent_inode))?;
        let expected_destination = Path::new(&record.parent).join(&record.destination_leaf);
        for candidate in candidates {
            let candidate = normalize_workdir_path(&PathBuf::from(candidate))?;
            let candidate = match candidate.canonicalize() {
                Ok(candidate) => candidate,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(Error::Io(error)),
            };
            if candidate != expected_destination {
                continue;
            }
            let Some(leaf) = candidate.file_name().and_then(|leaf| leaf.to_str()) else {
                continue;
            };
            match parent.open_dir(leaf) {
                Ok(directory)
                    if directory.identity()? == (record.owned_device, record.owned_inode) =>
                {
                    return Ok(true);
                }
                Ok(_) => {}
                Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(false)
    }

    fn materialization_record_has_pending_initialization(
        &self,
        record: &MaterializationOperationRecord,
    ) -> Result<bool> {
        let expected_destination = Path::new(&record.parent).join(&record.destination_leaf);
        let durable_initialization: bool = self.conn.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM lane_initializations
                 WHERE operation_id=?1 AND workdir=?2
                   AND phase='materialized')",
            params![
                record.operation_id,
                expected_destination.to_string_lossy().as_ref()
            ],
            |row| row.get(0),
        )?;
        if !durable_initialization {
            return Ok(false);
        }
        {
            let parent = SecureDirectory::open_absolute(Path::new(&record.parent))?;
            parent.verify_identity((record.parent_device, record.parent_inode))?;
            let Some(leaf) = expected_destination
                .file_name()
                .and_then(|leaf| leaf.to_str())
            else {
                return Ok(false);
            };
            match parent.open_dir(leaf) {
                Ok(directory) => {
                    Ok(directory.identity()? == (record.owned_device, record.owned_inode))
                }
                Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(error),
            }
        }
    }
}

fn write_materialization_record(
    record_path: &Path,
    record: &MaterializationOperationRecord,
) -> Result<()> {
    let parent = record_path
        .parent()
        .ok_or_else(|| Error::Corrupt("materialization operation record has no parent".into()))?;
    let leaf = record_path
        .file_name()
        .and_then(|leaf| leaf.to_str())
        .ok_or_else(|| Error::Corrupt("invalid materialization operation record leaf".into()))?;
    SecureDirectory::open_absolute(&parent.canonicalize()?)?
        .write_atomic_regular(leaf, &serde_json::to_vec(record)?)
}

fn remove_materialization_record(record_path: &Path) -> Result<()> {
    let parent = record_path
        .parent()
        .ok_or_else(|| Error::Corrupt("materialization operation record has no parent".into()))?;
    let leaf = record_path
        .file_name()
        .and_then(|leaf| leaf.to_str())
        .ok_or_else(|| Error::Corrupt("invalid materialization operation record leaf".into()))?;
    SecureDirectory::open_absolute(&parent.canonicalize()?)?.remove_leaf(leaf)
}

fn remove_owned_materialization_tree(
    record: &MaterializationOperationRecord,
    include_published_destination: bool,
) -> Result<()> {
    if record.owned_device == 0 || record.owned_inode == 0 {
        let parent = SecureDirectory::open_absolute(Path::new(&record.parent))?;
        parent.verify_identity((record.parent_device, record.parent_inode))?;
        return match parent.open_dir(&record.stage_leaf) {
            Ok(stage) if stage.entry_names()?.is_empty() => {
                let identity = stage.identity()?;
                parent.remove_owned_tree_leaf(&record.stage_leaf, identity)
            }
            Ok(_) => Err(Error::Corrupt(format!(
                "unbound materialization stage `{}` contains data without persisted inode authority",
                record.stage_leaf
            ))),
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        };
    }
    let parent = SecureDirectory::open_absolute(Path::new(&record.parent))?;
    parent.verify_identity((record.parent_device, record.parent_inode))?;
    let leaf = match parent.open_dir(&record.stage_leaf) {
        Ok(stage) => {
            stage.verify_identity((record.owned_device, record.owned_inode))?;
            record.stage_leaf.as_str()
        }
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            if !include_published_destination {
                return Ok(());
            }
            let destination = parent.open_dir(&record.destination_leaf)?;
            destination.verify_identity((record.owned_device, record.owned_inode))?;
            record.destination_leaf.as_str()
        }
        Err(error) => return Err(error),
    };
    parent.remove_owned_tree_leaf(leaf, (record.owned_device, record.owned_inode))
}

fn native_attempt_error(error: NativeAttemptError) -> Error {
    match error {
        NativeAttemptError::Unavailable(MaterializationFallbackReason::CloneUnsupported) => {
            Error::CloneUnsupported
        }
        NativeAttemptError::Unavailable(MaterializationFallbackReason::CrossDevice) => {
            Error::CloneCrossDevice
        }
        NativeAttemptError::Unavailable(MaterializationFallbackReason::NativeSourceUnavailable) => {
            Error::NativeCowSourceUnavailable
        }
        NativeAttemptError::Hard(error) => error,
    }
}

fn fallback_reason_for_clone(reason: NativeCloneUnavailable) -> MaterializationFallbackReason {
    match reason {
        NativeCloneUnavailable::Unsupported => MaterializationFallbackReason::CloneUnsupported,
        NativeCloneUnavailable::CrossDevice => MaterializationFallbackReason::CrossDevice,
    }
}

fn prepare_staged_destination(destination: &Path, custom_workdir: bool) -> Result<()> {
    let parent = destination.parent().ok_or_else(|| Error::InvalidPath {
        path: destination.to_string_lossy().to_string(),
        reason: "lane workdir has no parent".to_string(),
    })?;
    create_dir_all_durable(parent)?;
    match fs::symlink_metadata(destination) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(Error::InvalidPath {
                    path: destination.to_string_lossy().to_string(),
                    reason: "lane workdir must be an absent or empty directory".to_string(),
                });
            }
            if fs::read_dir(destination)?.next().is_some() {
                let qualifier = if custom_workdir { "custom " } else { "" };
                return Err(Error::InvalidInput(format!(
                    "{qualifier}lane workdir `{}` must be empty or absent",
                    destination.display()
                )));
            }
            fs::remove_dir(destination)?;
            sync_directory_strict(parent)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(Error::Io(error)),
    }
    Ok(())
}

#[cfg(unix)]
fn verify_same_native_filesystem(
    source: &Path,
    stage: &Path,
) -> std::result::Result<(), NativeAttemptError> {
    use std::os::unix::fs::MetadataExt;

    if fs::metadata(source)?.dev() != fs::metadata(stage)?.dev() {
        return Err(NativeAttemptError::Unavailable(
            MaterializationFallbackReason::CrossDevice,
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn verify_same_native_filesystem(
    _source: &Path,
    _stage: &Path,
) -> std::result::Result<(), NativeAttemptError> {
    Ok(())
}

fn probe_native_clone(stage: &Path) -> std::result::Result<(), NativeAttemptError> {
    let source = stage.join(".trail-native-cow-probe-source");
    let destination = stage.join(".trail-native-cow-probe-destination");
    fs::write(&source, b"trail-native-cow-probe")?;
    let outcome = clone_file_native(&source, &destination)?;
    let _ = fs::remove_file(&source);
    let _ = fs::remove_file(&destination);
    match outcome {
        NativeCloneOutcome::Cloned => Ok(()),
        NativeCloneOutcome::Unavailable(reason) => Err(NativeAttemptError::Unavailable(
            fallback_reason_for_clone(reason),
        )),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct NativeMaterializationDurabilityInventory {
    source_files: BTreeSet<PathBuf>,
    destination_files: BTreeSet<PathBuf>,
    directories_bottom_up: Vec<PathBuf>,
}

fn native_materialization_durability_inventory<'a, I>(
    source: &Path,
    stage: &Path,
    paths: I,
) -> Result<NativeMaterializationDurabilityInventory>
where
    I: IntoIterator<Item = &'a String>,
{
    let mut source_files = BTreeSet::new();
    let mut destination_files = BTreeSet::new();
    let mut directories = BTreeSet::new();
    for path in paths {
        let normalized = normalize_relative_path(path)?;
        let relative = path_from_rel(&normalized);
        source_files.insert(source.join(&relative));
        destination_files.insert(stage.join(&relative));
        let mut parent = relative.parent();
        while let Some(relative_parent) = parent {
            if relative_parent.as_os_str().is_empty() {
                break;
            }
            directories.insert(stage.join(relative_parent));
            parent = relative_parent.parent();
        }
    }
    let manifest = stage.join(".trail/workdir-manifest.json");
    destination_files.insert(manifest);
    directories.insert(stage.join(".trail"));
    directories.insert(stage.to_path_buf());
    if let Some(parent) = stage.parent() {
        directories.insert(parent.to_path_buf());
    }
    let mut directories_bottom_up = directories.into_iter().collect::<Vec<_>>();
    directories_bottom_up.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    Ok(NativeMaterializationDurabilityInventory {
        source_files,
        destination_files,
        directories_bottom_up,
    })
}

fn sync_native_materialization_stage<'a, I>(source: &Path, stage: &Path, paths: I) -> Result<()>
where
    I: IntoIterator<Item = &'a String>,
{
    let inventory = native_materialization_durability_inventory(source, stage, paths)?;
    let mut files = inventory.source_files.into_iter().collect::<Vec<_>>();
    files.extend(inventory.destination_files);
    // This durability barrier follows the same process-level concurrency
    // policy as native cloning above. Parallel fsync calls from every active
    // lane amplify contention without changing the required durability cut.
    files.iter().try_for_each(|path| -> Result<()> {
        let file = OpenOptions::new().read(true).open(path)?;
        // Rust's File::sync_all maps to F_FULLFSYNC on Apple platforms.
        // Use POSIX fsync for each inode, then one F_FULLFSYNC below.
        rustix::fs::fsync(&file).map_err(|error| Error::Io(error.into()))
    })?;
    for directory in inventory.directories_bottom_up {
        let directory = OpenOptions::new().read(true).open(directory)?;
        rustix::fs::fsync(&directory).map_err(|error| Error::Io(error.into()))?;
    }
    let stage_authority = OpenOptions::new().read(true).open(stage)?;
    #[cfg(target_os = "linux")]
    {
        rustix::fs::syncfs(&stage_authority).map_err(|error| Error::Io(error.into()))?;
    }
    #[cfg(target_os = "macos")]
    {
        rustix::fs::fcntl_fullfsync(&stage_authority).map_err(|error| Error::Io(error.into()))?;
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        stage_authority.sync_all()?;
    }
    Ok(())
}

fn ensure_staged_manifest_is_clean(trail: &Trail, stage: &Path, root_id: &ObjectId) -> Result<()> {
    if !matches!(
        trail.cached_workdir_manifest_status(stage, root_id)?,
        CachedWorkdirManifestStatus::Clean
    ) {
        return Err(Error::Corrupt(format!(
            "staged lane workdir `{}` did not verify against root `{}`",
            stage.display(),
            root_id.0
        )));
    }
    Ok(())
}

fn ensure_staged_manifest_is_clean_read_only(
    trail: &Trail,
    stage: &Path,
    root_id: &ObjectId,
    case_insensitive: bool,
) -> Result<()> {
    if !trail.clean_workdir_manifest_matches_read_only(stage, root_id, case_insensitive)? {
        return Err(Error::Corrupt(format!(
            "staged lane workdir `{}` did not verify read-only against root `{}`",
            stage.display(),
            root_id.0
        )));
    }
    Ok(())
}

#[cfg(test)]
fn publish_materialization_stage(stage: &Path, destination: &Path) -> Result<()> {
    let stage_parent = stage.parent().ok_or_else(|| Error::InvalidPath {
        path: stage.to_string_lossy().to_string(),
        reason: "materialization stage has no parent".into(),
    })?;
    let destination_parent = destination.parent().ok_or_else(|| Error::InvalidPath {
        path: destination.to_string_lossy().to_string(),
        reason: "lane workdir destination has no parent".into(),
    })?;
    let stage_parent = stage_parent.canonicalize()?;
    let destination_parent = destination_parent.canonicalize()?;
    if stage_parent != destination_parent {
        return Err(Error::InvalidInput(
            "materialization stage and destination must share a parent".into(),
        ));
    }
    let stage_leaf = stage
        .file_name()
        .and_then(|leaf| leaf.to_str())
        .ok_or_else(|| Error::InvalidInput("materialization stage leaf is invalid".into()))?;
    let destination_leaf = destination
        .file_name()
        .and_then(|leaf| leaf.to_str())
        .ok_or_else(|| Error::InvalidInput("lane workdir destination leaf is invalid".into()))?;
    let parent = SecureDirectory::open_absolute(&stage_parent)?;
    let stage_identity = parent.open_dir(stage_leaf)?.identity()?;
    parent.rename_leaf_noreplace(stage_leaf, destination_leaf)?;
    parent.sync()?;
    parent
        .open_dir(destination_leaf)?
        .verify_identity(stage_identity)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    struct ReadOnlyVerificationEntry {
        is_dir: bool,
        len: u64,
        modified: Option<std::time::SystemTime>,
        contents: Option<Vec<u8>>,
    }

    fn read_only_verification_snapshot(
        root: &Path,
    ) -> BTreeMap<PathBuf, ReadOnlyVerificationEntry> {
        fn visit(
            root: &Path,
            path: &Path,
            snapshot: &mut BTreeMap<PathBuf, ReadOnlyVerificationEntry>,
        ) {
            let metadata = fs::symlink_metadata(path).unwrap();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            snapshot.insert(
                relative,
                ReadOnlyVerificationEntry {
                    is_dir: metadata.is_dir(),
                    len: metadata.len(),
                    modified: metadata.modified().ok(),
                    contents: metadata.is_file().then(|| fs::read(path).unwrap()),
                },
            );
            if metadata.is_dir() {
                let mut children = fs::read_dir(path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .collect::<Vec<_>>();
                children.sort();
                for child in children {
                    visit(root, &child, snapshot);
                }
            }
        }

        let mut snapshot = BTreeMap::new();
        visit(root, root, &mut snapshot);
        snapshot
    }

    #[test]
    fn batch_durability_inventory_covers_nested_inodes_manifest_and_all_ancestors() {
        let source = Path::new("/source");
        let stage = Path::new("/parent/stage");
        let paths = ["nested/deeper/file.txt".to_string(), "top.txt".to_string()];

        let inventory =
            native_materialization_durability_inventory(source, stage, paths.iter()).unwrap();

        assert_eq!(
            inventory.source_files,
            BTreeSet::from([
                source.join("nested/deeper/file.txt"),
                source.join("top.txt")
            ])
        );
        assert_eq!(
            inventory.destination_files,
            BTreeSet::from([
                stage.join("nested/deeper/file.txt"),
                stage.join("top.txt"),
                stage.join(".trail/workdir-manifest.json")
            ])
        );
        let directories = inventory
            .directories_bottom_up
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            directories,
            BTreeSet::from([
                stage.join("nested/deeper"),
                stage.join("nested"),
                stage.join(".trail"),
                stage.to_path_buf(),
                stage.parent().unwrap().to_path_buf(),
            ])
        );
        assert!(inventory
            .directories_bottom_up
            .windows(2)
            .all(|pair| pair[0].components().count() >= pair[1].components().count()));
    }

    #[test]
    fn post_barrier_verifications_do_not_mutate_stage_or_published_destination() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root contents").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let root_id = db.resolve_branch_ref("main").unwrap().root_id;
        let files = db.load_root_files(&root_id).unwrap();

        let holder = tempfile::tempdir().unwrap();
        let stage = holder.path().join("stage");
        let destination = holder.path().join("destination");
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("README.md"), "root contents").unwrap();

        // This is the only case-sensitivity probe. It deliberately happens
        // before the manifest/barrier cut represented by the first snapshot.
        let case_insensitive = is_case_insensitive_filesystem(&stage).unwrap();
        db.write_clean_workdir_manifest(&stage, &root_id, &files, files.keys())
            .unwrap();

        let before_stage_verification = read_only_verification_snapshot(&stage);
        ensure_staged_manifest_is_clean_read_only(&db, &stage, &root_id, case_insensitive).unwrap();
        assert_eq!(
            read_only_verification_snapshot(&stage),
            before_stage_verification
        );

        fs::rename(&stage, &destination).unwrap();
        let before_destination_verification = read_only_verification_snapshot(&destination);
        ensure_staged_manifest_is_clean_read_only(&db, &destination, &root_id, case_insensitive)
            .unwrap();
        assert_eq!(
            read_only_verification_snapshot(&destination),
            before_destination_verification
        );
    }

    #[test]
    fn publication_never_overwrites_a_concurrent_destination() {
        let temp = tempfile::tempdir().unwrap();
        let stage = temp.path().join("stage");
        let destination = temp.path().join("workdir");
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("new.txt"), "new").unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("rival.txt"), "rival").unwrap();

        assert!(publish_materialization_stage(&stage, &destination).is_err());
        assert_eq!(
            fs::read_to_string(destination.join("rival.txt")).unwrap(),
            "rival"
        );
        assert_eq!(fs::read_to_string(stage.join("new.txt")).unwrap(), "new");
    }

    #[test]
    fn staged_destination_rejects_unowned_nonempty_directory() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("workdir");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("owned-by-user.txt"), "keep").unwrap();

        assert!(prepare_staged_destination(&destination, true).is_err());
        assert_eq!(
            fs::read_to_string(destination.join("owned-by-user.txt")).unwrap(),
            "keep"
        );
    }

    #[test]
    fn publication_rejects_parent_path_substitution() {
        let workspace = tempfile::tempdir().unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let root_id = db.resolve_branch_ref("main").unwrap().root_id;
        let holder = tempfile::tempdir().unwrap();
        let parent = holder.path().join("parent");
        let displaced = holder.path().join("displaced");
        fs::create_dir(&parent).unwrap();
        let destination = parent.join("workdir");
        let mut operation = db
            .create_materialization_stage(&destination, &root_id)
            .unwrap();
        let stage_leaf = operation.record.stage_leaf.clone();

        fs::rename(&parent, &displaced).unwrap();
        fs::create_dir(&parent).unwrap();
        assert!(operation.publish().is_err());
        assert!(!destination.exists());
        assert!(displaced.join(&stage_leaf).is_dir());

        fs::remove_dir(&parent).unwrap();
        fs::rename(&displaced, &parent).unwrap();
        operation.abort();
        assert!(!parent.join(stage_leaf).exists());
    }

    #[test]
    fn startup_recovery_removes_crashed_sparse_publication_before_association() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root contents").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let root_id = db.resolve_branch_ref("main").unwrap().root_id;
        let files = db
            .load_root_files_for_selections(&root_id, &["README.md".to_string()])
            .unwrap();
        let holder = tempfile::tempdir().unwrap();
        let destination = holder.path().join("sparse-workdir");
        let outcome = db
            .materialize_sparse_lane_root_staged(&root_id, &destination, true, &files)
            .unwrap();
        let record_path = db
            .db_dir
            .join("materialization-operations")
            .join(format!("{}.json", outcome.materialization_operation_id));
        let mut record: MaterializationOperationRecord =
            serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
        record.owner_pid = u32::MAX;
        record.owner_start_token = "dead:sparse-spawn".to_string();
        write_materialization_record(&record_path, &record).unwrap();
        assert!(destination.join("README.md").is_file());
        drop(db);

        Trail::open(workspace.path()).unwrap();

        assert!(!destination.exists());
        assert!(!record_path.exists());
    }

    #[test]
    fn startup_recovery_removes_only_registered_incomplete_stage() {
        let workspace = tempfile::tempdir().unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let root_id = db.resolve_branch_ref("main").unwrap().root_id;
        let parent = tempfile::tempdir().unwrap();
        let destination = parent.path().join("workdir");
        let registered = db
            .create_materialization_stage(&destination, &root_id)
            .unwrap();
        let mut registered = registered;
        let stage = registered.path().to_path_buf();
        let record = registered.record_path.clone();
        fs::write(stage.join("partial.txt"), "partial").unwrap();
        let unregistered = parent.path().join(".workdir.trail-unregistered");
        fs::create_dir(&unregistered).unwrap();
        registered.record.owner_pid = u32::MAX;
        registered.record.owner_start_token = "dead:test-owner".to_string();
        registered
            .set_state(MaterializationOperationState::Materializing)
            .unwrap();
        drop(registered);
        drop(db);

        Trail::open(workspace.path()).unwrap();

        assert!(!stage.exists());
        assert!(!record.exists());
        assert!(unregistered.is_dir());
    }

    #[test]
    fn startup_recovery_removes_published_destination_without_lane_association() {
        let workspace = tempfile::tempdir().unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let root_id = db.resolve_branch_ref("main").unwrap().root_id;
        let parent = tempfile::tempdir().unwrap();
        let destination = parent.path().join("workdir");
        let mut registered = db
            .create_materialization_stage(&destination, &root_id)
            .unwrap();
        let stage = registered.path().to_path_buf();
        let record = registered.record_path.clone();
        fs::write(stage.join("complete.txt"), "complete").unwrap();
        fs::rename(&stage, &destination).unwrap();
        registered.record.owner_pid = u32::MAX;
        registered.record.owner_start_token = "dead:test-owner".to_string();
        registered
            .set_state(MaterializationOperationState::Published)
            .unwrap();
        drop(registered);
        drop(db);

        Trail::open(workspace.path()).unwrap();

        assert!(!destination.exists());
        assert!(!record.exists());
    }

    #[test]
    fn startup_recovery_keeps_associated_destination_after_lane_head_advances() {
        let workspace = tempfile::tempdir().unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let head = db.resolve_branch_ref("main").unwrap();
        let parent = tempfile::tempdir().unwrap();
        let destination = parent.path().join("workdir");
        let mut registered = db
            .create_materialization_stage(&destination, &head.root_id)
            .unwrap();
        let stage = registered.path().to_path_buf();
        let record = registered.record_path.clone();
        fs::write(stage.join("complete.txt"), "complete").unwrap();
        fs::rename(&stage, &destination).unwrap();
        registered.record.owner_pid = u32::MAX;
        registered.record.owner_start_token = "dead:test-owner".to_string();
        registered
            .set_state(MaterializationOperationState::Published)
            .unwrap();
        let now = now_ts();
        db.conn
            .execute(
                "INSERT INTO lanes
                 (lane_id,name,kind,provider,model,created_at,metadata_json)
                 VALUES ('lane_recovery','recovery','coding-lane',NULL,NULL,?1,NULL)",
                [now],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO lane_branches
                 (lane_id,ref_name,base_change,head_change,base_root,head_root,session_id,
                  workdir,status,created_at,updated_at)
                 VALUES ('lane_recovery',?1,?2,?2,?3,?3,NULL,?4,'active',?5,?5)",
                params![
                    head.name,
                    head.change_id.0,
                    head.root_id.0,
                    destination.to_string_lossy(),
                    now,
                ],
            )
            .unwrap();
        db.conn
            .execute(
                "UPDATE lane_branches SET head_root='root_advanced_after_materialization'
                 WHERE lane_id='lane_recovery'",
                [],
            )
            .unwrap();
        drop(registered);
        drop(db);

        Trail::open(workspace.path()).unwrap();

        assert_eq!(
            fs::read_to_string(destination.join("complete.txt")).unwrap(),
            "complete"
        );
        assert!(!record.exists());
    }

    #[test]
    fn startup_recovery_ignores_atomic_journal_temporary_files() {
        let workspace = tempfile::tempdir().unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let journal_dir = db.db_dir.join("materialization-operations");
        fs::create_dir_all(&journal_dir).unwrap();
        let temporary = journal_dir.join(".materialize-1.json.trail-tmp-2");
        fs::write(&temporary, b"partial").unwrap();
        drop(db);

        Trail::open(workspace.path()).unwrap();

        assert!(temporary.exists());
    }
}
