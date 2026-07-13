use super::*;

const CLEAN_WORKDIR_MANIFEST_VERSION: u16 = 1;

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

impl Trail {
    pub(crate) fn cached_workdir_manifest_status(
        &self,
        dir: &Path,
        root_id: &ObjectId,
    ) -> Result<CachedWorkdirManifestStatus> {
        self.cached_workdir_manifest_status_from_path(
            dir,
            &clean_workdir_manifest_path(dir),
            root_id,
        )
    }

    pub(crate) fn cached_workdir_manifest_status_from_path(
        &self,
        dir: &Path,
        manifest_path: &Path,
        root_id: &ObjectId,
    ) -> Result<CachedWorkdirManifestStatus> {
        let Some(manifest) = self.read_clean_workdir_manifest_from_path(manifest_path)? else {
            return Ok(CachedWorkdirManifestStatus::Missing);
        };
        if manifest.version != CLEAN_WORKDIR_MANIFEST_VERSION {
            remove_clean_workdir_manifest_path(manifest_path)?;
            return Ok(CachedWorkdirManifestStatus::Missing);
        }

        let manifest_paths = manifest.files.keys().cloned().collect::<Vec<_>>();
        let stamps = self.scan_workdir_file_stamps_with_pinned_paths(dir, &manifest_paths)?;
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
        let pinned_paths = expected.iter().cloned().collect::<Vec<_>>();
        let stamps = self.scan_workdir_file_stamps_with_pinned_paths(dir, &pinned_paths)?;
        self.write_clean_workdir_manifest_from_stamps_for_paths(
            dir, root_id, files, expected, stamps,
        )
    }

    pub(crate) fn write_clean_workdir_manifest_to_path<'a, I>(
        &self,
        dir: &Path,
        manifest_path: &Path,
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
        let pinned_paths = expected.iter().cloned().collect::<Vec<_>>();
        let stamps = self.scan_workdir_file_stamps_with_pinned_paths(dir, &pinned_paths)?;
        let stamped = stamps.keys().cloned().collect::<BTreeSet<_>>();
        if stamped != expected {
            remove_clean_workdir_manifest_path(manifest_path)?;
            return Ok(());
        }

        let mut entries = BTreeMap::new();
        for path in expected {
            let Some(stamp) = stamps.get(&path).cloned() else {
                remove_clean_workdir_manifest_path(manifest_path)?;
                return Ok(());
            };
            let Some(file) = files.get(&path) else {
                remove_clean_workdir_manifest_path(manifest_path)?;
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

        self.write_clean_workdir_manifest_entries_to_path(manifest_path, root_id, entries)
    }

    pub(crate) fn write_clean_workdir_manifest_from_stamps<'a, I>(
        &self,
        dir: &Path,
        root_id: &ObjectId,
        files: &BTreeMap<String, FileEntry>,
        expected_paths: I,
        stamps: BTreeMap<String, WorkdirFileStamp>,
    ) -> Result<()>
    where
        I: IntoIterator<Item = &'a String>,
    {
        let expected = expected_paths
            .into_iter()
            .map(|path| normalize_relative_path(path))
            .collect::<Result<BTreeSet<_>>>()?;
        self.write_clean_workdir_manifest_from_stamps_for_paths(
            dir, root_id, files, expected, stamps,
        )
    }

    fn write_clean_workdir_manifest_from_stamps_for_paths(
        &self,
        dir: &Path,
        root_id: &ObjectId,
        files: &BTreeMap<String, FileEntry>,
        expected: BTreeSet<String>,
        stamps: BTreeMap<String, WorkdirFileStamp>,
    ) -> Result<()> {
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

        self.write_clean_workdir_manifest_entries(dir, root_id, entries)
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
        let pinned_paths = expected.iter().cloned().collect::<Vec<_>>();
        let stamps = self.scan_workdir_file_stamps_with_pinned_paths(dir, &pinned_paths)?;
        self.write_clean_workdir_manifest_from_disk_manifest_stamps_for_paths(
            dir,
            root_id,
            disk_manifest,
            expected,
            stamps,
        )
    }

