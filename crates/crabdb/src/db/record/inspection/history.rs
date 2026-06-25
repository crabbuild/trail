use super::*;

impl CrabDb {
    pub fn history_for_path(&self, path: &str) -> Result<HistoryResult> {
        let path = normalize_relative_path(path)?;
        let mut file_history = self.file_history_by_path(&path)?;
        if let Some(initial) = self.compacted_initial_file_history_for_path(&path, &file_history)? {
            file_history.insert(0, initial);
        }
        Ok(HistoryResult {
            selector: path.clone(),
            file_history,
            line_history: Vec::new(),
        })
    }

    pub fn history_for_file_id(&self, file_id: &str) -> Result<HistoryResult> {
        Ok(HistoryResult {
            selector: file_id.to_string(),
            file_history: self.file_history_by_file_id(file_id)?,
            line_history: Vec::new(),
        })
    }

    pub fn history_for_line_id(&self, line_id: &str) -> Result<HistoryResult> {
        Ok(HistoryResult {
            selector: line_id.to_string(),
            file_history: Vec::new(),
            line_history: self.line_history_by_line_id(line_id)?,
        })
    }

    pub fn code_from(&self, selector: &str) -> Result<CodeFromResult> {
        let mut changes = Vec::new();
        if let Some(agent) = selector.strip_prefix("agent:") {
            changes.extend(self.agent_change_ids(agent)?);
        } else if selector.starts_with("msg_") {
            let change_id: Option<String> = self
                .conn
                .query_row(
                    "SELECT change_id FROM messages WHERE message_id = ?1",
                    params![selector],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(change_id) = change_id else {
                return Err(Error::InvalidInput(format!(
                    "message `{selector}` not found"
                )));
            };
            changes.push(ChangeId(change_id));
        } else if selector.starts_with("ch_") {
            changes.push(ChangeId(selector.to_string()));
        } else if selector.starts_with("session_") {
            changes.extend(self.session_change_ids(selector)?);
        } else if let Ok(agent) = self.agent_branch(selector) {
            changes.extend(self.agent_change_ids(&agent.agent_id)?);
        } else {
            changes.extend(self.session_change_ids(selector)?);
        }

        let mut operations = Vec::new();
        for change in changes {
            let operation = self.operation(&change)?;
            operations.push(CodeFromOperation {
                change_id: operation.change_id.clone(),
                kind: operation.kind.clone(),
                branch: operation.branch.clone(),
                actor_id: operation.actor.id.clone(),
                session_id: operation.session_id.clone(),
                message: operation.message.clone(),
                changed_paths: summarize_file_changes(&operation.changes),
                created_at: operation.created_at,
            });
        }
        Ok(CodeFromResult {
            selector: selector.to_string(),
            operations,
        })
    }

    fn compacted_initial_file_history_for_path(
        &self,
        path: &str,
        existing: &[FileHistoryEntry],
    ) -> Result<Option<FileHistoryEntry>> {
        let head = self.resolve_branch_ref(&self.current_branch()?)?;
        let files = self.load_root_files_for_paths(&head.root_id, &[path.to_string()])?;
        let Some(entry) = files.get(path) else {
            return Ok(None);
        };
        if existing
            .iter()
            .any(|history| history.change_id == entry.created_by)
        {
            return Ok(None);
        }
        let row = self
            .conn
            .query_row(
                "SELECT created_at FROM operations \
                 WHERE change_id = ?1 AND before_root IS NULL \
                 AND kind IN ('Init', 'GitImport')",
                params![entry.created_by.0],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let Some(created_at) = row else {
            return Ok(None);
        };
        Ok(Some(FileHistoryEntry {
            file_id: file_id_key(&entry.file_id),
            change_id: entry.created_by.clone(),
            path: path.to_string(),
            old_path: None,
            kind: FileChangeKind::Added,
            before_hash: None,
            after_hash: Some(entry.content_hash.clone()),
            created_at,
        }))
    }
}
