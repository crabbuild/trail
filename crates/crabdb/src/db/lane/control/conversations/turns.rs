use super::*;

impl CrabDb {
    pub fn begin_lane_session_turn(
        &mut self,
        lane: &str,
        session_id: &str,
        metadata: Option<serde_json::Value>,
    ) -> Result<LaneTurnStartReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(lane)?;
        let branch = self.lane_branch(lane)?;
        let session = self.lane_session(session_id)?;
        if session.lane_id != branch.lane_id {
            return Err(Error::InvalidInput(format!(
                "session `{session_id}` does not belong to lane `{lane}`"
            )));
        }
        if session.status != "active" {
            return Err(Error::InvalidInput(format!(
                "session `{session_id}` is {}",
                session.status
            )));
        }

        self.conn.execute(
            "UPDATE lane_branches SET session_id = ?1, updated_at = ?2 WHERE lane_id = ?3",
            params![session_id, now_ts(), branch.lane_id],
        )?;
        let turn_id = self.open_lane_turn(
            &branch.lane_id,
            Some(session_id),
            &branch.base_change,
            &branch.head_change,
            Some(&metadata.unwrap_or_else(|| serde_json::json!({ "kind": "session_turn" }))),
        )?;
        self.insert_lane_event_with_context(
            &branch.lane_id,
            Some(session_id),
            Some(&turn_id),
            "turn_started",
            None,
            None,
            &serde_json::json!({
                "turn_id": turn_id.clone(),
                "session_id": session_id,
                "source": "session"
            }),
        )?;
        Ok(LaneTurnStartReport {
            turn: self.lane_turn(&turn_id)?,
            session,
            base_root: branch.head_root,
        })
    }

    pub fn begin_lane_turn(
        &mut self,
        lane: &str,
        from: Option<&str>,
        session_title: Option<String>,
        base_change: Option<&str>,
    ) -> Result<LaneTurnStartReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(lane)?;

        let branch = self.lane_branch_for_turn(lane, from, base_change)?;

        if let Some(expected_base) = base_change {
            if branch.head_change.0 != expected_base {
                return Err(Error::StaleBranch(branch.ref_name));
            }
        }

        let session_id = self.allocate_session_id(&branch.lane_id, session_title.as_deref());
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO lane_sessions \
             (session_id, lane_id, title, status, started_at, ended_at, metadata_json) \
             VALUES (?1, ?2, ?3, 'active', ?4, NULL, NULL)",
            params![session_id, branch.lane_id, session_title, now],
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
                "title": session_title.clone(),
                "source": "api"
            }),
        )?;
        let turn_id = self.open_lane_turn(
            &branch.lane_id,
            Some(&session_id),
            &branch.base_change,
            &branch.head_change,
            Some(&serde_json::json!({
                "kind": "api_turn",
                "from": from,
                "base_change": base_change
            })),
        )?;
        self.insert_lane_event_with_context(
            &branch.lane_id,
            Some(&session_id),
            Some(&turn_id),
            "turn_started",
            None,
            None,
            &serde_json::json!({
                "turn_id": turn_id.clone()
            }),
        )?;
        Ok(LaneTurnStartReport {
            turn: self.lane_turn(&turn_id)?,
            session: self.lane_session(&session_id)?,
            base_root: branch.head_root,
        })
    }

    pub fn add_lane_turn_event(
        &mut self,
        turn_id: &str,
        event_type: &str,
        payload: Option<serde_json::Value>,
        change_id: Option<&str>,
        message_id: Option<&str>,
    ) -> Result<LaneTurnEventReport> {
        let _lock = self.acquire_write_lock()?;
        let event_type = event_type.trim();
        if event_type.is_empty() {
            return Err(Error::InvalidInput(
                "event type cannot be empty".to_string(),
            ));
        }
        let turn = self.lane_turn(turn_id)?;
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
        let event_id = self.insert_lane_event_with_context(
            &turn.lane_id,
            turn.session_id.as_deref(),
            Some(turn_id),
            event_type,
            change_id.as_ref(),
            message_id.as_ref(),
            &payload.unwrap_or(serde_json::Value::Null),
        )?;
        Ok(LaneTurnEventReport {
            event: self.lane_event(&event_id)?,
        })
    }

    pub(crate) fn add_lane_turn_events_batch(
        &mut self,
        turn_id: &str,
        events: Vec<(
            String,
            Option<serde_json::Value>,
            Option<String>,
            Option<String>,
        )>,
    ) -> Result<usize> {
        if events.is_empty() {
            return Ok(0);
        }
        let _lock = self.acquire_write_lock()?;
        let turn = self.lane_turn(turn_id)?;
        if turn.ended_at.is_some() {
            return Err(Error::InvalidInput(format!(
                "turn `{turn_id}` is already ended"
            )));
        }
        let mut inserted = 0usize;
        for (event_type, payload, change_id, message_id) in events {
            let event_type = event_type.trim().to_string();
            if event_type.is_empty() {
                return Err(Error::InvalidInput(
                    "event type cannot be empty".to_string(),
                ));
            }
            let change_id = change_id
                .map(|change_id| {
                    let change = ChangeId(change_id);
                    self.operation(&change).map(|_| change)
                })
                .transpose()?;
            let message_id = message_id
                .map(|message_id| self.message(&message_id).map(|_| MessageId(message_id)))
                .transpose()?;
            self.insert_lane_event_with_context(
                &turn.lane_id,
                turn.session_id.as_deref(),
                Some(turn_id),
                &event_type,
                change_id.as_ref(),
                message_id.as_ref(),
                &payload.unwrap_or(serde_json::Value::Null),
            )?;
            inserted += 1;
        }
        Ok(inserted)
    }

    pub fn show_lane_turn(&self, turn_id: &str) -> Result<LaneTurnDetails> {
        let turn = self.lane_turn(turn_id)?;
        let session = turn
            .session_id
            .as_deref()
            .map(|session_id| self.lane_session(session_id))
            .transpose()?;
        Ok(LaneTurnDetails {
            messages: self.lane_turn_messages(turn_id)?,
            events: self.lane_turn_events(turn_id)?,
            operations: self.lane_turn_operations(turn_id)?,
            turn,
            session,
        })
    }
}
