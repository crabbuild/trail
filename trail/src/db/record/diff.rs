use super::*;

impl Trail {
    pub fn diff_range(&self, spec: &str, patches: bool) -> Result<DiffSummary> {
        let metrics = self.operation_metrics.clone();
        profile_operation_metrics(metrics.as_ref(), OperationMetricsKind::Diff, || {
            self.diff_range_with_options(spec, patches, false)
        })
    }

    pub fn diff_range_with_options(
        &self,
        spec: &str,
        patches: bool,
        line_changes: bool,
    ) -> Result<DiffSummary> {
        let metrics = self.operation_metrics.clone();
        profile_operation_metrics(metrics.as_ref(), OperationMetricsKind::Diff, || {
            let (left, right) = parse_range(spec)?;
            self.diff_refs_with_options(left, right, patches, line_changes)
        })
    }

    pub fn diff_refs(&self, left: &str, right: &str, patches: bool) -> Result<DiffSummary> {
        let metrics = self.operation_metrics.clone();
        profile_operation_metrics(metrics.as_ref(), OperationMetricsKind::Diff, || {
            self.diff_refs_with_options(left, right, patches, false)
        })
    }

    pub fn diff_refs_with_options(
        &self,
        left: &str,
        right: &str,
        patches: bool,
        line_changes: bool,
    ) -> Result<DiffSummary> {
        let metrics = self.operation_metrics.clone();
        let result_metrics = metrics.clone();
        profile_operation_metrics(metrics.as_ref(), OperationMetricsKind::Diff, || {
            let left_ref = self.resolve_refish(left)?;
            let right_ref = self.resolve_refish(right)?;
            let result = self.diff_root_files(
                left.to_string(),
                right.to_string(),
                &left_ref.root_id,
                &right_ref.root_id,
                patches,
                line_changes,
            );
            if let (Some(metrics), Ok(summary)) = (&result_metrics, &result) {
                metrics.add(OperationMetricsDelta {
                    final_path_count: saturating_u64_from_usize(summary.files.len()),
                    ..OperationMetricsDelta::default()
                });
            }
            result
        })
    }

    pub fn diff_roots(&self, spec: &str, patches: bool, line_changes: bool) -> Result<DiffSummary> {
        let metrics = self.operation_metrics.clone();
        let result_metrics = metrics.clone();
        profile_operation_metrics(metrics.as_ref(), OperationMetricsKind::Diff, || {
            let (left, right) = parse_range(spec)?;
            let left_id = ObjectId(left.to_string());
            let right_id = ObjectId(right.to_string());
            let result = self.diff_root_files(
                left.to_string(),
                right.to_string(),
                &left_id,
                &right_id,
                patches,
                line_changes,
            );
            if let (Some(metrics), Ok(summary)) = (&result_metrics, &result) {
                metrics.add(OperationMetricsDelta {
                    final_path_count: saturating_u64_from_usize(summary.files.len()),
                    ..OperationMetricsDelta::default()
                });
            }
            result
        })
    }

    pub fn diff_dirty(&mut self, patches: bool, line_changes: bool) -> Result<DiffSummary> {
        let metrics = self.operation_metrics.clone();
        let result_metrics = metrics.clone();
        profile_operation_metrics(metrics.as_ref(), OperationMetricsKind::Diff, || {
            let result = self.diff_dirty_profiled(patches, line_changes);
            if let (Some(metrics), Ok(summary)) = (&result_metrics, &result) {
                metrics.add(OperationMetricsDelta {
                    final_path_count: saturating_u64_from_usize(summary.files.len()),
                    ..OperationMetricsDelta::default()
                });
            }
            result
        })
    }

