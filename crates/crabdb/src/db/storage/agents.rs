use super::*;

impl CrabDb {
    pub(crate) fn allocate_session_id(&self, agent_id: &str, title: Option<&str>) -> String {
        let seed = format!(
            "{}:{}:{}:{}",
            self.config.workspace.id.0,
            agent_id,
            title.unwrap_or("session"),
            now_nanos()
        );
        format!("session_{}", crate::ids::short_hash(seed.as_bytes(), 16))
    }

    pub(crate) fn ensure_agent_session(
        &self,
        agent_id: &str,
        session_id: &str,
        title: Option<&str>,
    ) -> Result<()> {
        validate_session_id(session_id)?;
        if let Some(existing) = self.try_agent_session(session_id)? {
            if existing.agent_id != agent_id {
                return Err(Error::InvalidInput(format!(
                    "session `{session_id}` belongs to another agent"
                )));
            }
            return Ok(());
        }
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO agent_sessions \
             (session_id, agent_id, title, status, started_at, ended_at, metadata_json) \
             VALUES (?1, ?2, ?3, 'active', ?4, NULL, NULL)",
            params![session_id, agent_id, title, now],
        )?;
        Ok(())
    }

    pub(crate) fn try_agent_session(&self, session_id: &str) -> Result<Option<AgentSession>> {
        self.conn
            .query_row(
                "SELECT session_id, agent_id, title, status, started_at, ended_at, metadata_json \
                 FROM agent_sessions WHERE session_id = ?1",
                params![session_id],
                agent_session_row,
            )
            .optional()
            .map_err(Error::from)
    }

    pub(crate) fn agent_session(&self, session_id: &str) -> Result<AgentSession> {
        self.try_agent_session(session_id)?
            .ok_or_else(|| Error::InvalidInput(format!("session `{session_id}` not found")))
    }

    pub(crate) fn open_agent_turn(
        &self,
        agent_id: &str,
        session_id: Option<&str>,
        base_change: &ChangeId,
        before_change: &ChangeId,
        metadata_json: Option<&serde_json::Value>,
    ) -> Result<String> {
        if let Some(session_id) = session_id {
            self.ensure_agent_session(agent_id, session_id, None)?;
        }
        let seed = format!(
            "{}:{}:{}:{}:{}",
            agent_id,
            session_id.unwrap_or("none"),
            base_change.0,
            before_change.0,
            now_nanos()
        );
        let turn_id = format!("turn_{}", crate::ids::short_hash(seed.as_bytes(), 16));
        self.conn.execute(
            "INSERT INTO agent_turns \
             (turn_id, agent_id, session_id, base_change, before_change, after_change, status, started_at, ended_at, metadata_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, 'started', ?6, NULL, ?7)",
            params![
                turn_id,
                agent_id,
                session_id,
                base_change.0,
                before_change.0,
                now_ts(),
                metadata_json.map(serde_json::to_string).transpose()?
            ],
        )?;
        Ok(turn_id)
    }

    pub(crate) fn finish_agent_turn(
        &self,
        turn_id: &str,
        status: &str,
        after_change: Option<&ChangeId>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE agent_turns SET status = ?1, after_change = ?2, ended_at = ?3 WHERE turn_id = ?4",
            params![
                status,
                after_change.map(|change_id| change_id.0.clone()),
                now_ts(),
                turn_id
            ],
        )?;
        Ok(())
    }

    pub(crate) fn update_agent_turn_progress(
        &self,
        turn_id: &str,
        status: &str,
        after_change: Option<&ChangeId>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE agent_turns SET status = ?1, after_change = ?2 WHERE turn_id = ?3",
            params![
                status,
                after_change.map(|change_id| change_id.0.clone()),
                turn_id
            ],
        )?;
        Ok(())
    }

    pub(crate) fn agent_turn(&self, turn_id: &str) -> Result<AgentTurn> {
        self.conn
            .query_row(
                "SELECT turn_id, agent_id, session_id, base_change, before_change, after_change, status, started_at, ended_at, metadata_json \
                 FROM agent_turns WHERE turn_id = ?1",
                params![turn_id],
                agent_turn_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("turn `{turn_id}` not found")))
    }

    pub(crate) fn agent_session_turns(&self, session_id: &str) -> Result<Vec<AgentTurn>> {
        let mut stmt = self.conn.prepare(
            "SELECT turn_id, agent_id, session_id, base_change, before_change, after_change, status, started_at, ended_at, metadata_json \
             FROM agent_turns WHERE session_id = ?1 ORDER BY started_at ASC, turn_id ASC",
        )?;
        let rows = stmt.query_map(params![session_id], agent_turn_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn agent_session_messages(&self, session_id: &str) -> Result<Vec<Message>> {
        let mut stmt = self.conn.prepare(
            "SELECT object_id FROM messages WHERE session_id = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| row.get::<_, String>(0))?;
        let object_ids = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        object_ids
            .into_iter()
            .map(|object_id| self.get_object(MESSAGE_KIND, &ObjectId(object_id)))
            .collect()
    }

    pub(crate) fn agent_session_events(&self, session_id: &str) -> Result<Vec<AgentEventRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT event_id, agent_id, session_id, turn_id, event_type, change_id, message_id, payload_json, created_at \
             FROM agent_events WHERE session_id = ?1 ORDER BY created_at ASC, event_id ASC",
        )?;
        let rows = stmt.query_map(params![session_id], agent_event_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn agent_session_operations(&self, session_id: &str) -> Result<Vec<TimelineEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT change_id, kind, branch, actor_id, message, created_at, path_count \
             FROM operations WHERE session_id = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![session_id], timeline_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn agent_turn_messages(&self, turn_id: &str) -> Result<Vec<Message>> {
        let mut stmt = self.conn.prepare(
            "SELECT object_id FROM messages \
             WHERE message_id IN ( \
                 SELECT message_id FROM agent_events \
                 WHERE turn_id = ?1 AND message_id IS NOT NULL \
             ) \
             ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![turn_id], |row| row.get::<_, String>(0))?;
        let object_ids = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        object_ids
            .into_iter()
            .map(|object_id| self.get_object(MESSAGE_KIND, &ObjectId(object_id)))
            .collect()
    }

    pub(crate) fn agent_turn_events(&self, turn_id: &str) -> Result<Vec<AgentEventRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT event_id, agent_id, session_id, turn_id, event_type, change_id, message_id, payload_json, created_at \
             FROM agent_events WHERE turn_id = ?1 ORDER BY created_at ASC, event_id ASC",
        )?;
        let rows = stmt.query_map(params![turn_id], agent_event_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn agent_turn_operations(&self, turn_id: &str) -> Result<Vec<TimelineEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT o.change_id, o.kind, o.branch, o.actor_id, o.message, o.created_at, o.path_count \
             FROM operations o \
             JOIN agent_events e ON e.change_id = o.change_id \
             WHERE e.turn_id = ?1 \
             ORDER BY o.created_at ASC, o.change_id ASC",
        )?;
        let rows = stmt.query_map(params![turn_id], timeline_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn agent_event(&self, event_id: &str) -> Result<AgentEventRecord> {
        self.conn
            .query_row(
                "SELECT event_id, agent_id, session_id, turn_id, event_type, change_id, message_id, payload_json, created_at \
                 FROM agent_events WHERE event_id = ?1",
                params![event_id],
                agent_event_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("event `{event_id}` not found")))
    }

    pub(crate) fn validate_agent_run_context(
        &self,
        branch: &AgentBranch,
        session_id: Option<&str>,
        turn_id: Option<&str>,
    ) -> Result<(Option<String>, Option<String>)> {
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
        let resolved_session_id = session_id
            .map(str::to_string)
            .or_else(|| turn.as_ref().and_then(|turn| turn.session_id.clone()))
            .or_else(|| branch.session_id.clone());
        if let Some(session_id) = resolved_session_id.as_deref() {
            let session = self.agent_session(session_id)?;
            if session.agent_id != branch.agent_id {
                return Err(Error::InvalidInput(format!(
                    "session `{session_id}` does not belong to agent `{}`",
                    branch.agent_id
                )));
            }
        }
        Ok((resolved_session_id, turn_id.map(str::to_string)))
    }

    pub(crate) fn insert_agent_run_state(
        &self,
        agent_id: &str,
        session_id: Option<&str>,
        turn_id: Option<&str>,
        approval_id: Option<&str>,
        reason: &str,
        summary: &str,
        state: Option<serde_json::Value>,
        interruption: Option<serde_json::Value>,
    ) -> Result<AgentRunState> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(Error::InvalidInput(
                "agent run pause reason cannot be empty".to_string(),
            ));
        }
        let summary = summary.trim();
        if summary.is_empty() {
            return Err(Error::InvalidInput(
                "agent run pause summary cannot be empty".to_string(),
            ));
        }
        let redacted_reason = redact_sensitive_text(reason);
        let redacted_summary = redact_sensitive_text(summary);
        let redacted_state = redact_sensitive_json(state.unwrap_or_else(|| serde_json::json!({})));
        let redacted_interruption = interruption.map(redact_sensitive_json);
        let seed = format!(
            "{}:{}:{}:{}:{}:{}",
            agent_id,
            session_id.unwrap_or("none"),
            turn_id.unwrap_or("none"),
            approval_id.unwrap_or("none"),
            redacted_reason,
            now_nanos()
        );
        let run_id = format!("run_{}", crate::ids::short_hash(seed.as_bytes(), 16));
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO agent_run_states \
             (run_id, agent_id, session_id, turn_id, approval_id, status, reason, summary, state_json, interruption_json, created_at, updated_at, resumed_at, reviewer, note) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'paused', ?6, ?7, ?8, ?9, ?10, ?10, NULL, NULL, NULL)",
            params![
                run_id,
                agent_id,
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
        self.insert_agent_event_with_context(
            agent_id,
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
        self.agent_run_state(&run_id)
    }

    pub(crate) fn agent_run_state(&self, run_id: &str) -> Result<AgentRunState> {
        let run_id = run_id.trim();
        if run_id.is_empty() {
            return Err(Error::InvalidInput(
                "agent run id cannot be empty".to_string(),
            ));
        }
        self.conn
            .query_row(
                "SELECT run_id, agent_id, session_id, turn_id, approval_id, status, reason, summary, state_json, interruption_json, created_at, updated_at, resumed_at, reviewer, note \
                 FROM agent_run_states WHERE run_id = ?1",
                params![run_id],
                agent_run_state_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("agent run `{run_id}` not found")))
    }

    pub(crate) fn agent_run_states_for_approval(
        &self,
        approval_id: &str,
    ) -> Result<Vec<AgentRunState>> {
        let mut stmt = self.conn.prepare(
            "SELECT run_id, agent_id, session_id, turn_id, approval_id, status, reason, summary, state_json, interruption_json, created_at, updated_at, resumed_at, reviewer, note \
             FROM agent_run_states WHERE approval_id = ?1 ORDER BY updated_at DESC, run_id DESC",
        )?;
        let rows = stmt.query_map(params![approval_id], agent_run_state_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn agent_approval(&self, approval_id: &str) -> Result<AgentApproval> {
        self.conn
            .query_row(
                "SELECT approval_id, agent_id, session_id, turn_id, action, summary, payload_json, status, requested_at, decided_at, reviewer, note \
                 FROM agent_approvals WHERE approval_id = ?1",
                params![approval_id],
                agent_approval_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("approval `{approval_id}` not found")))
    }

    pub(crate) fn latest_agent_test(&self, agent_id: &str) -> Result<Option<AgentTestSummary>> {
        self.latest_agent_gate(agent_id, "test")
    }

    pub(crate) fn latest_agent_gate(
        &self,
        agent_id: &str,
        kind: &str,
    ) -> Result<Option<AgentTestSummary>> {
        let event_type = agent_gate_event_type(kind)?;
        let row = self
            .conn
            .query_row(
                "SELECT event_id, turn_id, payload_json, created_at \
                 FROM agent_events \
                 WHERE agent_id = ?1 AND event_type = ?2 \
                 ORDER BY rowid DESC LIMIT 1",
                params![agent_id, event_type],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((event_id, turn_id, payload_json, created_at)) = row else {
            return Ok(None);
        };
        parse_agent_gate_summary(&event_id, turn_id, kind, &payload_json, created_at).map(Some)
    }

    pub(crate) fn latest_agent_gate_for_suite(
        &self,
        agent_id: &str,
        kind: &str,
        suite: &str,
    ) -> Result<Option<AgentTestSummary>> {
        let event_type = agent_gate_event_type(kind)?;
        let mut stmt = self.conn.prepare(
            "SELECT event_id, turn_id, payload_json, created_at \
             FROM agent_events \
             WHERE agent_id = ?1 AND event_type = ?2 \
             ORDER BY rowid DESC",
        )?;
        let rows = stmt.query_map(params![agent_id, event_type], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        for row in rows {
            let (event_id, turn_id, payload_json, created_at) = row?;
            let summary =
                parse_agent_gate_summary(&event_id, turn_id, kind, &payload_json, created_at)?;
            if summary.suite.as_deref() == Some(suite) {
                return Ok(Some(summary));
            }
        }
        Ok(None)
    }

    pub(crate) fn required_gate_suite_issues(
        &self,
        agent_id: &str,
        kind: &str,
        suites: &[String],
    ) -> Result<Vec<AgentReadinessIssue>> {
        let mut issues = Vec::new();
        for suite in suites {
            match self.latest_agent_gate_for_suite(agent_id, kind, suite)? {
                Some(gate) if !gate.success => {
                    issues.push(readiness_issue(
                        format!("required_{kind}_suite_failed"),
                        format!("required {kind} suite `{suite}` did not pass"),
                        Some(serde_json::json!({
                            "suite": suite,
                            "event_id": gate.event_id,
                            "status": gate.status,
                            "exit_code": gate.exit_code,
                            "command": gate.command,
                            "score": gate.score,
                            "threshold": gate.threshold
                        })),
                    ));
                }
                Some(_) => {}
                None => issues.push(readiness_issue(
                    format!("missing_required_{kind}_suite"),
                    format!("required {kind} suite `{suite}` has not been recorded"),
                    Some(serde_json::json!({ "suite": suite })),
                )),
            }
        }
        Ok(issues)
    }

    pub(crate) fn agent_gate_history_for_id(
        &self,
        agent_id: &str,
        kind: Option<&str>,
        limit: usize,
    ) -> Result<Vec<AgentTestSummary>> {
        let rows = if let Some(kind) = kind {
            let event_type = agent_gate_event_type(kind)?;
            let mut stmt = self.conn.prepare(
                "SELECT event_id, turn_id, event_type, payload_json, created_at \
                 FROM agent_events \
                 WHERE agent_id = ?1 AND event_type = ?2 \
                 ORDER BY rowid DESC LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![agent_id, event_type, limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT event_id, turn_id, event_type, payload_json, created_at \
                 FROM agent_events \
                 WHERE agent_id = ?1 AND event_type IN ('test_finished', 'eval_finished') \
                 ORDER BY rowid DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![agent_id, limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };

        rows.into_iter()
            .map(
                |(event_id, turn_id, event_type, payload_json, created_at)| {
                    let kind = agent_gate_kind_from_event_type(&event_type)?;
                    parse_agent_gate_summary(&event_id, turn_id, kind, &payload_json, created_at)
                },
            )
            .collect()
    }
}
