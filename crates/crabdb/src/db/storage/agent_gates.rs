use super::*;

impl CrabDb {
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
