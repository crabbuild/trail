use super::*;

impl CrabDb {
    pub fn request_agent_approval(
        &mut self,
        agent: &str,
        action: &str,
        summary: &str,
        payload: Option<serde_json::Value>,
        session_id: Option<&str>,
        turn_id: Option<&str>,
    ) -> Result<AgentApprovalRequestReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;
        let action = action.trim();
        if action.is_empty() {
            return Err(Error::InvalidInput(
                "approval action cannot be empty".to_string(),
            ));
        }
        let summary = summary.trim();
        if summary.is_empty() {
            return Err(Error::InvalidInput(
                "approval summary cannot be empty".to_string(),
            ));
        }
        let branch = self.agent_branch(agent)?;
        let turn = turn_id
            .map(|turn_id| self.agent_turn(turn_id))
            .transpose()?;
        if let Some(turn) = &turn {
            if turn.agent_id != branch.agent_id {
                return Err(Error::InvalidInput(format!(
                    "turn `{}` does not belong to agent `{}`",
                    turn.turn_id, branch.agent_id
                )));
            }
            if turn.ended_at.is_some() {
                return Err(Error::InvalidInput(format!(
                    "turn `{}` is already ended",
                    turn.turn_id
                )));
            }
        }
        let approval_session_id = session_id
            .map(str::to_string)
            .or_else(|| turn.as_ref().and_then(|turn| turn.session_id.clone()))
            .or_else(|| branch.session_id.clone());
        if let Some(session_id) = approval_session_id.as_deref() {
            let session = self.agent_session(session_id)?;
            if session.agent_id != branch.agent_id {
                return Err(Error::InvalidInput(format!(
                    "session `{session_id}` does not belong to agent `{}`",
                    branch.agent_id
                )));
            }
        }