    pub(crate) fn write_clean_workdir_manifest_from_disk_manifest_to_path<'a, I>(
        &self,
        dir: &Path,
        manifest_path: &Path,
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
        let pinned_paths = expected.iter().cloned().collect::<Vec<_>>();
        let stamps = self.scan_workdir_file_stamps_with_pinned_paths(dir, &pinned_paths)?;
        let stamped = stamps.keys().cloned().collect::<BTreeSet<_>>();
        if stamped != expected {
            remove_clean_workdir_manifest_path(manifest_path)?;
            return Ok(());
        }

        let mut entries = BTreeMap::new();
        for path in expected {
            let Some(stamp) = stamps.get(&path).cloned() else {
                remove_clean_workdir_manifest_path(manifest_path)?;
                return Ok(());
            };
            let Some(file) = disk_manifest.get(&path) else {
                remove_clean_workdir_manifest_path(manifest_path)?;
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

        self.write_clean_workdir_manifest_entries_to_path(manifest_path, root_id, entries)
    }

    fn write_clean_workdir_manifest_from_disk_manifest_stamps_for_paths(
        &self,
        dir: &Path,
        root_id: &ObjectId,
        disk_manifest: &BTreeMap<String, DiskManifest>,
        expected: BTreeSet<String>,
        stamps: BTreeMap<String, WorkdirFileStamp>,
    ) -> Result<()> {
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
                    stamp: WorkdirFileStamp::from_metadata(&metadata),
                    kind: entry.kind.clone(),
                    content_hash: entry.content_hash.clone(),
                },
            );
        }

        self.write_clean_workdir_manifest_entries(dir, next_root_id, manifest.files)?;
        Ok(true)
    }

    pub(crate) fn clean_workdir_manifest_allows_touched_path_update(
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

        let case_insensitive = is_case_insensitive_filesystem(dir)?;
        let candidate_paths = previous
            .keys()
            .chain(target.keys())
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let observed =
            observed_exact_paths_for_candidates(dir, &candidate_paths, case_insensitive)?;
        let observed_by_folded = index_observed_paths_by_folded(&observed);

        for (path, entry) in previous {
            let Some(cached) = manifest.files.get(path) else {
                return Ok(false);
            };
            if cached.kind != entry.kind || cached.content_hash != entry.content_hash {
                return Ok(false);
            }
            if observed.get(path) != Some(&ObservedPathKind::RegularFile) {
                return Ok(false);
            }
            let metadata = match fs::symlink_metadata(safe_join(dir, path)?) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(err) => return Err(Error::Io(err)),
            };
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || cached.stamp != WorkdirFileStamp::from_metadata(&metadata)
            {
                return Ok(false);
            }
        }

        let removed_paths = previous
            .keys()
            .filter(|path| !target.contains_key(*path))
            .cloned()
            .collect::<BTreeSet<_>>();
        for path in target.keys().filter(|path| !previous.contains_key(*path)) {
            let folded = case_insensitive_path_key(path);
            let observed_aliases = observed_by_folded.get(&folded);
            let aliases_are_removed_previous = observed_aliases.is_none_or(|aliases| {
                aliases
                    .iter()
                    .all(|observed_path| removed_paths.contains(*observed_path))
            });
            if observed_aliases.is_some_and(|aliases| !aliases.is_empty())
                && !aliases_are_removed_previous
            {
                return Ok(false);
            }
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
        self.write_clean_workdir_manifest_entries_to_path(&path, root_id, entries)
    }

    fn write_clean_workdir_manifest_entries_to_path(
        &self,
        path: &Path,
        root_id: &ObjectId,
        entries: BTreeMap<String, CleanWorkdirManifestEntry>,
    ) -> Result<()> {
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
        write_file_atomic(&path, &serde_json::to_vec(&manifest)?, false)?;
        Ok(())
    }

    pub(crate) fn clean_workdir_manifest_tracks_file_subset(
        &self,
        dir: &Path,
        root_id: &ObjectId,
        target: &BTreeMap<String, FileEntry>,
    ) -> Result<bool> {
        let Some(manifest) = self.read_clean_workdir_manifest(dir)? else {
            return Ok(false);
        };
        if manifest.version != CLEAN_WORKDIR_MANIFEST_VERSION {
            remove_clean_workdir_manifest(dir)?;
            return Ok(false);
        }
        if manifest.root_id != root_id.0 {
            return Ok(false);
        }

        for (path, entry) in target {
            let Some(cached) = manifest.files.get(path) else {
                return Ok(false);
            };
            if cached.kind != entry.kind || cached.content_hash != entry.content_hash {
                return Ok(false);
            }
            let abs = safe_join(dir, path)?;
            let metadata = match fs::symlink_metadata(&abs) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(err) => return Err(Error::Io(err)),
            };
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Ok(false);
            }
            if cached.stamp != WorkdirFileStamp::from_metadata(&metadata) {
                return Ok(false);
            }
        }

        Ok(true)
    }

    fn read_clean_workdir_manifest(&self, dir: &Path) -> Result<Option<CleanWorkdirManifest>> {
        self.read_clean_workdir_manifest_from_path(&clean_workdir_manifest_path(dir))
    }

    fn read_clean_workdir_manifest_from_path(
        &self,
        path: &Path,
    ) -> Result<Option<CleanWorkdirManifest>> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(Error::Io(err)),
        };
        match serde_json::from_slice::<CleanWorkdirManifest>(&bytes) {
            Ok(manifest) => Ok(Some(manifest)),
            Err(_) => {
                remove_clean_workdir_manifest_path(path)?;
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
            .add_custom_ignore_filename(".trailignore");
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
            files.insert(rel, WorkdirFileStamp::from_metadata(&metadata));
        }
        Ok(files)
    }

    fn scan_workdir_file_stamps_with_pinned_paths(
        &self,
        root: &Path,
        pinned_paths: &[String],
    ) -> Result<BTreeMap<String, WorkdirFileStamp>> {
        let root = root.canonicalize()?;
        let case_insensitive = is_case_insensitive_filesystem(&root)?;
        self.scan_workdir_file_stamps_with_pinned_paths_case_sensitivity(
            &root,
            pinned_paths,
            case_insensitive,
        )
    }

    fn scan_workdir_file_stamps_with_pinned_paths_case_sensitivity(
        &self,
        root: &Path,
        pinned_paths: &[String],
        case_insensitive: bool,
    ) -> Result<BTreeMap<String, WorkdirFileStamp>> {
        let root = root.canonicalize()?;
        let mut files = self.scan_workdir_file_stamps(&root)?;
        let mut exact_paths = files.keys().cloned().collect::<BTreeSet<_>>();
        let observed = observed_exact_paths_for_candidates(&root, pinned_paths, case_insensitive)?;
        let actual_paths = observed.keys().cloned().collect::<BTreeSet<_>>();
        let folded_paths = actual_paths
            .iter()
            .map(|path| case_insensitive_path_key(path))
            .collect::<BTreeSet<_>>();
        for (path, kind) in &observed {
            if *kind != ObservedPathKind::RegularFile {
                continue;
            }
            let Some(stamp) = open_observed_exact_regular_file_stamp(&root, path)? else {
                continue;
            };
            exact_paths.insert(path.clone());
            files.insert(path.clone(), stamp);
        }
        for path in pinned_paths {
            let path = normalize_relative_path(path)?;
            if !pinned_path_needs_probe(
                case_insensitive,
                &exact_paths,
                &actual_paths,
                &folded_paths,
                &path,
            ) {
                continue;
            }
            let abs = safe_join(&root, &path)?;
            let metadata = match fs::symlink_metadata(&abs) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    files.remove(&path);
                    exact_paths.remove(&path);
                    continue;
                }
                Err(err) => return Err(Error::Io(err)),
            };
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                files.remove(&path);
                exact_paths.remove(&path);
                continue;
            }
            exact_paths.insert(path.clone());
            files.insert(path, WorkdirFileStamp::from_metadata(&metadata));
        }
        Ok(files)
    }
}

