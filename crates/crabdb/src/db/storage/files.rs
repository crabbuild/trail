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
            let previous_entry = previous.and_then(|entries| entries.get(&disk_file.path));
            let previous_entry = if previous_entry.is_none() {
                previous_by_hash
                    .get(&sha256_hex(&disk_file.bytes))
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

    pub(crate) fn build_root_for_selected_record(
        &self,
        previous: &BTreeMap<String, FileEntry>,
        disk_files: &[DiskFile],
        selected_paths: &[String],
        allow_ignored: bool,
        change_id: &ChangeId,
    ) -> Result<RootBuildResult> {
        let selected_disk_files =
            self.selected_record_disk_files(disk_files, selected_paths, allow_ignored)?;
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
            let previous_entry = previous.get(&disk_file.path).or_else(|| {
                previous_by_hash
                    .get(&sha256_hex(&disk_file.bytes))
                    .and_then(|matches| matches.first().map(|(_, entry)| entry))
            });
            let built = self.build_file_entry(
                &disk_file.path,
                disk_file.bytes,
                disk_file.executable,
                change_id,
                previous_entry,
                &mut file_seq,
                &mut line_seq,
            )?;
            files.insert(disk_file.path, built.entry);
        }
        self.build_root_from_file_entries(files, change_id)
    }

    pub(crate) fn selected_record_disk_files(
        &self,
        disk_files: &[DiskFile],
        selected_paths: &[String],
        allow_ignored: bool,
    ) -> Result<Vec<DiskFile>> {
        let mut selected = BTreeMap::new();
        for file in disk_files {
            if selected_paths
                .iter()
                .any(|path| path_matches_selection(&file.path, path))
            {
                selected.insert(file.path.clone(), file.clone());
            }
        }

        for path in selected_paths {
            let had_visible_match = selected
                .keys()
                .any(|candidate| path_matches_selection(candidate, path));
            if allow_ignored {
                for file in self.read_record_selection_unfiltered(path)? {
                    selected.insert(file.path.clone(), file);
                }
            } else if !had_visible_match {
                let abs = self.workspace_root.join(path_from_rel(path));
                if abs.exists() {
                    return Err(Error::IgnoredPath(path.clone()));
                }
            }
        }

        Ok(selected.into_values().collect())
    }

    pub(crate) fn read_record_selection_unfiltered(&self, path: &str) -> Result<Vec<DiskFile>> {
        if is_internal_path(path) {
            return Err(Error::IgnoredPath(path.to_string()));
        }
        let abs = self.workspace_root.join(path_from_rel(path));
        if !abs.exists() {
            return Ok(Vec::new());
        }
        let metadata = fs::symlink_metadata(&abs)?;
        if metadata.file_type().is_symlink() {
            return Ok(Vec::new());
        }
        if metadata.is_file() {
            return Ok(vec![DiskFile {
                path: path.to_string(),
                bytes: fs::read(&abs)?,
                executable: executable_from_metadata(&metadata),
            }]);
        }
        if !metadata.is_dir() {
            return Ok(Vec::new());
        }
        let mut files = Vec::new();
        self.read_record_dir_unfiltered(&abs, path, &mut files)?;
        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(files)
    }

    pub(crate) fn read_record_dir_unfiltered(
        &self,
        dir: &Path,
        rel_dir: &str,
        files: &mut Vec<DiskFile>,
    ) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            let rel = format!("{rel_dir}/{name}");
            if is_internal_path(&rel) {
                continue;
            }
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                self.read_record_dir_unfiltered(&path, &rel, files)?;
            } else if metadata.is_file() {
                files.push(DiskFile {
                    path: rel,
                    bytes: fs::read(&path)?,
                    executable: executable_from_metadata(&metadata),
                });
            }
        }
        Ok(())
    }

    pub(crate) fn build_root_from_file_entries(
        &self,
        files: BTreeMap<String, FileEntry>,
        change_id: &ChangeId,
    ) -> Result<RootBuildResult> {
        let mut path_builder = BatchBuilder::new(self.store.clone(), prolly_config());
        let mut file_index_builder = BatchBuilder::new(self.store.clone(), prolly_config());
        let mut stats = ImportStats::default();
        let mut total_text_bytes = 0;
        for (path, entry) in &files {
            path_builder.add(path.as_bytes().to_vec(), cbor(entry)?);
            file_index_builder.add(entry.file_id.encode_key(), path.as_bytes().to_vec());
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
            stats,
        })
    }

    pub(crate) fn build_file_entry(
        &self,
        path: &str,
        bytes: Vec<u8>,
        executable: bool,
        change_id: &ChangeId,
        previous: Option<&FileEntry>,
        file_seq: &mut u64,
        line_seq: &mut u64,
    ) -> Result<FileBuildResult> {
        let content_hash = sha256_hex(&bytes);
        let file_id = previous
            .map(|entry| entry.file_id.clone())
            .unwrap_or_else(|| {
                let id = FileId::new(change_id.clone(), *file_seq);
                *file_seq += 1;
                id
            });
        let created_by = previous
            .map(|entry| entry.created_by.clone())
            .unwrap_or_else(|| change_id.clone());
        let previous_text = previous.and_then(|entry| match &entry.content {
            FileContentRef::Text(text_id) => self.load_text_lines(text_id).ok(),
            _ => None,
        });
        let (kind, content, line_changes) = if looks_binary(&bytes) {
            let blob_id = self.put_blob(bytes.clone())?;
            (
                FileKind::Binary,
                FileContentRef::Binary(blob_id),
                Vec::new(),
            )
        } else if std::str::from_utf8(&bytes).is_err() {
            let blob_id = self.put_blob(bytes.clone())?;
            (
                FileKind::OpaqueText,
                FileContentRef::Opaque(blob_id),
                Vec::new(),
            )
        } else if bytes.len() as u64 > self.config.text.opaque_text_max_bytes {
            let blob_id = self.put_blob(bytes.clone())?;
            (
                FileKind::OpaqueText,
                FileContentRef::Opaque(blob_id),
                Vec::new(),
            )
        } else if max_line_len(&bytes) as u64 > self.config.text.max_line_bytes {
            let blob_id = self.put_blob(bytes.clone())?;
            (
                FileKind::OpaqueText,
                FileContentRef::Opaque(blob_id),
                Vec::new(),
            )
        } else {
            let built_text = self.build_text_content(
                &bytes,
                change_id,
                previous_text.as_deref(),
                line_seq,
                self.config.text.preserve_similarity,
            )?;
            (
                FileKind::Text,
                FileContentRef::Text(built_text.object_id),
                built_text.line_changes,
            )
        };
        let last_content_change =
            if previous.is_some_and(|entry| entry.content_hash == content_hash) {
                previous
                    .map(|entry| entry.last_content_change.clone())
                    .unwrap_or_else(|| change_id.clone())
            } else {
                change_id.clone()
            };
        let entry = FileEntry {
            file_id,
            kind,
            mode: if executable { 0o755 } else { 0o644 },
            executable,
            content,
            size_bytes: bytes.len() as u64,
            content_hash,
            created_by,
            last_content_change,
            last_path_change: previous.and_then(|entry| entry.last_path_change.clone()),
        };
        let line_changes = line_changes.into_iter().map(|line| line).collect();
        let _ = path;
        Ok(FileBuildResult {
            entry,
            line_changes,
        })
    }

    pub(crate) fn build_text_content(
        &self,
        bytes: &[u8],
        change_id: &ChangeId,
        previous: Option<&[LineEntry]>,
        line_seq: &mut u64,
        similarity_threshold: f32,
    ) -> Result<TextBuildResult> {
        let new_lines = split_lines(bytes);
        let previous = previous.unwrap_or(&[]);
        let new_hashes = new_lines
            .iter()
            .map(|line| sha256_hex(&line.text))
            .collect::<Vec<_>>();
        let mut used_old = HashSet::new();
        let mut entries = Vec::with_capacity(new_lines.len());

        for (idx, line) in new_lines.iter().enumerate() {
            let text_hash = new_hashes[idx].clone();
            let mut matched_idx = None;
            if let Some(old) = previous.get(idx) {
                if old.text_hash == text_hash && !used_old.contains(&idx) {
                    matched_idx = Some(idx);
                }
            }
            if matched_idx.is_none() {
                matched_idx = previous
                    .iter()
                    .enumerate()
                    .find(|(old_idx, old)| {
                        !used_old.contains(old_idx) && old.text_hash == text_hash
                    })
                    .map(|(old_idx, _)| old_idx);
            }
            if matched_idx.is_none() {
                matched_idx = previous
                    .iter()
                    .enumerate()
                    .find(|(old_idx, old)| {
                        !used_old.contains(old_idx)
                            && line_similarity(&old.text, &line.text) >= similarity_threshold
                    })
                    .map(|(old_idx, _)| old_idx);
            }
            if matched_idx.is_none() {
                if let Some(old) = previous.get(idx) {
                    let old_has_future_match =
                        new_lines
                            .iter()
                            .enumerate()
                            .skip(idx + 1)
                            .any(|(future_idx, future)| {
                                old.text_hash == new_hashes[future_idx]
                                    || line_similarity(&old.text, &future.text)
                                        >= similarity_threshold
                            });
                    if !used_old.contains(&idx) && !old_has_future_match {
                        matched_idx = Some(idx);
                    }
                }
            }
            let entry = if let Some(old_idx) = matched_idx {
                used_old.insert(old_idx);
                let old = &previous[old_idx];
                LineEntry {
                    line_id: old.line_id.clone(),
                    text: line.text.clone(),
                    newline: line.newline,
                    text_hash,
                    introduced_by: old.introduced_by.clone(),
                    last_content_change: if old.text == line.text && old.newline == line.newline {
                        old.last_content_change.clone()
                    } else {
                        change_id.clone()
                    },
                    last_move_change: if old_idx == idx {
                        old.last_move_change.clone()
                    } else {
                        Some(change_id.clone())
                    },
                    flags: old.flags.clone(),
                }
            } else {
                let line_id = LineId::new(change_id.clone(), *line_seq);
                *line_seq += 1;
                LineEntry {
                    line_id,
                    text: line.text.clone(),
                    newline: line.newline,
                    text_hash,
                    introduced_by: change_id.clone(),
                    last_content_change: change_id.clone(),
                    last_move_change: None,
                    flags: LineFlags::default(),
                }
            };
            entries.push(entry);
        }

        let old_positions = previous
            .iter()
            .enumerate()
            .map(|(idx, line)| (line.line_id.clone(), (idx, line)))
            .collect::<HashMap<_, _>>();
        let new_positions = entries
            .iter()
            .enumerate()
            .map(|(idx, line)| (line.line_id.clone(), (idx, line)))
            .collect::<HashMap<_, _>>();
        let mut line_changes = Vec::new();
        for (line_id, (new_idx, new_line)) in &new_positions {
            if let Some((old_idx, old_line)) = old_positions.get(line_id) {
                if old_line.text_hash != new_line.text_hash || old_line.newline != new_line.newline
                {
                    line_changes.push(LineChange {
                        line_id: line_id.clone(),
                        kind: LineChangeKind::Modified,
                        old_line_number: Some(*old_idx as u64 + 1),
                        new_line_number: Some(*new_idx as u64 + 1),
                        before_hash: Some(old_line.text_hash.clone()),
                        after_hash: Some(new_line.text_hash.clone()),
                    });
                } else if old_idx != new_idx {
                    line_changes.push(LineChange {
                        line_id: line_id.clone(),
                        kind: LineChangeKind::Moved,
                        old_line_number: Some(*old_idx as u64 + 1),
                        new_line_number: Some(*new_idx as u64 + 1),
                        before_hash: Some(old_line.text_hash.clone()),
                        after_hash: Some(new_line.text_hash.clone()),
                    });
                }
            } else {
                line_changes.push(LineChange {
                    line_id: line_id.clone(),
                    kind: LineChangeKind::Added,
                    old_line_number: None,
                    new_line_number: Some(*new_idx as u64 + 1),
                    before_hash: None,
                    after_hash: Some(new_line.text_hash.clone()),
                });
            }
        }
        for (line_id, (old_idx, old_line)) in old_positions {
            if !new_positions.contains_key(&line_id) {
                line_changes.push(LineChange {
                    line_id,
                    kind: LineChangeKind::Deleted,
                    old_line_number: Some(old_idx as u64 + 1),
                    new_line_number: None,
                    before_hash: Some(old_line.text_hash.clone()),
                    after_hash: None,
                });
            }
        }
        line_changes.sort_by_key(|change| {
            (
                change
                    .new_line_number
                    .or(change.old_line_number)
                    .unwrap_or(u64::MAX),
                change.line_id.local_seq,
            )
        });

        let mut order_builder = BatchBuilder::new(self.store.clone(), prolly_config());
        let mut index_builder = BatchBuilder::new(self.store.clone(), prolly_config());
        for (idx, entry) in entries.iter().enumerate() {
            let key = order_key(idx as u64 + 1);
            order_builder.add(key.clone(), cbor(entry)?);
            index_builder.add(entry.line_id.encode_key(), key);
        }
        let order_tree = order_builder.build()?;
        let index_tree = index_builder.build()?;
        let content = TextContent {
            version: TEXT_OBJECT_VERSION,
            content_hash: sha256_hex(bytes),
            line_count: entries.len() as u64,
            byte_count: bytes.len() as u64,
            order_map_root: tree_root_hex(&order_tree),
            line_index_map_root: tree_root_hex(&index_tree),
            representation: TextRepresentation::TreeText,
        };
        let object_id = self.put_object(TEXT_CONTENT_KIND, TEXT_OBJECT_VERSION, &content)?;
        Ok(TextBuildResult {
            object_id,
            line_changes,
        })
    }
}
