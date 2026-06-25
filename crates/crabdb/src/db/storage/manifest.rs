use super::*;

impl CrabDb {
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
        let disk_manifest = self.disk_manifest(&disk_files);
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

    pub(crate) fn diff_file_maps_to_manifest_for_paths(
        &self,
        left: &BTreeMap<String, FileEntry>,
        right: &BTreeMap<String, DiskManifest>,
        selected_paths: &[String],
    ) -> Vec<FileDiffSummary> {
        let mut paths = BTreeSet::new();
        paths.extend(selected_paths.iter().cloned());
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
