use super::*;
use crate::db::storage::worktree_index::file_kind_from_index;

impl Trail {
    pub(crate) fn diff_root_to_worktree_index(
        &self,
        root_id: &ObjectId,
    ) -> Result<Vec<FileDiffSummary>> {
        self.note_full_root_path_load();
        let mut root_metrics = OperationMetricsAccumulator::new(
            self.operation_metrics.as_ref(),
            OperationMetricsDelta {
                full_root_range_count: 1,
                ..OperationMetricsDelta::default()
            },
        );
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        let tree = root_map_tree_from_root_hex(root.path_map_root.as_deref())?;
        let mut root_iter = self.root_prolly.range(&tree, &[], None)?;
        let mut stmt = self.conn.prepare(
            "SELECT path, kind, executable, content_hash \
             FROM worktree_file_index ORDER BY path ASC",
        )?;
        let mut index_rows = stmt.query([])?;
        let mut left = next_root_file(&mut root_iter, &mut root_metrics)?;
        let mut right = next_index_file(&mut index_rows)?;
        let mut summaries = Vec::new();

        loop {
            match (&left, &right) {
                (Some((left_path, left_entry)), Some((right_path, right_entry))) => {
                    match left_path.cmp(right_path) {
                        std::cmp::Ordering::Less => {
                            summaries.push(deleted_manifest_summary(left_path.clone(), left_entry));
                            left = next_root_file(&mut root_iter, &mut root_metrics)?;
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
                            left = next_root_file(&mut root_iter, &mut root_metrics)?;
                            right = next_index_file(&mut index_rows)?;
                        }
                    }
                }
                (Some((left_path, left_entry)), None) => {
                    summaries.push(deleted_manifest_summary(left_path.clone(), left_entry));
                    left = next_root_file(&mut root_iter, &mut root_metrics)?;
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
        self.note_full_root_path_load();
        let mut root_metrics = OperationMetricsAccumulator::new(
            self.operation_metrics.as_ref(),
            OperationMetricsDelta {
                full_root_range_count: 1,
                ..OperationMetricsDelta::default()
            },
        );
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        let tree = root_map_tree_from_root_hex(root.path_map_root.as_deref())?;
        let mut root_iter = self.root_prolly.range(&tree, &[], None)?;
        let mut manifest_iter = manifest.iter();
        let mut left = next_root_file(&mut root_iter, &mut root_metrics)?;
        let mut right = manifest_iter.next();
        let mut summaries = Vec::new();

        loop {
            match (left.as_ref(), right) {
                (Some((left_path, left_entry)), Some((right_path, right_entry))) => {
                    match left_path.cmp(right_path) {
                        std::cmp::Ordering::Less => {
                            summaries.push(deleted_manifest_summary(left_path.clone(), left_entry));
                            left = next_root_file(&mut root_iter, &mut root_metrics)?;
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
                            left = next_root_file(&mut root_iter, &mut root_metrics)?;
                            right = manifest_iter.next();
                        }
                    }
                }
                (Some((left_path, left_entry)), None) => {
                    summaries.push(deleted_manifest_summary(left_path.clone(), left_entry));
                    left = next_root_file(&mut root_iter, &mut root_metrics)?;
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
        self.note_operation_metrics(OperationMetricsDelta {
            filesystem_hash_count: saturating_u64_from_usize(disk_files.len()),
            filesystem_hash_bytes: disk_files.iter().fold(0u64, |total, file| {
                total.saturating_add(saturating_u64_from_usize(file.bytes.len()))
            }),
            ..OperationMetricsDelta::default()
        });
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
        self.note_operation_metrics(OperationMetricsDelta {
            manifest_key_comparison_count: saturating_u64_from_usize(paths.len()),
            ..OperationMetricsDelta::default()
        });
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

    pub(crate) fn selected_worktree_snapshot_with_policy(
        &self,
        head_files: &BTreeMap<String, FileEntry>,
        candidate_paths: &[String],
        policy: &WorkspaceIgnorePolicySnapshot,
    ) -> Result<SelectedWorktreeSnapshot> {
        let disk_files = self.scan_visible_files_for_paths_with_policy(candidate_paths, policy)?;
        let disk_paths = disk_files
            .iter()
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        let disk_manifest = self.disk_manifest(&disk_files);
        self.sync_selected_worktree_index(candidate_paths, &disk_paths, &disk_manifest)?;
        let summaries =
            self.diff_file_maps_to_manifest_for_paths(head_files, &disk_manifest, candidate_paths)?;
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

    pub(crate) fn selected_worktree_snapshot_read_only_with_policy(
        &self,
        head_files: &BTreeMap<String, FileEntry>,
        candidate_paths: &[String],
        policy: &WorkspaceIgnorePolicySnapshot,
    ) -> Result<SelectedWorktreeSnapshot> {
        let disk_files = self.scan_visible_files_for_paths_with_policy(candidate_paths, policy)?;
        let disk_manifest = self.disk_manifest(&disk_files);
        let summaries =
            self.diff_file_maps_to_manifest_for_paths(head_files, &disk_manifest, candidate_paths)?;
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

    #[cfg(test)]
    pub(crate) fn selected_worktree_snapshot_for_root(
        &self,
        root_id: &ObjectId,
        candidate_paths: &[String],
    ) -> Result<SelectedWorktreeSnapshot> {
        let policy = self.workspace_ignore_policy_snapshot();
        self.selected_worktree_snapshot_for_root_with_policy(root_id, candidate_paths, &policy)
    }

    pub(crate) fn selected_worktree_snapshot_for_root_with_policy(
        &self,
        root_id: &ObjectId,
        candidate_paths: &[String],
        policy: &WorkspaceIgnorePolicySnapshot,
    ) -> Result<SelectedWorktreeSnapshot> {
        let head_files = self.load_root_files_for_selections(root_id, candidate_paths)?;
        self.selected_worktree_snapshot_with_policy(&head_files, candidate_paths, policy)
    }

    pub(crate) fn selected_worktree_snapshot_for_root_read_only_with_policy(
        &self,
        root_id: &ObjectId,
        candidate_paths: &[String],
        policy: &WorkspaceIgnorePolicySnapshot,
    ) -> Result<SelectedWorktreeSnapshot> {
        let head_files = self.load_root_files_for_selections(root_id, candidate_paths)?;
        self.selected_worktree_snapshot_read_only_with_policy(&head_files, candidate_paths, policy)
    }

    pub(crate) fn diff_file_maps_to_manifest_for_paths(
        &self,
        left: &BTreeMap<String, FileEntry>,
        right: &BTreeMap<String, DiskManifest>,
        selected_paths: &[String],
    ) -> Result<Vec<FileDiffSummary>> {
        let selections = SelectionSet::from_paths(selected_paths)?;
        if selections.as_slice().is_empty() {
            return Ok(Vec::new());
        }
        let mut paths = BTreeSet::new();
        paths.extend(left.keys().cloned());
        paths.extend(right.keys().cloned());
        let mut selection_comparison_count = 0u64;
        paths.retain(|path| {
            let (selected, comparisons) = selections.contains_counted(path);
            selection_comparison_count = selection_comparison_count.saturating_add(comparisons);
            selected
        });
        self.note_operation_metrics(OperationMetricsDelta {
            selection_comparison_count,
            ..OperationMetricsDelta::default()
        });
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        self.note_operation_metrics(OperationMetricsDelta {
            manifest_key_comparison_count: saturating_u64_from_usize(paths.len()),
            ..OperationMetricsDelta::default()
        });
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
        Ok(summaries)
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

fn next_root_file<S>(
    iter: &mut prolly::RangeIter<'_, S>,
    metrics: &mut OperationMetricsAccumulator,
) -> Result<Option<(String, FileEntry)>>
where
    S: prolly::Store,
{
    let Some(item) = iter.next() else {
        return Ok(None);
    };
    let (key, value) = item?;
    metrics.delta.root_range_row_count = metrics.delta.root_range_row_count.saturating_add(1);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_fixture() -> (tempfile::TempDir, Trail, FileEntry) {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "hello\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let head = db.resolve_branch_ref("main").unwrap();
        let entry = db
            .load_root_files_for_paths(&head.root_id, &["README.md".to_string()])
            .unwrap()
            .remove("README.md")
            .unwrap();
        (workspace, db, entry)
    }

    fn disk_manifest(entry: &FileEntry) -> DiskManifest {
        DiskManifest {
            kind: entry.kind.clone(),
            executable: entry.executable,
            content_hash: entry.content_hash.clone(),
        }
    }

    #[test]
    fn selected_manifest_key_union_preserves_both_exact_rename_endpoints() {
        let (_workspace, db, entry) = manifest_fixture();
        let left = BTreeMap::from([("old.txt".to_string(), entry.clone())]);
        let right = BTreeMap::from([("new.txt".to_string(), disk_manifest(&entry))]);

        let summaries = db
            .diff_file_maps_to_manifest_for_paths(
                &left,
                &right,
                &["old.txt".to_string(), "new.txt".to_string()],
            )
            .unwrap();

        assert_eq!(
            summaries
                .iter()
                .map(|summary| (summary.path.as_str(), summary.kind.clone()))
                .collect::<Vec<_>>(),
            [
                ("new.txt", FileChangeKind::Added),
                ("old.txt", FileChangeKind::Deleted),
            ]
        );
    }

    #[test]
    fn selected_manifest_key_union_filters_once_with_overlap_and_case_aliases() {
        let (_workspace, db, entry) = manifest_fixture();
        let mut changed_manifest = disk_manifest(&entry);
        changed_manifest.content_hash = "different".to_string();
        let left = BTreeMap::from([
            ("README.md".to_string(), entry.clone()),
            ("docs/old.txt".to_string(), entry.clone()),
            ("unrelated/left.txt".to_string(), entry.clone()),
        ]);
        let right = BTreeMap::from([
            ("docs/new.txt".to_string(), disk_manifest(&entry)),
            ("readme.md".to_string(), disk_manifest(&entry)),
            ("unrelated/right.txt".to_string(), changed_manifest),
        ]);
        let metrics = db.operation_metrics.as_ref().unwrap();
        let before = metrics.snapshot();

        let summaries = db
            .diff_file_maps_to_manifest_for_paths(
                &left,
                &right,
                &[
                    "docs/new.txt".to_string(),
                    "docs".to_string(),
                    "README.md".to_string(),
                    "readme.md".to_string(),
                ],
            )
            .unwrap();
        let after = metrics.snapshot();

        assert_eq!(
            summaries
                .iter()
                .map(|summary| summary.path.as_str())
                .collect::<Vec<_>>(),
            ["README.md", "docs/new.txt", "docs/old.txt", "readme.md"]
        );
        assert!(
            after.selection_comparison_count - before.selection_comparison_count <= 12,
            "each of six union keys should need at most exact plus interval membership"
        );
        assert_eq!(
            after.manifest_key_comparison_count - before.manifest_key_comparison_count,
            4
        );
    }

    #[test]
    fn selected_manifest_empty_selection_does_not_scan_maps_or_report_membership_work() {
        let (_workspace, db, entry) = manifest_fixture();
        let left = (0..1_000)
            .map(|idx| (format!("left/{idx:04}.txt"), entry.clone()))
            .collect::<BTreeMap<_, _>>();
        let right = (0..1_000)
            .map(|idx| (format!("right/{idx:04}.txt"), disk_manifest(&entry)))
            .collect::<BTreeMap<_, _>>();
        let metrics = db.operation_metrics.as_ref().unwrap();
        let before = metrics.snapshot();

        let summaries = db
            .diff_file_maps_to_manifest_for_paths(&left, &right, &[])
            .unwrap();
        let after = metrics.snapshot();

        assert!(summaries.is_empty());
        assert_eq!(
            after.selection_comparison_count - before.selection_comparison_count,
            0
        );
        assert_eq!(
            after.manifest_key_comparison_count - before.manifest_key_comparison_count,
            0
        );
    }
}
