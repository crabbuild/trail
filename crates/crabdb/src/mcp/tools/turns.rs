use serde_json::{json, Value};

use crate::mcp::response::object_schema;

pub(super) fn tools() -> Value {
    json!([
        {
            "name": "crabdb.begin_turn",
            "title": "Begin Agent Turn",
            "description": "Create or reuse an agent branch and start a durable agent turn.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "branch": { "type": "string" },
                "session_title": { "type": "string" },
                "base_change": { "type": "string" }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.add_message",
            "title": "Add Turn Message",
            "description": "Attach a user, assistant, tool, reviewer, or system message to a turn.",
            "inputSchema": object_schema(json!({
                "turn_id": { "type": "string" },
                "role": { "type": "string" },
                "content": { "type": "string" },
                "text": { "type": "string" }
            }), vec!["turn_id", "role"])
        },
        {
            "name": "crabdb.add_event",
            "title": "Add Turn Trace Event",
            "description": "Attach a tool call, tool result, guardrail, handoff, evaluation, or custom event to a turn.",
            "inputSchema": object_schema(json!({
                "turn_id": { "type": "string" },
                "event_type": { "type": "string" },
                "payload": { "type": "object" },
                "change_id": { "type": "string" },
                "message_id": { "type": "string" }
            }), vec!["turn_id", "event_type"])
        },
        {
            "name": "crabdb.event_list",
            "title": "List Trace Events",
            "description": "List recent agent trace events across agents, sessions, turns, and event types.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "session": { "type": "string" },
                "turn_id": { "type": "string" },
                "event_type": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 1000 }
            }), vec![])
        },
        {
            "name": "crabdb.span_start",
            "title": "Start Trace Span",
            "description": "Start a parentable trace span for an agent, tool call, guardrail, handoff, or evaluation within a turn.",
            "inputSchema": object_schema(json!({
                "turn_id": { "type": "string" },
                "span_type": { "type": "string" },
                "name": { "type": "string" },
                "parent": { "type": "string" },
                "parent_span_id": { "type": "string" },
                "trace": { "type": "string" },
                "trace_id": { "type": "string" },
                "attributes": { "type": "object" }
            }), vec!["turn_id", "span_type", "name"])
        },
        {
            "name": "crabdb.span_end",
            "title": "End Trace Span",
            "description": "End a trace span with a status and optional result payload.",
            "inputSchema": object_schema(json!({
                "span_id": { "type": "string" },
                "status": { "type": "string" },
                "result": { "type": "object" }
            }), vec!["span_id"])
        },
        {
            "name": "crabdb.span_list",
            "title": "List Trace Spans",
            "description": "List derived trace spans across agents, sessions, turns, and trace ids.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "session": { "type": "string" },
                "turn_id": { "type": "string" },
                "trace_id": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 1000 }
            }), vec![])
        },
        {
            "name": "crabdb.span_summary",
            "title": "Summarize Trace Spans",
            "description": "Summarize derived trace spans with status/type counts, open spans, failed spans, and slowest completed spans.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "session": { "type": "string" },
                "turn_id": { "type": "string" },
                "trace_id": { "type": "string" },
                "slowest": { "type": "integer", "minimum": 1, "maximum": 50 }
            }), vec![])
        },
        {
            "name": "crabdb.span_show",
            "title": "Show Trace Span",
            "description": "Show a single derived trace span.",
            "inputSchema": object_schema(json!({
                "span_id": { "type": "string" }
            }), vec!["span_id"])
        },
        {
            "name": "crabdb.apply_patch",
            "title": "Apply Agent Patch",
            "description": "Apply a native CrabDB patch or design-style files patch to a turn's agent branch.",
            "inputSchema": object_schema(json!({
                "turn_id": { "type": "string" },
                "message": { "type": "string" },
                "base_change": { "type": "string" },
                "session_id": { "type": "string" },
                "allow_ignored": { "type": "boolean" },
                "edits": { "type": "array", "items": { "type": "object" } },
                "files": { "type": "array", "items": { "type": "object" } }
            }), vec!["turn_id"])
        },
        {
            "name": "crabdb.end_turn",
            "title": "End Agent Turn",
            "description": "Close a durable agent turn with completed, failed, cancelled, or archived status.",
            "inputSchema": object_schema(json!({
                "turn_id": { "type": "string" },
                "status": { "type": "string", "enum": ["completed", "failed", "cancelled", "archived"] }
            }), vec!["turn_id"])
        },
        {
            "name": "crabdb.show_turn",
            "title": "Show Agent Turn",
            "description": "Return a turn with its session, messages, trace events, and operations.",
            "inputSchema": object_schema(json!({
                "turn_id": { "type": "string" }
            }), vec!["turn_id"])
        },
        {
            "name": "crabdb.diff_agent",
            "title": "Diff Agent Branch",
            "description": "Show the changes from an agent branch base to its current head.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "patch": { "type": "boolean" },
                "show_line_ids": { "type": "boolean" },
                "show-line-ids": { "type": "boolean" }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.run_test",
            "title": "Run Agent Test",
            "description": "Run a command in an agent workdir and record durable test_started/test_finished events plus stdout/stderr Blob output.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "command": { "type": "array", "items": { "type": "string" } },
                "turn_id": { "type": "string" },
                "timeout_secs": { "type": "integer", "minimum": 1 },
                "suite": { "type": "string" },
                "score": { "type": "number" },
                "threshold": { "type": "number" }
            }), vec!["agent", "command"])
        },
        {
            "name": "crabdb.run_eval",
            "title": "Run Agent Eval",
            "description": "Run an evaluation command in an agent workdir and record durable eval_started/eval_finished events plus stdout/stderr Blob output.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "command": { "type": "array", "items": { "type": "string" } },
                "turn_id": { "type": "string" },
                "timeout_secs": { "type": "integer", "minimum": 1 },
                "suite": { "type": "string" },
                "score": { "type": "number" },
                "threshold": { "type": "number" }
            }), vec!["agent", "command"])
        },
        {
            "name": "crabdb.sync_workdir",
            "title": "Sync Agent Workdir",
            "description": "Refresh an agent materialized workdir from its branch head, refusing dirty edits unless force is true.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "force": { "type": "boolean" }
            }), vec!["agent"])
        }
    ])
}
