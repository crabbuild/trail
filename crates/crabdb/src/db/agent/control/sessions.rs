use super::*;

impl CrabDb {
    pub fn start_agent_session(
        &mut self,
        agent: &str,
        title: Option<String>,
        requested_session_id: Option<String>,
    ) -> Result<AgentSessionStartReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;
        let branch = self.agent_branch(agent)?;
        let session_id = match requested_session_id {
            Some(session_id) => {
                validate_session_id(&session_id)?;
                session_id
            }
            None => self.allocate_session_id(&branch.agent_id, title.as_deref()),
        };
        if self.try_agent_session(&session_id)?.is_some() {
            return Err(Error::InvalidInput(format!(
                "session `{session_id}` already exists"
            )));
        }
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO agent_sessions \
             (session_id, agent_id, title, status, started_at, ended_at, metadata_json) \
             VALUES (?1, ?2, ?3, 'active', ?4, NULL, NULL)",
            params![session_id, branch.agent_id, title, now],
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
                "title": title.clone()
            }),
        )?;
        Ok(AgentSessionStartReport {
            session: self.agent_session(&session_id)?,
        })
    }

    pub fn list_agent_sessions(&self, agent: Option<&str>) -> Result<Vec<AgentSession>> {
        if let Some(agent) = agent {
            let branch = self.agent_branch(agent)?;
            let mut stmt = self.conn.prepare(
                "SELECT session_id, agent_id, title, status, started_at, ended_at, metadata_json \
                 FROM agent_sessions WHERE agent_id = ?1 ORDER BY started_at DESC, session_id DESC",
            )?;
            let rows = stmt.query_map(params![branch.agent_id], agent_session_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Error::from)
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT session_id, agent_id, title, status, started_at, ended_at, metadata_json \
                 FROM agent_sessions ORDER BY started_at DESC, session_id DESC",
            )?;
            let rows = stmt.query_map([], agent_session_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Error::from)
        }
    }

    pub fn current_agent_sessions(
        &self,
        agent: Option<&str>,
    ) -> Result<Vec<AgentSessionCurrentReport>> {
        if let Some(agent) = agent {
            let details = self.agent_details(agent)?;
            let session = details
                .branch
                .session_id
                .as_deref()
                .map(|session_id| self.agent_session(session_id))
                .transpose()?;
            return Ok(vec![AgentSessionCurrentReport {
                agent_id: details.record.agent_id,
                agent_name: details.record.name,
                ref_name: details.branch.ref_name,
                session,
            }]);
        }

        let mut reports = Vec::new();
        for details in self.list_agents()? {
            let Some(session_id) = details.branch.session_id.as_deref() else {
                continue;
            };
            reports.push(AgentSessionCurrentReport {
                agent_id: details.record.agent_id,
                agent_name: details.record.name,
                ref_name: details.branch.ref_name,
                session: Some(self.agent_session(session_id)?),
            });
        }
        Ok(reports)
    }

    pub fn show_agent_session(&self, session_id: &str) -> Result<AgentSessionDetails> {
        let session = self.agent_session(session_id)?;
        let turns = self.agent_session_turns(session_id)?;
        let messages = self.agent_session_messages(session_id)?;
        let events = self.agent_session_events(session_id)?;
        let operations = self.agent_session_operations(session_id)?;
        Ok(AgentSessionDetails {
            session,
            turns,
            messages,
            events,
            operations,
        })
    }

    pub fn agent_session_context(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<AgentSessionContextReport> {
        let limit = normalize_query_limit(limit, 200)?;
        let session = self.agent_session(session_id)?;
        let turns = self.agent_session_turns(session_id)?;
        let messages = self.agent_session_messages(session_id)?;
        let events = self.agent_session_events(session_id)?;
        let operations = self.agent_session_operations(session_id)?;
        Ok(AgentSessionContextReport {
            session,
            message_count: messages.len() as u64,
            event_count: events.len() as u64,
            turn_count: turns.len() as u64,
            operation_count: operations.len() as u64,
            recent_messages: tail_limited(&messages, limit),
            recent_events: tail_limited(&events, limit),
            recent_turns: tail_limited(&turns, limit),
            recent_operations: tail_limited(&operations, limit),
        })
    }

    pub fn end_agent_session(
        &mut self,
        session_id: &str,
        status: &str,
    ) -> Result<AgentSessionEndReport> {
        let _lock = self.acquire_write_lock()?;
        let status = parse_session_end_status(status)?;
        let session = self.agent_session(session_id)?;
        let now = now_ts();
        self.conn.execute(
            "UPDATE agent_sessions SET status = ?1, ended_at = ?2 WHERE session_id = ?3",
            params![status, now, session_id],
        )?;
        self.conn.execute(
            "UPDATE agent_branches SET session_id = NULL, updated_at = ?1 \
             WHERE agent_id = ?2 AND session_id = ?3",
            params![now, session.agent_id, session_id],
        )?;
        self.insert_agent_event_with_context(
            &session.agent_id,
            Some(session_id),
            None,
            "session_ended",
            None,
            None,
            &serde_json::json!({
                "session_id": session_id,
                "status": status
            }),
        )?;
        Ok(AgentSessionEndReport {
            session: self.agent_session(session_id)?,
        })
    }
}
