use super::*;

impl CrabDb {
    pub fn list_agent_events(
        &self,
        agent: Option<&str>,
        session_id: Option<&str>,
        turn_id: Option<&str>,
        event_type: Option<&str>,
        limit: usize,
    ) -> Result<Vec<AgentEventRecord>> {
        let limit = normalize_query_limit(limit, 1000)?;
        let agent_id = agent
            .map(|agent| self.agent_branch(agent).map(|branch| branch.agent_id))
            .transpose()?;
        if let Some(session_id) = session_id {
            self.agent_session(session_id)?;
        }
        if let Some(turn_id) = turn_id {
            self.agent_turn(turn_id)?;
        }
        let event_type = event_type
            .map(str::trim)
            .map(|event_type| {
                if event_type.is_empty() {
                    Err(Error::InvalidInput(
                        "event type filter cannot be empty".to_string(),
                    ))
                } else {
                    Ok(event_type)
                }
            })
            .transpose()?;

        let mut stmt = self.conn.prepare(
            "SELECT event_id, agent_id, session_id, turn_id, event_type, change_id, message_id, payload_json, created_at \
             FROM agent_events \
             WHERE (?1 IS NULL OR agent_id = ?1) \
               AND (?2 IS NULL OR session_id = ?2) \
               AND (?3 IS NULL OR turn_id = ?3) \
               AND (?4 IS NULL OR event_type = ?4) \
             ORDER BY created_at DESC, rowid DESC LIMIT ?5",
        )?;
        let rows = stmt.query_map(
            params![agent_id, session_id, turn_id, event_type, limit as i64],
            agent_event_row,
        )?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }
}
