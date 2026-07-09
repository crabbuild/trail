use serde_json::{json, Value};

use crate::mcp::response::object_schema;

fn closed_object(required: Vec<&str>, properties: Value) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": required,
        "properties": properties
    })
}

fn patch_edit_schema() -> Value {
    json!({
        "oneOf": [
            closed_object(vec!["op", "path", "content"], json!({
                "op": { "type": "string", "enum": ["write"] },
                "path": { "type": "string" },
                "content": { "type": "string" },
                "executable": { "type": "boolean" }
            })),
            closed_object(vec!["op", "path", "bytes_hex"], json!({
                "op": { "type": "string", "enum": ["write_bytes"] },
                "path": { "type": "string" },
                "bytes_hex": { "type": "string" },
                "executable": { "type": "boolean" }
            })),
            closed_object(vec!["op", "path", "line_id", "expected_text", "new_text"], json!({
                "op": { "type": "string", "enum": ["replace_line"] },
                "path": { "type": "string" },
                "line_id": { "type": "string" },
                "expected_text": { "type": "string" },
                "new_text": { "type": "string" }
            })),
            closed_object(vec!["op", "path"], json!({
                "op": { "type": "string", "enum": ["delete"] },
                "path": { "type": "string" }
            })),
            closed_object(vec!["op", "from", "to"], json!({
                "op": { "type": "string", "enum": ["rename"] },
                "from": { "type": "string" },
                "to": { "type": "string" }
            }))
        ]
    })
}

fn api_text_edit_schema() -> Value {
    json!({
        "oneOf": [
            closed_object(vec!["type", "line_id", "expected_text", "new_text"], json!({
                "type": { "type": "string", "enum": ["modify_line"] },
                "line_id": { "type": "string" },
                "expected_text": { "type": "string" },
                "new_text": { "type": "string" }
            }))
        ]
    })
}

fn api_patch_file_schema() -> Value {
    json!({
        "oneOf": [
            closed_object(vec!["type", "path", "content"], json!({
                "type": { "type": "string", "enum": ["add_text"] },
                "path": { "type": "string" },
                "content": { "type": "string" },
                "executable": { "type": "boolean" }
            })),
            closed_object(vec!["type", "path", "edits"], json!({
                "type": { "type": "string", "enum": ["modify_text"] },
                "path": { "type": "string" },
                "edits": {
                    "type": "array",
                    "minItems": 1,
                    "items": api_text_edit_schema()
                }
            })),
            closed_object(vec!["type", "path", "bytes_hex"], json!({
                "type": { "type": "string", "enum": ["write_bytes"] },
                "path": { "type": "string" },
                "bytes_hex": { "type": "string" },
                "executable": { "type": "boolean" }
            })),
            closed_object(vec!["type", "path"], json!({
                "type": { "type": "string", "enum": ["delete"] },
                "path": { "type": "string" }
            })),
            closed_object(vec!["type", "from", "to"], json!({
                "type": { "type": "string", "enum": ["rename"] },
                "from": { "type": "string" },
                "to": { "type": "string" }
            }))
        ]
    })
}

fn apply_patch_schema() -> Value {
    let mut schema = object_schema(
        json!({
            "turn_id": { "type": "string" },
            "message": { "type": "string" },
            "base_change": { "type": "string" },
            "session_id": { "type": "string" },
            "allow_ignored": { "type": "boolean" },
            "allow_stale": { "type": "boolean" },
            "edits": {
                "type": "array",
                "minItems": 1,
                "items": patch_edit_schema()
            },
            "files": {
                "type": "array",
                "minItems": 1,
                "items": api_patch_file_schema()
            }
        }),
        vec!["turn_id"],
    );
    if let Value::Object(object) = &mut schema {
        object.insert(
            "oneOf".to_string(),
            json!([
                { "required": ["edits"], "not": { "required": ["files"] } },
                { "required": ["files"], "not": { "required": ["edits"] } }
            ]),
        );
    }
    schema
}

