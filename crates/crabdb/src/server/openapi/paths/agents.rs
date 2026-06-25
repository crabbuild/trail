use serde_json::{json, Value};

use super::{openapi_operation, openapi_path_param, openapi_query};

pub(super) fn agent_paths() -> Value {
    json!({
        "/v1/agents": {
            "get": openapi_operation("agentList", "List agents", "List agent branches with metadata and branch state.", vec![], None, true),
            "post": openapi_operation("agentSpawn", "Spawn agent", "Create or reuse an agent branch.", vec![], Some("SpawnAgentRequest"), true)
        },
        "/v1/agents/{agent_or_id}": {
            "get": openapi_operation("agentShow", "Show agent", "Show agent metadata and branch state.", vec![
                openapi_path_param("agent_or_id", "string")
            ], None, true),
            "delete": openapi_operation("agentRemove", "Remove agent", "Remove an agent branch and its materialized workdir. Requires force when the branch has unmerged changes.", vec![
                openapi_path_param("agent_or_id", "string"),
                openapi_query("force", "boolean")
            ], None, true)
        },
        "/v1/agents/{agent_or_id}/status": {
            "get": openapi_operation("agentStatus", "Agent status", "Show an agent branch status.", vec![
                openapi_path_param("agent_or_id", "string")
            ], None, true)
        },
        "/v1/agents/{agent_or_id}/contribution": {
            "get": openapi_operation("agentContribution", "Agent contribution", "Summarize an agent branch for review with status, changed paths, operations, sessions, events, and approvals.", vec![
                openapi_path_param("agent_or_id", "string"),
                openapi_query("limit", "integer")
            ], None, true)
        },
        "/v1/agents/{agent_or_id}/gates": {
            "get": openapi_operation("agentGates", "Agent gate history", "List recent durable test/eval gate results for one agent branch.", vec![
                openapi_path_param("agent_or_id", "string"),
                openapi_query("kind", "string"),
                openapi_query("limit", "integer")
            ], None, true)
        },
        "/v1/agents/{agent_or_id}/readiness": {
            "get": openapi_operation("agentReadiness", "Agent readiness", "Assess whether an agent branch is ready to merge by checking conflicts, approvals, workdir state, tests, and evals.", vec![
                openapi_path_param("agent_or_id", "string")
            ], None, true)
        },
        "/v1/agents/{agent_or_id}/handoff": {
            "get": openapi_operation("agentHandoff", "Agent handoff", "Package agent branch, readiness, current session context, recent events, spans, operations, and next steps for transfer to another agent or reviewer.", vec![
                openapi_path_param("agent_or_id", "string"),
                openapi_query("limit", "integer")
            ], None, true)
        },
        "/v1/agents/{agent_or_id}/diff": {
            "get": openapi_operation("agentDiff", "Agent diff", "Show the diff from an agent branch base to head.", vec![
                openapi_path_param("agent_or_id", "string"),
                openapi_query("patch", "boolean"),
                openapi_query("show_line_ids", "boolean"),
                openapi_query("show-line-ids", "boolean")
            ], None, true)
        },
        "/v1/agents/{agent_or_id}/read-file": {
            "post": openapi_operation("agentReadFile", "Read agent file", "Read one file from an agent branch. Sparse workdirs hydrate lazily when hydrate is omitted; pass hydrate=false for a side-effect-free read.", vec![
                openapi_path_param("agent_or_id", "string")
            ], Some("AgentReadFileRequest"), true)
        },
        "/v1/agents/{agent_or_id}/sync-workdir": {
            "post": openapi_operation("agentSyncWorkdir", "Sync agent workdir", "Refresh a materialized agent workdir.", vec![
                openapi_path_param("agent_or_id", "string")
            ], Some("SyncWorkdirRequest"), true)
        },
        "/v1/agents/{agent_or_id}/tests": {
            "post": openapi_operation("agentRunTest", "Run agent test", "Run a command in an agent workdir and record test events.", vec![
                openapi_path_param("agent_or_id", "string")
            ], Some("AgentTestRequest"), true)
        },
        "/v1/agents/{agent_or_id}/evals": {
            "post": openapi_operation("agentRunEval", "Run agent eval", "Run an evaluation command in an agent workdir and record eval events.", vec![
                openapi_path_param("agent_or_id", "string")
            ], Some("AgentTestRequest"), true)
        },
        "/v1/agents/{agent_or_id}/patches": {
            "post": openapi_operation("agentApplyPatch", "Apply agent patch", "Apply a patch directly to an agent branch.", vec![
                openapi_path_param("agent_or_id", "string")
            ], Some("PatchRequest"), true)
        }
    })
}