fn open_observed_exact_regular_file_stamp(
    root: &Path,
    path: &str,
) -> Result<Option<WorkdirFileStamp>> {
    let abs = safe_join(root, path)?;
    #[cfg(not(unix))]
    {
        let metadata = match fs::symlink_metadata(&abs) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(Error::Io(err)),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Ok(None);
        }
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = match options.open(&abs) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(Error::Io(err)),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Ok(None);
    }
    #[cfg(not(unix))]
    {
        let final_metadata = fs::symlink_metadata(&abs)?;
        if final_metadata.file_type().is_symlink() || !final_metadata.is_file() {
            return Ok(None);
        }
    }
    Ok(Some(WorkdirFileStamp::from_metadata(&metadata)))
}

fn index_observed_paths_by_folded(
    observed: &BTreeMap<String, ObservedPathKind>,
) -> BTreeMap<String, Vec<&str>> {
    let mut indexed = BTreeMap::<String, Vec<&str>>::new();
    for path in observed.keys() {
        indexed
            .entry(case_insensitive_path_key(path))
            .or_default()
            .push(path);
    }
    indexed
}

fn pinned_path_needs_probe(
    case_insensitive: bool,
    exact_paths: &BTreeSet<String>,
    actual_paths: &BTreeSet<String>,
    folded_paths: &BTreeSet<String>,
    path: &str,
) -> bool {
    !case_insensitive
        || exact_paths.contains(path)
        || actual_paths.contains(path)
        || !folded_paths.contains(&case_insensitive_path_key(path))
}

