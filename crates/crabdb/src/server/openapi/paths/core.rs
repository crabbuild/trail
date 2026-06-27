use serde_json::{json, Value};

use super::{openapi_operation, openapi_path_param, openapi_query, openapi_required_query};

pub(super) fn core_paths() -> Value {
    json!({
        "/v1/health": {
            "get": openapi_operation("health", "Health check", "Return service liveness without authentication.", vec![], None, false)
        },
        "/v1/openapi.json": {
            "get": openapi_operation("openapi", "OpenAPI document", "Return this OpenAPI 3.1 document.", vec![], None, true)
        },
        "/v1/doctor": {
            "get": openapi_operation("doctor", "Workspace diagnostics", "Run read-only operational diagnostics.", vec![], None, true)
        },
        "/v1/status": {
            "get": openapi_operation("status", "Workspace status", "Return current branch status and changed paths.", vec![], None, true)
        },
        "/v1/record": {
            "post": openapi_operation("record", "Record workspace changes", "Record current workspace changes into a branch.", vec![], Some("RecordRequest"), true)
        },
        "/v1/diff": {
            "get": openapi_operation("diff", "Diff", "Show a ref range, root range, or dirty worktree diff.", vec![
                openapi_query("range", "string"),
                openapi_query("root", "string"),
                openapi_query("dirty", "boolean"),
                openapi_query("patch", "boolean"),
                openapi_query("show_line_ids", "boolean"),
                openapi_query("show-line-ids", "boolean")
            ], None, true)
        },
        "/v1/timeline": {
            "get": openapi_operation("timeline", "Timeline", "Return recent operations, optionally scoped by branch, session, or lane.", vec![
                openapi_query("branch", "string"),
                openapi_query("session", "string"),
                openapi_query("lane", "string"),
                openapi_query("limit", "integer")
            ], None, true)
        },
        "/v1/why": {
            "get": openapi_operation("why", "Explain line provenance", "Explain stable file and line identity for a path:line selector or line id.", vec![
                openapi_query("path_line", "string"),
                openapi_query("line_id", "string"),
                openapi_query("branch", "string"),
                openapi_query("at", "string")
            ], None, true)
        },
        "/v1/history": {
            "get": openapi_operation("history", "History", "Return file or line history by path, selector, file_id, or line_id.", vec![
                openapi_query("path", "string"),
                openapi_query("selector", "string"),
                openapi_query("file_id", "string"),
                openapi_query("line_id", "string")
            ], None, true)
        },
        "/v1/code-from": {
            "get": openapi_operation("codeFrom", "Trace code from source", "Find operations produced by a change, message, session, or lane branch.", vec![
                openapi_required_query("selector", "string")
            ], None, true)
        },
        "/v1/config": {
            "get": openapi_operation("configList", "List config", "List typed CrabDB workspace config entries.", vec![], None, true),
            "post": openapi_operation("configSet", "Set config", "Set one CrabDB workspace config entry.", vec![], Some("ConfigSetRequest"), true)
        },
        "/v1/config/{key}": {
            "get": openapi_operation("configGet", "Get config", "Read one typed workspace config entry.", vec![
                openapi_path_param("key", "string")
            ], None, true)
        },
        "/v1/ignore": {
            "get": openapi_operation("ignoreList", "List ignore rules", "List workspace .crabignore patterns.", vec![], None, true)
        },
        "/v1/ignore/patterns": {
            "post": openapi_operation("ignoreAdd", "Add ignore rule", "Add a workspace .crabignore pattern.", vec![], Some("IgnorePatternRequest"), true),
            "delete": openapi_operation("ignoreRemove", "Remove ignore rule", "Remove a workspace .crabignore pattern.", vec![], Some("IgnorePatternRequest"), true)
        },
        "/v1/ignore/check": {
            "post": openapi_operation("ignoreCheck", "Check ignored path", "Check whether a relative path is ignored.", vec![], Some("IgnoreCheckRequest"), true)
        },
        "/v1/guardrails/check": {
            "post": openapi_operation("guardrailCheck", "Guardrail check", "Preflight a lane action and return allowed, approval_required, or blocked.", vec![], Some("GuardrailCheckRequest"), true)
        }
    })
}