pub(super) fn tools() -> Value {
    json!([
        {
            "name": "trail.begin_turn",
            "title": "Begin Lane Turn",
            "description": "Create or reuse a lane branch and start a durable lane turn.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "branch": { "type": "string" },
                "session_title": { "type": "string" },
                "base_change": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.add_message",
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
            "name": "trail.add_event",
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
            "name": "trail.event_list",
            "title": "List Trace Events",
            "description": "List recent lane trace events across lanes, sessions, turns, and event types.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "session": { "type": "string" },
                "turn_id": { "type": "string" },
                "event_type": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 1000 }
            }), vec![])
        },
        {
            "name": "trail.span_start",
            "title": "Start Trace Span",
            "description": "Start a parentable trace span for a lane, tool call, guardrail, handoff, or evaluation within a turn.",
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
            "name": "trail.span_end",
            "title": "End Trace Span",
            "description": "End a trace span with a status and optional result payload.",
            "inputSchema": object_schema(json!({
                "span_id": { "type": "string" },
                "status": { "type": "string" },
                "result": { "type": "object" }
            }), vec!["span_id"])
        },
        {
            "name": "trail.span_list",
            "title": "List Trace Spans",
            "description": "List derived trace spans across lanes, sessions, turns, and trace ids.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "session": { "type": "string" },
                "turn_id": { "type": "string" },
                "trace_id": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 1000 }
            }), vec![])
        },
        {
            "name": "trail.span_summary",
            "title": "Summarize Trace Spans",
            "description": "Summarize derived trace spans with status/type counts, open spans, failed spans, and slowest completed spans.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "session": { "type": "string" },
                "turn_id": { "type": "string" },
                "trace_id": { "type": "string" },
                "slowest": { "type": "integer", "minimum": 1, "maximum": 50 }
            }), vec![])
        },
        {
            "name": "trail.span_show",
            "title": "Show Trace Span",
            "description": "Show a single derived trace span.",
            "inputSchema": object_schema(json!({
                "span_id": { "type": "string" }
            }), vec!["span_id"])
        },
        {
            "name": "trail.apply_patch",
            "title": "Apply Lane Patch",
            "description": "Apply a native Trail patch or design-style files patch to a turn's lane branch. Provide exactly one non-empty edit source: edits or files.",
            "inputSchema": apply_patch_schema()
        },
        {
            "name": "trail.end_turn",
            "title": "End Lane Turn",
            "description": "Close a durable lane turn with completed, failed, cancelled, or archived status.",
            "inputSchema": object_schema(json!({
                "turn_id": { "type": "string" },
                "status": { "type": "string", "enum": ["completed", "failed", "cancelled", "archived"] }
            }), vec!["turn_id"])
        },
        {
            "name": "trail.show_turn",
            "title": "Show Lane Turn",
            "description": "Return a turn with its session, messages, trace events, and operations.",
            "inputSchema": object_schema(json!({
                "turn_id": { "type": "string" }
            }), vec!["turn_id"])
        },
        {
            "name": "trail.diff_lane",
            "title": "Diff Lane Branch",
            "description": "Show the changes from a lane branch base to its current head.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "patch": { "type": "boolean" },
                "show_line_ids": { "type": "boolean" },
                "show-line-ids": { "type": "boolean" }
            }), vec!["lane"])
        },
        {
            "name": "trail.run_test",
            "title": "Run Lane Test",
            "description": "Run a command in a lane workdir and record durable test_started/test_finished events plus stdout/stderr Blob output.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "command": { "type": "array", "items": { "type": "string" } },
                "turn_id": { "type": "string" },
                "timeout_secs": { "type": "integer", "minimum": 1 },
                "suite": { "type": "string" },
                "score": { "type": "number" },
                "threshold": { "type": "number" }
            }), vec!["lane", "command"])
        },
        {
            "name": "trail.run_eval",
            "title": "Run Lane Eval",
            "description": "Run an evaluation command in a lane workdir and record durable eval_started/eval_finished events plus stdout/stderr Blob output.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "command": { "type": "array", "items": { "type": "string" } },
                "turn_id": { "type": "string" },
                "timeout_secs": { "type": "integer", "minimum": 1 },
                "suite": { "type": "string" },
                "score": { "type": "number" },
                "threshold": { "type": "number" }
            }), vec!["lane", "command"])
        },
        {
            "name": "trail.sync_workdir",
            "title": "Sync Lane Workdir",
            "description": "Refresh a lane materialized workdir from its branch head, or hydrate selected paths in a sparse workdir, refusing dirty edits unless force is true.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "force": { "type": "boolean" },
                "paths": { "type": "array", "items": { "type": "string" } },
                "include_neighbors": { "type": "boolean" },
                "include_neighborhood": { "type": "boolean" }
            }), vec!["lane"])
        },
        {
            "name": "trail.read_file",
            "title": "Read Lane File",
            "description": "Read one file from a lane branch without materializing the whole workdir. Sparse workdirs hydrate lazily when hydrate is omitted; pass hydrate=false for a side-effect-free read.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "path": { "type": "string" },
                "hydrate": { "type": "boolean" },
                "force": { "type": "boolean" },
                "include_neighbors": { "type": "boolean" },
                "include_neighborhood": { "type": "boolean" }
            }), vec!["lane", "path"])
        }
    ])
}
