use super::*;

impl CrabDb {
    pub fn pause_lane_run(
        &mut self,
        lane: &str,
        reason: &str,
        summary: &str,
        state: Option<serde_json::Value>,
        interruption: Option<serde_json::Value>,
        session_id: Option<&str>,
        turn_id: Option<&str>,
    ) -> Result<LaneRunPauseReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(lane)?;
        let branch = self.lane_branch(lane)?;
        let (session_id, turn_id) = self.validate_lane_run_context(&branch, session_id, turn_id)?;
        let run_state = self.insert_lane_run_state(
            &branch.lane_id,
            session_id.as_deref(),
            turn_id.as_deref(),
            None,
            reason,
            summary,
            state,
            interruption,
        )?;
        Ok(LaneRunPauseReport { run_state })
    }

    pub fn list_lane_run_states(
        &self,
        lane: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<LaneRunState>> {
        let status = status
            .map(parse_lane_run_status_filter)
            .transpose()?
            .flatten();
        match (lane, status) {
            (Some(lane), Some(status)) => {
                let branch = self.lane_branch(lane)?;
                let mut stmt = self.conn.prepare(
                    "SELECT run_id, lane_id, session_id, turn_id, approval_id, status, reason, summary, state_json, interruption_json, created_at, updated_at, resumed_at, reviewer, note \
                     FROM lane_run_states WHERE lane_id = ?1 AND status = ?2 ORDER BY updated_at DESC, run_id DESC",
                )?;
                let rows = stmt.query_map(params![branch.lane_id, status], lane_run_state_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
            (Some(lane), None) => {
                let branch = self.lane_branch(lane)?;
                let mut stmt = self.conn.prepare(
                    "SELECT run_id, lane_id, session_id, turn_id, approval_id, status, reason, summary, state_json, interruption_json, created_at, updated_at, resumed_at, reviewer, note \
                     FROM lane_run_states WHERE lane_id = ?1 ORDER BY updated_at DESC, run_id DESC",
                )?;
                let rows = stmt.query_map(params![branch.lane_id], lane_run_state_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
            (None, Some(status)) => {
                let mut stmt = self.conn.prepare(
                    "SELECT run_id, lane_id, session_id, turn_id, approval_id, status, reason, summary, state_json, interruption_json, created_at, updated_at, resumed_at, reviewer, note \
                     FROM lane_run_states WHERE status = ?1 ORDER BY updated_at DESC, run_id DESC",
                )?;
                let rows = stmt.query_map(params![status], lane_run_state_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
            (None, None) => {
                let mut stmt = self.conn.prepare(
                    "SELECT run_id, lane_id, session_id, turn_id, approval_id, status, reason, summary, state_json, interruption_json, created_at, updated_at, resumed_at, reviewer, note \
                     FROM lane_run_states ORDER BY updated_at DESC, run_id DESC",
                )?;
                let rows = stmt.query_map([], lane_run_state_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
        }
    }

    pub fn show_lane_run_state(&self, run_id: &str) -> Result<LaneRunState> {
        self.lane_run_state(run_id)
    }

    pub fn resume_lane_run(
        &mut self,
        run_id: &str,
        reviewer: Option<String>,
        note: Option<String>,
    ) -> Result<LaneRunResumeReport> {
        let _lock = self.acquire_write_lock()?;
        let run_state = self.lane_run_state(run_id)?;
        if run_state.status != "paused" {
            return Err(Error::InvalidInput(format!(
                "lane run `{}` is {} and cannot be resumed",
                run_state.run_id, run_state.status
            )));
        }
        if let Some(approval_id) = run_state.approval_id.as_deref() {
            let approval = self.lane_approval(approval_id)?;
            if approval.status != "approved" {
                return Err(Error::InvalidInput(format!(
                    "lane run `{}` is waiting on approval `{approval_id}` ({})",
                    run_state.run_id, approval.status
                )));
            }
        }
        let reviewer = reviewer.map(|reviewer| redact_sensitive_text(&reviewer));
        let note = note.map(|note| redact_sensitive_text(&note));
        let now = now_ts();
        self.conn.execute(
            "UPDATE lane_run_states SET status = 'resumed', updated_at = ?1, resumed_at = ?1, reviewer = ?2, note = ?3 WHERE run_id = ?4",
            params![now, reviewer.clone(), note.clone(), run_state.run_id],
        )?;
        self.insert_lane_event_with_context(
            &run_state.lane_id,
            run_state.session_id.as_deref(),
            run_state.turn_id.as_deref(),
            "run_resumed",
            None,
            None,
            &serde_json::json!({
                "run_id": run_state.run_id,
                "approval_id": run_state.approval_id,
                "reviewer": reviewer,
                "note": note
            }),
        )?;
        Ok(LaneRunResumeReport {
            run_state: self.lane_run_state(run_id)?,
        })
    }
}