fn clean_workdir_manifest_path(dir: &Path) -> PathBuf {
    dir.join(".trail").join("workdir-manifest.json")
}

fn remove_clean_workdir_manifest(dir: &Path) -> Result<()> {
    remove_clean_workdir_manifest_path(&clean_workdir_manifest_path(dir))
}

fn remove_clean_workdir_manifest_path(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(Error::Io(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workdir_manifest_from_materialization_stamps_fixture() -> (
        tempfile::TempDir,
        tempfile::TempDir,
        Trail,
        RefRecord,
        BTreeMap<String, FileEntry>,
    ) {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("a.txt"), "a1\n").unwrap();
        fs::create_dir(workspace.path().join("src")).unwrap();
        fs::write(workspace.path().join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let head = db.resolve_branch_ref("main").unwrap();
        let files = db.load_root_files(&head.root_id).unwrap();

        let workdir = tempfile::tempdir().unwrap();
        fs::write(workdir.path().join("a.txt"), "a1\n").unwrap();
        fs::create_dir(workdir.path().join("src")).unwrap();
        fs::write(workdir.path().join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
        (workspace, workdir, db, head, files)
    }

    #[test]
    fn workdir_manifest_from_materialization_stamps_matches_scan_manifest() {
        let (_workspace, workdir, db, head, files) =
            workdir_manifest_from_materialization_stamps_fixture();
        let stamps = db.scan_workdir_file_stamps(workdir.path()).unwrap();

        db.write_clean_workdir_manifest(workdir.path(), &head.root_id, &files, files.keys())
            .unwrap();
        let scan_manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(clean_workdir_manifest_path(workdir.path())).unwrap())
                .unwrap();
        remove_clean_workdir_manifest(workdir.path()).unwrap();

        db.write_clean_workdir_manifest_from_stamps(
            workdir.path(),
            &head.root_id,
            &files,
            files.keys(),
            stamps,
        )
        .unwrap();
        let stamp_manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(clean_workdir_manifest_path(workdir.path())).unwrap())
                .unwrap();

        assert_eq!(stamp_manifest, scan_manifest);
    }

    #[test]
    fn workdir_manifest_from_materialization_stamps_detects_dirty_file() {
        let (_workspace, workdir, db, head, files) =
            workdir_manifest_from_materialization_stamps_fixture();
        let stamps = db.scan_workdir_file_stamps(workdir.path()).unwrap();

        db.write_clean_workdir_manifest_from_stamps(
            workdir.path(),
            &head.root_id,
            &files,
            files.keys(),
            stamps,
        )
        .unwrap();
        fs::write(workdir.path().join("a.txt"), "a1\ndirty\n").unwrap();

        match db
            .cached_workdir_manifest_status(workdir.path(), &head.root_id)
            .unwrap()
        {
            CachedWorkdirManifestStatus::Dirty {
                disk_manifest,
                candidate_paths: Some(candidate_paths),
            } => {
                assert_ne!(
                    disk_manifest["a.txt"].content_hash,
                    files["a.txt"].content_hash
                );
                assert!(candidate_paths.contains(&"a.txt".to_string()));
            }
            _ => panic!("expected dirty manifest status"),
        }
    }

    #[test]
    fn workdir_manifest_subset_tracking_detects_stale_target() {
        let (_workspace, workdir, db, head, files) =
            workdir_manifest_from_materialization_stamps_fixture();
        let target = [("a.txt".to_string(), files["a.txt"].clone())]
            .into_iter()
            .collect::<BTreeMap<_, _>>();

        db.write_clean_workdir_manifest(workdir.path(), &head.root_id, &files, files.keys())
            .unwrap();
        assert!(db
            .clean_workdir_manifest_tracks_file_subset(workdir.path(), &head.root_id, &target)
            .unwrap());

        fs::write(workdir.path().join("a.txt"), "a1\ndirty\n").unwrap();

        assert!(!db
            .clean_workdir_manifest_tracks_file_subset(workdir.path(), &head.root_id, &target)
            .unwrap());
    }

    #[test]
    fn workdir_manifest_from_materialization_stamps_rejects_missing_stamp() {
        let (_workspace, workdir, db, head, files) =
            workdir_manifest_from_materialization_stamps_fixture();
        let mut stamps = db.scan_workdir_file_stamps(workdir.path()).unwrap();
        stamps.remove("a.txt");

        db.write_clean_workdir_manifest_from_stamps(
            workdir.path(),
            &head.root_id,
            &files,
            files.keys(),
            stamps,
        )
        .unwrap();

        assert!(!clean_workdir_manifest_path(workdir.path()).exists());
    }

    #[test]
    fn workdir_manifest_from_materialization_stamps_rejects_extra_stamp() {
        let (_workspace, workdir, db, head, files) =
            workdir_manifest_from_materialization_stamps_fixture();
        let mut stamps = db.scan_workdir_file_stamps(workdir.path()).unwrap();
        let stamp = stamps.get("a.txt").unwrap().clone();
        stamps.insert("extra.txt".to_string(), stamp);

        db.write_clean_workdir_manifest_from_stamps(
            workdir.path(),
            &head.root_id,
            &files,
            files.keys(),
            stamps,
        )
        .unwrap();

        assert!(!clean_workdir_manifest_path(workdir.path()).exists());
    }

    #[test]
    fn pinned_path_probe_skips_different_spelling_already_seen_by_directory_scan() {
        let visible_paths = ["readme.md".to_string()]
            .into_iter()
            .collect::<BTreeSet<_>>();
        let actual_paths = visible_paths.clone();
        let folded_paths = actual_paths
            .iter()
            .map(|path| case_insensitive_path_key(path))
            .collect::<BTreeSet<_>>();

        assert!(!pinned_path_needs_probe(
            true,
            &visible_paths,
            &actual_paths,
            &folded_paths,
            "README.md"
        ));
        assert!(pinned_path_needs_probe(
            true,
            &visible_paths,
            &actual_paths,
            &folded_paths,
            "readme.md"
        ));
        assert!(pinned_path_needs_probe(
            true,
            &visible_paths,
            &actual_paths,
            &folded_paths,
            "other.md"
        ));
        assert!(pinned_path_needs_probe(
            false,
            &visible_paths,
            &actual_paths,
            &folded_paths,
            "README.md"
        ));

        let ignored_visible = BTreeSet::new();
        assert!(!pinned_path_needs_probe(
            true,
            &ignored_visible,
            &actual_paths,
            &folded_paths,
            "README.md"
        ));
        assert!(pinned_path_needs_probe(
            true,
            &ignored_visible,
            &actual_paths,
            &folded_paths,
            "readme.md"
        ));
    }

    #[test]
    fn ignored_actual_spelling_prevents_fabricated_pinned_alias() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("readme.md"), "ignored\n").unwrap();
        fs::write(workspace.path().join(".trailignore"), "readme.md\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::Empty, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let visible = db.scan_workdir_file_stamps(workspace.path()).unwrap();
        assert!(!visible.contains_key("readme.md"));

        let observed =
            observed_exact_paths_for_candidates(workspace.path(), &["README.md".to_string()], true)
                .unwrap();
        let actual_paths = observed.keys().cloned().collect::<BTreeSet<_>>();
        let folded_paths = actual_paths
            .iter()
            .map(|path| case_insensitive_path_key(path))
            .collect::<BTreeSet<_>>();
        assert!(actual_paths.contains("readme.md"));
        assert!(!pinned_path_needs_probe(
            true,
            &BTreeSet::new(),
            &actual_paths,
            &folded_paths,
            "README.md",
        ));

        let stamps = db
            .scan_workdir_file_stamps_with_pinned_paths_case_sensitivity(
                workspace.path(),
                &["README.md".to_string()],
                true,
            )
            .unwrap();
        assert!(stamps.contains_key("readme.md"));
        assert!(!stamps.contains_key("README.md"));
    }

    #[test]
    fn observed_fold_index_supports_ten_thousand_constant_domain_lookups() {
        let observed = (0..10_000)
            .map(|index| {
                (
                    format!("Dir-{index:05}/File.txt"),
                    ObservedPathKind::RegularFile,
                )
            })
            .collect::<BTreeMap<_, _>>();

        let folded = index_observed_paths_by_folded(&observed);

        assert_eq!(folded.len(), 10_000);
        for path in observed.keys() {
            let aliases = folded
                .get(&case_insensitive_path_key(path))
                .expect("folded path is indexed once");
            assert_eq!(aliases.as_slice(), &[path.as_str()]);
        }
    }

    #[test]
    fn touched_manifest_guard_does_not_probe_unrelated_large_manifest_paths() {
        let (_workspace, workdir, db, head, files) =
            workdir_manifest_from_materialization_stamps_fixture();
        let metadata = fs::symlink_metadata(workdir.path().join("a.txt")).unwrap();
        let template = CleanWorkdirManifestEntry {
            stamp: WorkdirFileStamp::from_metadata(&metadata),
            kind: files["a.txt"].kind.clone(),
            content_hash: files["a.txt"].content_hash.clone(),
        };
        let mut entries = BTreeMap::new();
        entries.insert("a.txt".to_string(), template.clone());
        for idx in 0..10_000 {
            entries.insert(format!("missing/{idx:05}.txt"), template.clone());
        }
        db.write_clean_workdir_manifest_entries(workdir.path(), &head.root_id, entries)
            .unwrap();
        let previous = [("a.txt".to_string(), files["a.txt"].clone())]
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        let target = previous.clone();

        assert!(db
            .clean_workdir_manifest_allows_touched_path_update(
                workdir.path(),
                &head.root_id,
                &previous,
                &target,
            )
            .unwrap());
    }

    #[test]
    fn touched_manifest_guard_allows_case_only_rename_from_removed_path() {
        let (_workspace, workdir, db, head, files) =
            workdir_manifest_from_materialization_stamps_fixture();
        db.write_clean_workdir_manifest(workdir.path(), &head.root_id, &files, files.keys())
            .unwrap();
        let previous = [("a.txt".to_string(), files["a.txt"].clone())]
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        let target = [("A.txt".to_string(), files["a.txt"].clone())]
            .into_iter()
            .collect::<BTreeMap<_, _>>();

        assert!(db
            .clean_workdir_manifest_allows_touched_path_update(
                workdir.path(),
                &head.root_id,
                &previous,
                &target,
            )
            .unwrap());
    }

    #[test]
    fn touched_manifest_guard_rejects_external_new_target() {
        let (_workspace, workdir, db, head, files) =
            workdir_manifest_from_materialization_stamps_fixture();
        db.write_clean_workdir_manifest(workdir.path(), &head.root_id, &files, files.keys())
            .unwrap();
        fs::write(workdir.path().join("external.txt"), "external\n").unwrap();
        let previous = [("a.txt".to_string(), files["a.txt"].clone())]
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        let target = [
            ("a.txt".to_string(), files["a.txt"].clone()),
            ("external.txt".to_string(), files["a.txt"].clone()),
        ]
        .into_iter()
        .collect::<BTreeMap<_, _>>();

        assert!(!db
            .clean_workdir_manifest_allows_touched_path_update(
                workdir.path(),
                &head.root_id,
                &previous,
                &target,
            )
            .unwrap());
    }
}
