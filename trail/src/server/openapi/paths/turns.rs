use serde_json::{json, Value};

use super::{
    openapi_operation, openapi_operation_with_response_schema, openapi_path_param, openapi_query,
};

pub(super) fn turn_paths() -> Value {
    json!({
        "/v1/lane/turns": {
            "post": openapi_operation("turnBegin", "Begin turn", "Start a durable lane turn.", vec![], Some("BeginTurnRequest"), true)
        },
        "/v1/lane/events": {
            "get": openapi_operation("eventList", "List trace events", "List recent lane trace events filtered by lane, session, turn, or type.", vec![
                openapi_query("lane", "string"),
                openapi_query("session", "string"),
                openapi_query("turn_id", "string"),
                openapi_query("turn", "string"),
                openapi_query("event_type", "string"),
                openapi_query("type", "string"),
                openapi_query("limit", "integer")
            ], None, true)
        },
        "/v1/lane/spans": {
            "get": openapi_operation("spanList", "List trace spans", "List derived lane trace spans filtered by lane, session, turn, or trace.", vec![
                openapi_query("lane", "string"),
                openapi_query("session", "string"),
                openapi_query("turn_id", "string"),
                openapi_query("turn", "string"),
                openapi_query("trace_id", "string"),
                openapi_query("trace", "string"),
                openapi_query("limit", "integer")
            ], None, true)
        },
        "/v1/lane/spans/summary": {
            "get": openapi_operation("spanSummary", "Summarize trace spans", "Summarize derived lane trace spans with status/type counts, open spans, failed spans, and slowest completed spans.", vec![
                openapi_query("lane", "string"),
                openapi_query("session", "string"),
                openapi_query("turn_id", "string"),
                openapi_query("turn", "string"),
                openapi_query("trace_id", "string"),
                openapi_query("trace", "string"),
                openapi_query("slowest", "integer")
            ], None, true)
        },
        "/v1/lane/runs": {
            "get": openapi_operation("laneRunList", "List lane run states", "List durable paused/resumed lane run checkpoints, optionally scoped by lane and status.", vec![
                openapi_query("lane", "string"),
                openapi_query("status", "string")
            ], None, true),
            "post": openapi_operation("laneRunPause", "Pause lane run", "Persist a serialized paused lane run checkpoint for later resume.", vec![], Some("LaneRunPauseRequest"), true)
        },
        "/v1/lane/runs/{run_id}": {
            "get": openapi_operation("laneRunShow", "Show lane run state", "Show one durable lane run checkpoint.", vec![
                openapi_path_param("run_id", "string")
            ], None, true)
        },
        "/v1/lane/runs/{run_id}/resume": {
            "post": openapi_operation("laneRunResume", "Resume lane run", "Mark a paused checkpoint resumed after any linked approval is approved.", vec![
                openapi_path_param("run_id", "string")
            ], Some("LaneRunResumeRequest"), true)
        },
        "/v1/lane/spans/{span_id}": {
            "get": openapi_operation("spanShow", "Show trace span", "Show one derived lane trace span.", vec![
                openapi_path_param("span_id", "string")
            ], None, true)
        },
        "/v1/lane/spans/{span_id}/end": {
            "post": openapi_operation("spanEnd", "End trace span", "End a lane trace span and attach result metadata.", vec![
                openapi_path_param("span_id", "string")
            ], Some("EndSpanRequest"), true)
        },
        "/v1/lane/turns/{turn_id}": {
            "get": openapi_operation("turnShow", "Show turn", "Return a turn with messages, trace events, and operations.", vec![
                openapi_path_param("turn_id", "string")
            ], None, true)
        },
        "/v1/lane/turns/{turn_id}/messages": {
            "post": openapi_operation("turnAddMessage", "Add turn message", "Attach a message to a durable turn.", vec![
                openapi_path_param("turn_id", "string")
            ], Some("AddMessageRequest"), true)
        },
        "/v1/lane/turns/{turn_id}/events": {
            "post": openapi_operation("turnAddEvent", "Add trace event", "Attach a trace event to a durable turn.", vec![
                openapi_path_param("turn_id", "string")
            ], Some("AddEventRequest"), true)
        },
        "/v1/lane/turns/{turn_id}/spans": {
            "post": openapi_operation("turnStartSpan", "Start trace span", "Start a parentable trace span under a durable turn.", vec![
                openapi_path_param("turn_id", "string")
            ], Some("StartSpanRequest"), true)
        },
        "/v1/lane/turns/{turn_id}/patches": {
            "post": openapi_operation_with_response_schema("turnApplyPatch", "Apply turn patch", "Apply a patch linked to a durable turn.", vec![
                openapi_path_param("turn_id", "string")
            ], Some("PatchRequest"), "LanePatchReport", true)
        },
        "/v1/lane/turns/{turn_id}/end": {
            "post": openapi_operation("turnEnd", "End turn", "End a durable lane turn.", vec![
                openapi_path_param("turn_id", "string")
            ], Some("EndTurnRequest"), true)
        }
    })
}
