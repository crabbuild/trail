use super::*;

impl CrabDb {
    pub fn add_lane_session_event(
        &mut self,
        lane: &str,
        session_id: &str,
        event_type: &str,
        payload: Option<serde_json::Value>,
    ) -> Result<LaneTurnEventReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(lane)?;
        let branch = self.lane_branch(lane)?;
        let session = self.lane_session(session_id)?;
        if session.lane_id != branch.lane_id {
            return Err(Error::InvalidInput(format!(
                "session `{session_id}` does not belong to lane `{lane}`"
            )));
        }
        let event_id = self.insert_lane_event_with_context(
            &branch.lane_id,
            Some(session_id),
            None,
            event_type,
            None,
            None,
            &payload.unwrap_or(serde_json::Value::Null),
        )?;
        Ok(LaneTurnEventReport {
            event: self.lane_event(&event_id)?,
        })
    }

    pub fn list_lane_events(
        &self,
        lane: Option<&str>,
        session_id: Option<&str>,
        turn_id: Option<&str>,
        event_type: Option<&str>,
        limit: usize,
    ) -> Result<Vec<LaneEventRecord>> {
        let limit = normalize_query_limit(limit, 1000)?;
        let lane_id = lane
            .map(|lane| self.lane_branch(lane).map(|branch| branch.lane_id))
            .transpose()?;
        if let Some(session_id) = session_id {
            self.lane_session(session_id)?;
        }
        if let Some(turn_id) = turn_id {
            self.lane_turn(turn_id)?;
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

        let mut sql = "SELECT event_id, lane_id, session_id, turn_id, event_type, change_id, message_id, payload_json, created_at \
             FROM lane_events"
            .to_string();
        let mut filters = Vec::new();
        let mut values = Vec::new();
        if let Some(lane_id) = lane_id {
            filters.push("lane_id = ?");
            values.push(lane_id);
        }
        if let Some(session_id) = session_id {
            filters.push("session_id = ?");
            values.push(session_id.to_string());
        }
        if let Some(turn_id) = turn_id {
            filters.push("turn_id = ?");
            values.push(turn_id.to_string());
        }
        if let Some(event_type) = event_type {
            filters.push("event_type = ?");
            values.push(event_type.to_string());
        }
        if !filters.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&filters.join(" AND "));
        }
        sql.push_str(" ORDER BY created_at DESC, rowid DESC LIMIT ");
        sql.push_str(&limit.to_string());

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params_from_iter(values.iter().map(String::as_str)),
            lane_event_row,
        )?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }
}
