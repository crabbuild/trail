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

    pub fn begin_agent_turn(
        &mut self,
        agent: &str,
        from: Option<&str>,
        session_title: Option<String>,
        base_change: Option<&str>,
    ) -> Result<AgentTurnStartReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;

        let branch = match self.agent_branch(agent) {
            Ok(branch) => branch,
            Err(Error::RefNotFound(_)) => {
                let source_selector = match base_change.or(from) {
                    Some(selector) => selector.to_string(),
                    None => self.current_branch()?,
                };
                let source = self.resolve_refish(&source_selector)?;
                let agent_id = format!("agent_{}", crate::ids::short_hash(agent.as_bytes(), 8));
                let ref_name = agent_ref(agent);
                if self.try_get_ref(&ref_name)?.is_some() {
                    return Err(Error::InvalidInput(format!(
                        "agent `{agent}` already exists"
                    )));
                }
                let workdir = if self.config.agent.default_materialize {
                    let dir = self.materialize_agent_workdir(agent, &source.root_id, None)?;
                    Some(dir.to_string_lossy().to_string())
                } else {
                    None
                };
                self.set_ref(
                    &ref_name,
                    &source.change_id,
                    &source.root_id,
                    &source.operation_id,
                )?;
                let now = now_ts();
                self.conn.execute(
                    "INSERT INTO agents (agent_id, name, kind, provider, model, created_at, metadata_json) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        agent_id,
                        agent,
                        "coding-agent",
                        Option::<String>::None,
                        Option::<String>::None,
                        now,
                        Option::<String>::None
                    ],
                )?;
                self.conn.execute(
                    "INSERT INTO agent_branches \
                     (agent_id, ref_name, base_change, head_change, base_root, head_root, session_id, workdir, status, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, 'active', ?8, ?8)",
                    params![
                        agent_id,
                        ref_name,
                        source.change_id.0,
                        source.change_id.0,
                        source.root_id.0,
                        source.root_id.0,
                        workdir.clone(),
                        now
                    ],
                )?;
                self.insert_agent_event(
                    &format!("agent_{}", crate::ids::short_hash(agent.as_bytes(), 8)),
                    "agent_spawned",
                    Some(&source.change_id),
                    None,
                    &serde_json::json!({
                        "ref_name": agent_ref(agent),
                        "base_root": source.root_id.0.clone(),
                        "workdir": workdir.clone(),
                        "source": "api"
                    }),
                )?;
                self.agent_branch(agent)?
            }
            Err(err) => return Err(err),
        };

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
