use super::diff::{RenameLookupProbes, RenameMatchIndex};
use super::*;

impl Trail {
    pub(crate) fn visit_root_file_entries<F>(
        &self,
        root_id: &ObjectId,
        prefixes: &[String],
        mut visitor: F,
    ) -> Result<()>
    where
        F: FnMut(String, FileEntry) -> Result<()>,
    {
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        let tree = root_map_tree_from_root_hex(root.path_map_root.as_deref())?;
        if prefixes.is_empty() {
            for item in self.root_prolly.range(&tree, &[], None)? {
                let (key, value) = item?;
                visitor(path_from_key(key)?, from_cbor::<FileEntry>(&value)?)?;
            }
            return Ok(());
        }

        for prefix in prefixes {
            if let Some(value) = self.root_prolly.get(&tree, prefix.as_bytes())? {
                visitor(prefix.clone(), from_cbor::<FileEntry>(&value)?)?;
            }
            let start = format!("{prefix}/");
            let end = format!("{prefix}0");
            for item in self
                .root_prolly
                .range(&tree, start.as_bytes(), Some(end.as_bytes()))?
            {
                let (key, value) = item?;
                visitor(path_from_key(key)?, from_cbor::<FileEntry>(&value)?)?;
            }
        }
        Ok(())
    }

    pub(crate) fn diff_root_file_summaries(
        &self,
        left_root_id: &ObjectId,
        right_root_id: &ObjectId,
    ) -> Result<Vec<FileDiffSummary>> {
        let mut probes = RenameLookupProbes::default();
        self.diff_root_file_summaries_indexed(left_root_id, right_root_id, &mut probes)
    }

    #[cfg(test)]
    fn diff_root_file_summaries_with_rename_probe_count(
        &self,
        left_root_id: &ObjectId,
        right_root_id: &ObjectId,
    ) -> Result<(Vec<FileDiffSummary>, u64)> {
        let mut probes = RenameLookupProbes::default();
        let summaries =
            self.diff_root_file_summaries_indexed(left_root_id, right_root_id, &mut probes)?;
        Ok((summaries, probes.count()))
    }

