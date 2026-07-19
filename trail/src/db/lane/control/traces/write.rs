use super::*;

impl Trail {
    pub fn start_lane_trace_span(
        &mut self,
        turn_id: &str,
        span_type: &str,
        name: &str,
        parent_span_id: Option<&str>,
        trace_id: Option<&str>,
        attributes: Option<serde_json::Value>,
    ) -> Result<LaneTraceSpanStartReport> {
        let _lock = self.acquire_write_lock()?;
        let span_type = span_type.trim();
        if span_type.is_empty() {
            return Err(Error::InvalidInput("span type cannot be empty".to_string()));
        }
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::InvalidInput("span name cannot be empty".to_string()));
        }
        let turn = self.lane_turn(turn_id)?;
        if turn.ended_at.is_some() {
            return Err(Error::InvalidInput(format!(
                "turn `{turn_id}` is already ended"
            )));
        }

        let parent = parent_span_id
            .map(|span_id| self.show_lane_trace_span(span_id))
            .transpose()?;
        if let Some(parent) = &parent
            && (parent.lane_id != turn.lane_id
                || parent.turn_id.as_deref() != Some(turn_id)
                || parent.session_id != turn.session_id)
        {
            return Err(Error::InvalidInput(format!(
                "parent span `{}` does not belong to turn `{turn_id}`",
                parent.span_id
            )));
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
            turn.lane_id,
            turn.session_id.as_deref().unwrap_or("none"),
            turn_id,
            trace_id,
            name,
            now_nanos()
        );
        let span_id = format!("span_{}", crate::ids::short_hash(seed.as_bytes(), 16));
        self.insert_lane_event_with_context(
            &turn.lane_id,
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
        Ok(LaneTraceSpanStartReport {
            span: self.show_lane_trace_span(&span_id)?,
        })
    }

    pub fn end_lane_trace_span(
        &mut self,
        span_id: &str,
        status: &str,
        result: Option<serde_json::Value>,
    ) -> Result<LaneTraceSpanEndReport> {
        let _lock = self.acquire_write_lock()?;
        let span = self.show_lane_trace_span(span_id)?;
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
            let turn = self.lane_turn(turn_id)?;
            if turn.ended_at.is_some() {
                return Err(Error::InvalidInput(format!(
                    "turn `{turn_id}` is already ended"
                )));
            }
        }
        self.insert_lane_event_with_context(
            &span.lane_id,
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
        Ok(LaneTraceSpanEndReport {
            span: self.show_lane_trace_span(span_id)?,
        })
    }
}
