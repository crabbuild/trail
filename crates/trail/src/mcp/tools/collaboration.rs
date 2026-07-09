use serde_json::{json, Value};

use crate::mcp::response::object_schema;

pub(super) fn tools() -> Value {
    json!([
        {
            "name": "trail.session_start",
            "title": "Start Lane Session",
            "description": "Start an explicit durable session and attach it to a lane branch.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "title": { "type": "string" },
                "id": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.session_list",
            "title": "List Lane Sessions",
            "description": "List durable lane sessions, optionally scoped to one lane.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" }
            }), vec![])
        },
        {
            "name": "trail.session_current",
            "title": "Current Lane Session",
            "description": "Read current lane branch session attachments, optionally for one lane.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" }
            }), vec![])
        },
        {
            "name": "trail.session_show",
            "title": "Show Lane Session",
            "description": "Return a session with turns, messages, events, and operations.",
            "inputSchema": object_schema(json!({
                "session_id": { "type": "string" }
            }), vec!["session_id"])
        },
        {
            "name": "trail.session_context",
            "title": "Session Context",
            "description": "Return a bounded session context packet with total counts and recent messages, events, turns, and operations.",
            "inputSchema": object_schema(json!({
                "session_id": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 1000 }
            }), vec!["session_id"])
        },
        {
            "name": "trail.session_end",
            "title": "End Lane Session",
            "description": "End a durable lane session with completed, failed, cancelled, or archived status.",
            "inputSchema": object_schema(json!({
                "session_id": { "type": "string" },
                "status": { "type": "string", "enum": ["completed", "failed", "cancelled", "archived"] }
            }), vec!["session_id"])
        },
        {
            "name": "trail.approval_request",
            "title": "Request Human Approval",
            "description": "Create a durable pending approval for a sensitive lane action.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "action": { "type": "string" },
                "summary": { "type": "string" },
                "payload": { "type": "object" },
                "session_id": { "type": "string" },
                "turn_id": { "type": "string" }
            }), vec!["lane", "action", "summary"])
        },
        {
            "name": "trail.approval_list",
            "title": "List Human Approvals",
            "description": "List durable approval gates, optionally scoped by lane and status.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "status": { "type": "string", "enum": ["pending", "approved", "rejected", "cancelled", "all"] }
            }), vec![])
        },
        {
            "name": "trail.approval_show",
            "title": "Show Human Approval",
            "description": "Show one durable approval gate by id.",
            "inputSchema": object_schema(json!({
                "approval_id": { "type": "string" }
            }), vec!["approval_id"])
        },
        {
            "name": "trail.approval_decide",
            "title": "Decide Human Approval",
            "description": "Approve, reject, or cancel a pending approval gate.",
            "inputSchema": object_schema(json!({
                "approval_id": { "type": "string" },
                "decision": { "type": "string", "enum": ["approved", "rejected", "cancelled"] },
                "reviewer": { "type": "string" },
                "note": { "type": "string" }
            }), vec!["approval_id", "decision"])
        },
        {
            "name": "trail.run_pause",
            "title": "Pause Lane Run",
            "description": "Persist a serialized paused lane run checkpoint for later resume.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "reason": { "type": "string" },
                "summary": { "type": "string" },
                "state": { "type": "object" },
                "interruption": { "type": "object" },
                "session_id": { "type": "string" },
                "turn_id": { "type": "string" }
            }), vec!["lane", "reason", "summary"])
        },
        {
            "name": "trail.run_list",
            "title": "List Lane Run States",
            "description": "List durable paused/resumed lane checkpoints, optionally scoped by lane and status.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "status": { "type": "string", "enum": ["paused", "resumed", "blocked", "cancelled", "all"] }
            }), vec![])
        },
        {
            "name": "trail.run_show",
            "title": "Show Lane Run State",
            "description": "Show one durable lane run checkpoint by id.",
            "inputSchema": object_schema(json!({
                "run_id": { "type": "string" }
            }), vec!["run_id"])
        },
        {
            "name": "trail.run_resume",
            "title": "Resume Lane Run",
            "description": "Mark a paused checkpoint resumed after any linked approval is approved.",
            "inputSchema": object_schema(json!({
                "run_id": { "type": "string" },
                "reviewer": { "type": "string" },
                "note": { "type": "string" }
            }), vec!["run_id"])
        },
        {
            "name": "trail.lease_acquire",
            "title": "Acquire Path Lease",
            "description": "Acquire an advisory read or write lease for a lane path before editing.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "path": { "type": "string" },
                "mode": { "type": "string", "enum": ["read", "write"] },
                "ttl_secs": { "type": "integer", "minimum": 1 }
            }), vec!["lane"])
        },
        {
            "name": "trail.lease_list",
            "title": "List Path Leases",
            "description": "List active advisory leases, or all leases when all is true.",
            "inputSchema": object_schema(json!({
                "all": { "type": "boolean" }
            }), vec![])
        },
        {
            "name": "trail.lease_release",
            "title": "Release Path Lease",
            "description": "Release an advisory path lease by lease id.",
            "inputSchema": object_schema(json!({
                "lease_id": { "type": "string" }
            }), vec!["lease_id"])
        },
        {
            "name": "trail.anchor_create",
            "title": "Create Line Anchor",
            "description": "Create a durable review anchor for a path:line selector on an optional branch.",
            "inputSchema": object_schema(json!({
                "path_line": { "type": "string" },
                "label": { "type": "string" },
                "branch": { "type": "string" }
            }), vec!["path_line", "label"])
        },
        {
            "name": "trail.anchor_list",
            "title": "List Line Anchors",
            "description": "List durable review anchors.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "trail.anchor_resolve",
            "title": "Resolve Line Anchor",
            "description": "Resolve a durable review anchor on an optional branch.",
            "inputSchema": object_schema(json!({
                "anchor_id": { "type": "string" },
                "branch": { "type": "string" }
            }), vec!["anchor_id"])
        },
        {
            "name": "trail.anchor_delete",
            "title": "Delete Line Anchor",
            "description": "Delete a durable review anchor by id.",
            "inputSchema": object_schema(json!({
                "anchor_id": { "type": "string" }
            }), vec!["anchor_id"])
        }
    ])
}
