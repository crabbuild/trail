use super::*;

impl CrabDb {
    pub fn pause_agent_run(
        &mut self,
        agent: &str,
        reason: &str,
        summary: &str,
        state: Option<serde_json::Value>,
        interruption: Option<serde_json::Value>,
        session_id: Option<&str>,
        turn_id: Option<&str>,
    ) -> Result<AgentRunPauseReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;
        let branch = self.agent_branch(agent)?;
        let (session_id, turn_id) =
            self.validate_agent_run_context(&branch, session_id, turn_id)?;
        let run_state = self.insert_agent_run_state(
            &branch.agent_id,
            session_id.as_deref(),
            turn_id.as_deref(),
            None,
            reason,
            summary,
            state,
            interruption,
        )?;
        Ok(AgentRunPauseReport { run_state })
    }

    pub fn list_agent_run_states(
        &self,
        agent: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<AgentRunState>> {
        let status = status
            .map(parse_agent_run_status_filter)
            .transpose()?
            .flatten();
        match (agent, status) {
            (Some(agent), Some(status)) => {
                let branch = self.agent_branch(agent)?;
                let mut stmt = self.conn.prepare(
                    "SELECT run_id, agent_id, session_id, turn_id, approval_id, status, reason, summary, state_json, interruption_json, created_at, updated_at, resumed_at, reviewer, note \
                     FROM agent_run_states WHERE agent_id = ?1 AND status = ?2 ORDER BY updated_at DESC, run_id DESC",
                )?;
                let rows = stmt.query_map(params![branch.agent_id, status], agent_run_state_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
            (Some(agent), None) => {
                let branch = self.agent_branch(agent)?;
                let mut stmt = self.conn.prepare(
                    "SELECT run_id, agent_id, session_id, turn_id, approval_id, status, reason, summary, state_json, interruption_json, created_at, updated_at, resumed_at, reviewer, note \
                     FROM agent_run_states WHERE agent_id = ?1 ORDER BY updated_at DESC, run_id DESC",
                )?;
                let rows = stmt.query_map(params![branch.agent_id], agent_run_state_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
            (None, Some(status)) => {
                let mut stmt = self.conn.prepare(
                    "SELECT run_id, agent_id, session_id, turn_id, approval_id, status, reason, summary, state_json, interruption_json, created_at, updated_at, resumed_at, reviewer, note \
                     FROM agent_run_states WHERE status = ?1 ORDER BY updated_at DESC, run_id DESC",
                )?;
                let rows = stmt.query_map(params![status], agent_run_state_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
            (None, None) => {
                let mut stmt = self.conn.prepare(
                    "SELECT run_id, agent_id, session_id, turn_id, approval_id, status, reason, summary, state_json, interruption_json, created_at, updated_at, resumed_at, reviewer, note \
                     FROM agent_run_states ORDER BY updated_at DESC, run_id DESC",
                )?;
                let rows = stmt.query_map([], agent_run_state_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
        }
    }

    pub fn show_agent_run_state(&self, run_id: &str) -> Result<AgentRunState> {
        self.agent_run_state(run_id)
    }

    pub fn resume_agent_run(
        &mut self,
        run_id: &str,
        reviewer: Option<String>,
        note: Option<String>,
    ) -> Result<AgentRunResumeReport> {
        let _lock = self.acquire_write_lock()?;
        let run_state = self.agent_run_state(run_id)?;
        if run_state.status != "paused" {
            return Err(Error::InvalidInput(format!(
                "agent run `{}` is {} and cannot be resumed",
                run_state.run_id, run_state.status
            )));
        }
        if let Some(approval_id) = run_state.approval_id.as_deref() {
            let approval = self.agent_approval(approval_id)?;
            if approval.status != "approved" {
                return Err(Error::InvalidInput(format!(
                    "agent run `{}` is waiting on approval `{approval_id}` ({})",
                    run_state.run_id, approval.status
                )));
            }
        }
        let reviewer = reviewer.map(|reviewer| redact_sensitive_text(&reviewer));
        let note = note.map(|note| redact_sensitive_text(&note));
        let now = now_ts();
        self.conn.execute(
            "UPDATE agent_run_states SET status = 'resumed', updated_at = ?1, resumed_at = ?1, reviewer = ?2, note = ?3 WHERE run_id = ?4",
            params![now, reviewer.clone(), note.clone(), run_state.run_id],
        )?;
        self.insert_agent_event_with_context(
            &run_state.agent_id,
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
        Ok(AgentRunResumeReport {
            run_state: self.agent_run_state(run_id)?,
        })
    }
}
