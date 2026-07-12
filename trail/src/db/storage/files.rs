use super::*;
use rayon::prelude::*;

const PATH_READ_BATCH: usize = 128;

struct PathFileRead {
    path: String,
    bytes: Vec<u8>,
    executable: bool,
    content_hash: String,
}

impl Trail {
    pub(crate) fn build_root_from_git_tracked_paths(
        &self,
        paths: &[String],
        change_id: &ChangeId,
    ) -> Result<RootBuildResult> {
        self.build_root_from_paths(paths, change_id)
    }

    pub(crate) fn build_root_from_worktree_paths(
        &self,
        paths: &[String],
        change_id: &ChangeId,
    ) -> Result<RootBuildResult> {
        self.build_root_from_paths(paths, change_id)
    }

    fn build_root_from_paths(
        &self,
        paths: &[String],
        change_id: &ChangeId,
    ) -> Result<RootBuildResult> {
        let paths = normalize_root_build_paths(paths)?;
        validate_no_case_fold_collisions(paths.iter())?;

        let mut files = BTreeMap::new();
        let mut disk_manifest = BTreeMap::new();
        let mut path_builder =
            SortedBatchBuilder::new(self.store.clone(), root_map_prolly_config());
        let mut file_index_builder =
            SortedBatchBuilder::new(self.store.clone(), root_map_prolly_config());
        let mut file_seq = 1;
        let mut line_seq = 1;
        let mut stats = ImportStats::default();
        let mut total_text_bytes = 0u64;

        for chunk in paths.chunks(PATH_READ_BATCH) {
            let reads = read_path_file_batch(&self.workspace_root, chunk)?;
            for read in reads {
                let built = self.build_file_entry(
                    &read.path,
                    read.bytes,
                    read.content_hash,
                    read.executable,
                    change_id,
                    None,
                    &mut file_seq,
                    &mut line_seq,
                )?;
                path_builder.add(read.path.as_bytes().to_vec(), cbor(&built.entry)?)?;
                file_index_builder.add(
                    built.entry.file_id.encode_key(),
                    read.path.as_bytes().to_vec(),
                )?;
                add_entry_import_stats(&mut stats, &built.entry);
                if built.entry.kind == FileKind::Text {
                    total_text_bytes += built.entry.size_bytes;
                }
                disk_manifest.insert(read.path.clone(), built.disk_manifest);
                files.insert(read.path, built.entry);
            }
        }

        let path_tree = path_builder.build()?;
        let file_index_tree = file_index_builder.build()?;
        let root = WorktreeRoot {
            version: ROOT_OBJECT_VERSION,
            path_map_root: tree_root_hex(&path_tree),
            file_index_map_root: tree_root_hex(&file_index_tree),
            file_count: files.len() as u64,
            total_text_bytes,
            created_by: change_id.clone(),
        };
        let root_id = self.put_object(WORKTREE_ROOT_KIND, ROOT_OBJECT_VERSION, &root)?;
        Ok(RootBuildResult {
            root_id,
            files,
            disk_manifest,
            stats,
        })
    }

