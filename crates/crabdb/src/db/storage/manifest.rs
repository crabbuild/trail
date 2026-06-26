use super::*;
use crate::db::storage::worktree_index::file_kind_from_index;

impl CrabDb {
    pub(crate) fn diff_root_to_worktree_index(
        &self,
        root_id: &ObjectId,
    ) -> Result<Vec<FileDiffSummary>> {
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        let tree = root_map_tree_from_root_hex(root.path_map_root.as_deref())?;
        let mut root_iter = self.root_prolly.range(&tree, &[], None)?;
        let mut stmt = self.conn.prepare(
            "SELECT path, kind, executable, content_hash \
             FROM worktree_file_index ORDER BY path ASC",
        )?;
        let mut index_rows = stmt.query([])?;
        let mut left = next_root_file(&mut root_iter)?;
        let mut right = next_index_file(&mut index_rows)?;
        let mut summaries = Vec::new();

        loop {
            match (&left, &right) {
                (Some((left_path, left_entry)), Some((right_path, right_entry))) => {
                    match left_path.cmp(right_path) {
                        std::cmp::Ordering::Less => {
                            summaries.push(deleted_manifest_summary(left_path.clone(), left_entry));
                            left = next_root_file(&mut root_iter)?;
                        }
                        std::cmp::Ordering::Greater => {
                            summaries.push(added_manifest_summary(right_path.clone(), right_entry));
                            right = next_index_file(&mut index_rows)?;
                        }
                        std::cmp::Ordering::Equal => {
                            if left_entry.content_hash != right_entry.content_hash
                                || left_entry.executable != right_entry.executable
                                || left_entry.kind != right_entry.kind
                            {
                                summaries.push(changed_manifest_summary(
                                    left_path.clone(),
                                    left_entry,
                                    right_entry,
                                ));
                            }
                            left = next_root_file(&mut root_iter)?;
                            right = next_index_file(&mut index_rows)?;
                        }
                    }
                }
                (Some((left_path, left_entry)), None) => {
                    summaries.push(deleted_manifest_summary(left_path.clone(), left_entry));
                    left = next_root_file(&mut root_iter)?;
                }
                (None, Some((right_path, right_entry))) => {
                    summaries.push(added_manifest_summary(right_path.clone(), right_entry));
                    right = next_index_file(&mut index_rows)?;
                }
                (None, None) => break,
            }
        }
        Ok(summaries)
    }

    pub(crate) fn diff_root_to_disk_manifest(
        &self,
        root_id: &ObjectId,
        manifest: &BTreeMap<String, DiskManifest>,
    ) -> Result<Vec<FileDiffSummary>> {
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        let tree = root_map_tree_from_root_hex(root.path_map_root.as_deref())?;
        let mut root_iter = self.root_prolly.range(&tree, &[], None)?;
        let mut manifest_iter = manifest.iter();
        let mut left = next_root_file(&mut root_iter)?;
        let mut right = manifest_iter.next();
        let mut summaries = Vec::new();

        loop {
            match (left.as_ref(), right) {
                (Some((left_path, left_entry)), Some((right_path, right_entry))) => {
                    match left_path.cmp(right_path) {
                        std::cmp::Ordering::Less => {
                            summaries.push(deleted_manifest_summary(left_path.clone(), left_entry));
                            left = next_root_file(&mut root_iter)?;
                        }
                        std::cmp::Ordering::Greater => {
                            summaries.push(added_manifest_summary(right_path.clone(), right_entry));
                            right = manifest_iter.next();
                        }
                        std::cmp::Ordering::Equal => {
                            if left_entry.content_hash != right_entry.content_hash
                                || left_entry.executable != right_entry.executable
                                || left_entry.kind != right_entry.kind
                            {
                                summaries.push(changed_manifest_summary(
                                    left_path.clone(),
                                    left_entry,
                                    right_entry,
                                ));
                            }
                            left = next_root_file(&mut root_iter)?;
                            right = manifest_iter.next();
                        }
                    }
                }
                (Some((left_path, left_entry)), None) => {
                    summaries.push(deleted_manifest_summary(left_path.clone(), left_entry));
                    left = next_root_file(&mut root_iter)?;
                }
                (None, Some((right_path, right_entry))) => {
                    summaries.push(added_manifest_summary(right_path.clone(), right_entry));
                    right = manifest_iter.next();
                }
                (None, None) => break,
            }
        }
        Ok(summaries)
    }

