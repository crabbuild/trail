use super::*;

impl CrabDb {
    pub(crate) fn diff_root_files(
        &self,
        from: String,
        to: String,
        left_root_id: &ObjectId,
        right_root_id: &ObjectId,
        patches: bool,
        include_line_changes: bool,
    ) -> Result<DiffSummary> {
        let mut patch_left = BTreeMap::new();
        let mut patch_right = BTreeMap::new();
        let mut diff = self.diff_root_file_maps(
            left_root_id,
            right_root_id,
            &mut patch_left,
            &mut patch_right,
        )?;
        if include_line_changes {
            attach_line_changes(&diff.changes, &mut diff.summaries);
        }
        if patches {
            self.attach_patches(&patch_left, &patch_right, &mut diff.summaries)?;
        }
        Ok(DiffSummary {
            from,
            to,
            files: diff.summaries,
        })
    }

    pub(crate) fn diff_files(
        &self,
        from: String,
        to: String,
        left: &BTreeMap<String, FileEntry>,
        right: &BTreeMap<String, FileEntry>,
        patches: bool,
        include_line_changes: bool,
    ) -> Result<DiffSummary> {
        let mut diff = self.diff_file_maps(left, right)?;
        if include_line_changes {
            attach_line_changes(&diff.changes, &mut diff.summaries);
        }
        if patches {
            self.attach_patches(left, right, &mut diff.summaries)?;
        }
        Ok(DiffSummary {
            from,
            to,
            files: diff.summaries,
        })
    }

    pub(crate) fn diff_file_maps(
        &self,
        left: &BTreeMap<String, FileEntry>,
        right: &BTreeMap<String, FileEntry>,
    ) -> Result<RootDiff> {
        let mut paths = BTreeSet::new();
        paths.extend(left.keys().cloned());
        paths.extend(right.keys().cloned());
        let mut changes = Vec::new();
        let mut summaries = Vec::new();
        let mut removed_by_hash: HashMap<String, Vec<(String, FileEntry)>> = HashMap::new();
        for (path, entry) in left {
            if !right.contains_key(path) {
                removed_by_hash
                    .entry(entry.content_hash.clone())
                    .or_default()
                    .push((path.clone(), entry.clone()));
            }
        }

        let mut handled_renames = HashSet::new();
        for path in paths {
            let old = left.get(&path);
            let new = right.get(&path);
            match (old, new) {
                (None, Some(new_entry)) => {
                    let rename = removed_by_hash
                        .get(&new_entry.content_hash)
                        .and_then(|candidates| candidates.first());
                    if let Some((old_path, old_entry)) = rename {
                        if old_entry.file_id == new_entry.file_id {
                            handled_renames.insert(old_path.clone());
                            let change = FileChange {
                                path: path.clone(),
                                old_path: Some(old_path.clone()),
                                file_id: Some(new_entry.file_id.clone()),
                                kind: FileChangeKind::Renamed,
                                before_hash: Some(old_entry.content_hash.clone()),
                                after_hash: Some(new_entry.content_hash.clone()),
                                line_changes: Vec::new(),
                            };
                            summaries.push(FileDiffSummary {
                                path: path.clone(),
                                old_path: Some(old_path.clone()),
                                kind: FileChangeKind::Renamed,
                                before_hash: Some(old_entry.content_hash.clone()),
                                after_hash: Some(new_entry.content_hash.clone()),
                                additions: 0,
                                deletions: 0,
                                line_changes: Vec::new(),
                                patch: None,
                            });
                            changes.push(change);
                            continue;
                        }
                    }
                    let line_changes = self.added_line_changes(&path, new_entry)?;
                    let (adds, dels) = count_line_delta(&line_changes);
                    changes.push(FileChange {
                        path: path.clone(),
                        old_path: None,
                        file_id: Some(new_entry.file_id.clone()),
                        kind: FileChangeKind::Added,
                        before_hash: None,
                        after_hash: Some(new_entry.content_hash.clone()),
                        line_changes,
                    });
                    summaries.push(FileDiffSummary {
                        path,
                        old_path: None,
                        kind: FileChangeKind::Added,
                        before_hash: None,
                        after_hash: Some(new_entry.content_hash.clone()),
                        additions: adds,
                        deletions: dels,
                        line_changes: Vec::new(),
                        patch: None,
                    });
                }
                (Some(old_entry), None) => {
                    if handled_renames.contains(&path) {
                        continue;
                    }
                    let line_changes = self.deleted_line_changes(&path, old_entry)?;
                    let (adds, dels) = count_line_delta(&line_changes);
                    changes.push(FileChange {
                        path: path.clone(),
                        old_path: None,
                        file_id: Some(old_entry.file_id.clone()),
                        kind: FileChangeKind::Deleted,
                        before_hash: Some(old_entry.content_hash.clone()),
                        after_hash: None,
                        line_changes,
                    });
                    summaries.push(FileDiffSummary {
                        path,
                        old_path: None,
                        kind: FileChangeKind::Deleted,
                        before_hash: Some(old_entry.content_hash.clone()),
                        after_hash: None,
                        additions: adds,
                        deletions: dels,
                        line_changes: Vec::new(),
                        patch: None,
                    });
                }
                (Some(old_entry), Some(new_entry)) => {
                    if old_entry.content_hash == new_entry.content_hash
                        && old_entry.executable == new_entry.executable
                        && old_entry.kind == new_entry.kind
                    {
                        continue;
                    }
                    let line_changes = self.modified_line_changes(old_entry, new_entry)?;
                    let (adds, dels) = count_line_delta(&line_changes);
                    let kind = if old_entry.kind != new_entry.kind {
                        FileChangeKind::TypeChanged
                    } else {
                        FileChangeKind::Modified
                    };
                    changes.push(FileChange {
                        path: path.clone(),
                        old_path: None,
                        file_id: Some(new_entry.file_id.clone()),
                        kind: kind.clone(),
                        before_hash: Some(old_entry.content_hash.clone()),
                        after_hash: Some(new_entry.content_hash.clone()),
                        line_changes,
                    });
                    summaries.push(FileDiffSummary {
                        path,
                        old_path: None,
                        kind,
                        before_hash: Some(old_entry.content_hash.clone()),
                        after_hash: Some(new_entry.content_hash.clone()),
                        additions: adds,
                        deletions: dels,
                        line_changes: Vec::new(),
                        patch: None,
                    });
                }
                (None, None) => {}
            }
        }
        Ok(RootDiff { changes, summaries })
    }
}