    pub(crate) fn build_root_from_git_tracked_paths_incremental(
        &self,
        paths: &[String],
        previous_root_id: &ObjectId,
        change_id: &ChangeId,
    ) -> Result<GitTrackedRootBuildResult> {
        let paths = normalize_root_build_paths(paths)?;
        self.ensure_git_tracked_incremental_final_paths_safe(previous_root_id, &paths)?;

        let previous_root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, previous_root_id)?;
        let mut path_tree = root_map_tree_from_root_hex(previous_root.path_map_root.as_deref())?;
        let mut file_index_tree =
            root_map_tree_from_root_hex(previous_root.file_index_map_root.as_deref())?;
        let previous_tree = path_tree.clone();
        let new_paths = paths.iter().map(String::as_str).collect::<HashSet<_>>();
        let mut previous_by_hash: HashMap<String, Vec<(String, FileEntry)>> = HashMap::new();
        let mut file_count = previous_root.file_count as i128;
        let mut total_text_bytes = previous_root.total_text_bytes as i128;
        let mut stats = ImportStats::default();
        for item in self.root_prolly.range(&previous_tree, &[], None)? {
            let (key, value) = item?;
            let path = String::from_utf8(key)
                .map_err(|err| Error::Corrupt(format!("non UTF-8 path key: {err}")))?;
            let entry: FileEntry = from_cbor(&value)?;
            add_entry_import_stats(&mut stats, &entry);
            if new_paths.contains(path.as_str()) {
                continue;
            }
            path_tree = self.root_prolly.delete(&path_tree, path.as_bytes())?;
            file_index_tree = self
                .root_prolly
                .delete(&file_index_tree, &entry.file_id.encode_key())?;
            file_count -= 1;
            total_text_bytes -= entry_text_bytes(&entry) as i128;
            remove_entry_import_stats(&mut stats, &entry);
            previous_by_hash
                .entry(entry.content_hash.clone())
                .or_default()
                .push((path, entry));
        }

        let mut disk_manifest = BTreeMap::new();
        let mut file_seq = 1;
        let mut line_seq = 1;
        for chunk in paths.chunks(PATH_READ_BATCH) {
            let normalized_paths = chunk.to_vec();
            let mut read_paths = Vec::new();
            for path in normalized_paths {
                let previous_same_path = self
                    .root_prolly
                    .get(&previous_tree, path.as_bytes())?
                    .map(|value| from_cbor::<FileEntry>(&value))
                    .transpose()?;
                let abs = self.workspace_root.join(path_from_rel(&path));
                let metadata = match fs::symlink_metadata(&abs) {
                    Ok(metadata) => metadata,
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                        if let Some(previous_entry) = previous_same_path {
                            path_tree = self.root_prolly.delete(&path_tree, path.as_bytes())?;
                            file_index_tree = self
                                .root_prolly
                                .delete(&file_index_tree, &previous_entry.file_id.encode_key())?;
                            file_count -= 1;
                            total_text_bytes -= entry_text_bytes(&previous_entry) as i128;
                            remove_entry_import_stats(&mut stats, &previous_entry);
                        }
                        continue;
                    }
                    Err(err) => return Err(Error::Io(err)),
                };
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    if let Some(previous_entry) = previous_same_path {
                        path_tree = self.root_prolly.delete(&path_tree, path.as_bytes())?;
                        file_index_tree = self
                            .root_prolly
                            .delete(&file_index_tree, &previous_entry.file_id.encode_key())?;
                        file_count -= 1;
                        total_text_bytes -= entry_text_bytes(&previous_entry) as i128;
                        remove_entry_import_stats(&mut stats, &previous_entry);
                    }
                    continue;
                }

