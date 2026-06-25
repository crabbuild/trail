use super::*;

impl CrabDb {
    pub fn list_agent_trace_spans(
        &self,
        agent: Option<&str>,
        session_id: Option<&str>,
        turn_id: Option<&str>,
        trace_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<AgentTraceSpan>> {
        let limit = normalize_query_limit(limit, 1000)?;
        let trace_id = trace_id
            .map(str::trim)
            .map(|trace_id| {
                if trace_id.is_empty() {
                    Err(Error::InvalidInput(
                        "trace id filter cannot be empty".to_string(),
                    ))
                } else {
                    Ok(trace_id)
                }
            })
            .transpose()?;
        let events = self.list_agent_trace_span_events(agent, session_id, turn_id)?;
        let mut spans = build_agent_trace_spans(events);
        if let Some(trace_id) = trace_id {
            spans.retain(|span| span.trace_id == trace_id);
        }
        spans.sort_by(|left, right| {
            right
                .started_at
                .cmp(&left.started_at)
                .then_with(|| right.span_id.cmp(&left.span_id))
        });
        spans.truncate(limit);
        Ok(spans)
    }

    pub fn summarize_agent_trace_spans(
        &self,
        agent: Option<&str>,
        session_id: Option<&str>,
        turn_id: Option<&str>,
        trace_id: Option<&str>,
        slowest_limit: usize,
    ) -> Result<AgentTraceSummaryReport> {
        let slowest_limit = normalize_query_limit(slowest_limit, 50)?;
        let agent_id = agent
            .map(|agent| self.agent_branch(agent).map(|branch| branch.agent_id))
            .transpose()?;
        let trace_id = trace_id
            .map(str::trim)
            .map(|trace_id| {
                if trace_id.is_empty() {
                    Err(Error::InvalidInput(
                        "trace id filter cannot be empty".to_string(),
                    ))
                } else {
                    Ok(trace_id.to_string())
                }
            })
            .transpose()?;

        let events = self.list_agent_trace_span_events(agent, session_id, turn_id)?;
        let mut spans = build_agent_trace_spans(events);
        if let Some(trace_id) = trace_id.as_deref() {
            spans.retain(|span| span.trace_id == trace_id);
        }

        let mut status_counts = BTreeMap::new();
        let mut span_type_counts = BTreeMap::new();
        let mut trace_counts = BTreeMap::new();
        let mut open_spans = Vec::new();
        let mut slowest_spans = Vec::new();
        let mut total_duration_ms = 0u64;
        let mut max_duration_ms = 0u64;
        let mut duration_count = 0u64;
        let mut failed_span_count = 0u64;
        let mut ended_span_count = 0u64;

        for span in &spans {
            *status_counts.entry(span.status.clone()).or_insert(0) += 1;
            *span_type_counts.entry(span.span_type.clone()).or_insert(0) += 1;
            *trace_counts.entry(span.trace_id.clone()).or_insert(0) += 1;
            if span.ended_at.is_some() {
                ended_span_count += 1;
            } else {
                open_spans.push(span.clone());
            }
            if agent_trace_status_is_failed(&span.status) {
                failed_span_count += 1;
            }
            if let Some(duration_ms) = span.duration_ms {
                total_duration_ms = total_duration_ms.saturating_add(duration_ms);
                max_duration_ms = max_duration_ms.max(duration_ms);
                duration_count += 1;
                slowest_spans.push(span.clone());
            }
        }

        let open_span_count = open_spans.len() as u64;

        slowest_spans.sort_by(|left, right| {
            right
                .duration_ms
                .cmp(&left.duration_ms)
                .then_with(|| right.started_at.cmp(&left.started_at))
                .then_with(|| right.span_id.cmp(&left.span_id))
        });
        slowest_spans.truncate(slowest_limit);
        open_spans.sort_by(|left, right| {
            right
                .started_at
                .cmp(&left.started_at)
                .then_with(|| right.span_id.cmp(&left.span_id))
        });
        open_spans.truncate(slowest_limit);

        Ok(AgentTraceSummaryReport {
            agent_id,
            session_id: session_id.map(str::to_string),
            turn_id: turn_id.map(str::to_string),
            trace_id,
            span_count: spans.len() as u64,
            open_span_count,
            ended_span_count,
            failed_span_count,
            total_duration_ms,
            max_duration_ms,
            average_duration_ms: if duration_count == 0 {
                None
            } else {
                Some(total_duration_ms as f64 / duration_count as f64)
            },
            status_counts: named_counts(status_counts),
            span_type_counts: named_counts(span_type_counts),
            trace_counts: named_counts(trace_counts),
            slowest_spans,
            open_spans,
        })
    }

    pub fn show_agent_trace_span(&self, span_id: &str) -> Result<AgentTraceSpan> {
        let span_id = span_id.trim();
        if span_id.is_empty() {
            return Err(Error::InvalidInput("span id cannot be empty".to_string()));
        }
        build_agent_trace_spans(self.list_agent_trace_span_events(None, None, None)?)
            .into_iter()
            .find(|span| span.span_id == span_id)
            .ok_or_else(|| Error::InvalidInput(format!("span `{span_id}` not found")))
    }

    pub(crate) fn list_agent_trace_span_events(
        &self,
        agent: Option<&str>,
        session_id: Option<&str>,
        turn_id: Option<&str>,
    ) -> Result<Vec<AgentEventRecord>> {
        let agent_id = agent
            .map(|agent| self.agent_branch(agent).map(|branch| branch.agent_id))
            .transpose()?;
        if let Some(session_id) = session_id {
            self.agent_session(session_id)?;
        }
        if let Some(turn_id) = turn_id {
            self.agent_turn(turn_id)?;
        }

        let mut stmt = self.conn.prepare(
            "SELECT event_id, agent_id, session_id, turn_id, event_type, change_id, message_id, payload_json, created_at \
             FROM agent_events \
             WHERE (?1 IS NULL OR agent_id = ?1) \
               AND (?2 IS NULL OR session_id = ?2) \
               AND (?3 IS NULL OR turn_id = ?3) \
               AND event_type IN ('span_started', 'span_ended') \
             ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![agent_id, session_id, turn_id], agent_event_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }
}
