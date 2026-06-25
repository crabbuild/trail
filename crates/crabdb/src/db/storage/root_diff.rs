use super::*;

impl CrabDb {
    pub(crate) fn diff_root_file_summaries(
        &self,
        left_root_id: &ObjectId,
        right_root_id: &ObjectId,
    ) -> Result<Vec<FileDiffSummary>> {
        let left_root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, left_root_id)?;
        let right_root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, right_root_id)?;
        let left_tree = worktree_root_map_tree_from_root_hex(left_root.path_map_root.as_deref())?;
        let right_tree = worktree_root_map_tree_from_root_hex(right_root.path_map_root.as_deref())?;
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
        let mut removed_by_hash: HashMap<String, Vec<(String, FileEntry)>> = HashMap::new();
        for (path, entry) in &removed {
            removed_by_hash
                .entry(entry.content_hash.clone())
                .or_default()
                .push((path.clone(), entry.clone()));
        }

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
                summaries.push(file_diff_summary(
                    path,
                    Some(old_path.clone()),
                    FileChangeKind::Renamed,
                    Some(old_entry.content_hash.clone()),
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
        let left_root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, left_root_id)?;
        let right_root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, right_root_id)?;
        let left_tree = worktree_root_map_tree_from_root_hex(left_root.path_map_root.as_deref())?;
        let right_tree = worktree_root_map_tree_from_root_hex(right_root.path_map_root.as_deref())?;
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
        let mut removed_by_hash: HashMap<String, Vec<(String, FileEntry)>> = HashMap::new();
        for (path, entry) in &removed {
            removed_by_hash
                .entry(entry.content_hash.clone())
                .or_default()
                .push((path.clone(), entry.clone()));
        }

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
                patch_left.insert(old_path.clone(), old_entry.clone());
                patch_right.insert(path.clone(), new_entry.clone());
                changes.push(FileChange {
                    path,
                    old_path: Some(old_path.clone()),
                    file_id: Some(new_entry.file_id),
                    kind: FileChangeKind::Renamed,
                    before_hash: Some(old_entry.content_hash.clone()),
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
