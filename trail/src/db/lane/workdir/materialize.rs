use super::*;

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
        let source_stamps =
            match self.workspace_file_stamps_if_clean_index_matches(root_id, &files)? {
                Some(stamps) => Some(stamps),
                None => self.workspace_file_stamps_if_entries_match(&files)?,
            }
            .ok_or(NativeAttemptError::Unavailable(
                MaterializationFallbackReason::NativeSourceUnavailable,
            ))?;

        let stage = create_materialization_stage(destination)?;
        let result = (|| {
            verify_same_native_filesystem(&self.workspace_root, &stage)?;
            probe_native_clone(&stage)?;

            let mut stamps = BTreeMap::new();
            let mut report = MaterializationReport::default();
            for (path, entry) in &files {
                let source_stamp =
                    source_stamps
                        .get(path)
                        .ok_or(NativeAttemptError::Unavailable(
                            MaterializationFallbackReason::NativeSourceUnavailable,
                        ))?;
                match materialize_workspace_file_cow_status_if_stamp_matches(
                    &self.workspace_root,
                    &stage,
                    path,
                    entry,
                    *source_stamp,
                    false,
                )? {
                    WorkspaceCowMaterializeStatus::Cloned(stamp) => {
                        stamps.insert(path.clone(), stamp);
                        report.cloned_files += 1;
                        report.cloned_bytes += entry.size_bytes;
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
            publish_materialization_stage(&stage, destination)?;
            Ok(MaterializationOutcome {
                resolved_mode: LaneWorkdirMode::NativeCow,
                backend: WorkdirBackend::Clone,
                report,
            })
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&stage);
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
        let source_stamps =
            match self.workspace_file_stamps_if_clean_index_matches(root_id, &files)? {
                Some(stamps) => Some(stamps),
                None => self.workspace_file_stamps_if_entries_match(&files)?,
            };
        let stage = create_materialization_stage(destination)?;
        let result = (|| {
            let mut stamps = BTreeMap::new();
            let mut report = MaterializationReport {
                fallback_reason,
                ..MaterializationReport::default()
            };
            let empty = BTreeMap::new();
            for (path, entry) in &files {
                let mut cloned = false;
                if let Some(source_stamp) = source_stamps
                    .as_ref()
                    .and_then(|source_stamps| source_stamps.get(path))
                {
                    match materialize_workspace_file_cow_status_if_stamp_matches(
                        &self.workspace_root,
                        &stage,
                        path,
                        entry,
                        *source_stamp,
                        false,
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
            publish_materialization_stage(&stage, destination)?;
            Ok(MaterializationOutcome {
                resolved_mode: LaneWorkdirMode::PortableCopy,
                backend,
                report,
            })
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&stage);
        }
        result
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

fn create_materialization_stage(destination: &Path) -> Result<PathBuf> {
    let parent = destination.parent().ok_or_else(|| Error::InvalidPath {
        path: destination.to_string_lossy().to_string(),
        reason: "lane workdir has no parent".to_string(),
    })?;
    let leaf = destination
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "workdir".into());
    for _ in 0..32 {
        let stage = parent.join(format!(".{leaf}.trail-materialize-{}", now_nanos()));
        match fs::create_dir(&stage) {
            Ok(()) => return Ok(stage),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(Error::Io(error)),
        }
    }
    Err(Error::InvalidInput(
        "could not create a unique workdir materialization stage".to_string(),
    ))
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
