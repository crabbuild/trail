use serde_json::{json, Value};

use crate::mcp::response::object_schema;

pub(super) fn tools() -> Value {
    json!([
        {
            "name": "crabdb.agent_spawn",
            "title": "Spawn Agent Branch",
            "description": "Create or reuse an isolated agent branch, optionally materializing its workdir.",
            "inputSchema": object_schema(json!({
                "name": { "type": "string" },
                "from_ref": { "type": "string" },
                "materialize": { "type": "boolean" },
                "workdir": { "type": "string" },
                "workdir_path": { "type": "string" },
                "provider": { "type": "string" },
                "model": { "type": "string" }
            }), vec!["name"])
        },
        {
            "name": "crabdb.agent_claim",
            "title": "Claim Agent Path",
            "description": "Create a soft advisory write claim for an agent path, returning conflicts as warnings instead of hard failures.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "path": { "type": "string" },
                "ttl_secs": { "type": "integer", "minimum": 1 },
                "ttl": { "type": "integer", "minimum": 1 }
            }), vec!["agent", "path"])
        },
        {
            "name": "crabdb.agent_list",
            "title": "List Agents",
            "description": "List agent metadata and branch state for coordinator discovery.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "crabdb.agent_show",
            "title": "Show Agent",
            "description": "Show one agent's metadata and branch state by name or agent id.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.agent_status",
            "title": "Agent Status",
            "description": "Show one agent branch status, including workdir and latest test state.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.agent_contribution",
            "title": "Agent Contribution",
            "description": "Summarize one agent branch for review with status, changed paths, operations, sessions, events, approvals, and latest gates.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1 }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.gate_history",
            "title": "Agent Gate History",
            "description": "List recent durable test/eval gate results for one agent branch, optionally filtered by kind.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "kind": { "type": "string", "enum": ["all", "test", "tests", "eval", "evals"] },
                "limit": { "type": "integer", "minimum": 1 }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.agent_readiness",
            "title": "Agent Readiness",
            "description": "Assess whether one agent branch is ready to merge by checking conflicts, approvals, workdir state, tests, and evals.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.agent_handoff",
            "title": "Agent Handoff",
            "description": "Package one agent branch for transfer with readiness, current session context, recent events, spans, operations, and next steps.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1 }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.agent_remove",
            "title": "Remove Agent",
            "description": "Remove an agent branch and materialized workdir. Requires force when the branch has unmerged changes.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "force": { "type": "boolean" }
            }), vec!["agent"])
        }
    ])
}