    pub(crate) fn disk_manifest(&self, disk_files: &[DiskFile]) -> BTreeMap<String, DiskManifest> {
        disk_files
            .iter()
            .map(|file| {
                (
                    file.path.clone(),
                    DiskManifest {
                        kind: classify_file_kind(&file.bytes, &self.config.text),
                        executable: file.executable,
                        content_hash: sha256_hex(&file.bytes),
                    },
                )
            })
            .collect()
    }

    pub(crate) fn diff_file_maps_to_manifest(
        &self,
        left: &BTreeMap<String, FileEntry>,
        right: &BTreeMap<String, DiskManifest>,
    ) -> Vec<FileDiffSummary> {
        let mut paths = BTreeSet::new();
        paths.extend(left.keys().cloned());
        paths.extend(right.keys().cloned());
        let mut summaries = Vec::new();
        for path in paths {
            match (left.get(&path), right.get(&path)) {
                (None, Some(new_entry)) => summaries.push(FileDiffSummary {
                    path,
                    old_path: None,
                    kind: FileChangeKind::Added,
                    before_hash: None,
                    after_hash: Some(new_entry.content_hash.clone()),
                    additions: 0,
                    deletions: 0,
                    line_changes: Vec::new(),
                    patch: None,
                }),
                (Some(old_entry), None) => summaries.push(FileDiffSummary {
                    path,
                    old_path: None,
                    kind: FileChangeKind::Deleted,
                    before_hash: Some(old_entry.content_hash.clone()),
                    after_hash: None,
                    additions: 0,
                    deletions: 0,
                    line_changes: Vec::new(),
                    patch: None,
                }),
                (Some(old_entry), Some(new_entry)) => {
                    if old_entry.content_hash == new_entry.content_hash
                        && old_entry.executable == new_entry.executable
                        && old_entry.kind == new_entry.kind
                    {
                        continue;
                    }
                    summaries.push(FileDiffSummary {
                        path,
                        old_path: None,
                        kind: if old_entry.kind == new_entry.kind {
                            FileChangeKind::Modified
                        } else {
                            FileChangeKind::TypeChanged
                        },
                        before_hash: Some(old_entry.content_hash.clone()),
                        after_hash: Some(new_entry.content_hash.clone()),
                        additions: 0,
                        deletions: 0,
                        line_changes: Vec::new(),
                        patch: None,
                    });
                }
                (None, None) => {}
            }
        }
        summaries
    }

    pub(crate) fn selected_worktree_snapshot(
        &self,
        head_files: &BTreeMap<String, FileEntry>,
        candidate_paths: &[String],
    ) -> Result<SelectedWorktreeSnapshot> {
        let disk_files = self.scan_visible_files_for_paths(candidate_paths)?;
        let disk_paths = disk_files
            .iter()
            .map(|file| file.path.clone())
            .collect::<BTreeSet<_>>();
        self.prune_worktree_index_for_selections(candidate_paths, &disk_paths)?;
        let disk_manifest = self.disk_manifest(&disk_files);
        self.update_worktree_index_from_disk_files_and_manifest(&disk_files, &disk_manifest)?;
        let summaries =
            self.diff_file_maps_to_manifest_for_paths(head_files, &disk_manifest, candidate_paths);
        let paths = summaries
            .iter()
            .map(|summary| summary.path.clone())
            .collect::<Vec<_>>();
        let path_set = paths.iter().map(String::as_str).collect::<HashSet<_>>();
        let files = disk_files
            .into_iter()
            .filter(|file| path_set.contains(file.path.as_str()))
            .collect();
        Ok(SelectedWorktreeSnapshot {
            paths,
            files,
            summaries,
        })
    }

    pub(crate) fn selected_worktree_snapshot_for_root(
        &self,
        root_id: &ObjectId,
        candidate_paths: &[String],
    ) -> Result<SelectedWorktreeSnapshot> {
        let head_files = self.load_root_files_for_selections(root_id, candidate_paths)?;
        self.selected_worktree_snapshot(&head_files, candidate_paths)
    }

