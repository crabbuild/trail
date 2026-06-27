use serde_json::{json, Value};

use crate::mcp::response::object_schema;

pub(super) fn tools() -> Value {
    json!([
        {
            "name": "crabdb.doctor",
            "title": "CrabDB Doctor",
            "description": "Run read-only operational diagnostics for workspace health, locks, fsck, approvals, leases, merge queue, conflicts, and lane workdirs.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "crabdb.status",
            "title": "CrabDB Status",
            "description": "Read the current CrabDB branch status and changed paths.",
            "inputSchema": object_schema(json!({
                "branch": { "type": "string", "description": "Optional CrabDB branch name." }
            }), vec![])
        },
        {
            "name": "crabdb.diff",
            "title": "CrabDB Diff",
            "description": "Show a ref range, root range, or dirty worktree diff with optional patches and stable line ids.",
            "inputSchema": object_schema(json!({
                "range": { "type": "string", "description": "Ref range such as main..feature or ch_a..ch_b." },
                "root": { "type": "string", "description": "Root id range such as obj_a..obj_b." },
                "dirty": { "type": "boolean", "description": "Diff the current branch head against the materialized worktree." },
                "patch": { "type": "boolean" },
                "show_line_ids": { "type": "boolean" },
                "show-line-ids": { "type": "boolean" }
            }), vec![])
        },
        {
            "name": "crabdb.timeline",
            "title": "CrabDB Timeline",
            "description": "Read recent operations, optionally scoped to one branch, session, or lane.",
            "inputSchema": object_schema(json!({
                "branch": { "type": "string" },
                "session": { "type": "string" },
                "lane": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1 }
            }), vec![])
        },
        {
            "name": "crabdb.why",
            "title": "Explain Line Provenance",
            "description": "Explain the stable file and line identity plus recorded history for a path:line selector or line id.",
            "inputSchema": object_schema(json!({
                "path_line": { "type": "string" },
                "line_id": { "type": "string" },
                "branch": { "type": "string" },
                "at": { "type": "string" }
            }), vec![])
        },
        {
            "name": "crabdb.history",
            "title": "Read File Or Line History",
            "description": "Read file history by path/file id or line history by line id.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string" },
                "path": { "type": "string" },
                "file_id": { "type": "string" },
                "line_id": { "type": "string" }
            }), vec![])
        },
        {
            "name": "crabdb.code_from",
            "title": "Trace Code From Source",
            "description": "Find operations and changed paths produced by a change, message, session, or lane branch.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string" }
            }), vec!["selector"])
        },
        {
            "name": "crabdb.config_list",
            "title": "List CrabDB Config",
            "description": "List validated CrabDB workspace configuration entries.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "crabdb.config_get",
            "title": "Get CrabDB Config",
            "description": "Read one validated CrabDB workspace configuration entry.",
            "inputSchema": object_schema(json!({
                "key": { "type": "string" }
            }), vec!["key"])
        },
        {
            "name": "crabdb.config_set",
            "title": "Set CrabDB Config",
            "description": "Set one CrabDB workspace configuration entry using the same validation as the CLI.",
            "inputSchema": object_schema(json!({
                "key": { "type": "string" },
                "value": { "type": "string" }
            }), vec!["key", "value"])
        },
        {
            "name": "crabdb.ignore_list",
            "title": "List Ignore Rules",
            "description": "List workspace .crabignore patterns visible to CrabDB.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "crabdb.ignore_add",
            "title": "Add Ignore Rule",
            "description": "Add a workspace .crabignore pattern under CrabDB's write lock.",
            "inputSchema": object_schema(json!({
                "pattern": { "type": "string" }
            }), vec!["pattern"])
        },
        {
            "name": "crabdb.ignore_remove",
            "title": "Remove Ignore Rule",
            "description": "Remove a workspace .crabignore pattern under CrabDB's write lock.",
            "inputSchema": object_schema(json!({
                "pattern": { "type": "string" }
            }), vec!["pattern"])
        },
        {
            "name": "crabdb.ignore_check",
            "title": "Check Ignored Path",
            "description": "Check whether a relative path is ignored by the hardcoded denylist or workspace ignore files.",
            "inputSchema": object_schema(json!({
                "path": { "type": "string" }
            }), vec!["path"])
        },
        {
            "name": "crabdb.guardrail_check",
            "title": "Guardrail Check",
            "description": "Preflight a lane action against CrabDB path policy, risky tool categories, and pending approvals. Returns allowed, approval_required, or blocked.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "action": { "type": "string" },
                "summary": { "type": "string" },
                "payload": { "type": "object" },
                "paths": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            }), vec!["action"]),
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true,
                "openWorldHint": false
            }
        }
    ])
}
