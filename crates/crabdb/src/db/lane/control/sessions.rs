use super::*;

impl CrabDb {
    pub fn start_lane_session(
        &mut self,
        lane: &str,
        title: Option<String>,
        requested_session_id: Option<String>,
    ) -> Result<LaneSessionStartReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(lane)?;
        let branch = self.lane_branch(lane)?;
        let session_id = match requested_session_id {
            Some(session_id) => {
                validate_session_id(&session_id)?;
                session_id
            }
            None => self.allocate_session_id(&branch.lane_id, title.as_deref()),
        };
        if self.try_lane_session(&session_id)?.is_some() {
            return Err(Error::InvalidInput(format!(
                "session `{session_id}` already exists"
            )));
        }
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO lane_sessions \
             (session_id, lane_id, title, status, started_at, ended_at, metadata_json) \
             VALUES (?1, ?2, ?3, 'active', ?4, NULL, NULL)",
            params![session_id, branch.lane_id, title, now],
        )?;
        self.conn.execute(
            "UPDATE lane_branches SET session_id = ?1, updated_at = ?2 WHERE lane_id = ?3",
            params![session_id, now, branch.lane_id],
        )?;
        self.insert_lane_event_with_context(
            &branch.lane_id,
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
        Ok(LaneSessionStartReport {
            session: self.lane_session(&session_id)?,
        })
    }

    pub fn list_lane_sessions(&self, lane: Option<&str>) -> Result<Vec<LaneSession>> {
        if let Some(lane) = lane {
            let branch = self.lane_branch(lane)?;
            let mut stmt = self.conn.prepare(
                "SELECT session_id, lane_id, title, status, started_at, ended_at, metadata_json \
                 FROM lane_sessions WHERE lane_id = ?1 ORDER BY started_at DESC, session_id DESC",
            )?;
            let rows = stmt.query_map(params![branch.lane_id], lane_session_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Error::from)
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT session_id, lane_id, title, status, started_at, ended_at, metadata_json \
                 FROM lane_sessions ORDER BY started_at DESC, session_id DESC",
            )?;
            let rows = stmt.query_map([], lane_session_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Error::from)
        }
    }

    pub fn current_lane_sessions(
        &self,
        lane: Option<&str>,
    ) -> Result<Vec<LaneSessionCurrentReport>> {
        if let Some(lane) = lane {
            let details = self.lane_details(lane)?;
            let session = details
                .branch
                .session_id
                .as_deref()
                .map(|session_id| self.lane_session(session_id))
                .transpose()?;
            return Ok(vec![LaneSessionCurrentReport {
                lane_id: details.record.lane_id,
                lane_name: details.record.name,
                ref_name: details.branch.ref_name,
                session,
            }]);
        }

        let mut reports = Vec::new();
        for details in self.list_lanes()? {
            let Some(session_id) = details.branch.session_id.as_deref() else {
                continue;
            };
            reports.push(LaneSessionCurrentReport {
                lane_id: details.record.lane_id,
                lane_name: details.record.name,
                ref_name: details.branch.ref_name,
                session: Some(self.lane_session(session_id)?),
            });
        }
        Ok(reports)
    }

    pub fn show_lane_session(&self, session_id: &str) -> Result<LaneSessionDetails> {
        let session = self.lane_session(session_id)?;
        let turns = self.lane_session_turns(session_id)?;
        let messages = self.lane_session_messages(session_id)?;
        let events = self.lane_session_events(session_id)?;
        let operations = self.lane_session_operations(session_id)?;
        Ok(LaneSessionDetails {
            session,
            turns,
            messages,
            events,
            operations,
        })
    }

    pub fn lane_session_context(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<LaneSessionContextReport> {
        let limit = normalize_query_limit(limit, 200)?;
        let session = self.lane_session(session_id)?;
        let turns = self.lane_session_turns(session_id)?;
        let messages = self.lane_session_messages(session_id)?;
        let events = self.lane_session_events(session_id)?;
        let operations = self.lane_session_operations(session_id)?;
        Ok(LaneSessionContextReport {
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

    pub fn end_lane_session(
        &mut self,
        session_id: &str,
        status: &str,
    ) -> Result<LaneSessionEndReport> {
        let _lock = self.acquire_write_lock()?;
        let status = parse_session_end_status(status)?;
        let session = self.lane_session(session_id)?;
        let now = now_ts();
        self.conn.execute(
            "UPDATE lane_sessions SET status = ?1, ended_at = ?2 WHERE session_id = ?3",
            params![status, now, session_id],
        )?;
        self.conn.execute(
            "UPDATE lane_branches SET session_id = NULL, updated_at = ?1 \
             WHERE lane_id = ?2 AND session_id = ?3",
            params![now, session.lane_id, session_id],
        )?;
        self.insert_lane_event_with_context(
            &session.lane_id,
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
        Ok(LaneSessionEndReport {
            session: self.lane_session(session_id)?,
        })
    }
}
