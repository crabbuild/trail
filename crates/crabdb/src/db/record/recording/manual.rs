use super::*;

impl CrabDb {
    pub fn record(
        &mut self,
        branch: Option<&str>,
        message: Option<String>,
        actor: Actor,
        watch: bool,
    ) -> Result<RecordReport> {
        self.record_with_options(
            branch,
            message,
            actor,
            RecordOptions {
                kind: Some(if watch {
                    OperationKind::WatchRecord
                } else {
                    OperationKind::ManualRecord
                }),
                ..RecordOptions::default()
            },
        )
    }

    pub fn record_with_options(
        &mut self,
        branch: Option<&str>,
        message: Option<String>,
        actor: Actor,
        options: RecordOptions,
    ) -> Result<RecordReport> {
        let _lock = self.acquire_write_lock()?;
        self.record_with_options_unlocked(branch, message, actor, options)
    }

    pub(crate) fn record_with_options_unlocked(
        &mut self,
        branch: Option<&str>,
        message: Option<String>,
        actor: Actor,
        options: RecordOptions,
    ) -> Result<RecordReport> {
        let branch = branch.map(str::to_string).unwrap_or(self.current_branch()?);
        let ref_name = branch_ref(&branch);
        let head = self.get_ref(&ref_name)?;
        let selected_paths = normalize_record_paths(&options.paths)?;
        let session_id = options
            .session_id
            .map(|session_id| {
                validate_session_id(&session_id)?;
                self.agent_session(&session_id)?;
                Ok::<String, Error>(session_id)
            })
            .transpose()?;
        let daemon_snapshot = if selected_paths.is_empty() {
            self.daemon_worktree_snapshot()
        } else {
            None
        };
        let fallback_generation = match &daemon_snapshot {
            Some(
                DaemonWorktreeSnapshot::Clean { generation, .. }
                | DaemonWorktreeSnapshot::Dirty { generation, .. }
                | DaemonWorktreeSnapshot::Overflow { generation },
            ) => Some(*generation),
            None => None,
        };
        let daemon_dirty_paths = match daemon_snapshot {
            Some(DaemonWorktreeSnapshot::Clean {
                generation: _,
                root_id: Some(clean_root),
            }) if clean_root == head.root_id => {
                return Ok(RecordReport {
                    branch,
                    operation: None,
                    root_id: head.root_id,
                    changed_paths: Vec::new(),
                });
            }
            Some(DaemonWorktreeSnapshot::Dirty { generation, paths })
                if paths.len() <= self.daemon_dirty_path_limit() =>
            {
                Some((generation, paths))
            }
            _ => None,
        };
        let fast_dirty_paths = if selected_paths.is_empty() && daemon_dirty_paths.is_none() {
            self.scan_git_dirty_tracked_paths()?
        } else {
            None
        };
        let disk_files;
        let build_selected_paths;
        let previous_files;
        let record_generation;
        if let Some((generation, paths)) = daemon_dirty_paths {
            previous_files = self.load_root_files_for_selections(&head.root_id, &paths)?;
            let snapshot = self.selected_worktree_snapshot(&previous_files, &paths)?;
            self.reconcile_daemon_status_paths(
                &head.root_id,
                &paths,
                &snapshot.summaries,
                generation,
            );
            if snapshot.paths.is_empty() {
                return Ok(RecordReport {
                    branch,
                    operation: None,
                    root_id: head.root_id,
                    changed_paths: Vec::new(),
                });
            }
            disk_files = snapshot.files;
            build_selected_paths = Some(snapshot.paths);
            record_generation = Some(generation);
        } else if let Some(paths) = fast_dirty_paths {
            if paths.is_empty() {
                return Ok(RecordReport {
                    branch,
                    operation: None,
                    root_id: head.root_id,
                    changed_paths: Vec::new(),
                });
            }
            previous_files = self.load_root_files_for_paths(&head.root_id, &paths)?;
            let snapshot = self.selected_worktree_snapshot(&previous_files, &paths)?;
            if snapshot.paths.is_empty() {
                return Ok(RecordReport {
                    branch,
                    operation: None,
                    root_id: head.root_id,
                    changed_paths: Vec::new(),
                });
            }
            disk_files = snapshot.files;
            build_selected_paths = Some(snapshot.paths);
            record_generation = fallback_generation;
        } else if !selected_paths.is_empty() {
            previous_files = self.load_root_files_for_selections(&head.root_id, &selected_paths)?;
            disk_files =
                self.scan_record_selection_files(&selected_paths, options.allow_ignored)?;
            build_selected_paths = Some(selected_paths.clone());
            record_generation = None;
        } else {
            let refresh = self.refresh_worktree_index_streaming_report()?;
            if !refresh.changed
                && self
                    .worktree_index_baseline_root()?
                    .is_some_and(|baseline| baseline == head.root_id.clone())
            {
                return Ok(RecordReport {
                    branch,
                    operation: None,
                    root_id: head.root_id,
                    changed_paths: Vec::new(),
                });
            }
            let summaries = self.diff_root_to_worktree_index(&head.root_id)?;
            if summaries.is_empty() {
                self.set_worktree_index_baseline(&head.root_id)?;
                return Ok(RecordReport {
                    branch,
                    operation: None,
                    root_id: head.root_id,
                    changed_paths: Vec::new(),
                });
            }
            let paths = summaries
                .iter()
                .map(|summary| summary.path.clone())
                .collect::<Vec<_>>();
            disk_files = self.scan_visible_files_for_paths(&paths)?;
            previous_files = self.load_root_files_for_paths(&head.root_id, &paths)?;
            build_selected_paths = Some(paths);
            record_generation = fallback_generation;
        }
        let change_id = self.allocate_change_id(&actor.id, "record")?;
        let built = if let Some(paths) = build_selected_paths.as_deref() {
            self.build_root_for_selected_record_incremental(
                &head.root_id,
                &previous_files,
                &disk_files,
                paths,
                options.allow_ignored,
                &change_id,
            )?
        } else {
            self.build_root_from_disk_files(&disk_files, &change_id, Some(&previous_files))?
        };
        let diff = self.diff_file_maps(&previous_files, &built.files)?;

        if diff.changes.is_empty() {
            return Ok(RecordReport {
                branch,
                operation: None,
                root_id: head.root_id,
                changed_paths: Vec::new(),
            });
        }

        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind: options.kind.unwrap_or(OperationKind::ManualRecord),
            parents: vec![head.change_id.clone()],
            before_root: Some(head.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: branch.clone(),
            actor,
            session_id,
            message: message.map(|message| redact_sensitive_text(&message)),
            changes: diff.changes,
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&head, &change_id, &built.root_id, &operation_id)?;
        if selected_paths.is_empty() {
            self.set_worktree_index_baseline(&built.root_id)?;
            self.reconcile_daemon_full_status(&built.root_id, &[], record_generation);
        }
        Ok(RecordReport {
            branch,
            operation: Some(change_id),
            root_id: built.root_id,
            changed_paths: diff.summaries,
        })
    }
}