        let requested_at = now_ts();
        let redacted_action = redact_sensitive_text(action);
        let redacted_summary = redact_sensitive_text(summary);
        let redacted_payload = payload.map(redact_sensitive_json);
        let seed = format!(
            "{}:{}:{}:{}:{}",
            branch.agent_id,
            approval_session_id.as_deref().unwrap_or("none"),
            turn_id.unwrap_or("none"),
            redacted_action,
            now_nanos()
        );
        let approval_id = format!("approval_{}", crate::ids::short_hash(seed.as_bytes(), 16));
        let payload_json = redacted_payload
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        self.conn.execute(
            "INSERT INTO agent_approvals \
             (approval_id, agent_id, session_id, turn_id, action, summary, payload_json, status, requested_at, decided_at, reviewer, note) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8, NULL, NULL, NULL)",
            params![
                approval_id,
                branch.agent_id,
                approval_session_id,
                turn_id,
                redacted_action.clone(),
                redacted_summary.clone(),
                payload_json,
                requested_at
            ],
        )?;
        self.insert_agent_event_with_context(
            &branch.agent_id,
            approval_session_id.as_deref(),
            turn_id,
            "approval_requested",
            None,
            None,
            &serde_json::json!({
                "approval_id": approval_id,
                "action": redacted_action,
                "summary": redacted_summary
            }),
        )?;
        let approval = self.agent_approval(&approval_id)?;
        let run_state = self.insert_agent_run_state(
            &approval.agent_id,
            approval.session_id.as_deref(),
            approval.turn_id.as_deref(),
            Some(&approval.approval_id),
            "approval_required",
            &approval.summary,
            Some(serde_json::json!({
                "agent_id": approval.agent_id.clone(),
                "session_id": approval.session_id.clone(),
                "turn_id": approval.turn_id.clone(),
                "approval_id": approval.approval_id.clone(),
                "action": approval.action.clone(),
                "summary": approval.summary.clone(),
                "payload": approval.payload.clone()
            })),
            Some(serde_json::json!({
                "type": "approval_required",
                "approval_id": approval.approval_id.clone(),
                "action": approval.action.clone(),
                "summary": approval.summary.clone()
            })),
        )?;
        Ok(AgentApprovalRequestReport {
            approval,
            run_state: Some(run_state),
        })
    }

    pub fn list_agent_approvals(
        &self,
        agent: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<AgentApproval>> {
        let status = status
            .map(parse_approval_status_filter)
            .transpose()?
            .flatten();
        match (agent, status) {
            (Some(agent), Some(status)) => {
                let branch = self.agent_branch(agent)?;
                let mut stmt = self.conn.prepare(
                    "SELECT approval_id, agent_id, session_id, turn_id, action, summary, payload_json, status, requested_at, decided_at, reviewer, note \
                     FROM agent_approvals WHERE agent_id = ?1 AND status = ?2 ORDER BY requested_at DESC, approval_id DESC",
                )?;
                let rows = stmt.query_map(params![branch.agent_id, status], agent_approval_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
            (Some(agent), None) => {
                let branch = self.agent_branch(agent)?;
                let mut stmt = self.conn.prepare(
                    "SELECT approval_id, agent_id, session_id, turn_id, action, summary, payload_json, status, requested_at, decided_at, reviewer, note \
                     FROM agent_approvals WHERE agent_id = ?1 ORDER BY requested_at DESC, approval_id DESC",
                )?;
                let rows = stmt.query_map(params![branch.agent_id], agent_approval_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
            (None, Some(status)) => {
                let mut stmt = self.conn.prepare(
                    "SELECT approval_id, agent_id, session_id, turn_id, action, summary, payload_json, status, requested_at, decided_at, reviewer, note \
                     FROM agent_approvals WHERE status = ?1 ORDER BY requested_at DESC, approval_id DESC",
                )?;
                let rows = stmt.query_map(params![status], agent_approval_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
            (None, None) => {
                let mut stmt = self.conn.prepare(
                    "SELECT approval_id, agent_id, session_id, turn_id, action, summary, payload_json, status, requested_at, decided_at, reviewer, note \
                     FROM agent_approvals ORDER BY requested_at DESC, approval_id DESC",
                )?;
                let rows = stmt.query_map([], agent_approval_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
        }
    }

    pub fn show_agent_approval(&self, approval_id: &str) -> Result<AgentApproval> {
        self.agent_approval(approval_id)
    }

    pub fn decide_agent_approval(
        &mut self,
        approval_id: &str,
        decision: &str,
        reviewer: Option<String>,
        note: Option<String>,
    ) -> Result<AgentApprovalDecisionReport> {
        let _lock = self.acquire_write_lock()?;
        let decision = parse_approval_decision(decision)?;
        let approval = self.agent_approval(approval_id)?;
        if approval.status != "pending" {
            return Err(Error::InvalidInput(format!(
                "approval `{approval_id}` is already {}",
                approval.status
            )));
        }
        let reviewer = reviewer.map(|reviewer| redact_sensitive_text(&reviewer));
        let note = note.map(|note| redact_sensitive_text(&note));
        let decided_at = now_ts();
        self.conn.execute(
            "UPDATE agent_approvals SET status = ?1, decided_at = ?2, reviewer = ?3, note = ?4 WHERE approval_id = ?5",
            params![decision, decided_at, reviewer.clone(), note.clone(), approval_id],
        )?;
        self.insert_agent_event_with_context(
            &approval.agent_id,
            approval.session_id.as_deref(),
            approval.turn_id.as_deref(),
            "approval_decided",
            None,
            None,
            &serde_json::json!({
                "approval_id": approval_id,
                "decision": decision,
                "reviewer": reviewer,
                "note": note
            }),
        )?;
        if matches!(decision, "rejected" | "cancelled") {
            let run_status = if decision == "rejected" {
                "blocked"
            } else {
                "cancelled"
            };
            self.conn.execute(
                "UPDATE agent_run_states SET status = ?1, updated_at = ?2, reviewer = ?3, note = ?4 WHERE approval_id = ?5 AND status = 'paused'",
                params![run_status, decided_at, reviewer.clone(), note.clone(), approval_id],
            )?;
        }
        Ok(AgentApprovalDecisionReport {
            approval: self.agent_approval(approval_id)?,
            decision: decision.to_string(),
            run_states: self.agent_run_states_for_approval(approval_id)?,
        })
    }
}