    fn diff_root_file_summaries_indexed(
        &self,
        left_root_id: &ObjectId,
        right_root_id: &ObjectId,
        probes: &mut RenameLookupProbes,
    ) -> Result<Vec<FileDiffSummary>> {
        let left_root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, left_root_id)?;
        let right_root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, right_root_id)?;
        let left_tree = root_map_tree_from_root_hex(left_root.path_map_root.as_deref())?;
        let right_tree = root_map_tree_from_root_hex(right_root.path_map_root.as_deref())?;
        let diffs = self
            .root_prolly
            .range_diff(&left_tree, &right_tree, &[], None)?;

        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut changed = Vec::new();
        for diff in diffs {
            match diff {
                Diff::Added { key, val } => {
                    added.push((path_from_key(key)?, from_cbor::<FileEntry>(&val)?));
                }
                Diff::Removed { key, val } => {
                    removed.push((path_from_key(key)?, from_cbor::<FileEntry>(&val)?));
                }
                Diff::Changed { key, old, new } => {
                    changed.push((
                        path_from_key(key)?,
                        from_cbor::<FileEntry>(&old)?,
                        from_cbor::<FileEntry>(&new)?,
                    ));
                }
            }
        }

        added.sort_by(|left, right| left.0.cmp(&right.0));
        removed.sort_by(|left, right| left.0.cmp(&right.0));
        changed.sort_by(|left, right| left.0.cmp(&right.0));

        let mut summaries = Vec::new();
        let mut removed_by_identity = RenameMatchIndex::default();
        for (path, entry) in &removed {
            removed_by_identity.insert(path.clone(), entry.clone());
        }

        let mut consumed_removed = HashSet::new();
        for (path, new_entry) in added {
            probes.note_lookup();
            let rename = removed_by_identity.take(&new_entry);
            if let Some((old_path, old_entry)) = rename {
                consumed_removed.insert(old_path.clone());
                summaries.push(file_diff_summary(
                    path,
                    Some(old_path),
                    FileChangeKind::Renamed,
                    Some(old_entry.content_hash),
                    Some(new_entry.content_hash),
                ));
                continue;
            }

            summaries.push(file_diff_summary(
                path,
                None,
                FileChangeKind::Added,
                None,
                Some(new_entry.content_hash),
            ));
        }

        for (path, old_entry) in removed {
            if consumed_removed.contains(&path) {
                continue;
            }
            summaries.push(file_diff_summary(
                path,
                None,
                FileChangeKind::Deleted,
                Some(old_entry.content_hash),
                None,
            ));
        }

        for (path, old_entry, new_entry) in changed {
            if old_entry.content_hash == new_entry.content_hash
                && old_entry.executable == new_entry.executable
                && old_entry.kind == new_entry.kind
            {
                continue;
            }
            let kind = if old_entry.kind != new_entry.kind {
                FileChangeKind::TypeChanged
            } else {
                FileChangeKind::Modified
            };
            summaries.push(file_diff_summary(
                path,
                None,
                kind,
                Some(old_entry.content_hash),
                Some(new_entry.content_hash),
            ));
        }

        summaries.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| left.old_path.cmp(&right.old_path))
        });
        Ok(summaries)
    }

    pub(crate) fn diff_root_file_maps(
        &self,
        left_root_id: &ObjectId,
        right_root_id: &ObjectId,
        patch_left: &mut BTreeMap<String, FileEntry>,
        patch_right: &mut BTreeMap<String, FileEntry>,
    ) -> Result<RootDiff> {
        let mut probes = RenameLookupProbes::default();
        self.diff_root_file_maps_indexed(
            left_root_id,
            right_root_id,
            patch_left,
            patch_right,
            &mut probes,
        )
    }

    #[cfg(test)]
    fn diff_root_file_maps_with_rename_probe_count(
        &self,
        left_root_id: &ObjectId,
        right_root_id: &ObjectId,
        patch_left: &mut BTreeMap<String, FileEntry>,
        patch_right: &mut BTreeMap<String, FileEntry>,
    ) -> Result<(RootDiff, u64)> {
        let mut probes = RenameLookupProbes::default();
        let diff = self.diff_root_file_maps_indexed(
            left_root_id,
            right_root_id,
            patch_left,
            patch_right,
            &mut probes,
        )?;
        Ok((diff, probes.count()))
    }

    fn diff_root_file_maps_indexed(
        &self,
        left_root_id: &ObjectId,
        right_root_id: &ObjectId,
        patch_left: &mut BTreeMap<String, FileEntry>,
        patch_right: &mut BTreeMap<String, FileEntry>,
        probes: &mut RenameLookupProbes,
    ) -> Result<RootDiff> {
        let left_root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, left_root_id)?;
        let right_root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, right_root_id)?;
        let left_tree = root_map_tree_from_root_hex(left_root.path_map_root.as_deref())?;
        let right_tree = root_map_tree_from_root_hex(right_root.path_map_root.as_deref())?;
        let diffs = self
            .root_prolly
            .range_diff(&left_tree, &right_tree, &[], None)?;

        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut changed = Vec::new();
        for diff in diffs {
            match diff {
                Diff::Added { key, val } => {
                    added.push((path_from_key(key)?, from_cbor::<FileEntry>(&val)?));
                }
                Diff::Removed { key, val } => {
                    removed.push((path_from_key(key)?, from_cbor::<FileEntry>(&val)?));
                }
                Diff::Changed { key, old, new } => {
                    changed.push((
                        path_from_key(key)?,
                        from_cbor::<FileEntry>(&old)?,
                        from_cbor::<FileEntry>(&new)?,
                    ));
                }
            }
        }

        added.sort_by(|left, right| left.0.cmp(&right.0));
        removed.sort_by(|left, right| left.0.cmp(&right.0));
        changed.sort_by(|left, right| left.0.cmp(&right.0));

        let mut changes = Vec::new();
        let mut removed_by_identity = RenameMatchIndex::default();
        for (path, entry) in &removed {
            removed_by_identity.insert(path.clone(), entry.clone());
        }

        let mut consumed_removed = HashSet::new();
        for (path, new_entry) in added {
            probes.note_lookup();
            let rename = removed_by_identity.take(&new_entry);
            if let Some((old_path, old_entry)) = rename {
                consumed_removed.insert(old_path.clone());
                patch_left.insert(old_path.clone(), old_entry.clone());
                patch_right.insert(path.clone(), new_entry.clone());
                changes.push(FileChange {
                    path,
                    old_path: Some(old_path),
                    file_id: Some(new_entry.file_id),
                    kind: FileChangeKind::Renamed,
                    before_hash: Some(old_entry.content_hash),
                    after_hash: Some(new_entry.content_hash),
                    line_changes: Vec::new(),
                });
                continue;
            }

            patch_right.insert(path.clone(), new_entry.clone());
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

        for (path, old_entry) in removed {
            if consumed_removed.contains(&path) {
                continue;
            }
            patch_left.insert(path.clone(), old_entry.clone());
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
            if old_entry.content_hash == new_entry.content_hash
                && old_entry.executable == new_entry.executable
                && old_entry.kind == new_entry.kind
            {
                continue;
            }
            patch_left.insert(path.clone(), old_entry.clone());
            patch_right.insert(path.clone(), new_entry.clone());
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
fn path_from_key(key: Vec<u8>) -> Result<String> {
    String::from_utf8(key).map_err(|err| Error::Corrupt(format!("non UTF-8 path key: {err}")))
}

fn file_diff_summary(
    path: String,
    old_path: Option<String>,
    kind: FileChangeKind,
    before_hash: Option<String>,
    after_hash: Option<String>,
) -> FileDiffSummary {
    FileDiffSummary {
        path,
        old_path,
        kind,
        before_hash,
        after_hash,
        additions: 0,
        deletions: 0,
        line_changes: Vec::new(),
        patch: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_diff_rename_matching_is_linear_for_same_content() {
        const FILE_COUNT: usize = 1_000;

        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("seed.bin"), [0, 1, 2, 3]).unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let template = db
            .load_root_files(&head.root_id)
            .unwrap()
            .into_values()
            .next()
            .unwrap();
        let mut left = BTreeMap::new();
        let mut right = BTreeMap::new();
        for index in 0..FILE_COUNT {
            let mut entry = template.clone();
            entry.file_id = FileId::new(
                ChangeId("change_root_diff_source".to_string()),
                index as u64,
            );
            left.insert(format!("old/{index:04}.bin"), entry.clone());
            right.insert(format!("new/{index:04}.bin"), entry);
        }
        let left_root = db
            .build_root_from_file_entries(left, &ChangeId("change_root_diff_left".to_string()))
            .unwrap();
        let right_root = db
            .build_root_from_file_entries(right, &ChangeId("change_root_diff_right".to_string()))
            .unwrap();

        let (summaries, summary_lookups) = db
            .diff_root_file_summaries_with_rename_probe_count(
                &left_root.root_id,
                &right_root.root_id,
            )
            .unwrap();
        let mut patch_left = BTreeMap::new();
        let mut patch_right = BTreeMap::new();
        let (diff, map_lookups) = db
            .diff_root_file_maps_with_rename_probe_count(
                &left_root.root_id,
                &right_root.root_id,
                &mut patch_left,
                &mut patch_right,
            )
            .unwrap();

        assert_eq!(summary_lookups, FILE_COUNT as u64);
        assert_eq!(map_lookups, FILE_COUNT as u64);
        assert_eq!(summaries.len(), FILE_COUNT);
        assert_eq!(diff.changes.len(), FILE_COUNT);
        assert_eq!(diff.summaries.len(), summaries.len());
        for (map_summary, summary) in diff.summaries.iter().zip(&summaries) {
            assert_eq!(map_summary.path, summary.path);
            assert_eq!(map_summary.old_path, summary.old_path);
            assert_eq!(map_summary.kind, summary.kind);
            assert_eq!(map_summary.before_hash, summary.before_hash);
            assert_eq!(map_summary.after_hash, summary.after_hash);
        }
        assert_eq!(patch_left.len(), FILE_COUNT);
        assert_eq!(patch_right.len(), FILE_COUNT);
        assert!(summaries
            .iter()
            .all(|summary| summary.kind == FileChangeKind::Renamed));
    }
}
