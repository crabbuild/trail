use super::*;

impl CrabDb {
    pub fn status(&self, branch: Option<&str>) -> Result<StatusReport> {
        let current_branch = self.current_branch()?;
        let branch = branch.map(str::to_string).unwrap_or(current_branch.clone());
        let head = self.resolve_branch_ref(&branch)?;
        if branch == current_branch {
            if let Some(report) = self.status_from_daemon_cache(&branch, &head)? {
                return Ok(report);
            }
        }
        let snapshot_generation = self
            .daemon_worktree_snapshot()
            .map(|snapshot| match snapshot {
                DaemonWorktreeSnapshot::Clean { generation, .. }
                | DaemonWorktreeSnapshot::Dirty { generation, .. }
                | DaemonWorktreeSnapshot::Overflow { generation } => generation,
            });
        let changed_paths =
            self.status_changed_paths_uncached(&current_branch, &branch, &head.root_id)?;
        if branch == current_branch {
            self.reconcile_daemon_full_status(&head.root_id, &changed_paths, snapshot_generation);
        };
        let worktree_state = worktree_state_from_changes(&changed_paths);
        Ok(StatusReport {
            branch,
            head,
            worktree_state,
            changed_paths,
        })
    }

    pub(crate) fn status_changed_paths_uncached(
        &self,
        current_branch: &str,
        branch: &str,
        root_id: &ObjectId,
    ) -> Result<Vec<FileDiffSummary>> {
        if branch == current_branch {
            if let Some(paths) = self.scan_git_dirty_tracked_paths()? {
                if paths.is_empty() {
                    return Ok(Vec::new());
                }
                return Ok(self
                    .selected_worktree_snapshot_for_root(root_id, &paths)?
                    .summaries);
            }
        }
        let refresh = self.refresh_worktree_index_streaming_report()?;
        if !refresh.changed
            && self
                .worktree_index_baseline_root()?
                .is_some_and(|baseline| baseline == root_id.clone())
        {
            return Ok(Vec::new());
        }
        let summaries = self.diff_root_to_worktree_index(root_id)?;
        if summaries.is_empty() {
            self.set_worktree_index_baseline(root_id)?;
        }
        Ok(summaries)
    }

    fn status_from_daemon_cache(
        &self,
        branch: &str,
        head: &RefRecord,
    ) -> Result<Option<StatusReport>> {
        let Some(snapshot) = self.daemon_worktree_snapshot() else {
            return Ok(None);
        };
        match snapshot {
            DaemonWorktreeSnapshot::Clean {
                generation: _,
                root_id: Some(root_id),
            } if root_id == head.root_id => Ok(Some(clean_status_report(branch, head))),
            DaemonWorktreeSnapshot::Dirty { generation, paths } => {
                if paths.len() > self.daemon_dirty_path_limit() {
                    return Ok(None);
                }
                let snapshot = self.selected_worktree_snapshot_for_root(&head.root_id, &paths)?;
                let changed_paths = snapshot.summaries;
                self.reconcile_daemon_status_paths(
                    &head.root_id,
                    &paths,
                    &changed_paths,
                    generation,
                );
                let worktree_state = worktree_state_from_changes(&changed_paths);
                Ok(Some(StatusReport {
                    branch: branch.to_string(),
                    head: head.clone(),
                    worktree_state,
                    changed_paths,
                }))
            }
            DaemonWorktreeSnapshot::Clean { .. } | DaemonWorktreeSnapshot::Overflow { .. } => {
                Ok(None)
            }
        }
    }
}

fn clean_status_report(branch: &str, head: &RefRecord) -> StatusReport {
    StatusReport {
        branch: branch.to_string(),
        head: head.clone(),
        worktree_state: WorktreeState::Clean,
        changed_paths: Vec::new(),
    }
}