                if let Some(previous_entry) = previous_same_path {
                    if previous_entry.executable == executable_from_metadata(&metadata) {
                        if let Some(cached_manifest) =
                            self.cached_worktree_manifest_for_metadata(&path, &metadata)?
                        {
                            if cached_manifest.content_hash == previous_entry.content_hash
                                && cached_manifest.executable == previous_entry.executable
                                && cached_manifest.kind == previous_entry.kind
                            {
                                disk_manifest.insert(path.clone(), cached_manifest);
                                continue;
                            }
                        }
                    }
                }
                read_paths.push(path);
            }

            let reads = read_path_file_batch(&self.workspace_root, &read_paths)?;
            for read in reads {
                let path = read.path;
                let content_hash = read.content_hash;
                let executable = read.executable;
                let previous_same_path = self
                    .root_prolly
                    .get(&previous_tree, path.as_bytes())?
                    .map(|value| from_cbor::<FileEntry>(&value))
                    .transpose()?;
                if let Some(previous_entry) = &previous_same_path {
                    if previous_entry.content_hash == content_hash
                        && previous_entry.executable == executable
                    {
                        disk_manifest.insert(
                            path.clone(),
                            DiskManifest {
                                kind: previous_entry.kind.clone(),
                                executable: previous_entry.executable,
                                content_hash: previous_entry.content_hash.clone(),
                            },
                        );
                        continue;
                    }
                }
                let previous_entry = previous_same_path.as_ref().or_else(|| {
                    previous_by_hash
                        .get(&content_hash)
                        .and_then(|matches| matches.first().map(|(_, entry)| entry))
                });
                let built = self.build_file_entry(
                    &path,
                    read.bytes,
                    content_hash,
                    executable,
                    change_id,
                    previous_entry,
                    &mut file_seq,
                    &mut line_seq,
                )?;
                if let Some(old_entry) = previous_same_path.as_ref() {
                    if old_entry.file_id != built.entry.file_id {
                        file_index_tree = self
                            .root_prolly
                            .delete(&file_index_tree, &old_entry.file_id.encode_key())?;
                    }
                    total_text_bytes -= entry_text_bytes(old_entry) as i128;
                    remove_entry_import_stats(&mut stats, old_entry);
                } else {
                    file_count += 1;
                }
                total_text_bytes += entry_text_bytes(&built.entry) as i128;
                add_entry_import_stats(&mut stats, &built.entry);
                path_tree = self.root_prolly.put(
                    &path_tree,
                    path.as_bytes().to_vec(),
                    cbor(&built.entry)?,
                )?;
                file_index_tree = self.root_prolly.put(
                    &file_index_tree,
                    built.entry.file_id.encode_key(),
                    path.as_bytes().to_vec(),
                )?;
                disk_manifest.insert(path.clone(), built.disk_manifest);
            }
        }

        if file_count < 0 || total_text_bytes < 0 {
            return Err(Error::Corrupt(
                "git tracked incremental root update produced negative root stats".to_string(),
            ));
        }
        let root = WorktreeRoot {
            version: ROOT_OBJECT_VERSION,
            path_map_root: tree_root_hex(&path_tree),
            file_index_map_root: tree_root_hex(&file_index_tree),
            file_count: file_count as u64,
            total_text_bytes: total_text_bytes as u64,
            created_by: change_id.clone(),
        };
        let root_id = self.put_object(WORKTREE_ROOT_KIND, ROOT_OBJECT_VERSION, &root)?;
        Ok(GitTrackedRootBuildResult {
            root_id,
            disk_manifest,
            stats,
        })
    }

    fn ensure_git_tracked_incremental_final_paths_safe(
        &self,
        previous_root_id: &ObjectId,
        paths: &[String],
    ) -> Result<()> {
        let mut final_paths = self
            .load_root_paths(previous_root_id)?
            .into_iter()
            .collect::<BTreeSet<_>>();
        for path in paths {
            final_paths.remove(path);
            let abs = self.workspace_root.join(path_from_rel(path));
            let metadata = match fs::symlink_metadata(&abs) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => return Err(Error::Io(err)),
            };
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                continue;
            }
            final_paths.insert(path.clone());
        }
        validate_no_case_fold_collisions(final_paths.iter())
    }

    pub(crate) fn build_root_from_disk_files(
        &self,
        disk_files: &[DiskFile],
        change_id: &ChangeId,
        previous: Option<&BTreeMap<String, FileEntry>>,
    ) -> Result<RootBuildResult> {
        validate_disk_file_root_paths(disk_files)?;

        let mut files = BTreeMap::new();
        let mut disk_manifest = BTreeMap::new();
        let mut file_seq = 1;
        let mut line_seq = 1;
        let new_paths = disk_files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<HashSet<_>>();
        let mut previous_by_hash: HashMap<String, Vec<(String, FileEntry)>> = HashMap::new();
        if let Some(previous) = previous {
            for (path, entry) in previous {
                if new_paths.contains(path.as_str()) {
                    continue;
                }
                previous_by_hash
                    .entry(entry.content_hash.clone())
                    .or_default()
                    .push((path.clone(), entry.clone()));
            }
        }
        for disk_file in disk_files {
            let content_hash = sha256_hex(&disk_file.bytes);
            let previous_entry = previous.and_then(|entries| entries.get(&disk_file.path));
            if let Some(previous_entry) = previous_entry {
                if previous_entry.content_hash == content_hash
                    && previous_entry.executable == disk_file.executable
                {
                    disk_manifest.insert(
                        disk_file.path.clone(),
                        DiskManifest {
                            kind: previous_entry.kind.clone(),
                            executable: previous_entry.executable,
                            content_hash: previous_entry.content_hash.clone(),
                        },
                    );
                    files.insert(disk_file.path.clone(), previous_entry.clone());
                    continue;
                }
            }
            let previous_entry = if previous_entry.is_none() {
                previous_by_hash
                    .get(&content_hash)
                    .and_then(|matches| matches.first().map(|(_, entry)| entry))
            } else {
                previous_entry
            };
            let built = self.build_file_entry(
                &disk_file.path,
                disk_file.bytes.clone(),
                content_hash,
                disk_file.executable,
                change_id,
                previous_entry,
                &mut file_seq,
                &mut line_seq,
            )?;
            disk_manifest.insert(disk_file.path.clone(), built.disk_manifest);
            files.insert(disk_file.path.clone(), built.entry);
        }
        self.build_root_from_file_entries_and_manifest(files, disk_manifest, change_id)
    }

    pub(crate) fn build_root_for_selected_record_incremental(
        &self,
        previous_root_id: &ObjectId,
        previous: &BTreeMap<String, FileEntry>,
        disk_files: &[DiskFile],
        selected_paths: &[String],
        allow_ignored: bool,
        change_id: &ChangeId,
    ) -> Result<RootBuildResult> {
        let selected_disk_files =
            self.selected_record_disk_files(disk_files, selected_paths, allow_ignored)?;
        self.build_root_for_selected_disk_files_incremental(
            previous_root_id,
            previous,
            &selected_disk_files,
            selected_paths,
            change_id,
        )
    }

    pub(crate) fn build_root_for_selected_disk_files_incremental(
        &self,
        previous_root_id: &ObjectId,
        previous: &BTreeMap<String, FileEntry>,
        disk_files: &[DiskFile],
        selected_paths: &[String],
        change_id: &ChangeId,
    ) -> Result<RootBuildResult> {
        let selected_disk_files = disk_files
            .iter()
            .filter(|file| {
                selected_paths
                    .iter()
                    .any(|selected| path_matches_selection(&file.path, selected))
            })
            .cloned()
            .collect::<Vec<_>>();
        self.ensure_selected_disk_final_root_paths_safe(
            previous_root_id,
            selected_paths,
            &selected_disk_files,
        )?;

        let previous_root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, previous_root_id)?;
        let mut path_tree = root_map_tree_from_root_hex(previous_root.path_map_root.as_deref())?;
        let mut file_index_tree =
            root_map_tree_from_root_hex(previous_root.file_index_map_root.as_deref())?;
        let mut file_count = previous_root.file_count as i128;
        let mut total_text_bytes = previous_root.total_text_bytes as i128;

        let mut files = previous.clone();
        let mut removed_entries = Vec::new();
        for selected in selected_paths {
            let removed_paths = files
                .keys()
                .filter(|path| path_matches_selection(path, selected))
                .cloned()
                .collect::<Vec<_>>();
            for path in removed_paths {
                if let Some(entry) = files.remove(&path) {
                    path_tree = self.root_prolly.delete(&path_tree, path.as_bytes())?;
                    file_index_tree = self
                        .root_prolly
                        .delete(&file_index_tree, &entry.file_id.encode_key())?;
                    file_count -= 1;
                    total_text_bytes -= entry_text_bytes(&entry) as i128;
                    removed_entries.push((path, entry));
                }
            }
        }

        let mut previous_by_hash: HashMap<String, Vec<(String, FileEntry)>> = HashMap::new();
        for (path, entry) in removed_entries {
            previous_by_hash
                .entry(entry.content_hash.clone())
                .or_default()
                .push((path, entry));
        }

        let mut file_seq = 1;
        let mut line_seq = 1;
        for disk_file in selected_disk_files {
            let DiskFile {
                path,
                bytes,
                executable,
            } = disk_file;
            let content_hash = sha256_hex(&bytes);
            let entry = if let Some(previous_entry) = previous.get(&path) {
                if previous_entry.content_hash == content_hash
                    && previous_entry.executable == executable
                {
                    previous_entry.clone()
                } else {
                    let built = self.build_file_entry(
                        &path,
                        bytes,
                        content_hash,
                        executable,
                        change_id,
                        Some(previous_entry),
                        &mut file_seq,
                        &mut line_seq,
                    )?;
                    built.entry
                }
            } else {
                let previous_entry = previous_by_hash
                    .get(&content_hash)
                    .and_then(|matches| matches.first().map(|(_, entry)| entry));
                let built = self.build_file_entry(
                    &path,
                    bytes,
                    content_hash,
                    executable,
                    change_id,
                    previous_entry,
                    &mut file_seq,
                    &mut line_seq,
                )?;
                built.entry
            };
            path_tree =
                self.root_prolly
                    .put(&path_tree, path.as_bytes().to_vec(), cbor(&entry)?)?;
            file_index_tree = self.root_prolly.put(
                &file_index_tree,
                entry.file_id.encode_key(),
                path.as_bytes().to_vec(),
            )?;
            file_count += 1;
            total_text_bytes += entry_text_bytes(&entry) as i128;
            files.insert(path, entry);
        }

        if file_count < 0 || total_text_bytes < 0 {
            return Err(Error::Corrupt(
                "selected incremental root update produced negative root stats".to_string(),
            ));
        }

        let (stats, _) = root_stats(&files);
        let root = WorktreeRoot {
            version: ROOT_OBJECT_VERSION,
            path_map_root: tree_root_hex(&path_tree),
            file_index_map_root: tree_root_hex(&file_index_tree),
            file_count: file_count as u64,
            total_text_bytes: total_text_bytes as u64,
            created_by: change_id.clone(),
        };
        let root_id = self.put_object(WORKTREE_ROOT_KIND, ROOT_OBJECT_VERSION, &root)?;
        Ok(RootBuildResult {
            root_id,
            files,
            disk_manifest: BTreeMap::new(),
            stats,
        })
    }

    fn ensure_selected_disk_final_root_paths_safe(
        &self,
        previous_root_id: &ObjectId,
        selected_paths: &[String],
        selected_disk_files: &[DiskFile],
    ) -> Result<()> {
        let mut paths = self
            .load_root_paths(previous_root_id)?
            .into_iter()
            .collect::<BTreeSet<_>>();
        for selected in selected_paths {
            let selected = normalize_relative_path(selected)?;
            paths.retain(|path| !path_matches_selection(path, &selected));
        }
        paths.extend(selected_disk_files.iter().map(|file| file.path.clone()));
        validate_no_case_fold_collisions(paths.iter())
    }

    pub(crate) fn build_root_from_touched_file_entries_incremental(
        &self,
        previous_root_id: &ObjectId,
        previous: &BTreeMap<String, FileEntry>,
        target: &BTreeMap<String, FileEntry>,
        change_id: &ChangeId,
    ) -> Result<IncrementalRootBuildResult> {
        self.ensure_touched_final_root_paths_safe(previous_root_id, previous, target)?;

        let previous_root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, previous_root_id)?;
        let mut path_tree = root_map_tree_from_root_hex(previous_root.path_map_root.as_deref())?;
        let mut file_index_tree =
            root_map_tree_from_root_hex(previous_root.file_index_map_root.as_deref())?;
        let mut file_count = previous_root.file_count as i128;
        let mut total_text_bytes = previous_root.total_text_bytes as i128;

        let mut paths = BTreeSet::new();
        paths.extend(previous.keys().cloned());
        paths.extend(target.keys().cloned());

        for path in paths {
            let old = previous.get(&path);
            let new = target.get(&path);
            if old == new {
                continue;
            }

            if let Some(old_entry) = old {
                path_tree = self.root_prolly.delete(&path_tree, path.as_bytes())?;
                file_index_tree = self
                    .root_prolly
                    .delete(&file_index_tree, &old_entry.file_id.encode_key())?;
                file_count -= 1;
                total_text_bytes -= entry_text_bytes(old_entry) as i128;
            }

            if let Some(new_entry) = new {
                path_tree =
                    self.root_prolly
                        .put(&path_tree, path.as_bytes().to_vec(), cbor(new_entry)?)?;
                file_index_tree = self.root_prolly.put(
                    &file_index_tree,
                    new_entry.file_id.encode_key(),
                    path.as_bytes().to_vec(),
                )?;
                file_count += 1;
                total_text_bytes += entry_text_bytes(new_entry) as i128;
            }
        }

        if file_count < 0 || total_text_bytes < 0 {
            return Err(Error::Corrupt(
                "incremental root update produced negative root stats".to_string(),
            ));
        }

        let root = WorktreeRoot {
            version: ROOT_OBJECT_VERSION,
            path_map_root: tree_root_hex(&path_tree),
            file_index_map_root: tree_root_hex(&file_index_tree),
            file_count: file_count as u64,
            total_text_bytes: total_text_bytes as u64,
            created_by: change_id.clone(),
        };
        let root_id = self.put_object(WORKTREE_ROOT_KIND, ROOT_OBJECT_VERSION, &root)?;
        Ok(IncrementalRootBuildResult { root_id })
    }

    fn ensure_touched_final_root_paths_safe(
        &self,
        previous_root_id: &ObjectId,
        previous: &BTreeMap<String, FileEntry>,
        target: &BTreeMap<String, FileEntry>,
    ) -> Result<()> {
        validate_no_case_fold_collisions(target.keys())?;
        if target.keys().all(|path| previous.contains_key(path)) {
            return Ok(());
        }

        let mut paths = self
            .load_root_paths(previous_root_id)?
            .into_iter()
            .collect::<BTreeSet<_>>();
        for path in previous.keys() {
            paths.remove(path);
        }
        paths.extend(target.keys().cloned());
        validate_no_case_fold_collisions(paths.iter())
    }

    pub(crate) fn build_root_from_file_entries(
        &self,
        files: BTreeMap<String, FileEntry>,
        change_id: &ChangeId,
    ) -> Result<RootBuildResult> {
        let disk_manifest = files
            .iter()
            .map(|(path, entry)| {
                (
                    path.clone(),
                    DiskManifest {
                        kind: entry.kind.clone(),
                        executable: entry.executable,
                        content_hash: entry.content_hash.clone(),
                    },
                )
            })
            .collect();
        self.build_root_from_file_entries_and_manifest(files, disk_manifest, change_id)
    }

    pub(crate) fn build_root_from_file_entries_and_manifest(
        &self,
        files: BTreeMap<String, FileEntry>,
        disk_manifest: BTreeMap<String, DiskManifest>,
        change_id: &ChangeId,
    ) -> Result<RootBuildResult> {
        validate_no_case_fold_collisions(files.keys())?;
        validate_no_case_fold_collisions(disk_manifest.keys())?;

        let mut path_builder =
            SortedBatchBuilder::new(self.store.clone(), root_map_prolly_config());
        let mut file_index_builder =
            BatchBuilder::new(self.store.clone(), root_map_prolly_config());
        for (path, entry) in &files {
            path_builder.add(path.as_bytes().to_vec(), cbor(entry)?)?;
            file_index_builder.add(entry.file_id.encode_key(), path.as_bytes().to_vec());
        }
        let path_tree = path_builder.build()?;
        let file_index_tree = file_index_builder.build()?;
        let (stats, total_text_bytes) = root_stats(&files);
        let root = WorktreeRoot {
            version: ROOT_OBJECT_VERSION,
            path_map_root: tree_root_hex(&path_tree),
            file_index_map_root: tree_root_hex(&file_index_tree),
            file_count: files.len() as u64,
            total_text_bytes,
            created_by: change_id.clone(),
        };
        let root_id = self.put_object(WORKTREE_ROOT_KIND, ROOT_OBJECT_VERSION, &root)?;
        Ok(RootBuildResult {
            root_id,
            files,
            disk_manifest,
            stats,
        })
    }
}

