use serde_json::{json, Value};

use super::{openapi_operation, openapi_path_param, openapi_query};

pub(super) fn turn_paths() -> Value {
    json!({
        "/v1/agent/turns": {
            "post": openapi_operation("turnBegin", "Begin turn", "Start a durable agent turn.", vec![], Some("BeginTurnRequest"), true)
        },
        "/v1/agent/events": {
            "get": openapi_operation("eventList", "List trace events", "List recent agent trace events filtered by agent, session, turn, or type.", vec![
                openapi_query("agent", "string"),
                openapi_query("session", "string"),
                openapi_query("turn_id", "string"),
                openapi_query("turn", "string"),
                openapi_query("event_type", "string"),
                openapi_query("type", "string"),
                openapi_query("limit", "integer")
            ], None, true)
        },
        "/v1/agent/spans": {
            "get": openapi_operation("spanList", "List trace spans", "List derived agent trace spans filtered by agent, session, turn, or trace.", vec![
                openapi_query("agent", "string"),
                openapi_query("session", "string"),
                openapi_query("turn_id", "string"),
                openapi_query("turn", "string"),
                openapi_query("trace_id", "string"),
                openapi_query("trace", "string"),
                openapi_query("limit", "integer")
            ], None, true)
        },
        "/v1/agent/spans/summary": {
            "get": openapi_operation("spanSummary", "Summarize trace spans", "Summarize derived agent trace spans with status/type counts, open spans, failed spans, and slowest completed spans.", vec![
                openapi_query("agent", "string"),
                openapi_query("session", "string"),
                openapi_query("turn_id", "string"),
                openapi_query("turn", "string"),
                openapi_query("trace_id", "string"),
                openapi_query("trace", "string"),
                openapi_query("slowest", "integer")
            ], None, true)
        },
        "/v1/agent/runs": {
            "get": openapi_operation("agentRunList", "List agent run states", "List durable paused/resumed agent run checkpoints, optionally scoped by agent and status.", vec![
                openapi_query("agent", "string"),
                openapi_query("status", "string")
            ], None, true),
            "post": openapi_operation("agentRunPause", "Pause agent run", "Persist a serialized paused agent run checkpoint for later resume.", vec![], Some("AgentRunPauseRequest"), true)
        },
        "/v1/agent/runs/{run_id}": {
            "get": openapi_operation("agentRunShow", "Show agent run state", "Show one durable agent run checkpoint.", vec![
                openapi_path_param("run_id", "string")
            ], None, true)
        },
        "/v1/agent/runs/{run_id}/resume": {
            "post": openapi_operation("agentRunResume", "Resume agent run", "Mark a paused checkpoint resumed after any linked approval is approved.", vec![
                openapi_path_param("run_id", "string")
            ], Some("AgentRunResumeRequest"), true)
        },
        "/v1/agent/spans/{span_id}": {
            "get": openapi_operation("spanShow", "Show trace span", "Show one derived agent trace span.", vec![
                openapi_path_param("span_id", "string")
            ], None, true)
        },
        "/v1/agent/spans/{span_id}/end": {
            "post": openapi_operation("spanEnd", "End trace span", "End an agent trace span and attach result metadata.", vec![
                openapi_path_param("span_id", "string")
            ], Some("EndSpanRequest"), true)
        },
        "/v1/agent/turns/{turn_id}": {
            "get": openapi_operation("turnShow", "Show turn", "Return a turn with messages, trace events, and operations.", vec![
                openapi_path_param("turn_id", "string")
            ], None, true)
        },
        "/v1/agent/turns/{turn_id}/messages": {
            "post": openapi_operation("turnAddMessage", "Add turn message", "Attach a message to a durable turn.", vec![
                openapi_path_param("turn_id", "string")
            ], Some("AddMessageRequest"), true)
        },
        "/v1/agent/turns/{turn_id}/events": {
            "post": openapi_operation("turnAddEvent", "Add trace event", "Attach a trace event to a durable turn.", vec![
                openapi_path_param("turn_id", "string")
            ], Some("AddEventRequest"), true)
        },
        "/v1/agent/turns/{turn_id}/spans": {
            "post": openapi_operation("turnStartSpan", "Start trace span", "Start a parentable trace span under a durable turn.", vec![
                openapi_path_param("turn_id", "string")
            ], Some("StartSpanRequest"), true)
        },
        "/v1/agent/turns/{turn_id}/patches": {
            "post": openapi_operation("turnApplyPatch", "Apply turn patch", "Apply a patch linked to a durable turn.", vec![
                openapi_path_param("turn_id", "string")
            ], Some("PatchRequest"), true)
        },
        "/v1/agent/turns/{turn_id}/end": {
            "post": openapi_operation("turnEnd", "End turn", "End a durable agent turn.", vec![
                openapi_path_param("turn_id", "string")
            ], Some("EndTurnRequest"), true)
        }
    })
}
