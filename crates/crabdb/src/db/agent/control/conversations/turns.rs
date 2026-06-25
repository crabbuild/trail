use super::*;

impl CrabDb {
    pub fn begin_agent_turn(
        &mut self,
        agent: &str,
        from: Option<&str>,
        session_title: Option<String>,
        base_change: Option<&str>,
    ) -> Result<AgentTurnStartReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;

        let branch = self.agent_branch_for_turn(agent, from, base_change)?;

        if let Some(expected_base) = base_change {
            if branch.head_change.0 != expected_base {
                return Err(Error::StaleBranch(branch.ref_name));
            }
        }

        let session_id = self.allocate_session_id(&branch.agent_id, session_title.as_deref());
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO agent_sessions \
             (session_id, agent_id, title, status, started_at, ended_at, metadata_json) \
             VALUES (?1, ?2, ?3, 'active', ?4, NULL, NULL)",
            params![session_id, branch.agent_id, session_title, now],
        )?;
        self.conn.execute(
            "UPDATE agent_branches SET session_id = ?1, updated_at = ?2 WHERE agent_id = ?3",
            params![session_id, now, branch.agent_id],
        )?;
        self.insert_agent_event_with_context(
            &branch.agent_id,
            Some(&session_id),
            None,
            "session_started",
            Some(&branch.head_change),
            None,
            &serde_json::json!({
                "session_id": session_id.clone(),
                "title": session_title.clone(),
                "source": "api"
            }),
        )?;
        let turn_id = self.open_agent_turn(
            &branch.agent_id,
            Some(&session_id),
            &branch.base_change,
            &branch.head_change,
            Some(&serde_json::json!({
                "kind": "api_turn",
                "from": from,
                "base_change": base_change
            })),
        )?;
        self.insert_agent_event_with_context(
            &branch.agent_id,
            Some(&session_id),
            Some(&turn_id),
            "turn_started",
            None,
            None,
            &serde_json::json!({
                "turn_id": turn_id.clone()
            }),
        )?;
        Ok(AgentTurnStartReport {
            turn: self.agent_turn(&turn_id)?,
            session: self.agent_session(&session_id)?,
            base_root: branch.head_root,
        })
    }

    pub fn add_agent_turn_event(
        &mut self,
        turn_id: &str,
        event_type: &str,
        payload: Option<serde_json::Value>,
        change_id: Option<&str>,
        message_id: Option<&str>,
    ) -> Result<AgentTurnEventReport> {
        let _lock = self.acquire_write_lock()?;
        let event_type = event_type.trim();
        if event_type.is_empty() {
            return Err(Error::InvalidInput(
                "event type cannot be empty".to_string(),
            ));
        }
        let turn = self.agent_turn(turn_id)?;
        if turn.ended_at.is_some() {
            return Err(Error::InvalidInput(format!(
                "turn `{turn_id}` is already ended"
            )));
        }
        let change_id = change_id
            .map(|change_id| {
                let change = ChangeId(change_id.to_string());
                self.operation(&change).map(|_| change)
            })
            .transpose()?;
        let message_id = message_id
            .map(|message_id| {
                self.message(message_id)
                    .map(|_| MessageId(message_id.to_string()))
            })
            .transpose()?;
        let event_id = self.insert_agent_event_with_context(
            &turn.agent_id,
            turn.session_id.as_deref(),
            Some(turn_id),
            event_type,
            change_id.as_ref(),
            message_id.as_ref(),
            &payload.unwrap_or(serde_json::Value::Null),
        )?;
        Ok(AgentTurnEventReport {
            event: self.agent_event(&event_id)?,
        })
    }

    pub fn show_agent_turn(&self, turn_id: &str) -> Result<AgentTurnDetails> {
        let turn = self.agent_turn(turn_id)?;
        let session = turn
            .session_id
            .as_deref()
            .map(|session_id| self.agent_session(session_id))
            .transpose()?;
        Ok(AgentTurnDetails {
            messages: self.agent_turn_messages(turn_id)?,
            events: self.agent_turn_events(turn_id)?,
            operations: self.agent_turn_operations(turn_id)?,
            turn,
            session,
        })
    }
}
