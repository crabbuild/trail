use super::*;

pub(crate) fn build_agent_trace_spans(events: Vec<AgentEventRecord>) -> Vec<AgentTraceSpan> {
    let mut builders: BTreeMap<String, AgentTraceSpanBuilder> = BTreeMap::new();

    for event in events {
        let Some(payload) = event.payload.as_ref() else {
            continue;
        };
        let Some(span_id) = payload_string(payload, "span_id") else {
            continue;
        };

        match event.event_type.as_str() {
            "span_started" => {
                let trace_id = payload_string(payload, "trace_id").unwrap_or_else(|| {
                    event
                        .turn_id
                        .as_deref()
                        .map(default_trace_id_for_turn)
                        .unwrap_or_else(|| default_trace_id_for_turn(&event.event_id))
                });
                let builder = AgentTraceSpanBuilder {
                    span_id: span_id.clone(),
                    trace_id,
                    agent_id: event.agent_id.clone(),
                    session_id: event.session_id.clone(),
                    turn_id: event.turn_id.clone(),
                    parent_span_id: payload_string(payload, "parent_span_id"),
                    span_type: payload_string(payload, "span_type")
                        .unwrap_or_else(|| "custom".to_string()),
                    name: payload_string(payload, "name").unwrap_or_else(|| span_id.clone()),
                    started_event_id: event.event_id.clone(),
                    started_at: event.created_at,
                    attributes: payload_value(payload, "attributes"),
                    ended_event_id: None,
                    ended_at: None,
                    status: None,
                    result: None,
                };
                builders.entry(span_id).or_insert(builder);
            }
            "span_ended" => {
                if let Some(builder) = builders.get_mut(&span_id) {
                    builder.ended_event_id = Some(event.event_id.clone());
                    builder.ended_at = Some(event.created_at);
                    builder.status = payload_string(payload, "status");
                    builder.result = payload_value(payload, "result");
                }
            }
            _ => {}
        }
    }

    builders
        .into_values()
        .map(agent_trace_span_from_builder)
        .collect()
}

pub(crate) fn agent_trace_span_from_builder(builder: AgentTraceSpanBuilder) -> AgentTraceSpan {
    let duration_ms = builder
        .ended_at
        .and_then(|ended_at| ended_at.checked_sub(builder.started_at))
        .map(|seconds| seconds as u64 * 1000);
    AgentTraceSpan {
        span_id: builder.span_id,
        trace_id: builder.trace_id,
        agent_id: builder.agent_id,
        session_id: builder.session_id,
        turn_id: builder.turn_id,
        parent_span_id: builder.parent_span_id,
        span_type: builder.span_type,
        name: builder.name,
        status: builder.status.unwrap_or_else(|| {
            if builder.ended_at.is_some() {
                "completed".to_string()
            } else {
                "running".to_string()
            }
        }),
        started_event_id: builder.started_event_id,
        ended_event_id: builder.ended_event_id,
        started_at: builder.started_at,
        ended_at: builder.ended_at,
        duration_ms,
        attributes: builder.attributes,
        result: builder.result,
    }
}

pub(crate) fn named_counts(counts: BTreeMap<String, u64>) -> Vec<NamedCount> {
    counts
        .into_iter()
        .map(|(name, count)| NamedCount { name, count })
        .collect()
}

pub(crate) fn tail_limited<T: Clone>(values: &[T], limit: usize) -> Vec<T> {
    let start = values.len().saturating_sub(limit);
    values[start..].to_vec()
}

pub(crate) fn agent_trace_status_is_failed(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "failed" | "error" | "errored" | "cancelled" | "canceled" | "timeout" | "timed_out"
    )
}

pub(crate) fn payload_string(payload: &serde_json::Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

pub(crate) fn payload_value(payload: &serde_json::Value, key: &str) -> Option<serde_json::Value> {
    payload.get(key).filter(|value| !value.is_null()).cloned()
}

pub(crate) fn default_trace_id_for_turn(turn_id: &str) -> String {
    format!("trace_{}", crate::ids::short_hash(turn_id.as_bytes(), 16))
}
