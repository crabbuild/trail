use super::*;

impl CrabDb {
    pub fn status(&self, branch: Option<&str>) -> Result<StatusReport> {
        let current_branch = self.current_branch()?;
        let branch = branch.map(str::to_string).unwrap_or(current_branch.clone());
        let head = self.resolve_branch_ref(&branch)?;
        let head_files = self.load_root_files(&head.root_id)?;
        let changed_paths = if branch == current_branch {
            if let Some(paths) = self.scan_git_dirty_tracked_paths()? {
                if paths.is_empty() {
                    Vec::new()
                } else {
                    self.selected_worktree_snapshot(&head_files, &paths)?
                        .summaries
                }
            } else {
                let disk_files = self.scan_worktree_files()?;
                let disk_manifest = self.disk_manifest(&disk_files);
                self.diff_file_maps_to_manifest(&head_files, &disk_manifest)
            }
        } else {
            let disk_files = self.scan_worktree_files()?;
            let disk_manifest = self.disk_manifest(&disk_files);
            self.diff_file_maps_to_manifest(&head_files, &disk_manifest)
        };
        let worktree_state = worktree_state_from_changes(&changed_paths);
        Ok(StatusReport {
            branch,
            head,
            worktree_state,
            changed_paths,
        })
    }
}
