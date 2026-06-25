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
        let previous_files = self.load_root_files(&head.root_id)?;
        let selected_paths = normalize_record_paths(&options.paths)?;
        let session_id = options
            .session_id
            .map(|session_id| {
                validate_session_id(&session_id)?;
                self.agent_session(&session_id)?;
                Ok::<String, Error>(session_id)
            })
            .transpose()?;
        let fast_dirty_paths = if selected_paths.is_empty() {
            self.scan_git_dirty_tracked_paths()?
        } else {
            None
        };
        let disk_files;
        let build_selected_paths;
        if let Some(paths) = fast_dirty_paths {
            if paths.is_empty() {
                return Ok(RecordReport {
                    branch,
                    operation: None,
                    root_id: head.root_id,
                    changed_paths: Vec::new(),
                });
            }
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
        } else if !selected_paths.is_empty() {
            disk_files =
                self.scan_record_selection_files(&selected_paths, options.allow_ignored)?;
            build_selected_paths = Some(selected_paths.clone());
        } else {
            disk_files = self.scan_worktree_files()?;
            build_selected_paths = None;
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
        Ok(RecordReport {
            branch,
            operation: Some(change_id),
            root_id: built.root_id,
            changed_paths: diff.summaries,
        })
    }
}
