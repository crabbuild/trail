use super::*;
use rayon::prelude::*;

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
    destination: String,
    stage: String,
    state: MaterializationOperationState,
    owner_pid: u32,
    owner_start_token: String,
}

struct RegisteredMaterializationStage {
    record_path: PathBuf,
    record: MaterializationOperationRecord,
}

impl RegisteredMaterializationStage {
    fn path(&self) -> &Path {
        Path::new(&self.record.stage)
    }

    fn set_state(&mut self, state: MaterializationOperationState) -> Result<()> {
        self.record.state = state;
        write_file_atomic(&self.record_path, &serde_json::to_vec(&self.record)?, true)
    }

    fn finish(self) -> Result<()> {
        match fs::remove_file(&self.record_path) {
            Ok(()) => {
                if let Some(parent) = self.record_path.parent() {
                    sync_directory(parent);
                }
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Error::Io(error)),
        }
    }

    fn abort(self) {
        let _ = fs::remove_dir_all(self.path());
        let _ = fs::remove_file(&self.record_path);
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

    fn materialize_strict_native_attempt(
        &self,
        root_id: &ObjectId,
        destination: &Path,
    ) -> std::result::Result<MaterializationOutcome, NativeAttemptError> {
        let files = self.load_root_files(root_id)?;
        let source = self
            .resolve_native_materialization_source(root_id, &files)?
            .ok_or(NativeAttemptError::Unavailable(
                MaterializationFallbackReason::NativeSourceUnavailable,
            ))?;

        let mut operation = self.create_materialization_stage(destination)?;
        let stage = operation.path().to_path_buf();
        let result = (|| {
            verify_same_native_filesystem(&source.root, &stage)?;
            probe_native_clone(&stage)?;

            let mut stamps = BTreeMap::new();
            let mut report = MaterializationReport::default();
            let results = files
                .par_iter()
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
            ensure_staged_manifest_is_clean(self, &stage, root_id)?;
            operation.set_state(MaterializationOperationState::Verified)?;
            publish_materialization_stage(&stage, destination)?;
            operation.set_state(MaterializationOperationState::Published)?;
            Ok(MaterializationOutcome {
                resolved_mode: LaneWorkdirMode::NativeCow,
                backend: WorkdirBackend::Clone,
                report,
            })
        })();
        if result.is_ok() {
            operation.finish()?;
        } else {
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
        let mut operation = self.create_materialization_stage(destination)?;
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
                            false,
                        )?)
                    } else {
                        None
                    }
                } else {
                    Some(materialize_workspace_file_cow_status_if_matching(
                        &self.workspace_root,
                        &stage,
                        path,
                        entry,
                    )?)
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
            publish_materialization_stage(&stage, destination)?;
            operation.set_state(MaterializationOperationState::Published)?;
            Ok(MaterializationOutcome {
                resolved_mode: LaneWorkdirMode::PortableCopy,
                backend,
                report,
            })
        })();
        if result.is_ok() {
            operation.finish()?;
        } else {
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
    ) -> Result<RegisteredMaterializationStage> {
        let parent = destination.parent().ok_or_else(|| Error::InvalidPath {
            path: destination.to_string_lossy().to_string(),
            reason: "lane workdir has no parent".to_string(),
        })?;
        let leaf = destination
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_else(|| "workdir".into());
        let journal_dir = self.db_dir.join("materialization-operations");
        fs::create_dir_all(&journal_dir)?;
        for _ in 0..32 {
            let operation_id = format!("materialize-{}", now_nanos());
            let stage = parent.join(format!(".{leaf}.trail-{operation_id}"));
            let record_path = journal_dir.join(format!("{operation_id}.json"));
            let record = MaterializationOperationRecord {
                version: 1,
                operation_id,
                destination: destination.to_string_lossy().to_string(),
                stage: stage.to_string_lossy().to_string(),
                state: MaterializationOperationState::Preparing,
                owner_pid: std::process::id(),
                owner_start_token: current_process_start_token(),
            };
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&record_path)
            {
                Ok(mut file) => {
                    file.write_all(&serde_json::to_vec(&record)?)?;
                    file.sync_all()?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(Error::Io(error)),
            }
            match fs::create_dir(&stage) {
                Ok(()) => {
                    let mut registered = RegisteredMaterializationStage {
                        record_path,
                        record,
                    };
                    registered.set_state(MaterializationOperationState::Materializing)?;
                    return Ok(registered);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let _ = fs::remove_file(&record_path);
                }
                Err(error) => {
                    let _ = fs::remove_file(&record_path);
                    return Err(Error::Io(error));
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
        for entry in fs::read_dir(&journal_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let record_path = entry.path();
            if record_path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let record: MaterializationOperationRecord =
                serde_json::from_slice(&fs::read(&record_path)?)?;
            if record.version != 1
                || record_path.file_stem().and_then(|value| value.to_str())
                    != Some(record.operation_id.as_str())
            {
                return Err(Error::Corrupt(format!(
                    "invalid materialization operation record `{}`",
                    record_path.display()
                )));
            }
            if process_matches_start_token(record.owner_pid, &record.owner_start_token) {
                continue;
            }
            let destination = PathBuf::from(&record.destination);
            let stage = PathBuf::from(&record.stage);
            let owned = stage.parent() == destination.parent()
                && stage
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| name.contains(&record.operation_id));
            if !owned {
                return Err(Error::Corrupt(format!(
                    "materialization operation `{}` does not own stage `{}`",
                    record.operation_id,
                    stage.display()
                )));
            }
            if record.state != MaterializationOperationState::Published {
                match fs::remove_dir_all(&stage) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(Error::Io(error)),
                }
            }
            fs::remove_file(&record_path)?;
        }
        sync_directory(&journal_dir);
        Ok(())
    }
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
    fs::create_dir_all(parent)?;
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn publish_materialization_stage(stage: &Path, destination: &Path) -> Result<()> {
    use rustix::fs::{renameat_with, RenameFlags, CWD};

    renameat_with(CWD, stage, CWD, destination, RenameFlags::NOREPLACE)
        .map_err(|error| Error::Io(error.into()))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn publish_materialization_stage(stage: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        return Err(Error::InvalidInput(format!(
            "lane workdir destination `{}` was created concurrently",
            destination.display()
        )));
    }
    fs::rename(stage, destination)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn startup_recovery_removes_only_registered_incomplete_stage() {
        let workspace = tempfile::tempdir().unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let parent = tempfile::tempdir().unwrap();
        let destination = parent.path().join("workdir");
        let registered = db.create_materialization_stage(&destination).unwrap();
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
    fn startup_recovery_keeps_a_published_destination() {
        let workspace = tempfile::tempdir().unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let parent = tempfile::tempdir().unwrap();
        let destination = parent.path().join("workdir");
        let mut registered = db.create_materialization_stage(&destination).unwrap();
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
