use super::*;

impl CrabDb {
    pub fn add_lane_message(
        &mut self,
        lane: &str,
        role: &str,
        text: &str,
        session_id: Option<String>,
    ) -> Result<LaneMessageReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(lane)?;
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
        let branch = self.lane_branch(lane)?;
        let session_id = session_id.or(branch.session_id.clone());
        if let Some(session_id) = &session_id {
            self.ensure_lane_session(&branch.lane_id, session_id, None)?;
            self.conn.execute(
                "UPDATE lane_branches SET session_id = ?1, updated_at = ?2 WHERE lane_id = ?3",
                params![session_id, now_ts(), branch.lane_id],
            )?;
        }
        let turn_id = self.open_lane_turn(
            &branch.lane_id,
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
            Some(&branch.lane_id),
            session_id.as_deref(),
            None,
            created_at,
        )?;
        self.insert_lane_event_with_context(
            &branch.lane_id,
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
        self.finish_lane_turn(&turn_id, "completed", None)?;
        Ok(LaneMessageReport {
            lane_id: branch.lane_id,
            message_id,
            role: role.to_string(),
            session_id,
        })
    }

    pub fn add_lane_turn_message(
        &mut self,
        turn_id: &str,
        role: &str,
        text: &str,
    ) -> Result<LaneMessageReport> {
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
        let turn = self.lane_turn(turn_id)?;
        if turn.ended_at.is_some() {
            return Err(Error::InvalidInput(format!(
                "turn `{turn_id}` is already ended"
            )));
        }
        let created_at = now_ts();
        let message_id = self.store_message(
            role,
            text,
            Some(&turn.lane_id),
            turn.session_id.as_deref(),
            None,
            created_at,
        )?;
        self.insert_lane_event_with_context(
            &turn.lane_id,
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
        Ok(LaneMessageReport {
            lane_id: turn.lane_id,
            message_id,
            role: role.to_string(),
            session_id: turn.session_id,
        })
    }
}
