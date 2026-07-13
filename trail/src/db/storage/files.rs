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
    #[allow(dead_code)] // Task 5 resets metrics around scale scenarios.
    pub(crate) fn reset_case_fold_index_metrics(&self) {
        self.case_fold_index_metrics
            .set(CaseFoldIndexMetrics::default());
    }

    #[allow(dead_code)] // Task 5 reports these metrics from scale scenarios.
    pub(crate) fn case_fold_index_metrics_report(&self) -> CaseFoldIndexMetricsReport {
        let metrics = self.case_fold_index_metrics.get();
        CaseFoldIndexMetricsReport {
            mode: metrics.mode.as_str().to_string(),
            lookup_count: metrics.lookup_count,
            full_root_path_load_count: metrics.full_root_path_load_count,
        }
    }

    pub(crate) fn note_full_root_path_load(&self) {
        let mut metrics = self.case_fold_index_metrics.get();
        metrics.full_root_path_load_count = metrics.full_root_path_load_count.saturating_add(1);
        self.case_fold_index_metrics.set(metrics);
    }

    fn note_case_fold_index_lookups(&self, lookup_count: usize) {
        let mut metrics = self.case_fold_index_metrics.get();
        metrics.mode = CaseFoldIndexMode::Indexed;
        metrics.lookup_count = metrics.lookup_count.saturating_add(lookup_count as u64);
        self.case_fold_index_metrics.set(metrics);
    }

    pub(crate) fn build_case_fold_map_tree<'a, I>(&self, paths: I) -> Result<Tree>
    where
        I: IntoIterator<Item = &'a String>,
    {
        let mut mappings = BTreeMap::new();
        for path in paths {
            insert_case_fold_mapping(&mut mappings, path)?;
        }

        self.build_case_fold_map_tree_from_sorted_mappings(mappings)
    }

    fn build_case_fold_map_tree_from_sorted_mappings(
        &self,
        mappings: BTreeMap<String, String>,
    ) -> Result<Tree> {
        let mut builder = SortedBatchBuilder::new(self.store.clone(), root_map_prolly_config());
        for (folded, canonical) in mappings {
            builder.add(folded.into_bytes(), canonical.into_bytes())?;
        }
        Ok(builder.build()?)
    }

    pub(crate) fn validate_and_update_case_fold_index(
        &self,
        previous_root: &WorktreeRoot,
        removals: &[String],
        additions: &[String],
    ) -> Result<Tree> {
        for path in removals.iter().chain(additions) {
            let normalized = normalize_relative_path(path)?;
            if normalized != *path {
                return Err(Error::InvalidPath {
                    path: path.clone(),
                    reason: format!("path must be normalized as `{normalized}`"),
                });
            }
        }
        let previous_tree = match previous_root.case_fold_map_root.as_deref() {
            Some(case_fold_root) => root_map_tree_from_root_hex(Some(case_fold_root))?,
            None if previous_root.file_count == 0 => self.root_prolly.create(),
            None => {
                return Err(Error::PathIndexRequired(
                    "legacy root has no case-fold index; run `trail index rebuild`".to_string(),
                ));
            }
        };
        let touched_keys = removals
            .iter()
            .chain(additions)
            .map(|path| case_insensitive_path_key(path))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        self.note_case_fold_index_lookups(touched_keys.len());
        let existing = self.root_prolly.get_many(&previous_tree, &touched_keys)?;
        let mut before = BTreeMap::new();
        for (folded, canonical) in touched_keys.into_iter().zip(existing) {
            let canonical = canonical
                .map(|value| validate_case_fold_index_value(&folded, value))
                .transpose()?;
            before.insert(folded, canonical);
        }
        let mut after = before.clone();

        for path in removals {
            let folded = case_insensitive_path_key(path);
            if after.get(&folded).and_then(Option::as_deref) == Some(path.as_str()) {
                after.insert(folded, None);
            }
        }
        for path in additions {
            let folded = case_insensitive_path_key(path);
            if let Some(previous) = after.get(&folded).and_then(Option::as_deref) {
                if previous != path {
                    return Err(Error::InvalidPath {
                        path: path.clone(),
                        reason: format!("case-insensitive path collision with `{previous}`"),
                    });
                }
            }
            after.insert(folded, Some(path.clone()));
        }

        let mutations = after
            .into_iter()
            .filter_map(|(folded, canonical)| {
                (before.get(&folded) != Some(&canonical)).then(|| {
                    let key = folded.into_bytes();
                    match canonical {
                        Some(canonical) => prolly::Mutation::Upsert {
                            key,
                            val: canonical.into_bytes(),
                        },
                        None => prolly::Mutation::Delete { key },
                    }
                })
            })
            .collect::<Vec<_>>();
        if mutations.is_empty() {
            return Ok(previous_tree);
        }

        Ok(self.root_prolly.batch(&previous_tree, mutations)?)
    }

    pub(crate) fn preflight_record_case_fold_candidates(
        &self,
        previous_root_id: &ObjectId,
        candidate_paths: &[String],
        disk_manifest: &BTreeMap<String, DiskManifest>,
    ) -> Result<RecordCaseFoldPreflight> {
        let previous_root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, previous_root_id)?;
        let previous_tree = match previous_root.case_fold_map_root.as_deref() {
            Some(case_fold_root) => root_map_tree_from_root_hex(Some(case_fold_root))?,
            None if previous_root.file_count == 0 => self.root_prolly.create(),
            None => {
                return Err(Error::PathIndexRequired(
                    "legacy root has no case-fold index; run `trail index rebuild`".to_string(),
                ));
            }
        };

        let mut candidates_by_folded = BTreeMap::<String, BTreeSet<String>>::new();
        for path in candidate_paths {
            let path = normalize_relative_path(path)?;
            candidates_by_folded
                .entry(case_insensitive_path_key(&path))
                .or_default()
                .insert(path);
        }
        let touched_keys = candidates_by_folded.keys().cloned().collect::<Vec<_>>();
        self.note_case_fold_index_lookups(touched_keys.len());
        let existing = self.root_prolly.get_many(&previous_tree, &touched_keys)?;

        let mut selected_paths = BTreeSet::new();
        let mut mutations = Vec::new();
        for ((folded, candidates), existing) in
            candidates_by_folded.into_iter().zip(existing.into_iter())
        {
            selected_paths.extend(candidates.iter().cloned());
            let previous = existing
                .map(|value| validate_case_fold_index_value(&folded, value))
                .transpose()?;
            if let Some(previous) = &previous {
                selected_paths.insert(previous.clone());
            }

            let mut present = candidates
                .iter()
                .filter(|path| disk_manifest.contains_key(*path))
                .cloned()
                .collect::<BTreeSet<_>>();
            if let Some(previous) = &previous {
                if disk_manifest.contains_key(previous) || !candidates.contains(previous) {
                    present.insert(previous.clone());
                }
            }
            if present.len() > 1 {
                let mut paths = present.into_iter();
                let previous = paths.next().expect("present path exists");
                let path = paths.next().expect("collision path exists");
                return Err(Error::InvalidPath {
                    path,
                    reason: format!("case-insensitive path collision with `{previous}`"),
                });
            }
            let final_path = present.into_iter().next();
            if previous == final_path {
                continue;
            }
            let key = folded.into_bytes();
            mutations.push(match final_path {
                Some(path) => prolly::Mutation::Upsert {
                    key,
                    val: path.into_bytes(),
                },
                None => prolly::Mutation::Delete { key },
            });
        }

        let case_fold_tree = if mutations.is_empty() {
            previous_tree
        } else {
            self.root_prolly.batch(&previous_tree, mutations)?
        };
        Ok(RecordCaseFoldPreflight {
            selected_paths: selected_paths.into_iter().collect(),
            case_fold_tree,
        })
    }

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
        let mut case_fold_mappings = BTreeMap::new();
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
                insert_case_fold_mapping(&mut case_fold_mappings, &read.path)?;
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

        let case_fold_tree =
            self.build_case_fold_map_tree_from_sorted_mappings(case_fold_mappings)?;
        let path_tree = path_builder.build()?;
        let file_index_tree = file_index_builder.build()?;
        let root = WorktreeRoot {
            version: ROOT_OBJECT_VERSION,
            path_map_root: tree_root_hex(&path_tree),
            file_index_map_root: tree_root_hex(&file_index_tree),
            case_fold_map_root: tree_root_hex(&case_fold_tree),
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
        let mut accepted_paths = Vec::new();
        for path in paths {
            let abs = self.workspace_root.join(path_from_rel(&path));
            let metadata = match fs::symlink_metadata(&abs) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => return Err(Error::Io(err)),
            };
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                continue;
            }
            accepted_paths.push(path);
        }

        let previous_root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, previous_root_id)?;
        let mut path_tree = root_map_tree_from_root_hex(previous_root.path_map_root.as_deref())?;
        let mut file_index_tree =
            root_map_tree_from_root_hex(previous_root.file_index_map_root.as_deref())?;
        let previous_tree = path_tree.clone();
        let new_paths = accepted_paths
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let mut previous_by_hash: HashMap<String, Vec<(String, FileEntry)>> = HashMap::new();
        let mut file_count = previous_root.file_count as i128;
        let mut total_text_bytes = previous_root.total_text_bytes as i128;
        let mut stats = ImportStats::default();
        let mut removed_entries = Vec::new();
        let mut existing_accepted_paths = HashSet::new();
        for item in self.root_prolly.range(&previous_tree, &[], None)? {
            let (key, value) = item?;
            let path = String::from_utf8(key)
                .map_err(|err| Error::Corrupt(format!("non UTF-8 path key: {err}")))?;
            let entry: FileEntry = from_cbor(&value)?;
            add_entry_import_stats(&mut stats, &entry);
            if new_paths.contains(path.as_str()) {
                existing_accepted_paths.insert(path);
                continue;
            }
            removed_entries.push((path, entry));
        }
        let removals = removed_entries
            .iter()
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        let additions = accepted_paths
            .iter()
            .filter(|path| !existing_accepted_paths.contains(path.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let case_fold_tree =
            self.validate_and_update_case_fold_index(&previous_root, &removals, &additions)?;
        for (path, entry) in removed_entries {
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
        for chunk in accepted_paths.chunks(PATH_READ_BATCH) {
            let normalized_paths = chunk.to_vec();
            let mut read_paths = Vec::new();
            for path in normalized_paths {
                let previous_same_path = self
                    .root_prolly
                    .get(&previous_tree, path.as_bytes())?
                    .map(|value| from_cbor::<FileEntry>(&value))
                    .transpose()?;
                if let Some(previous_entry) = previous_same_path {
                    // The first stat preceded the previous-root range. Refresh
                    // metadata before trusting the unchanged-file cache, while
                    // still avoiding a content read for a true cache hit.
                    let metadata = fresh_regular_file_metadata(&self.workspace_root, &path)?;
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

            let reads = read_known_regular_path_file_batch(&self.workspace_root, &read_paths)?;
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
            case_fold_map_root: tree_root_hex(&case_fold_tree),
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
        let selected_paths = selected_paths
            .iter()
            .map(|path| normalize_relative_path(path))
            .collect::<Result<Vec<_>>>()?;
        let selected_disk_files = disk_files
            .iter()
            .filter(|file| {
                selected_paths
                    .iter()
                    .any(|selected| path_matches_selection(&file.path, selected))
            })
            .cloned()
            .collect::<Vec<_>>();
        validate_disk_file_root_paths(&selected_disk_files)?;

        let previous_root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, previous_root_id)?;
        let removals = previous
            .keys()
            .filter(|path| {
                selected_paths
                    .iter()
                    .any(|selected| path_matches_selection(path, selected))
            })
            .cloned()
            .collect::<Vec<_>>();
        let additions = selected_disk_files
            .iter()
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        let case_fold_tree =
            self.validate_and_update_case_fold_index(&previous_root, &removals, &additions)?;
        self.build_root_for_selected_disk_files_incremental_with_case_fold_tree(
            previous_root_id,
            previous,
            &selected_disk_files,
            &selected_paths,
            change_id,
            case_fold_tree,
        )
    }

    pub(crate) fn build_root_for_selected_disk_files_incremental_with_case_fold_tree(
        &self,
        previous_root_id: &ObjectId,
        previous: &BTreeMap<String, FileEntry>,
        disk_files: &[DiskFile],
        selected_paths: &[String],
        change_id: &ChangeId,
        case_fold_tree: Tree,
    ) -> Result<RootBuildResult> {
        let selected_paths = selected_paths
            .iter()
            .map(|path| normalize_relative_path(path))
            .collect::<Result<Vec<_>>>()?;
        let selected_disk_files = disk_files
            .iter()
            .filter(|file| {
                selected_paths
                    .iter()
                    .any(|selected| path_matches_selection(&file.path, selected))
            })
            .cloned()
            .collect::<Vec<_>>();
        validate_disk_file_root_paths(&selected_disk_files)?;
        let previous_root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, previous_root_id)?;
        let mut path_tree = root_map_tree_from_root_hex(previous_root.path_map_root.as_deref())?;
        let mut file_index_tree =
            root_map_tree_from_root_hex(previous_root.file_index_map_root.as_deref())?;
        let mut file_count = previous_root.file_count as i128;
        let mut total_text_bytes = previous_root.total_text_bytes as i128;

        let mut files = previous.clone();
        let mut removed_entries = Vec::new();
        for selected in &selected_paths {
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
            case_fold_map_root: tree_root_hex(&case_fold_tree),
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

    pub(crate) fn build_root_from_touched_file_entries_incremental(
        &self,
        previous_root_id: &ObjectId,
        previous: &BTreeMap<String, FileEntry>,
        target: &BTreeMap<String, FileEntry>,
        change_id: &ChangeId,
    ) -> Result<IncrementalRootBuildResult> {
        let previous_root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, previous_root_id)?;
        let (removals, additions) = touched_path_map_changes(previous, target);
        let case_fold_tree =
            self.validate_and_update_case_fold_index(&previous_root, &removals, &additions)?;
        self.build_root_from_touched_file_entries_incremental_with_case_fold_tree(
            previous_root_id,
            previous,
            target,
            change_id,
            case_fold_tree,
        )
    }

    pub(crate) fn build_root_from_touched_file_entries_incremental_with_case_fold_tree(
        &self,
        previous_root_id: &ObjectId,
        previous: &BTreeMap<String, FileEntry>,
        target: &BTreeMap<String, FileEntry>,
        change_id: &ChangeId,
        case_fold_tree: Tree,
    ) -> Result<IncrementalRootBuildResult> {
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
            case_fold_map_root: tree_root_hex(&case_fold_tree),
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
        let case_fold_tree = self.build_case_fold_map_tree(files.keys())?;

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
            case_fold_map_root: tree_root_hex(&case_fold_tree),
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

fn validate_case_fold_index_value(folded: &str, value: Vec<u8>) -> Result<String> {
    let canonical = String::from_utf8(value).map_err(|err| {
        Error::Corrupt(format!(
            "case-fold index key {folded:?} stores non UTF-8 canonical path bytes: {err}"
        ))
    })?;
    let normalized = normalize_relative_path(&canonical).map_err(|err| {
        Error::Corrupt(format!(
            "case-fold index key {folded:?} stores invalid canonical path {canonical:?}: {err}"
        ))
    })?;
    if normalized != canonical {
        return Err(Error::Corrupt(format!(
            "case-fold index key {folded:?} stores noncanonical path {canonical:?}; canonical path must be normalized as {normalized:?}"
        )));
    }
    let canonical_folded = case_insensitive_path_key(&canonical);
    if canonical_folded != folded {
        return Err(Error::Corrupt(format!(
            "case-fold index key {folded:?} stores canonical path {canonical:?}, which folds to {canonical_folded:?}"
        )));
    }
    Ok(canonical)
}

fn insert_case_fold_mapping(mappings: &mut BTreeMap<String, String>, path: &str) -> Result<()> {
    let folded = case_insensitive_path_key(path);
    if let Some(previous) = mappings.insert(folded, path.to_string()) {
        if previous != path {
            return Err(Error::InvalidPath {
                path: path.to_string(),
                reason: format!("case-insensitive path collision with `{previous}`"),
            });
        }
    }
    Ok(())
}

fn touched_path_map_changes(
    previous: &BTreeMap<String, FileEntry>,
    target: &BTreeMap<String, FileEntry>,
) -> (Vec<String>, Vec<String>) {
    let removals = previous
        .keys()
        .filter(|path| !target.contains_key(*path))
        .cloned()
        .collect();
    let additions = target
        .keys()
        .filter(|path| !previous.contains_key(*path))
        .cloned()
        .collect();
    (removals, additions)
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

fn read_known_regular_path_file_batch(root: &Path, paths: &[String]) -> Result<Vec<PathFileRead>> {
    paths
        .par_iter()
        .map(|path| {
            let abs = root.join(path_from_rel(path));
            let mut options = OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
            }
            #[cfg(not(unix))]
            {
                let metadata = fs::symlink_metadata(&abs)?;
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(Error::InvalidInput(format!(
                        "tracked path `{path}` changed file type during import"
                    )));
                }
            }
            let mut file = options.open(&abs)?;
            let metadata = file.metadata()?;
            if !metadata.is_file() {
                return Err(Error::InvalidInput(format!(
                    "tracked path `{path}` changed file type during import"
                )));
            }
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            #[cfg(not(unix))]
            {
                let final_metadata = fs::symlink_metadata(&abs)?;
                if final_metadata.file_type().is_symlink() || !final_metadata.is_file() {
                    return Err(Error::InvalidInput(format!(
                        "tracked path `{path}` changed file type during import"
                    )));
                }
            }
            let content_hash = sha256_hex(&bytes);
            Ok(PathFileRead {
                path: path.clone(),
                bytes,
                executable: executable_from_metadata(&metadata),
                content_hash,
            })
        })
        .collect()
}

fn fresh_regular_file_metadata(root: &Path, path: &str) -> Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(root.join(path_from_rel(path)))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(Error::InvalidInput(format!(
            "tracked path `{path}` changed file type during import"
        )));
    }
    Ok(metadata)
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

    fn assert_case_fold_mapping(
        db: &Trail,
        root_id: &ObjectId,
        folded_path: &str,
        canonical_path: &str,
    ) {
        let root: WorktreeRoot = db.get_object(WORKTREE_ROOT_KIND, root_id).unwrap();
        let tree = root_map_tree_from_root_hex(root.case_fold_map_root.as_deref()).unwrap();
        assert_eq!(
            db.root_prolly.get(&tree, folded_path.as_bytes()).unwrap(),
            Some(canonical_path.as_bytes().to_vec())
        );
    }

    fn root_map_entries(db: &Trail, root_hex: Option<&str>) -> Vec<(Vec<u8>, Vec<u8>)> {
        let tree = root_map_tree_from_root_hex(root_hex).unwrap();
        db.root_prolly
            .range(&tree, &[], None)
            .unwrap()
            .map(|item| item.unwrap())
            .collect()
    }

    fn root_with_case_fold_value(
        db: &Trail,
        root: &WorktreeRoot,
        folded: &str,
        canonical: &str,
    ) -> WorktreeRoot {
        let tree = root_map_tree_from_root_hex(root.case_fold_map_root.as_deref()).unwrap();
        let tree = db
            .root_prolly
            .put(
                &tree,
                folded.as_bytes().to_vec(),
                canonical.as_bytes().to_vec(),
            )
            .unwrap();
        let mut corrupt = root.clone();
        corrupt.case_fold_map_root = tree_root_hex(&tree);
        corrupt
    }

    #[test]
    fn legacy_worktree_root_deserialization_defaults_case_fold_index_to_none() {
        #[derive(Serialize)]
        struct LegacyWorktreeRoot {
            version: u16,
            path_map_root: Option<String>,
            file_index_map_root: Option<String>,
            file_count: u64,
            total_text_bytes: u64,
            created_by: ChangeId,
        }

        let bytes = cbor(&LegacyWorktreeRoot {
            version: ROOT_OBJECT_VERSION,
            path_map_root: Some("path-root".to_string()),
            file_index_map_root: Some("file-index-root".to_string()),
            file_count: 2,
            total_text_bytes: 12,
            created_by: ChangeId("change_legacy_root".to_string()),
        })
        .unwrap();

        let root: WorktreeRoot = from_cbor(&bytes).unwrap();
        assert_eq!(root.case_fold_map_root, None);
    }

    #[test]
    fn indexed_case_fold_update_renames_a_path_and_preserves_untouched_entries() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        fs::write(temp.path().join("LICENSE"), "license\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let mut root: WorktreeRoot = db.get_object(WORKTREE_ROOT_KIND, &head.root_id).unwrap();
        root.path_map_root = Some("path-tree-must-not-be-loaded".to_string());
        root.file_index_map_root = Some("file-index-tree-must-not-be-loaded".to_string());

        let next = db
            .validate_and_update_case_fold_index(
                &root,
                &["README.md".to_string()],
                &["docs/Guide.md".to_string()],
            )
            .unwrap();

        assert_eq!(db.root_prolly.get(&next, b"readme.md").unwrap(), None);
        assert_eq!(
            db.root_prolly.get(&next, b"docs/guide.md").unwrap(),
            Some(b"docs/Guide.md".to_vec())
        );
        assert_eq!(
            db.root_prolly.get(&next, b"license").unwrap(),
            Some(b"LICENSE".to_vec())
        );
    }

    #[test]
    fn indexed_case_fold_update_applies_removal_before_case_only_addition() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let root: WorktreeRoot = db.get_object(WORKTREE_ROOT_KIND, &head.root_id).unwrap();

        let next = db
            .validate_and_update_case_fold_index(
                &root,
                &["README.md".to_string()],
                &["readme.md".to_string()],
            )
            .unwrap();

        assert_eq!(
            db.root_prolly.get(&next, b"readme.md").unwrap(),
            Some(b"readme.md".to_vec())
        );
    }

    #[test]
    fn indexed_case_fold_update_adds_the_first_path_to_an_empty_root() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let root: WorktreeRoot = db.get_object(WORKTREE_ROOT_KIND, &head.root_id).unwrap();
        assert_eq!(root.file_count, 0);
        assert_eq!(root.case_fold_map_root, None);

        let next = db
            .validate_and_update_case_fold_index(&root, &[], &["docs/ReadMe.md".to_string()])
            .unwrap();

        assert_eq!(
            db.root_prolly.get(&next, b"docs/readme.md").unwrap(),
            Some(b"docs/ReadMe.md".to_vec())
        );
    }

    #[test]
    fn indexed_case_fold_update_empty_root_collision_writes_no_nodes() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let root: WorktreeRoot = db.get_object(WORKTREE_ROOT_KIND, &head.root_id).unwrap();
        let prolly_nodes_before: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM prolly_nodes", [], |row| row.get(0))
            .unwrap();

        let err = db
            .validate_and_update_case_fold_index(
                &root,
                &[],
                &["README.md".to_string(), "readme.md".to_string()],
            )
            .unwrap_err();

        assert!(matches!(
            err,
            Error::InvalidPath { ref path, ref reason }
                if path == "readme.md"
                    && reason == "case-insensitive path collision with `README.md`"
        ));
        assert_eq!(
            db.conn
                .query_row("SELECT COUNT(*) FROM prolly_nodes", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            prolly_nodes_before
        );
    }

    #[test]
    fn indexed_case_fold_update_rejects_traversal_and_non_nfc_paths_without_writes() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let root: WorktreeRoot = db.get_object(WORKTREE_ROOT_KIND, &head.root_id).unwrap();
        let prolly_nodes_before: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM prolly_nodes", [], |row| row.get(0))
            .unwrap();

        let traversal = db
            .validate_and_update_case_fold_index(&root, &["../README.md".to_string()], &[])
            .unwrap_err();
        assert!(matches!(
            traversal,
            Error::InvalidPath { ref reason, .. } if reason.contains("inside the workspace")
        ));

        let non_nfc = db
            .validate_and_update_case_fold_index(&root, &[], &["docs/cafe\u{0301}.md".to_string()])
            .unwrap_err();
        assert!(matches!(
            non_nfc,
            Error::InvalidPath { ref reason, .. } if reason.contains("Unicode NFC")
        ));
        assert_eq!(
            db.conn
                .query_row("SELECT COUNT(*) FROM prolly_nodes", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            prolly_nodes_before
        );
    }

    #[test]
    fn indexed_case_fold_update_rejects_ascii_and_unicode_within_batch_collisions() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("LICENSE"), "license\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let root: WorktreeRoot = db.get_object(WORKTREE_ROOT_KIND, &head.root_id).unwrap();

        let ascii = db
            .validate_and_update_case_fold_index(
                &root,
                &[],
                &["README.md".to_string(), "readme.md".to_string()],
            )
            .unwrap_err();
        assert!(matches!(
            ascii,
            Error::InvalidPath { ref path, ref reason }
                if path == "readme.md"
                    && reason == "case-insensitive path collision with `README.md`"
        ));

        let unicode = db
            .validate_and_update_case_fold_index(
                &root,
                &[],
                &["src/Ｋernel.rs".to_string(), "src/kernel.rs".to_string()],
            )
            .unwrap_err();
        assert!(matches!(
            unicode,
            Error::InvalidPath { ref path, ref reason }
                if path == "src/kernel.rs"
                    && reason == "case-insensitive path collision with `src/Ｋernel.rs`"
        ));
    }

    #[test]
    fn indexed_case_fold_update_rejects_collision_with_existing_root_without_writes() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let root: WorktreeRoot = db.get_object(WORKTREE_ROOT_KIND, &head.root_id).unwrap();
        let prolly_nodes_before: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM prolly_nodes", [], |row| row.get(0))
            .unwrap();

        let err = db
            .validate_and_update_case_fold_index(&root, &[], &["readme.md".to_string()])
            .unwrap_err();

        assert!(matches!(
            err,
            Error::InvalidPath { ref path, ref reason }
                if path == "readme.md"
                    && reason == "case-insensitive path collision with `README.md`"
        ));
        assert_eq!(
            db.conn
                .query_row("SELECT COUNT(*) FROM prolly_nodes", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            prolly_nodes_before
        );
    }

    #[test]
    fn indexed_case_fold_update_rejects_stored_key_value_mismatch_without_writes() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let root: WorktreeRoot = db.get_object(WORKTREE_ROOT_KIND, &head.root_id).unwrap();
        let corrupt = root_with_case_fold_value(&db, &root, "readme.md", "Other.md");
        let objects_before: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM objects", [], |row| row.get(0))
            .unwrap();
        let prolly_nodes_before: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM prolly_nodes", [], |row| row.get(0))
            .unwrap();

        let err = db
            .validate_and_update_case_fold_index(&corrupt, &["README.md".to_string()], &[])
            .unwrap_err();

        assert!(matches!(
            err,
            Error::Corrupt(ref message)
                if message.contains("readme.md")
                    && message.contains("Other.md")
                    && message.contains("other.md")
        ));
        assert_eq!(
            db.conn
                .query_row("SELECT COUNT(*) FROM objects", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            objects_before
        );
        assert_eq!(
            db.conn
                .query_row("SELECT COUNT(*) FROM prolly_nodes", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            prolly_nodes_before
        );
    }

    #[test]
    fn indexed_case_fold_update_rejects_invalid_stored_canonical_paths_without_writes() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let root: WorktreeRoot = db.get_object(WORKTREE_ROOT_KIND, &head.root_id).unwrap();

        for (stored, expected_reason) in [
            ("../README.md", "inside the workspace"),
            ("docs/./ReadMe.md", "must be normalized"),
            ("docs/cafe\u{0301}.md", "Unicode NFC"),
        ] {
            let corrupt = root_with_case_fold_value(&db, &root, "readme.md", stored);
            let objects_before: i64 = db
                .conn
                .query_row("SELECT COUNT(*) FROM objects", [], |row| row.get(0))
                .unwrap();
            let prolly_nodes_before: i64 = db
                .conn
                .query_row("SELECT COUNT(*) FROM prolly_nodes", [], |row| row.get(0))
                .unwrap();

            let err = db
                .validate_and_update_case_fold_index(&corrupt, &["README.md".to_string()], &[])
                .unwrap_err();

            assert!(matches!(
                err,
                Error::Corrupt(ref message)
                    if message.contains("readme.md")
                        && message.contains(expected_reason)
                        && message.contains("canonical path")
            ));
            assert_eq!(
                db.conn
                    .query_row("SELECT COUNT(*) FROM objects", [], |row| row
                        .get::<_, i64>(0))
                    .unwrap(),
                objects_before,
                "stored value {stored:?} wrote an object"
            );
            assert_eq!(
                db.conn
                    .query_row("SELECT COUNT(*) FROM prolly_nodes", [], |row| row
                        .get::<_, i64>(0))
                    .unwrap(),
                prolly_nodes_before,
                "stored value {stored:?} wrote a Prolly node"
            );
        }
    }

    #[test]
    fn indexed_case_fold_update_duplicate_and_noop_inputs_reuse_previous_tree() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let root: WorktreeRoot = db.get_object(WORKTREE_ROOT_KIND, &head.root_id).unwrap();
        let previous_tree =
            root_map_tree_from_root_hex(root.case_fold_map_root.as_deref()).unwrap();
        let prolly_nodes_before: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM prolly_nodes", [], |row| row.get(0))
            .unwrap();

        let duplicate = db
            .validate_and_update_case_fold_index(
                &root,
                &[],
                &["README.md".to_string(), "README.md".to_string()],
            )
            .unwrap();
        let wrong_case_removal = db
            .validate_and_update_case_fold_index(&root, &["readme.md".to_string()], &[])
            .unwrap();
        let delete_then_add = db
            .validate_and_update_case_fold_index(
                &root,
                &["README.md".to_string()],
                &["README.md".to_string()],
            )
            .unwrap();
        let empty = db
            .validate_and_update_case_fold_index(&root, &[], &[])
            .unwrap();

        assert_eq!(tree_root_hex(&duplicate), tree_root_hex(&previous_tree));
        assert_eq!(
            tree_root_hex(&wrong_case_removal),
            tree_root_hex(&previous_tree)
        );
        assert_eq!(
            tree_root_hex(&delete_then_add),
            tree_root_hex(&previous_tree)
        );
        assert_eq!(tree_root_hex(&empty), tree_root_hex(&previous_tree));
        assert_eq!(
            db.root_prolly
                .get(&wrong_case_removal, b"readme.md")
                .unwrap(),
            Some(b"README.md".to_vec())
        );
        assert_eq!(
            db.conn
                .query_row("SELECT COUNT(*) FROM prolly_nodes", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            prolly_nodes_before
        );
    }

    #[test]
    fn indexed_case_fold_update_legacy_root_requires_rebuild_without_reads_or_writes() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let mut root: WorktreeRoot = db.get_object(WORKTREE_ROOT_KIND, &head.root_id).unwrap();
        root.case_fold_map_root = None;
        root.path_map_root = Some("invalid-path-tree-that-must-not-be-loaded".to_string());
        let objects_before: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM objects", [], |row| row.get(0))
            .unwrap();
        let prolly_nodes_before: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM prolly_nodes", [], |row| row.get(0))
            .unwrap();

        let err = db
            .validate_and_update_case_fold_index(&root, &[], &["new.md".to_string()])
            .unwrap_err();

        assert!(matches!(
            err,
            Error::PathIndexRequired(ref message)
                if message.contains("trail index rebuild")
        ));
        assert_eq!(err.code(), "PATH_INDEX_REQUIRED");
        assert_eq!(
            db.conn
                .query_row("SELECT COUNT(*) FROM objects", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            objects_before
        );
        assert_eq!(
            db.conn
                .query_row("SELECT COUNT(*) FROM prolly_nodes", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            prolly_nodes_before
        );
    }

    #[test]
    fn full_path_list_root_build_persists_ascii_and_unicode_case_fold_keys() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::create_dir_all(temp.path().join("docs")).unwrap();
        fs::write(temp.path().join("src/Ｋernel.rs"), "kernel\n").unwrap();
        fs::write(temp.path().join("docs/ReadMe.md"), "readme\n").unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let built = db
            .build_root_from_worktree_paths(
                &["src/Ｋernel.rs".to_string(), "docs/ReadMe.md".to_string()],
                &ChangeId("change_case_fold_path_root".to_string()),
            )
            .unwrap();

        assert_case_fold_mapping(&db, &built.root_id, "docs/readme.md", "docs/ReadMe.md");
        assert_case_fold_mapping(&db, &built.root_id, "src/kernel.rs", "src/Ｋernel.rs");
    }

    #[cfg(unix)]
    #[test]
    fn full_path_list_root_case_fold_domain_excludes_filtered_paths() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();
        fs::create_dir_all(temp.path().join("docs")).unwrap();
        fs::create_dir_all(temp.path().join("directory-only")).unwrap();
        fs::write(temp.path().join("docs/ReadMe.md"), "readme\n").unwrap();
        symlink_file("docs/ReadMe.md", temp.path().join("readme-link.md")).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let built = db
            .build_root_from_worktree_paths(
                &[
                    "docs/ReadMe.md".to_string(),
                    "directory-only".to_string(),
                    "missing.md".to_string(),
                    "readme-link.md".to_string(),
                ],
                &ChangeId("change_case_fold_filtered_paths".to_string()),
            )
            .unwrap();
        let root: WorktreeRoot = db.get_object(WORKTREE_ROOT_KIND, &built.root_id).unwrap();

        let path_entries = root_map_entries(&db, root.path_map_root.as_deref());
        let case_fold_entries = root_map_entries(&db, root.case_fold_map_root.as_deref());
        let path_domain = path_entries
            .iter()
            .map(|(path, _)| String::from_utf8(path.clone()).unwrap())
            .collect::<Vec<_>>();
        let case_fold_domain = case_fold_entries
            .iter()
            .map(|(_, canonical)| String::from_utf8(canonical.clone()).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(path_domain, vec!["docs/ReadMe.md"]);
        assert_eq!(case_fold_domain, path_domain);
        assert_eq!(
            case_fold_entries,
            vec![(b"docs/readme.md".to_vec(), b"docs/ReadMe.md".to_vec())]
        );
    }

    #[test]
    fn full_file_entry_root_build_persists_ascii_and_unicode_case_fold_keys() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::create_dir_all(temp.path().join("docs")).unwrap();
        fs::write(temp.path().join("src/Ｋernel.rs"), "kernel\n").unwrap();
        fs::write(temp.path().join("docs/ReadMe.md"), "readme\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let files = db.load_root_files(&head.root_id).unwrap();
        let built = db
            .build_root_from_file_entries(
                files,
                &ChangeId("change_case_fold_file_entry_root".to_string()),
            )
            .unwrap();

        assert_case_fold_mapping(&db, &built.root_id, "docs/readme.md", "docs/ReadMe.md");
        assert_case_fold_mapping(&db, &built.root_id, "src/kernel.rs", "src/Ｋernel.rs");
    }

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
                &["README.md".to_string(), "readme.md".to_string()],
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

    fn assert_indexed_case_fold_metrics(db: &Trail, max_lookups: u64) {
        let metrics = db.case_fold_index_metrics_report();
        assert_eq!(metrics.mode, "indexed");
        assert!(
            metrics.lookup_count <= max_lookups,
            "expected at most {max_lookups} indexed lookups, got {metrics:?}"
        );
        assert_eq!(metrics.full_root_path_load_count, 0);
    }

    #[test]
    fn git_tracked_incremental_root_updates_index_for_accepted_regular_file_domain() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("old.txt"), "old\n").unwrap();
        fs::write(temp.path().join("keep.txt"), "keep\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        fs::remove_file(temp.path().join("old.txt")).unwrap();
        fs::write(temp.path().join("new.txt"), "new\n").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("keep.txt", temp.path().join("linked.txt")).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        db.reset_case_fold_index_metrics();
        let mut paths = vec![
            "keep.txt".to_string(),
            "missing.txt".to_string(),
            "new.txt".to_string(),
        ];
        #[cfg(unix)]
        paths.push("linked.txt".to_string());
        let built = db
            .build_root_from_git_tracked_paths_incremental(
                &paths,
                &head.root_id,
                &ChangeId("change_git_indexed_domain".to_string()),
            )
            .unwrap();

        let root: WorktreeRoot = db.get_object(WORKTREE_ROOT_KIND, &built.root_id).unwrap();
        let entries = root_map_entries(&db, root.case_fold_map_root.as_deref());
        assert_eq!(
            entries,
            vec![
                (b"keep.txt".to_vec(), b"keep.txt".to_vec()),
                (b"new.txt".to_vec(), b"new.txt".to_vec()),
            ]
        );
        assert_indexed_case_fold_metrics(&db, 2);
    }

    #[test]
    fn selected_directory_incremental_root_indexes_actual_removed_and_added_files() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("docs")).unwrap();
        fs::write(temp.path().join("docs/old.md"), "old\n").unwrap();
        fs::write(temp.path().join("keep.md"), "keep\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let previous = db
            .load_root_files_for_selections(&head.root_id, &["docs".to_string()])
            .unwrap();
        let disk_files = vec![DiskFile {
            path: "docs/New.md".to_string(),
            bytes: b"new\n".to_vec(),
            executable: false,
        }];
        db.reset_case_fold_index_metrics();
        let built = db
            .build_root_for_selected_disk_files_incremental(
                &head.root_id,
                &previous,
                &disk_files,
                &["docs".to_string()],
                &ChangeId("change_selected_directory_index".to_string()),
            )
            .unwrap();

        let root: WorktreeRoot = db.get_object(WORKTREE_ROOT_KIND, &built.root_id).unwrap();
        let entries = root_map_entries(&db, root.case_fold_map_root.as_deref());
        assert_eq!(
            entries,
            vec![
                (b"docs/new.md".to_vec(), b"docs/New.md".to_vec()),
                (b"keep.md".to_vec(), b"keep.md".to_vec()),
            ]
        );
        assert_indexed_case_fold_metrics(&db, 2);
    }

    #[test]
    fn touched_incremental_root_handles_case_only_rename_and_reuses_unchanged_index() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let previous = db
            .load_root_files_for_paths(&head.root_id, &["README.md".to_string()])
            .unwrap();
        let mut renamed = BTreeMap::new();
        renamed.insert(
            "readme.md".to_string(),
            previous.get("README.md").unwrap().clone(),
        );
        db.reset_case_fold_index_metrics();
        let renamed_root = db
            .build_root_from_touched_file_entries_incremental(
                &head.root_id,
                &previous,
                &renamed,
                &ChangeId("change_case_only_rename_index".to_string()),
            )
            .unwrap();
        assert_case_fold_mapping(&db, &renamed_root.root_id, "readme.md", "readme.md");
        assert_indexed_case_fold_metrics(&db, 1);

        let renamed_root_value: WorktreeRoot = db
            .get_object(WORKTREE_ROOT_KIND, &renamed_root.root_id)
            .unwrap();
        let renamed_files = db.load_root_files(&renamed_root.root_id).unwrap();
        let mut content_only = renamed_files.clone();
        content_only.get_mut("readme.md").unwrap().content_hash = "changed".to_string();
        db.reset_case_fold_index_metrics();
        let content_root = db
            .build_root_from_touched_file_entries_incremental(
                &renamed_root.root_id,
                &renamed_files,
                &content_only,
                &ChangeId("change_content_only_index_reuse".to_string()),
            )
            .unwrap();
        let content_root_value: WorktreeRoot = db
            .get_object(WORKTREE_ROOT_KIND, &content_root.root_id)
            .unwrap();
        assert_eq!(
            content_root_value.case_fold_map_root,
            renamed_root_value.case_fold_map_root
        );
        assert_indexed_case_fold_metrics(&db, 0);
    }

    #[test]
    fn patch_and_record_policy_preflights_return_indexed_trees_without_full_root_loads() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let previous = db
            .load_root_files_for_paths(&head.root_id, &["README.md".to_string()])
            .unwrap();
        db.reset_case_fold_index_metrics();
        let patch_tree = db
            .ensure_patch_final_root_paths_safe(
                &head.root_id,
                &previous,
                &[PatchEdit::Rename {
                    from: "README.md".to_string(),
                    to: "readme.md".to_string(),
                }],
            )
            .unwrap();
        assert!(tree_root_hex(&patch_tree).is_some());
        assert_indexed_case_fold_metrics(&db, 1);

        let summary = FileDiffSummary {
            path: "readme.md".to_string(),
            old_path: Some("README.md".to_string()),
            kind: FileChangeKind::Renamed,
            before_hash: None,
            after_hash: None,
            additions: 0,
            deletions: 0,
            line_changes: Vec::new(),
            patch: None,
        };
        db.reset_case_fold_index_metrics();
        let record_tree = db
            .ensure_record_final_root_paths_safe_from_summaries(&head.root_id, &[summary])
            .unwrap();
        assert!(tree_root_hex(&record_tree).is_some());
        assert_indexed_case_fold_metrics(&db, 1);
    }

    #[test]
    fn touched_incremental_deletion_to_empty_root_keeps_empty_index_semantics() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let previous = db.load_root_files(&head.root_id).unwrap();
        db.reset_case_fold_index_metrics();
        let built = db
            .build_root_from_touched_file_entries_incremental(
                &head.root_id,
                &previous,
                &BTreeMap::new(),
                &ChangeId("change_delete_to_empty_index".to_string()),
            )
            .unwrap();

        let root: WorktreeRoot = db.get_object(WORKTREE_ROOT_KIND, &built.root_id).unwrap();
        assert_eq!(root.file_count, 0);
        assert_eq!(root.case_fold_map_root, None);
        assert_indexed_case_fold_metrics(&db, 1);
    }

    #[test]
    fn touched_incremental_legacy_root_fails_before_tree_or_object_writes() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let mut legacy: WorktreeRoot = db.get_object(WORKTREE_ROOT_KIND, &head.root_id).unwrap();
        legacy.case_fold_map_root = None;
        let legacy_root_id = db
            .put_object(WORKTREE_ROOT_KIND, ROOT_OBJECT_VERSION, &legacy)
            .unwrap();
        let previous = db.load_root_files(&legacy_root_id).unwrap();
        let mut target = previous.clone();
        target.insert(
            "new.md".to_string(),
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
                &legacy_root_id,
                &previous,
                &target,
                &ChangeId("change_legacy_index_failure".to_string()),
            )
            .unwrap_err();
        assert_eq!(err.code(), "PATH_INDEX_REQUIRED");
        assert_eq!(count_rows("objects"), objects_before);
        assert_eq!(count_rows("prolly_nodes"), prolly_nodes_before);
    }

    #[test]
    fn high_level_patch_and_record_use_one_bounded_index_preflight() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let mut db = Trail::open(temp.path()).unwrap();
        db.spawn_lane("metric-patch", Some("main"), false, None, None)
            .unwrap();
        db.reset_case_fold_index_metrics();
        db.apply_lane_patch(
            "metric-patch",
            PatchDocument {
                base_change: None,
                allow_stale: true,
                allow_ignored: false,
                session_id: None,
                message: None,
                edits: vec![PatchEdit::Write {
                    path: "notes.md".to_string(),
                    content: "notes\n".to_string(),
                    executable: false,
                }],
            },
        )
        .unwrap();
        assert_indexed_case_fold_metrics(&db, 1);

        let spawned = db
            .spawn_lane("metric-record", Some("main"), true, None, None)
            .unwrap();
        let workdir = PathBuf::from(spawned.workdir.unwrap());
        fs::write(workdir.join("recorded.md"), "recorded\n").unwrap();
        db.reset_case_fold_index_metrics();
        db.record_lane_workdir("metric-record", None).unwrap();
        assert_indexed_case_fold_metrics(&db, 1);
    }

    #[test]
    fn record_candidate_preflight_distinguishes_partial_collision_from_explicit_rename() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let disk_manifest = [(
            "readme.md".to_string(),
            DiskManifest {
                kind: FileKind::Text,
                executable: false,
                content_hash: sha256_hex(b"hello\n"),
            },
        )]
        .into_iter()
        .collect::<BTreeMap<_, _>>();

        let err = match db.preflight_record_case_fold_candidates(
            &head.root_id,
            &["readme.md".to_string()],
            &disk_manifest,
        ) {
            Ok(_) => panic!("expected partial-manifest collision"),
            Err(err) => err,
        };
        assert!(
            matches!(err, Error::InvalidPath { ref reason, .. } if reason.contains("case-insensitive path collision")),
            "expected partial-manifest collision, got {err:?}"
        );

        let preflight = db
            .preflight_record_case_fold_candidates(
                &head.root_id,
                &["README.md".to_string(), "readme.md".to_string()],
                &disk_manifest,
            )
            .unwrap();
        assert_eq!(
            preflight.selected_paths,
            vec!["README.md".to_string(), "readme.md".to_string()]
        );
        assert_eq!(
            db.root_prolly
                .get(&preflight.case_fold_tree, b"readme.md")
                .unwrap(),
            Some(b"readme.md".to_vec())
        );
    }

    #[cfg(unix)]
    #[test]
    fn known_regular_file_batch_rejects_symlink_swap_and_disappearance() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("tracked.txt"), "tracked\n").unwrap();
        fs::write(temp.path().join("target.txt"), "target\n").unwrap();
        let paths = vec!["tracked.txt".to_string()];

        fs::remove_file(temp.path().join("tracked.txt")).unwrap();
        std::os::unix::fs::symlink("target.txt", temp.path().join("tracked.txt")).unwrap();
        assert!(fresh_regular_file_metadata(temp.path(), "tracked.txt").is_err());
        assert!(read_known_regular_path_file_batch(temp.path(), &paths).is_err());

        fs::remove_file(temp.path().join("tracked.txt")).unwrap();
        assert!(read_known_regular_path_file_batch(temp.path(), &paths).is_err());
    }
}