    pub(crate) fn diff_file_maps_to_manifest_for_paths(
        &self,
        left: &BTreeMap<String, FileEntry>,
        right: &BTreeMap<String, DiskManifest>,
        selected_paths: &[String],
    ) -> Vec<FileDiffSummary> {
        let mut paths = BTreeSet::new();
        for selected in selected_paths {
            paths.insert(selected.clone());
            paths.extend(
                left.keys()
                    .filter(|path| path_matches_selection(path, selected))
                    .cloned(),
            );
            paths.extend(
                right
                    .keys()
                    .filter(|path| path_matches_selection(path, selected))
                    .cloned(),
            );
        }
        let mut summaries = Vec::new();
        for path in paths {
            match (left.get(&path), right.get(&path)) {
                (None, Some(new_entry)) => summaries.push(FileDiffSummary {
                    path,
                    old_path: None,
                    kind: FileChangeKind::Added,
                    before_hash: None,
                    after_hash: Some(new_entry.content_hash.clone()),
                    additions: 0,
                    deletions: 0,
                    line_changes: Vec::new(),
                    patch: None,
                }),
                (Some(old_entry), None) => summaries.push(FileDiffSummary {
                    path,
                    old_path: None,
                    kind: FileChangeKind::Deleted,
                    before_hash: Some(old_entry.content_hash.clone()),
                    after_hash: None,
                    additions: 0,
                    deletions: 0,
                    line_changes: Vec::new(),
                    patch: None,
                }),
                (Some(old_entry), Some(new_entry)) => {
                    if old_entry.content_hash == new_entry.content_hash
                        && old_entry.executable == new_entry.executable
                        && old_entry.kind == new_entry.kind
                    {
                        continue;
                    }
                    summaries.push(FileDiffSummary {
                        path,
                        old_path: None,
                        kind: if old_entry.kind == new_entry.kind {
                            FileChangeKind::Modified
                        } else {
                            FileChangeKind::TypeChanged
                        },
                        before_hash: Some(old_entry.content_hash.clone()),
                        after_hash: Some(new_entry.content_hash.clone()),
                        additions: 0,
                        deletions: 0,
                        line_changes: Vec::new(),
                        patch: None,
                    });
                }
                (None, None) => {}
            }
        }
        summaries
    }
}

fn added_manifest_summary(path: String, entry: &DiskManifest) -> FileDiffSummary {
    FileDiffSummary {
        path,
        old_path: None,
        kind: FileChangeKind::Added,
        before_hash: None,
        after_hash: Some(entry.content_hash.clone()),
        additions: 0,
        deletions: 0,
        line_changes: Vec::new(),
        patch: None,
    }
}

fn next_root_file<S>(iter: &mut prolly::RangeIter<'_, S>) -> Result<Option<(String, FileEntry)>>
where
    S: prolly::Store,
{
    let Some(item) = iter.next() else {
        return Ok(None);
    };
    let (key, value) = item?;
    let path = String::from_utf8(key)
        .map_err(|err| Error::Corrupt(format!("non UTF-8 path key: {err}")))?;
    Ok(Some((path, from_cbor::<FileEntry>(&value)?)))
}

fn next_index_file(rows: &mut rusqlite::Rows<'_>) -> Result<Option<(String, DiskManifest)>> {
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let path = row.get::<_, String>(0)?;
    let kind = file_kind_from_index(&row.get::<_, String>(1)?)?;
    let executable = row.get::<_, i64>(2)? != 0;
    let content_hash = row.get::<_, String>(3)?;
    Ok(Some((
        path,
        DiskManifest {
            kind,
            executable,
            content_hash,
        },
    )))
}

fn deleted_manifest_summary(path: String, entry: &FileEntry) -> FileDiffSummary {
    FileDiffSummary {
        path,
        old_path: None,
        kind: FileChangeKind::Deleted,
        before_hash: Some(entry.content_hash.clone()),
        after_hash: None,
        additions: 0,
        deletions: 0,
        line_changes: Vec::new(),
        patch: None,
    }
}

fn changed_manifest_summary(
    path: String,
    old_entry: &FileEntry,
    new_entry: &DiskManifest,
) -> FileDiffSummary {
    FileDiffSummary {
        path,
        old_path: None,
        kind: if old_entry.kind == new_entry.kind {
            FileChangeKind::Modified
        } else {
            FileChangeKind::TypeChanged
        },
        before_hash: Some(old_entry.content_hash.clone()),
        after_hash: Some(new_entry.content_hash.clone()),
        additions: 0,
        deletions: 0,
        line_changes: Vec::new(),
        patch: None,
    }
}
