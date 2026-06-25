use super::*;

impl CrabDb {
    pub fn add_agent_message(
        &mut self,
        agent: &str,
        role: &str,
        text: &str,
        session_id: Option<String>,
    ) -> Result<AgentMessageReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;
        if role.trim().is_empty() {
            return Err(Error::InvalidInput(
                "message role cannot be empty".to_string(),
            ));
        }
        if text.is_empty() {
            return Err(Error::InvalidInput(
                "message text cannot be empty".to_string(),
            ));
        }
        let branch = self.agent_branch(agent)?;
        let session_id = session_id.or(branch.session_id.clone());
        if let Some(session_id) = &session_id {
            self.ensure_agent_session(&branch.agent_id, session_id, None)?;
            self.conn.execute(
                "UPDATE agent_branches SET session_id = ?1, updated_at = ?2 WHERE agent_id = ?3",
                params![session_id, now_ts(), branch.agent_id],
            )?;
        }
        let turn_id = self.open_agent_turn(
            &branch.agent_id,
            session_id.as_deref(),
            &branch.base_change,
            &branch.head_change,
            Some(&serde_json::json!({
                "kind": "message",
                "role": role
            })),
        )?;
        let created_at = now_ts();
        let message_id = self.store_message(
            role,
            text,
            Some(&branch.agent_id),
            session_id.as_deref(),
            None,
            created_at,
        )?;
        self.insert_agent_event_with_context(
            &branch.agent_id,
            session_id.as_deref(),
            Some(&turn_id),
            "message_added",
            None,
            Some(&message_id),
            &serde_json::json!({
                "role": role,
                "session_id": session_id.clone()
            }),
        )?;
        self.finish_agent_turn(&turn_id, "completed", None)?;
        Ok(AgentMessageReport {
            agent_id: branch.agent_id,
            message_id,
            role: role.to_string(),
            session_id,
        })
    }

    pub fn add_agent_turn_message(
        &mut self,
        turn_id: &str,
        role: &str,
        text: &str,
    ) -> Result<AgentMessageReport> {
        let _lock = self.acquire_write_lock()?;
        if role.trim().is_empty() {
            return Err(Error::InvalidInput(
                "message role cannot be empty".to_string(),
            ));
        }
        if text.is_empty() {
            return Err(Error::InvalidInput(
                "message text cannot be empty".to_string(),
            ));
        }
        let turn = self.agent_turn(turn_id)?;
        if turn.ended_at.is_some() {
            return Err(Error::InvalidInput(format!(
                "turn `{turn_id}` is already ended"
            )));
        }
        let created_at = now_ts();
        let message_id = self.store_message(
            role,
            text,
            Some(&turn.agent_id),
            turn.session_id.as_deref(),
            None,
            created_at,
        )?;
        self.insert_agent_event_with_context(
            &turn.agent_id,
            turn.session_id.as_deref(),
            Some(turn_id),
            "message_added",
            None,
            Some(&message_id),
            &serde_json::json!({
                "role": role,
                "session_id": turn.session_id
            }),
        )?;
        Ok(AgentMessageReport {
            agent_id: turn.agent_id,
            message_id,
            role: role.to_string(),
            session_id: turn.session_id,
        })
    }
}
