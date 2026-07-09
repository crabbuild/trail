use super::*;

impl Trail {
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
        let suggestions = self.status_suggestions(&branch, &worktree_state, &changed_paths);
        Ok(StatusReport {
            branch,
            head,
            worktree_state,
            changed_paths,
            suggestions,
        })
    }

    pub(crate) fn status_read_only(&self, branch: Option<&str>) -> Result<StatusReport> {
        let current_branch = self.current_branch()?;
        let branch = branch.map(str::to_string).unwrap_or(current_branch.clone());
        let head = self.resolve_branch_ref(&branch)?;
        let changed_paths =
            self.status_changed_paths_read_only(&current_branch, &branch, &head.root_id)?;
        let worktree_state = worktree_state_from_changes(&changed_paths);
        let suggestions = self.status_suggestions(&branch, &worktree_state, &changed_paths);
        Ok(StatusReport {
            branch,
            head,
            worktree_state,
            changed_paths,
            suggestions,
        })
    }

    fn status_suggestions(
        &self,
        branch: &str,
        worktree_state: &WorktreeState,
        changed_paths: &[FileDiffSummary],
    ) -> Vec<StatusSuggestion> {
        let mut suggestions = Vec::new();
        if !changed_paths.is_empty() || !matches!(worktree_state, WorktreeState::Clean) {
            suggestions.push(StatusSuggestion {
                command: "trail record -m \"describe the change\"".to_string(),
                reason: "record uncheckpointed workspace changes".to_string(),
            });
        }
        if let Ok(acp_sessions) = self.list_lane_acp_sessions(None) {
            if let Some(session) = acp_sessions.sessions.first() {
                if let Ok(lane_name) = self.resolve_lane_handle(&session.lane_id) {
                    suggestions.push(StatusSuggestion {
                        command: format!("trail transcript {lane_name}"),
                        reason: "review the latest captured agent session".to_string(),
                    });
                    suggestions.push(StatusSuggestion {
                        command: format!("trail lane review {lane_name}"),
                        reason: "inspect checkpoint evidence before merge or rewind".to_string(),
                    });
                }
            }
        }
        if suggestions.is_empty() {
            suggestions.push(StatusSuggestion {
                command: "trail doctor".to_string(),
                reason: format!("verify workspace health for branch `{branch}`"),
            });
        }
        suggestions
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

    fn status_changed_paths_read_only(
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
                    .selected_worktree_snapshot_for_root_read_only(root_id, &paths)?
                    .summaries);
            }
        }
        let disk_files = self.scan_files_under(&self.workspace_root)?;
        let disk_manifest = self.disk_manifest(&disk_files);
        self.diff_root_to_disk_manifest(root_id, &disk_manifest)
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
                let suggestions = self.status_suggestions(branch, &worktree_state, &changed_paths);
                Ok(Some(StatusReport {
                    branch: branch.to_string(),
                    head: head.clone(),
                    worktree_state,
                    changed_paths,
                    suggestions,
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
        suggestions: vec![StatusSuggestion {
            command: "trail doctor".to_string(),
            reason: format!("verify workspace health for branch `{branch}`"),
        }],
    }
}
