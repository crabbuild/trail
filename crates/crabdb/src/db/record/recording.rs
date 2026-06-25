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
        let disk_files = self.scan_worktree_files()?;
        let selected_paths = normalize_record_paths(&options.paths)?;
        let session_id = options
            .session_id
            .map(|session_id| {
                validate_session_id(&session_id)?;
                self.agent_session(&session_id)?;
                Ok::<String, Error>(session_id)
            })
            .transpose()?;
        let change_id = self.allocate_change_id(&actor.id, "record")?;
        let built = if selected_paths.is_empty() {
            self.build_root_from_disk_files(&disk_files, &change_id, Some(&previous_files))?
        } else {
            self.build_root_for_selected_record(
                &previous_files,
                &disk_files,
                &selected_paths,
                options.allow_ignored,
                &change_id,
            )?
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

    pub fn git_import_update(
        &mut self,
        branch: Option<&str>,
        message: Option<String>,
    ) -> Result<GitImportReport> {
        let _lock = self.acquire_write_lock()?;
        let branch = branch.map(str::to_string).unwrap_or(self.current_branch()?);
        let ref_name = branch_ref(&branch);
        let head = self.get_ref(&ref_name)?;
        let previous_files = self.load_root_files(&head.root_id)?;
        let disk_files = self.scan_git_tracked_files_required()?;
        let actor = Actor::system();
        let change_id = self.allocate_change_id(&actor.id, "git-import-update")?;
        let built =
            self.build_root_from_disk_files(&disk_files, &change_id, Some(&previous_files))?;
        let diff = self.diff_file_maps(&previous_files, &built.files)?;

        if diff.changes.is_empty() {
            return Ok(GitImportReport {
                branch,
                operation: None,
                root_id: head.root_id,
                imported: built.stats,
                changed_paths: Vec::new(),
                mapping: None,
            });
        }

        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind: OperationKind::GitImport,
            parents: vec![head.change_id.clone()],
            before_root: Some(head.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: branch.clone(),
            actor,
            session_id: None,
            message: message
                .map(|message| redact_sensitive_text(&message))
                .or_else(|| Some("Import Git-tracked workspace update".to_string())),
            changes: diff.changes,
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&head, &change_id, &built.root_id, &operation_id)?;
        let mapping = self.insert_git_mapping("import", &branch, &change_id, &built.root_id)?;

        Ok(GitImportReport {
            branch,
            operation: Some(change_id),
            root_id: built.root_id,
            imported: built.stats,
            changed_paths: diff.summaries,
            mapping,
        })
    }

    pub fn git_mappings(&self, limit: usize) -> Result<Vec<GitMapping>> {
        let mut stmt = self.conn.prepare(
            "SELECT mapping_id, direction, branch, git_head, git_dirty, crab_change, crab_root, created_at \
             FROM git_mappings ORDER BY created_at DESC, rowid DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], git_mapping_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub fn timeline(&self, branch: Option<&str>, limit: usize) -> Result<Vec<TimelineEntry>> {
        let mut sql = String::from(
            "SELECT change_id, kind, branch, actor_id, message, created_at, path_count \
             FROM operations",
        );
        if let Some(branch) = branch {
            let (branch_ref, bare_branch) = self.timeline_branch_terms(branch)?;
            if let Some(bare_branch) = bare_branch {
                sql.push_str(" WHERE branch = ?1 OR branch = ?2");
                sql.push_str(" ORDER BY created_at DESC, rowid DESC LIMIT ?3");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows =
                    stmt.query_map(params![branch_ref, bare_branch, limit as i64], timeline_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            } else {
                sql.push_str(" WHERE branch = ?1");
                sql.push_str(" ORDER BY created_at DESC, rowid DESC LIMIT ?2");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map(params![branch_ref, limit as i64], timeline_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
        } else {
            sql.push_str(" ORDER BY created_at DESC, rowid DESC LIMIT ?1");
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(params![limit as i64], timeline_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Error::from)
        }
    }

    pub fn timeline_query(
        &self,
        branch: Option<&str>,
        session: Option<&str>,
        agent: Option<&str>,
        limit: usize,
    ) -> Result<Vec<TimelineEntry>> {
        let scoped = [branch.is_some(), session.is_some(), agent.is_some()]
            .into_iter()
            .filter(|set| *set)
            .count();
        if scoped > 1 {
            return Err(Error::InvalidInput(
                "timeline accepts only one of branch, session, or agent".to_string(),
            ));
        }
        if let Some(session_id) = session {
            return self.session_timeline(session_id, limit);
        }
        if let Some(agent) = agent {
            return self.agent_timeline(agent, limit);
        }
        self.timeline(branch, limit)
    }

    pub(crate) fn timeline_branch_terms(&self, branch: &str) -> Result<(String, Option<String>)> {
        let record = self.resolve_refish(branch)?;
        if record.name.starts_with(MAIN_REF_PREFIX) {
            let bare_branch = record
                .name
                .strip_prefix(MAIN_REF_PREFIX)
                .map(str::to_string);
            Ok((record.name, bare_branch))
        } else if record.name.starts_with(AGENT_REF_PREFIX) {
            Ok((record.name, None))
        } else {
            Err(Error::InvalidInput(format!(
                "timeline --branch expects a branch or agent ref, got `{branch}`"
            )))
        }
    }

    pub fn session_timeline(&self, session_id: &str, limit: usize) -> Result<Vec<TimelineEntry>> {
        self.agent_session(session_id)?;
        let mut stmt = self.conn.prepare(
            "SELECT change_id, kind, branch, actor_id, message, created_at, path_count \
             FROM operations WHERE session_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![session_id, limit as i64], timeline_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }
}
