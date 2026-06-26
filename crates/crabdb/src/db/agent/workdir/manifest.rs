use super::*;

const CLEAN_WORKDIR_MANIFEST_VERSION: u16 = 1;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct WorkdirFileStamp {
    size_bytes: u64,
    modified_ns: i64,
    changed_ns: i64,
    #[serde(default)]
    device_id: i64,
    #[serde(default)]
    inode: i64,
    executable: bool,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct CleanWorkdirManifest {
    version: u16,
    root_id: String,
    files: BTreeMap<String, CleanWorkdirManifestEntry>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct CleanWorkdirManifestEntry {
    stamp: WorkdirFileStamp,
    kind: FileKind,
    content_hash: String,
}

impl CrabDb {
    pub(crate) fn cached_workdir_manifest_status(
        &self,
        dir: &Path,
        root_id: &ObjectId,
    ) -> Result<CachedWorkdirManifestStatus> {
        let Some(manifest) = self.read_clean_workdir_manifest(dir)? else {
            return Ok(CachedWorkdirManifestStatus::Missing);
        };
        if manifest.version != CLEAN_WORKDIR_MANIFEST_VERSION {
            remove_clean_workdir_manifest(dir)?;
            return Ok(CachedWorkdirManifestStatus::Missing);
        }

        let stamps = self.scan_workdir_file_stamps(dir)?;
        let root_matches = manifest.root_id == root_id.0;
        let clean = root_matches
            && stamps.len() == manifest.files.len()
            && stamps.iter().all(|(path, stamp)| {
                manifest
                    .files
                    .get(path)
                    .is_some_and(|cached| cached.stamp == *stamp)
            });
        if clean {
            return Ok(CachedWorkdirManifestStatus::Clean);
        }

        let mut candidate_paths = BTreeSet::new();
        if root_matches {
            for path in manifest.files.keys() {
                if !stamps.contains_key(path) {
                    candidate_paths.insert(path.clone());
                }
            }
        }

        let mut disk_manifest = BTreeMap::new();
        for (path, stamp) in stamps {
            if let Some(cached) = manifest
                .files
                .get(&path)
                .filter(|cached| cached.stamp == stamp)
            {
                disk_manifest.insert(
                    path,
                    DiskManifest {
                        kind: cached.kind.clone(),
                        executable: stamp.executable,
                        content_hash: cached.content_hash.clone(),
                    },
                );
                continue;
            }

            if root_matches {
                candidate_paths.insert(path.clone());
            }
            let abs = safe_join(dir, &path)?;
            let bytes = fs::read(abs)?;
            disk_manifest.insert(
                path,
                DiskManifest {
                    kind: classify_file_kind(&bytes, &self.config.text),
                    executable: stamp.executable,
                    content_hash: sha256_hex(&bytes),
                },
            );
        }
        Ok(CachedWorkdirManifestStatus::Dirty {
            disk_manifest,
            candidate_paths: root_matches.then(|| candidate_paths.into_iter().collect()),
        })
    }

    pub(crate) fn write_clean_workdir_manifest<'a, I>(
        &self,
        dir: &Path,
        root_id: &ObjectId,
        files: &BTreeMap<String, FileEntry>,
        expected_paths: I,
    ) -> Result<()>
    where
        I: IntoIterator<Item = &'a String>,
    {
        let expected = expected_paths
            .into_iter()
            .map(|path| normalize_relative_path(path))
            .collect::<Result<BTreeSet<_>>>()?;
        let stamps = self.scan_workdir_file_stamps(dir)?;
        let stamped = stamps.keys().cloned().collect::<BTreeSet<_>>();
        if stamped != expected {
            remove_clean_workdir_manifest(dir)?;
            return Ok(());
        }

        let mut entries = BTreeMap::new();
        for path in expected {
            let Some(stamp) = stamps.get(&path).cloned() else {
                remove_clean_workdir_manifest(dir)?;
                return Ok(());
            };
            let Some(file) = files.get(&path) else {
                remove_clean_workdir_manifest(dir)?;
                return Ok(());
            };
            entries.insert(
                path,
                CleanWorkdirManifestEntry {
                    stamp,
                    kind: file.kind.clone(),
                    content_hash: file.content_hash.clone(),
                },
            );
        }

        let path = clean_workdir_manifest_path(dir);
        let parent = path.parent().ok_or_else(|| Error::InvalidPath {
            path: path.to_string_lossy().to_string(),
            reason: "clean workdir manifest has no parent".to_string(),
        })?;
        fs::create_dir_all(parent)?;
        let manifest = CleanWorkdirManifest {
            version: CLEAN_WORKDIR_MANIFEST_VERSION,
            root_id: root_id.0.clone(),
            files: entries,
        };
        fs::write(path, serde_json::to_vec(&manifest)?)?;
        Ok(())
    }

    pub(crate) fn write_clean_workdir_manifest_from_disk_manifest<'a, I>(
        &self,
        dir: &Path,
        root_id: &ObjectId,
        disk_manifest: &BTreeMap<String, DiskManifest>,
        expected_paths: I,
    ) -> Result<()>
    where
        I: IntoIterator<Item = &'a String>,
    {
        let expected = expected_paths
            .into_iter()
            .map(|path| normalize_relative_path(path))
            .collect::<Result<BTreeSet<_>>>()?;
        let stamps = self.scan_workdir_file_stamps(dir)?;
        let stamped = stamps.keys().cloned().collect::<BTreeSet<_>>();
        if stamped != expected {
            remove_clean_workdir_manifest(dir)?;
            return Ok(());
        }

        let mut entries = BTreeMap::new();
        for path in expected {
            let Some(stamp) = stamps.get(&path).cloned() else {
                remove_clean_workdir_manifest(dir)?;
                return Ok(());
            };
            let Some(file) = disk_manifest.get(&path) else {
                remove_clean_workdir_manifest(dir)?;
                return Ok(());
            };
            entries.insert(
                path,
                CleanWorkdirManifestEntry {
                    stamp,
                    kind: file.kind.clone(),
                    content_hash: file.content_hash.clone(),
                },
            );
        }

        self.write_clean_workdir_manifest_entries(dir, root_id, entries)
    }

    pub(crate) fn update_clean_workdir_manifest_from_file_subset(
        &self,
        dir: &Path,
        previous_root_id: &ObjectId,
        next_root_id: &ObjectId,
        previous: &BTreeMap<String, FileEntry>,
        target: &BTreeMap<String, FileEntry>,
    ) -> Result<bool> {
        let Some(mut manifest) = self.read_clean_workdir_manifest(dir)? else {
            return Ok(false);
        };
        if manifest.version != CLEAN_WORKDIR_MANIFEST_VERSION {
            remove_clean_workdir_manifest(dir)?;
            return Ok(false);
        }
        if manifest.root_id != previous_root_id.0 {
            return Ok(false);
        }

        for path in previous.keys() {
            if !target.contains_key(path) {
                manifest.files.remove(path);
            }
        }
        for (path, entry) in target {
            let abs = safe_join(dir, path)?;
            let metadata = match fs::symlink_metadata(&abs) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    remove_clean_workdir_manifest(dir)?;
                    return Ok(false);
                }
                Err(err) => return Err(Error::Io(err)),
            };
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                remove_clean_workdir_manifest(dir)?;
                return Ok(false);
            }
            manifest.files.insert(
                path.clone(),
                CleanWorkdirManifestEntry {
                    stamp: WorkdirFileStamp {
                        size_bytes: metadata.len(),
                        modified_ns: metadata_modified_ns(&metadata),
                        changed_ns: metadata_changed_ns(&metadata),
                        device_id: metadata_device_id(&metadata),
                        inode: metadata_inode(&metadata),
                        executable: executable_from_metadata(&metadata),
                    },
                    kind: entry.kind.clone(),
                    content_hash: entry.content_hash.clone(),
                },
            );
        }

        self.write_clean_workdir_manifest_entries(dir, next_root_id, manifest.files)?;
        Ok(true)
    }

    pub(crate) fn clean_workdir_manifest_allows_file_subset_update(
        &self,
        dir: &Path,
        previous_root_id: &ObjectId,
        previous: &BTreeMap<String, FileEntry>,
        target: &BTreeMap<String, FileEntry>,
    ) -> Result<bool> {
        let Some(manifest) = self.read_clean_workdir_manifest(dir)? else {
            return Ok(false);
        };
        if manifest.version != CLEAN_WORKDIR_MANIFEST_VERSION {
            remove_clean_workdir_manifest(dir)?;
            return Ok(false);
        }
        if manifest.root_id != previous_root_id.0 {
            return Ok(false);
        }

        let mut paths = manifest.files.keys().cloned().collect::<BTreeSet<_>>();
        for path in previous.keys() {
            if !target.contains_key(path) {
                paths.remove(path);
            }
        }
        paths.extend(target.keys().cloned());
        if is_case_insensitive_filesystem(dir)? {
            validate_no_case_fold_collisions(paths.iter())?;
        }
        Ok(true)
    }

    fn write_clean_workdir_manifest_entries(
        &self,
        dir: &Path,
        root_id: &ObjectId,
        entries: BTreeMap<String, CleanWorkdirManifestEntry>,
    ) -> Result<()> {
        let path = clean_workdir_manifest_path(dir);
        let parent = path.parent().ok_or_else(|| Error::InvalidPath {
            path: path.to_string_lossy().to_string(),
            reason: "clean workdir manifest has no parent".to_string(),
        })?;
        fs::create_dir_all(parent)?;
        let manifest = CleanWorkdirManifest {
            version: CLEAN_WORKDIR_MANIFEST_VERSION,
            root_id: root_id.0.clone(),
            files: entries,
        };
        fs::write(path, serde_json::to_vec(&manifest)?)?;
        Ok(())
    }

    fn read_clean_workdir_manifest(&self, dir: &Path) -> Result<Option<CleanWorkdirManifest>> {
        let path = clean_workdir_manifest_path(dir);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(Error::Io(err)),
        };
        match serde_json::from_slice::<CleanWorkdirManifest>(&bytes) {
            Ok(manifest) => Ok(Some(manifest)),
            Err(_) => {
                remove_clean_workdir_manifest(dir)?;
                Ok(None)
            }
        }
    }

    fn scan_workdir_file_stamps(&self, root: &Path) -> Result<BTreeMap<String, WorkdirFileStamp>> {
        let root = root.canonicalize()?;
        let mut builder = WalkBuilder::new(&root);
        builder
            .hidden(false)
            .git_ignore(self.config.recording.ignore_gitignored)
            .git_exclude(self.config.recording.ignore_gitignored)
            .git_global(self.config.recording.ignore_gitignored)
            .add_custom_ignore_filename(".crabignore");
        let walker = builder.build();
        let mut files = BTreeMap::new();
        for item in walker {
            let entry = item.map_err(|err| Error::InvalidInput(err.to_string()))?;
            let path = entry.path();
            if path == root {
                continue;
            }
            let rel = path
                .strip_prefix(&root)
                .map_err(|err| Error::InvalidInput(err.to_string()))?;
            let rel = normalize_relative_path(&rel.to_string_lossy())?;
            if entry.file_type().is_some_and(|kind| kind.is_dir()) && is_default_ignored(&rel) {
                continue;
            }
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            if is_default_ignored(&rel) {
                continue;
            }
            let metadata = fs::symlink_metadata(path)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                continue;
            }
            files.insert(
                rel,
                WorkdirFileStamp {
                    size_bytes: metadata.len(),
                    modified_ns: metadata_modified_ns(&metadata),
                    changed_ns: metadata_changed_ns(&metadata),
                    device_id: metadata_device_id(&metadata),
                    inode: metadata_inode(&metadata),
                    executable: executable_from_metadata(&metadata),
                },
            );
        }
        Ok(files)
    }
}

fn clean_workdir_manifest_path(dir: &Path) -> PathBuf {
    dir.join(".crabdb").join("workdir-manifest.json")
}

fn remove_clean_workdir_manifest(dir: &Path) -> Result<()> {
    let path = clean_workdir_manifest_path(dir);
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
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

fn duration_ns(duration: Duration) -> i64 {
    let ns = (duration.as_secs() as u128)
        .saturating_mul(1_000_000_000)
        .saturating_add(duration.subsec_nanos() as u128);
    ns.min(i64::MAX as u128) as i64
}
