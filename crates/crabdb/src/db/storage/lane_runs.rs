use super::*;

impl CrabDb {
    pub(crate) fn validate_lane_run_context(
        &self,
        branch: &LaneBranch,
        session_id: Option<&str>,
        turn_id: Option<&str>,
    ) -> Result<(Option<String>, Option<String>)> {
        let turn = turn_id.map(|turn_id| self.lane_turn(turn_id)).transpose()?;
        if let Some(turn) = &turn {
            if turn.lane_id != branch.lane_id {
                return Err(Error::InvalidInput(format!(
                    "turn `{}` does not belong to lane `{}`",
                    turn.turn_id, branch.lane_id
                )));
            }
            if turn.ended_at.is_some() {
                return Err(Error::InvalidInput(format!(
                    "turn `{}` is already ended",
                    turn.turn_id
                )));
            }
        }
        let resolved_session_id = session_id
            .map(str::to_string)
            .or_else(|| turn.as_ref().and_then(|turn| turn.session_id.clone()))
            .or_else(|| branch.session_id.clone());
        if let Some(session_id) = resolved_session_id.as_deref() {
            let session = self.lane_session(session_id)?;
            if session.lane_id != branch.lane_id {
                return Err(Error::InvalidInput(format!(
                    "session `{session_id}` does not belong to lane `{}`",
                    branch.lane_id
                )));
            }
        }
        Ok((resolved_session_id, turn_id.map(str::to_string)))
    }

    pub(crate) fn insert_lane_run_state(
        &self,
        lane_id: &str,
        session_id: Option<&str>,
        turn_id: Option<&str>,
        approval_id: Option<&str>,
        reason: &str,
        summary: &str,
        state: Option<serde_json::Value>,
        interruption: Option<serde_json::Value>,
    ) -> Result<LaneRunState> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(Error::InvalidInput(
                "lane run pause reason cannot be empty".to_string(),
            ));
        }
        let summary = summary.trim();
        if summary.is_empty() {
            return Err(Error::InvalidInput(
                "lane run pause summary cannot be empty".to_string(),
            ));
        }
        let redacted_reason = redact_sensitive_text(reason);
        let redacted_summary = redact_sensitive_text(summary);
        let redacted_state = redact_sensitive_json(state.unwrap_or_else(|| serde_json::json!({})));
        let redacted_interruption = interruption.map(redact_sensitive_json);
        let seed = format!(
            "{}:{}:{}:{}:{}:{}",
            lane_id,
            session_id.unwrap_or("none"),
            turn_id.unwrap_or("none"),
            approval_id.unwrap_or("none"),
            redacted_reason,
            now_nanos()
        );
        let run_id = format!("run_{}", crate::ids::short_hash(seed.as_bytes(), 16));
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO lane_run_states \
             (run_id, lane_id, session_id, turn_id, approval_id, status, reason, summary, state_json, interruption_json, created_at, updated_at, resumed_at, reviewer, note) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'paused', ?6, ?7, ?8, ?9, ?10, ?10, NULL, NULL, NULL)",
            params![
                run_id,
                lane_id,
                session_id,
                turn_id,
                approval_id,
                redacted_reason,
                redacted_summary,
                serde_json::to_string(&redacted_state)?,
                redacted_interruption
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?,
                now
            ],
        )?;
        self.insert_lane_event_with_context(
            lane_id,
            session_id,
            turn_id,
            "run_paused",
            None,
            None,
            &serde_json::json!({
                "run_id": run_id,
                "approval_id": approval_id,
                "reason": redacted_reason,
                "summary": redacted_summary
            }),
        )?;
        self.lane_run_state(&run_id)
    }

    pub(crate) fn lane_run_state(&self, run_id: &str) -> Result<LaneRunState> {
        let run_id = run_id.trim();
        if run_id.is_empty() {
            return Err(Error::InvalidInput(
                "lane run id cannot be empty".to_string(),
            ));
        }
        self.conn
            .query_row(
                "SELECT run_id, lane_id, session_id, turn_id, approval_id, status, reason, summary, state_json, interruption_json, created_at, updated_at, resumed_at, reviewer, note \
                 FROM lane_run_states WHERE run_id = ?1",
                params![run_id],
                lane_run_state_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("lane run `{run_id}` not found")))
    }

    pub(crate) fn lane_run_states_for_approval(
        &self,
        approval_id: &str,
    ) -> Result<Vec<LaneRunState>> {
        let mut stmt = self.conn.prepare(
            "SELECT run_id, lane_id, session_id, turn_id, approval_id, status, reason, summary, state_json, interruption_json, created_at, updated_at, resumed_at, reviewer, note \
             FROM lane_run_states WHERE approval_id = ?1 ORDER BY updated_at DESC, run_id DESC",
        )?;
        let rows = stmt.query_map(params![approval_id], lane_run_state_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn lane_approval(&self, approval_id: &str) -> Result<LaneApproval> {
        self.conn
            .query_row(
                "SELECT approval_id, lane_id, session_id, turn_id, action, summary, payload_json, status, requested_at, decided_at, reviewer, note \
                 FROM lane_approvals WHERE approval_id = ?1",
                params![approval_id],
                lane_approval_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("approval `{approval_id}` not found")))
    }
}
