use super::*;

impl CrabDb {
    pub fn history_for_path(&self, path: &str) -> Result<HistoryResult> {
        let path = normalize_relative_path(path)?;
        Ok(HistoryResult {
            selector: path.clone(),
            file_history: self.file_history_by_path(&path)?,
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
}
