use serde_json::{json, Value};

use crate::mcp::response::object_schema;

pub(super) fn tools() -> Value {
    json!([
        {
            "name": "trail.lane_spawn",
            "title": "Spawn Lane Branch",
            "description": "Create or reuse an isolated lane branch, optionally materializing its workdir.",
            "inputSchema": object_schema(json!({
                "name": { "type": "string" },
                "from_ref": { "type": "string" },
                "materialize": { "type": "boolean" },
                "workdir_mode": { "type": "string", "enum": ["virtual", "sparse", "full-cow", "overlay-cow", "nfs-cow"] },
                "workdir": { "type": "string" },
                "workdir_path": { "type": "string" },
                "paths": { "type": "array", "items": { "type": "string" } },
                "include_neighbors": { "type": "boolean" },
                "include_neighborhood": { "type": "boolean" },
                "provider": { "type": "string" },
                "model": { "type": "string" }
            }), vec!["name"])
        },
        {
            "name": "trail.lane_hydrate",
            "title": "Hydrate Lane Workdir Paths",
            "description": "Hydrate selected paths into a sparse lane workdir before filesystem edits.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "paths": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                "force": { "type": "boolean" },
                "include_neighbors": { "type": "boolean" },
                "include_neighborhood": { "type": "boolean" }
            }), vec!["lane", "paths"])
        },
        {
            "name": "trail.lane_claim",
            "title": "Claim Lane Path",
            "description": "Create a soft advisory write claim for a lane path, returning conflicts as warnings instead of hard failures.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "path": { "type": "string" },
                "ttl_secs": { "type": "integer", "minimum": 1 },
                "ttl": { "type": "integer", "minimum": 1 }
            }), vec!["lane", "path"])
        },
        {
            "name": "trail.lane_list",
            "title": "List Lanes",
            "description": "List lane metadata and branch state for coordinator discovery.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "trail.lane_show",
            "title": "Show Lane",
            "description": "Show one lane's metadata and branch state by name or lane id.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.lane_status",
            "title": "Lane Status",
            "description": "Show one lane branch status, including workdir and latest test state.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.lane_review",
            "title": "Lane Review Packet",
            "description": "Produce a compact read-only review packet for one lane branch with readiness, evidence summaries, gates, approvals, conflicts, operations, and next steps.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1 }
            }), vec!["lane"])
        },
        {
            "name": "trail.lane_contribution",
            "title": "Lane Contribution",
            "description": "Summarize one lane branch for review with status, changed paths, operations, sessions, events, approvals, and latest gates.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1 }
            }), vec!["lane"])
        },
        {
            "name": "trail.gate_history",
            "title": "Lane Gate History",
            "description": "List recent durable test/eval gate results for one lane branch, optionally filtered by kind.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "kind": { "type": "string", "enum": ["all", "test", "tests", "eval", "evals"] },
                "limit": { "type": "integer", "minimum": 1 }
            }), vec!["lane"])
        },
        {
            "name": "trail.lane_readiness",
            "title": "Lane Readiness",
            "description": "Assess whether one lane branch is ready to merge by checking conflicts, approvals, workdir state, tests, and evals.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.lane_refresh_preview",
            "title": "Lane Refresh Preview",
            "description": "Preview refreshing one lane onto a target branch, including operations-behind, incoming changed paths, conflicts, and next steps, without mutating refs or recording conflict state.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "target": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.lane_handoff",
            "title": "Lane Handoff",
            "description": "Package one lane branch for transfer with readiness, current session context, recent events, spans, operations, and next steps.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1 }
            }), vec!["lane"])
        },
        {
            "name": "trail.lane_remove",
            "title": "Remove Lane",
            "description": "Remove a lane branch and materialized workdir. Requires force when the branch has unmerged changes.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "force": { "type": "boolean" }
            }), vec!["lane"])
        },
        {
            "name": "trail.lane_rewind",
            "title": "Rewind Lane",
            "description": "Move a lane branch back to a known-good change or root, optionally preserving the current head and syncing the materialized workdir.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "to": { "type": "string" },
                "target": { "type": "string" },
                "record_current": { "type": "boolean" },
                "sync_workdir": { "type": "boolean" }
            }), vec!["lane", "to"])
        }
    ])
}