    fn diff_dirty_profiled(&mut self, patches: bool, line_changes: bool) -> Result<DiffSummary> {
        let _lock = self.acquire_write_lock()?;
        let branch = self.current_branch()?;
        let head = self.resolve_branch_ref(&branch)?;
        if let Some(diff) =
            self.diff_dirty_from_daemon_cache(&branch, &head.root_id, patches, line_changes)?
        {
            return Ok(diff);
        }
        let git_policy = self.workspace_ignore_policy_snapshot();
        let fast_dirty_paths = self.scan_git_dirty_tracked_paths_with_policy(&git_policy)?;
        let disk_files;
        let build_selected_paths;
        let previous_files;
        if let Some(paths) = fast_dirty_paths {
            if paths.is_empty() {
                return Ok(DiffSummary {
                    from: branch,
                    to: "dirty".to_string(),
                    files: Vec::new(),
                });
            }
            previous_files = self.load_root_files_for_paths(&head.root_id, &paths)?;
            let snapshot =
                self.selected_worktree_snapshot_with_policy(&previous_files, &paths, &git_policy)?;
            if snapshot.paths.is_empty() {
                return Ok(DiffSummary {
                    from: branch,
                    to: "dirty".to_string(),
                    files: Vec::new(),
                });
            }
            disk_files = snapshot.files;
            build_selected_paths = Some(snapshot.paths);
        } else {
            let refresh = self.refresh_worktree_index_streaming_report()?;
            let baseline = self.worktree_index_baseline_root()?;
            if !refresh.changed
                && self.clean_baseline_matches_visible_root(baseline.as_ref(), &head.root_id)
            {
                return Ok(DiffSummary {
                    from: branch,
                    to: "dirty".to_string(),
                    files: Vec::new(),
                });
            }
            let summaries = self.diff_root_to_worktree_index(&head.root_id)?;
            if summaries.is_empty() {
                self.set_worktree_index_baseline(&head.root_id)?;
                return Ok(DiffSummary {
                    from: branch,
                    to: "dirty".to_string(),
                    files: Vec::new(),
                });
            }
            let paths = summaries
                .iter()
                .map(|summary| summary.path.clone())
                .collect::<Vec<_>>();
            disk_files = self.scan_visible_files_for_paths(&paths)?;
            previous_files = self.load_root_files_for_paths(&head.root_id, &paths)?;
            build_selected_paths = Some(paths);
        }
        let change_id = self.allocate_change_id("trail", "dirty-diff")?;
        let built = if let Some(paths) = build_selected_paths.as_deref() {
            self.build_root_for_selected_record_incremental(
                &head.root_id,
                &previous_files,
                &disk_files,
                paths,
                false,
                &change_id,
            )?
        } else {
            self.build_root_from_disk_files(&disk_files, &change_id, Some(&previous_files))?
        };
        self.diff_files(
            branch,
            "dirty".to_string(),
            &previous_files,
            &built.files,
            patches,
            line_changes,
        )
    }

    fn diff_dirty_from_daemon_cache(
        &self,
        branch: &str,
        root_id: &ObjectId,
        patches: bool,
        line_changes: bool,
    ) -> Result<Option<DiffSummary>> {
        let Some(snapshot) = self.daemon_worktree_snapshot() else {
            return Ok(None);
        };
        let (generation, paths) = match snapshot {
            DaemonWorktreeSnapshot::Clean {
                generation: _,
                root_id: Some(clean_root),
            } => {
                if self.clean_baseline_matches_visible_root(Some(&clean_root), root_id) {
                    return Ok(Some(DiffSummary {
                        from: branch.to_string(),
                        to: "dirty".to_string(),
                        files: Vec::new(),
                    }));
                }
                return Ok(None);
            }
            DaemonWorktreeSnapshot::Dirty { generation, paths }
                if paths.len() <= self.daemon_dirty_path_limit() =>
            {
                (generation, paths)
            }
            DaemonWorktreeSnapshot::Clean { .. }
            | DaemonWorktreeSnapshot::Dirty { .. }
            | DaemonWorktreeSnapshot::Overflow { .. } => return Ok(None),
        };

        let previous_files = self.load_root_files_for_selections(root_id, &paths)?;
        let policy = self.workspace_ignore_policy_snapshot();
        let snapshot =
            self.selected_worktree_snapshot_with_policy(&previous_files, &paths, &policy)?;
        self.reconcile_daemon_status_paths(root_id, &paths, &snapshot.summaries, generation);
        if snapshot.paths.is_empty() {
            return Ok(Some(DiffSummary {
                from: branch.to_string(),
                to: "dirty".to_string(),
                files: Vec::new(),
            }));
        }

        let change_id = self.allocate_change_id("trail", "dirty-diff")?;
        let built = self.build_root_for_selected_record_incremental(
            root_id,
            &previous_files,
            &snapshot.files,
            &snapshot.paths,
            false,
            &change_id,
        )?;
        self.diff_files(
            branch.to_string(),
            "dirty".to_string(),
            &previous_files,
            &built.files,
            patches,
            line_changes,
        )
        .map(Some)
    }
}