fn normalize_root_build_paths(paths: &[String]) -> Result<Vec<String>> {
    let mut normalized = BTreeSet::new();
    for path in paths {
        normalized.insert(normalize_relative_path(path)?);
    }
    Ok(normalized.into_iter().collect())
}

fn validate_disk_file_root_paths(disk_files: &[DiskFile]) -> Result<()> {
    let mut paths = BTreeSet::new();
    for disk_file in disk_files {
        let normalized = normalize_relative_path(&disk_file.path)?;
        if normalized != disk_file.path {
            return Err(Error::InvalidPath {
                path: disk_file.path.clone(),
                reason: format!("disk file path must be normalized as `{normalized}`"),
            });
        }
        if !paths.insert(disk_file.path.clone()) {
            return Err(Error::InvalidPath {
                path: disk_file.path.clone(),
                reason: "duplicate disk file path".to_string(),
            });
        }
    }
    validate_no_case_fold_collisions(paths.iter())
}

fn read_path_file_batch(root: &Path, paths: &[String]) -> Result<Vec<PathFileRead>> {
    paths
        .par_iter()
        .map(|path| read_path_file(root, path))
        .collect::<Result<Vec<_>>>()
        .map(|reads| reads.into_iter().flatten().collect())
}

fn read_path_file(root: &Path, path: &str) -> Result<Option<PathFileRead>> {
    let abs = root.join(path_from_rel(path));
    let metadata = match fs::symlink_metadata(&abs) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(Error::Io(err)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(&abs)?;
    let content_hash = sha256_hex(&bytes);
    Ok(Some(PathFileRead {
        path: path.to_string(),
        bytes,
        executable: executable_from_metadata(&metadata),
        content_hash,
    }))
}

fn entry_text_bytes(entry: &FileEntry) -> u64 {
    if entry.kind == FileKind::Text {
        entry.size_bytes
    } else {
        0
    }
}

fn add_entry_import_stats(stats: &mut ImportStats, entry: &FileEntry) {
    stats.files += 1;
    match entry.kind {
        FileKind::Text => stats.text += 1,
        FileKind::OpaqueText => stats.opaque += 1,
        FileKind::Binary => stats.binary += 1,
    }
}

fn remove_entry_import_stats(stats: &mut ImportStats, entry: &FileEntry) {
    stats.files = stats.files.saturating_sub(1);
    match entry.kind {
        FileKind::Text => stats.text = stats.text.saturating_sub(1),
        FileKind::OpaqueText => stats.opaque = stats.opaque.saturating_sub(1),
        FileKind::Binary => stats.binary = stats.binary.saturating_sub(1),
    }
}

fn root_stats(files: &BTreeMap<String, FileEntry>) -> (ImportStats, u64) {
    let mut stats = ImportStats::default();
    let mut total_text_bytes = 0;
    for entry in files.values() {
        add_entry_import_stats(&mut stats, entry);
        if entry.kind == FileKind::Text {
            total_text_bytes += entry.size_bytes;
        }
    }
    (stats, total_text_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touched_incremental_root_rejects_final_case_fold_collisions_before_objects() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let previous = db
            .load_root_files_for_paths(&head.root_id, &["README.md".to_string()])
            .unwrap();
        let mut target = previous.clone();
        target.insert(
            "readme.md".to_string(),
            previous.get("README.md").unwrap().clone(),
        );
        let count_rows = |table: &str| -> i64 {
            db.conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap()
        };
        let objects_before = count_rows("objects");
        let prolly_nodes_before = count_rows("prolly_nodes");

        let err = db
            .build_root_from_touched_file_entries_incremental(
                &head.root_id,
                &previous,
                &target,
                &ChangeId("change_collision_test".to_string()),
            )
            .unwrap_err();

        assert!(
            matches!(err, Error::InvalidPath { ref reason, .. } if reason.contains("case-insensitive path collision")),
            "expected final root collision, got {err:?}"
        );
        assert_eq!(count_rows("objects"), objects_before);
        assert_eq!(count_rows("prolly_nodes"), prolly_nodes_before);
    }

    #[test]
    fn full_file_entry_root_rejects_case_fold_collisions_before_objects() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let mut files = db.load_root_files(&head.root_id).unwrap();
        let entry = files.get("README.md").unwrap().clone();
        files.insert("ＲＥＡＤＭＥ.md".to_string(), entry);
        let count_rows = |table: &str| -> i64 {
            db.conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap()
        };
        let objects_before = count_rows("objects");
        let prolly_nodes_before = count_rows("prolly_nodes");

        let err = db
            .build_root_from_file_entries(
                files,
                &ChangeId("change_full_collision_test".to_string()),
            )
            .unwrap_err();

        assert!(
            matches!(err, Error::InvalidPath { ref reason, .. } if reason.contains("case-insensitive path collision")),
            "expected full root collision, got {err:?}"
        );
        assert_eq!(count_rows("objects"), objects_before);
        assert_eq!(count_rows("prolly_nodes"), prolly_nodes_before);
    }

    #[test]
    fn disk_file_root_rejects_case_fold_collisions_before_objects() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let disk_files = vec![
            DiskFile {
                path: "README.md".to_string(),
                bytes: b"hello\n".to_vec(),
                executable: false,
            },
            DiskFile {
                path: "readme.md".to_string(),
                bytes: b"collision\n".to_vec(),
                executable: false,
            },
        ];
        let count_rows = |table: &str| -> i64 {
            db.conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap()
        };
        let objects_before = count_rows("objects");
        let prolly_nodes_before = count_rows("prolly_nodes");

        let err = db
            .build_root_from_disk_files(
                &disk_files,
                &ChangeId("change_disk_file_collision_test".to_string()),
                None,
            )
            .unwrap_err();

        assert!(
            matches!(err, Error::InvalidPath { ref reason, .. } if reason.contains("case-insensitive path collision")),
            "expected disk-file root collision, got {err:?}"
        );
        assert_eq!(count_rows("objects"), objects_before);
        assert_eq!(count_rows("prolly_nodes"), prolly_nodes_before);
    }

    #[test]
    fn path_list_root_build_rejects_case_fold_collisions_before_objects() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        fs::write(temp.path().join("readme.md"), "collision\n").unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let count_rows = |table: &str| -> i64 {
            db.conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap()
        };
        let objects_before = count_rows("objects");
        let prolly_nodes_before = count_rows("prolly_nodes");

        let err = db
            .build_root_from_worktree_paths(
                &["README.md".to_string(), "readme.md".to_string()],
                &ChangeId("change_path_list_collision_test".to_string()),
            )
            .unwrap_err();

        assert!(
            matches!(err, Error::InvalidPath { ref reason, .. } if reason.contains("case-insensitive path collision")),
            "expected path-list root collision, got {err:?}"
        );
        assert_eq!(count_rows("objects"), objects_before);
        assert_eq!(count_rows("prolly_nodes"), prolly_nodes_before);
    }

    #[test]
    fn git_tracked_incremental_root_rejects_final_case_fold_collisions_before_objects() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        fs::write(temp.path().join("readme.md"), "collision\n").unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let count_rows = |table: &str| -> i64 {
            db.conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap()
        };
        let objects_before = count_rows("objects");
        let prolly_nodes_before = count_rows("prolly_nodes");

        let err = db
            .build_root_from_git_tracked_paths_incremental(
                &["readme.md".to_string()],
                &head.root_id,
                &ChangeId("change_git_incremental_collision_test".to_string()),
            )
            .unwrap_err();

        assert!(
            matches!(err, Error::InvalidPath { ref reason, .. } if reason.contains("case-insensitive path collision")),
            "expected git-tracked incremental collision, got {err:?}"
        );
        assert_eq!(count_rows("objects"), objects_before);
        assert_eq!(count_rows("prolly_nodes"), prolly_nodes_before);
    }
}
