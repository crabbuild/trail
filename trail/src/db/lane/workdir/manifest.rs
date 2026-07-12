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
        let mut files = self.scan_workdir_file_stamps(&root)?;
        for path in pinned_paths {
            let path = normalize_relative_path(path)?;
            let abs = safe_join(&root, &path)?;
            let metadata = match fs::symlink_metadata(&abs) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    files.remove(&path);
                    continue;
                }
                Err(err) => return Err(Error::Io(err)),
            };
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                files.remove(&path);
                continue;
            }
            files.insert(path, WorkdirFileStamp::from_metadata(&metadata));
        }
        Ok(files)
    }
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
}
