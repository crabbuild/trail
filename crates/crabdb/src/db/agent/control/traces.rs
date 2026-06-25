use super::*;

impl CrabDb {
    pub fn start_agent_trace_span(
        &mut self,
        turn_id: &str,
        span_type: &str,
        name: &str,
        parent_span_id: Option<&str>,
        trace_id: Option<&str>,
        attributes: Option<serde_json::Value>,
    ) -> Result<AgentTraceSpanStartReport> {
        let _lock = self.acquire_write_lock()?;
        let span_type = span_type.trim();
        if span_type.is_empty() {
            return Err(Error::InvalidInput("span type cannot be empty".to_string()));
        }
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::InvalidInput("span name cannot be empty".to_string()));
        }
        let turn = self.agent_turn(turn_id)?;
        if turn.ended_at.is_some() {
            return Err(Error::InvalidInput(format!(
                "turn `{turn_id}` is already ended"
            )));
        }

        let parent = parent_span_id
            .map(|span_id| self.show_agent_trace_span(span_id))
            .transpose()?;
        if let Some(parent) = &parent {
            if parent.agent_id != turn.agent_id
                || parent.turn_id.as_deref() != Some(turn_id)
                || parent.session_id != turn.session_id
            {
                return Err(Error::InvalidInput(format!(
                    "parent span `{}` does not belong to turn `{turn_id}`",
                    parent.span_id
                )));
            }
        }

        let trace_id = match (trace_id.map(str::trim), parent.as_ref()) {
            (Some(""), _) => {
                return Err(Error::InvalidInput("trace id cannot be empty".to_string()));
            }
            (Some(trace_id), Some(parent)) if trace_id != parent.trace_id => {
                return Err(Error::InvalidInput(format!(
                    "trace id `{trace_id}` does not match parent span trace `{}`",
                    parent.trace_id
                )));
            }
            (Some(trace_id), _) => trace_id.to_string(),
            (None, Some(parent)) => parent.trace_id.clone(),
            (None, None) => default_trace_id_for_turn(turn_id),
        };

        let seed = format!(
            "{}:{}:{}:{}:{}:{}",
            turn.agent_id,
            turn.session_id.as_deref().unwrap_or("none"),
            turn_id,
            trace_id,
            name,
            now_nanos()
        );
        let span_id = format!("span_{}", crate::ids::short_hash(seed.as_bytes(), 16));
        self.insert_agent_event_with_context(
            &turn.agent_id,
            turn.session_id.as_deref(),
            Some(turn_id),
            "span_started",
            None,
            None,
            &serde_json::json!({
                "span_id": span_id.clone(),
                "trace_id": trace_id,
                "parent_span_id": parent_span_id,
                "span_type": span_type,
                "name": name,
                "attributes": attributes.unwrap_or(serde_json::Value::Null)
            }),
        )?;
        Ok(AgentTraceSpanStartReport {
            span: self.show_agent_trace_span(&span_id)?,
        })
    }

    pub fn end_agent_trace_span(
        &mut self,
        span_id: &str,
        status: &str,
        result: Option<serde_json::Value>,
    ) -> Result<AgentTraceSpanEndReport> {
        let _lock = self.acquire_write_lock()?;
        let span = self.show_agent_trace_span(span_id)?;
        if span.ended_at.is_some() {
            return Err(Error::InvalidInput(format!(
                "span `{span_id}` is already ended"
            )));
        }
        let status = status.trim();
        if status.is_empty() {
            return Err(Error::InvalidInput(
                "span status cannot be empty".to_string(),
            ));
        }
        if let Some(turn_id) = span.turn_id.as_deref() {
            let turn = self.agent_turn(turn_id)?;
            if turn.ended_at.is_some() {
                return Err(Error::InvalidInput(format!(
                    "turn `{turn_id}` is already ended"
                )));
            }
        }
        self.insert_agent_event_with_context(
            &span.agent_id,
            span.session_id.as_deref(),
            span.turn_id.as_deref(),
            "span_ended",
            None,
            None,
            &serde_json::json!({
                "span_id": span.span_id,
                "trace_id": span.trace_id,
                "status": status,
                "result": result.unwrap_or(serde_json::Value::Null)
            }),
        )?;
        Ok(AgentTraceSpanEndReport {
            span: self.show_agent_trace_span(span_id)?,
        })
    }

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
