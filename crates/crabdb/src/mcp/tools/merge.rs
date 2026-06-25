use serde_json::{json, Value};

use crate::mcp::response::object_schema;

pub(super) fn tools() -> Value {
    json!([
        {
            "name": "crabdb.merge_queue_add",
            "title": "Queue Merge",
            "description": "Queue an agent or branch ref for serialized merge into a target branch.",
            "inputSchema": object_schema(json!({
                "source": { "type": "string" },
                "target": { "type": "string" },
                "priority": { "type": "integer" }
            }), vec!["source", "target"])
        },
        {
            "name": "crabdb.merge_queue_list",
            "title": "List Merge Queue",
            "description": "List queued, running, merged, cancelled, failed, and conflicted merge queue entries.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "crabdb.merge_queue_run",
            "title": "Run Merge Queue",
            "description": "Run queued merges serially, pausing on the first conflict or failure.",
            "inputSchema": object_schema(json!({
                "limit": { "type": "integer", "minimum": 1 }
            }), vec![])
        },
        {
            "name": "crabdb.merge_queue_remove",
            "title": "Remove Merge Queue Entry",
            "description": "Cancel a queued or conflicted merge queue entry by queue id, agent, branch, or ref.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string" }
            }), vec!["selector"])
        },
        {
            "name": "crabdb.conflict_list",
            "title": "List Merge Conflicts",
            "description": "List structured conflict sets opened by merge queue runs.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "crabdb.conflict_show",
            "title": "Show Merge Conflict",
            "description": "Show one structured conflict set with source, target, status, and details.",
            "inputSchema": object_schema(json!({
                "conflict_set_id": { "type": "string" }
            }), vec!["conflict_set_id"])
        },
        {
            "name": "crabdb.conflict_resolve",
            "title": "Resolve Merge Conflict",
            "description": "Resolve a conflict set by taking source, taking target, or providing manual content for every conflicted path.",
            "inputSchema": object_schema(json!({
                "conflict_set_id": { "type": "string" },
                "take": { "type": "string", "enum": ["source", "target"] },
                "manual": {
                    "type": "object",
                    "properties": {
                        "files": {
                            "type": "object",
                            "additionalProperties": {
                                "oneOf": [
                                    { "type": "string" },
                                    {
                                        "type": "object",
                                        "properties": {
                                            "content": { "type": "string" },
                                            "delete": { "type": "boolean" },
                                            "executable": { "type": "boolean" }
                                        }
                                    }
                                ]
                            }
                        }
                    }
                }
            }), vec!["conflict_set_id"])
        }
    ])
}
