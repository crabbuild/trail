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
        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut changed = Vec::new();
        let mut removed_by_hash: HashMap<String, Vec<(String, FileEntry)>> = HashMap::new();
        for (path, entry) in left {
            match right.get(path) {
                Some(new_entry)
                    if entry.content_hash != new_entry.content_hash
                        || entry.executable != new_entry.executable
                        || entry.kind != new_entry.kind =>
                {
                    changed.push((path.clone(), entry.clone(), new_entry.clone()));
                }
                Some(_) => {}
                None => {
                    removed.push((path.clone(), entry.clone()));
                    removed_by_hash
                        .entry(entry.content_hash.clone())
                        .or_default()
                        .push((path.clone(), entry.clone()));
                }
            }
        }
        for (path, entry) in right {
            if !left.contains_key(path) {
                added.push((path.clone(), entry.clone()));
            }
        }

        let mut changes = Vec::new();
        let mut consumed_removed = HashSet::new();
        for (path, new_entry) in added {
            let rename = removed_by_hash
                .get(&new_entry.content_hash)
                .and_then(|candidates| {
                    candidates.iter().find(|(old_path, old_entry)| {
                        !consumed_removed.contains(old_path)
                            && old_entry.file_id == new_entry.file_id
                    })
                });
            if let Some((old_path, old_entry)) = rename {
                consumed_removed.insert(old_path.clone());
                changes.push(FileChange {
                    path,
                    old_path: Some(old_path.clone()),
                    file_id: Some(new_entry.file_id),
                    kind: FileChangeKind::Renamed,
                    before_hash: Some(old_entry.content_hash.clone()),
                    after_hash: Some(new_entry.content_hash),
                    line_changes: Vec::new(),
                });
            } else {
                let line_changes = self.added_line_changes(&path, &new_entry)?;
                changes.push(FileChange {
                    path,
                    old_path: None,
                    file_id: Some(new_entry.file_id),
                    kind: FileChangeKind::Added,
                    before_hash: None,
                    after_hash: Some(new_entry.content_hash),
                    line_changes,
                });
            }
        }

        for (path, old_entry) in removed {
            if consumed_removed.contains(&path) {
                continue;
            }
            let line_changes = self.deleted_line_changes(&path, &old_entry)?;
            changes.push(FileChange {
                path,
                old_path: None,
                file_id: Some(old_entry.file_id),
                kind: FileChangeKind::Deleted,
                before_hash: Some(old_entry.content_hash),
                after_hash: None,
                line_changes,
            });
        }

        for (path, old_entry, new_entry) in changed {
            let line_changes = self.modified_line_changes(&old_entry, &new_entry)?;
            let kind = if old_entry.kind != new_entry.kind {
                FileChangeKind::TypeChanged
            } else {
                FileChangeKind::Modified
            };
            changes.push(FileChange {
                path,
                old_path: None,
                file_id: Some(new_entry.file_id),
                kind,
                before_hash: Some(old_entry.content_hash),
                after_hash: Some(new_entry.content_hash),
                line_changes,
            });
        }

        changes.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| left.old_path.cmp(&right.old_path))
        });
        let summaries = summarize_file_changes(&changes);
        Ok(RootDiff { changes, summaries })
    }
}
