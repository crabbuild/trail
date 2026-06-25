use super::*;

impl CrabDb {
    pub fn record_agent_workdir(
        &mut self,
        agent: &str,
        message: Option<String>,
    ) -> Result<AgentRecordReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;
        let branch = self.agent_branch(agent)?;
        let Some(workdir) = branch.workdir.clone() else {
            return Err(Error::InvalidInput(format!(
                "agent `{agent}` does not have a materialized workdir"
            )));
        };
        let workdir_path = PathBuf::from(&workdir);
        if !workdir_path.is_dir() {
            return Err(Error::WorkspaceNotFound(workdir_path));
        }
        let head = self.get_ref(&branch.ref_name)?;
        let previous_files = self.load_root_files(&head.root_id)?;
        let disk_files = self.scan_files_under(&workdir_path)?;
        let actor = Actor::agent(agent);
        let change_id = self.allocate_change_id(&actor.id, "agent_record")?;
        let built =
            self.build_root_from_disk_files(&disk_files, &change_id, Some(&previous_files))?;
        let diff = self.diff_file_maps(&previous_files, &built.files)?;
        if diff.changes.is_empty() {
            return Ok(AgentRecordReport {
                agent_id: branch.agent_id,
                operation: None,
                root_id: head.root_id,
                changed_paths: Vec::new(),
            });
        }
        if let Some(session_id) = &branch.session_id {
            self.ensure_agent_session(&branch.agent_id, session_id, None)?;
        }
        let turn_id = self.open_agent_turn(
            &branch.agent_id,
            branch.session_id.as_deref(),
            &branch.base_change,
            &head.change_id,
            Some(&serde_json::json!({
                "kind": "workdir_record",
                "path_count": diff.summaries.len()
            })),
        )?;

        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind: OperationKind::AgentRecord,
            parents: vec![head.change_id.clone()],
            before_root: Some(head.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: branch.ref_name.clone(),
            actor,
            session_id: branch.session_id.clone(),
            message: message.as_deref().map(redact_sensitive_text),
            changes: diff.changes,
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&head, &change_id, &built.root_id, &operation_id)?;
        let message_id = if let Some(message) = message {
            Some(self.store_message(
                "agent",
                &message,
                Some(&branch.agent_id),
                branch.session_id.as_deref(),
                Some(&change_id),
                operation.created_at,
            )?)
        } else {
            None
        };
        self.conn.execute(
            "UPDATE agent_branches SET head_change = ?1, head_root = ?2, updated_at = ?3 WHERE agent_id = ?4",
            params![change_id.0, built.root_id.0, now_ts(), branch.agent_id],
        )?;
        self.insert_agent_event_with_context(
            &branch.agent_id,
            branch.session_id.as_deref(),
            Some(&turn_id),
            "workdir_recorded",
            Some(&change_id),
            message_id.as_ref(),
            &serde_json::json!({
                "workdir": workdir,
                "root_id": built.root_id.0.clone(),
                "session_id": branch.session_id.clone(),
                "changed_paths": diff.summaries.iter().map(|item| item.path.clone()).collect::<Vec<_>>()
            }),
        )?;
        self.finish_agent_turn(&turn_id, "completed", Some(&change_id))?;
        Ok(AgentRecordReport {
            agent_id: branch.agent_id,
            operation: Some(change_id),
            root_id: built.root_id,
            changed_paths: diff.summaries,
        })
    }

    pub fn watch_agent_workdir(
        &mut self,
        agent: &str,
        message: Option<String>,
        interval: Duration,
        iterations: Option<u64>,
    ) -> Result<AgentWatchReport> {
        validate_ref_segment(agent)?;
        let branch = self.agent_branch(agent)?;
        if branch.workdir.is_none() {
            return Err(Error::InvalidInput(format!(
                "agent `{agent}` does not have a materialized workdir"
            )));
        }
        let mut report = AgentWatchReport {
            agent_id: branch.agent_id,
            iterations: 0,
            recorded_operations: Vec::new(),
            changed_paths: Vec::new(),
        };
        loop {
            let record = self.record_agent_workdir(agent, message.clone())?;
            report.iterations += 1;
            if let Some(operation) = record.operation {
                report.recorded_operations.push(operation);
                report.changed_paths.extend(record.changed_paths);
            }
            if iterations.is_some_and(|limit| report.iterations >= limit) {
                break;
            }
            std::thread::sleep(interval);
        }
        Ok(report)
    }
}
