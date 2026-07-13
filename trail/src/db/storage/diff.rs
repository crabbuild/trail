use super::*;
use std::collections::VecDeque;

#[derive(Default)]
pub(super) struct RenameMatchIndex {
    by_hash_and_file_id: HashMap<String, HashMap<FileId, VecDeque<(String, FileEntry)>>>,
}

impl RenameMatchIndex {
    pub(super) fn insert(&mut self, path: String, entry: FileEntry) {
        self.by_hash_and_file_id
            .entry(entry.content_hash.clone())
            .or_default()
            .entry(entry.file_id.clone())
            .or_default()
            .push_back((path, entry));
    }

    pub(super) fn take(&mut self, entry: &FileEntry) -> Option<(String, FileEntry)> {
        self.by_hash_and_file_id
            .get_mut(&entry.content_hash)
            .and_then(|by_file_id| by_file_id.get_mut(&entry.file_id))
            .and_then(VecDeque::pop_front)
    }
}

#[derive(Default)]
pub(super) struct RenameLookupProbes {
    #[cfg(test)]
    count: u64,
}

impl RenameLookupProbes {
    #[inline]
    pub(super) fn note_lookup(&mut self) {
        #[cfg(test)]
        {
            self.count = self.count.saturating_add(1);
        }
    }

    #[cfg(test)]
    pub(super) fn count(&self) -> u64 {
        self.count
    }
}

impl Trail {
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
        let mut probes = RenameLookupProbes::default();
        self.diff_file_maps_indexed(left, right, &mut probes)
    }

    #[cfg(test)]
    fn diff_file_maps_with_rename_probe_count(
        &self,
        left: &BTreeMap<String, FileEntry>,
        right: &BTreeMap<String, FileEntry>,
    ) -> Result<(RootDiff, u64)> {
        let mut probes = RenameLookupProbes::default();
        let diff = self.diff_file_maps_indexed(left, right, &mut probes)?;
        Ok((diff, probes.count()))
    }

    fn diff_file_maps_indexed(
        &self,
        left: &BTreeMap<String, FileEntry>,
        right: &BTreeMap<String, FileEntry>,
        probes: &mut RenameLookupProbes,
    ) -> Result<RootDiff> {
        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut changed = Vec::new();
        let mut removed_by_identity = RenameMatchIndex::default();
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
                    removed_by_identity.insert(path.clone(), entry.clone());
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
            probes.note_lookup();
            let rename = removed_by_identity.take(&new_entry);
            if let Some((old_path, old_entry)) = rename {
                consumed_removed.insert(old_path.clone());
                changes.push(FileChange {
                    path,
                    old_path: Some(old_path),
                    file_id: Some(new_entry.file_id),
                    kind: FileChangeKind::Renamed,
                    before_hash: Some(old_entry.content_hash),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_entry(db: &Trail, root_id: &ObjectId) -> FileEntry {
        db.load_root_files(root_id)
            .unwrap()
            .into_values()
            .next()
            .unwrap()
    }

    #[test]
    fn indexed_rename_matching_is_linear_for_same_content_and_preserves_identity() {
        const FILE_COUNT: usize = 1_000;

        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("seed.bin"), [0, 1, 2, 3]).unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let template = fixture_entry(&db, &head.root_id);
        let mut left = BTreeMap::new();
        let mut right = BTreeMap::new();
        for index in 0..FILE_COUNT {
            let mut entry = template.clone();
            entry.file_id = FileId::new(ChangeId("change_rename_source".to_string()), index as u64);
            left.insert(format!("old/{index:04}.bin"), entry.clone());
            right.insert(format!("new/{index:04}.bin"), entry);
        }

        let (diff, rename_identity_lookup_count) = db
            .diff_file_maps_with_rename_probe_count(&left, &right)
            .unwrap();

        assert_eq!(diff.changes.len(), FILE_COUNT);
        assert_eq!(diff.summaries.len(), FILE_COUNT);
        assert_eq!(rename_identity_lookup_count, FILE_COUNT as u64);
        for (index, change) in diff.changes.iter().enumerate() {
            assert_eq!(change.kind, FileChangeKind::Renamed);
            assert_eq!(change.path, format!("new/{index:04}.bin"));
            assert_eq!(
                change.old_path.as_deref(),
                Some(format!("old/{index:04}.bin").as_str())
            );
            assert_eq!(
                change.file_id,
                Some(FileId::new(
                    ChangeId("change_rename_source".to_string()),
                    index as u64,
                ))
            );
            assert_eq!(diff.summaries[index].path, change.path);
            assert_eq!(diff.summaries[index].old_path, change.old_path);
            assert_eq!(diff.summaries[index].kind, FileChangeKind::Renamed);
        }
    }

    #[test]
    fn indexed_rename_matching_preserves_first_unconsumed_path_order() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("seed.bin"), [0, 1, 2, 3]).unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let entry = fixture_entry(&db, &head.root_id);
        let left = [
            ("old/a.bin".to_string(), entry.clone()),
            ("old/b.bin".to_string(), entry.clone()),
        ]
        .into_iter()
        .collect();
        let right = [
            ("new/a.bin".to_string(), entry.clone()),
            ("new/b.bin".to_string(), entry),
        ]
        .into_iter()
        .collect();

        let diff = db.diff_file_maps(&left, &right).unwrap();

        assert_eq!(diff.changes.len(), 2);
        assert_eq!(diff.changes[0].path, "new/a.bin");
        assert_eq!(diff.changes[0].old_path.as_deref(), Some("old/a.bin"));
        assert_eq!(diff.changes[1].path, "new/b.bin");
        assert_eq!(diff.changes[1].old_path.as_deref(), Some("old/b.bin"));
    }
}
