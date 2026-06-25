use super::*;

impl CrabDb {
    pub(crate) fn build_root_from_disk_files(
        &self,
        disk_files: &[DiskFile],
        change_id: &ChangeId,
        previous: Option<&BTreeMap<String, FileEntry>>,
    ) -> Result<RootBuildResult> {
        let mut files = BTreeMap::new();
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
                disk_file.executable,
                change_id,
                previous_entry,
                &mut file_seq,
                &mut line_seq,
            )?;
            files.insert(disk_file.path.clone(), built.entry);
        }
        self.build_root_from_file_entries(files, change_id)
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
        let previous_root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, previous_root_id)?;
        let mut path_tree =
            worktree_root_map_tree_from_root_hex(previous_root.path_map_root.as_deref())?;
        let mut file_index_tree =
            worktree_root_map_tree_from_root_hex(previous_root.file_index_map_root.as_deref())?;
        let mut file_count = previous_root.file_count as i128;
        let mut total_text_bytes = previous_root.total_text_bytes as i128;
        let selected_disk_files = disk_files
            .iter()
            .filter(|file| {
                selected_paths
                    .iter()
                    .any(|selected| path_matches_selection(&file.path, selected))
            })
            .cloned()
            .collect::<Vec<_>>();

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
            stats,
        })
    }

    pub(crate) fn build_root_from_touched_file_entries_incremental(
        &self,
        previous_root_id: &ObjectId,
        previous: &BTreeMap<String, FileEntry>,
        target: &BTreeMap<String, FileEntry>,
        change_id: &ChangeId,
    ) -> Result<IncrementalRootBuildResult> {
        let previous_root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, previous_root_id)?;
        let mut path_tree =
            worktree_root_map_tree_from_root_hex(previous_root.path_map_root.as_deref())?;
        let mut file_index_tree =
            worktree_root_map_tree_from_root_hex(previous_root.file_index_map_root.as_deref())?;
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

    pub(crate) fn build_root_from_file_entries(
        &self,
        files: BTreeMap<String, FileEntry>,
        change_id: &ChangeId,
    ) -> Result<RootBuildResult> {
        let mut path_builder =
            BatchBuilder::new(self.store.clone(), worktree_root_map_prolly_config());
        let mut file_index_builder =
            BatchBuilder::new(self.store.clone(), worktree_root_map_prolly_config());
        for (path, entry) in &files {
            path_builder.add(path.as_bytes().to_vec(), cbor(entry)?);
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
            stats,
        })
    }
}

fn entry_text_bytes(entry: &FileEntry) -> u64 {
    if entry.kind == FileKind::Text {
        entry.size_bytes
    } else {
        0
    }
}

fn root_stats(files: &BTreeMap<String, FileEntry>) -> (ImportStats, u64) {
    let mut stats = ImportStats::default();
    let mut total_text_bytes = 0;
    for entry in files.values() {
        stats.files += 1;
        match entry.kind {
            FileKind::Text => {
                stats.text += 1;
                total_text_bytes += entry.size_bytes;
            }
            FileKind::OpaqueText => stats.opaque += 1,
            FileKind::Binary => stats.binary += 1,
        }
    }
    (stats, total_text_bytes)
}
