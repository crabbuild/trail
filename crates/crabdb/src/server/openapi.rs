use serde_json::{json, Value};

pub fn openapi_spec() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "CrabDB Local API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Loopback JSON API for CrabDB editor integrations, agent runners, and local coordinators."
        },
        "servers": [
            {
                "url": "http://127.0.0.1:8765",
                "description": "Default local CrabDB daemon"
            }
        ],
        "security": [
            { "bearerAuth": [] },
            { "crabdbToken": [] }
        ],
        "paths": {
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
                "get": openapi_operation("timeline", "Timeline", "Return recent operations, optionally scoped by branch, session, or agent.", vec![
                    openapi_query("branch", "string"),
                    openapi_query("session", "string"),
                    openapi_query("agent", "string"),
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
                "get": openapi_operation("codeFrom", "Trace code from source", "Find operations produced by a change, message, session, or agent branch.", vec![
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
                "post": openapi_operation("guardrailCheck", "Guardrail check", "Preflight an agent action and return allowed, approval_required, or blocked.", vec![], Some("GuardrailCheckRequest"), true)
            },
            "/v1/sessions": {
                "get": openapi_operation("sessionList", "List sessions", "List durable agent sessions.", vec![
                    openapi_query("agent", "string")
                ], None, true),
                "post": openapi_operation("sessionStart", "Start session", "Start an explicit durable agent session.", vec![], Some("SessionStartRequest"), true)
            },
            "/v1/sessions/current": {
                "get": openapi_operation("sessionCurrent", "Current sessions", "Read current agent branch session attachments.", vec![
                    openapi_query("agent", "string")
                ], None, true)
            },
            "/v1/sessions/{session_id}": {
                "get": openapi_operation("sessionShow", "Show session", "Return a session with turns, messages, events, and operations.", vec![
                    openapi_path_param("session_id", "string")
                ], None, true)
            },
            "/v1/sessions/{session_id}/context": {
                "get": openapi_operation("sessionContext", "Session context", "Return a bounded session context packet with total counts and recent messages, events, turns, and operations.", vec![
                    openapi_path_param("session_id", "string"),
                    openapi_query("limit", "integer")
                ], None, true)
            },
            "/v1/sessions/{session_id}/end": {
                "post": openapi_operation("sessionEnd", "End session", "End a durable agent session.", vec![
                    openapi_path_param("session_id", "string")
                ], Some("SessionEndRequest"), true)
            },
            "/v1/approvals": {
                "get": openapi_operation("approvalList", "List approvals", "List durable human approval gates.", vec![
                    openapi_query("agent", "string"),
                    openapi_query("status", "string")
                ], None, true),
                "post": openapi_operation("approvalRequest", "Request approval", "Create a durable pending approval for a sensitive action.", vec![], Some("ApprovalRequest"), true)
            },
            "/v1/approvals/{approval_id}": {
                "get": openapi_operation("approvalShow", "Show approval", "Show one durable approval gate.", vec![
                    openapi_path_param("approval_id", "string")
                ], None, true)
            },
            "/v1/approvals/{approval_id}/decision": {
                "post": openapi_operation("approvalDecide", "Decide approval", "Approve, reject, or cancel an approval gate.", vec![
                    openapi_path_param("approval_id", "string")
                ], Some("ApprovalDecisionRequest"), true)
            },
            "/v1/leases": {
                "get": openapi_operation("leaseList", "List leases", "List active advisory leases, or all leases when requested.", vec![
                    openapi_query("all", "boolean")
                ], None, true),
                "post": openapi_operation("leaseAcquire", "Acquire lease", "Acquire an advisory path lease.", vec![], Some("LeaseAcquireRequest"), true)
            },
            "/v1/leases/{lease_id}": {
                "delete": openapi_operation("leaseRelease", "Release lease", "Release an advisory path lease.", vec![
                    openapi_path_param("lease_id", "string")
                ], None, true)
            },
            "/v1/agents/{agent_or_id}/claims": {
                "post": openapi_operation("agentClaim", "Claim agent path", "Create an advisory path claim for an agent, or return active claim conflicts as a warning.", vec![
                    openapi_path_param("agent_or_id", "string")
                ], Some("AgentClaimRequest"), true)
            },
            "/v1/anchors": {
                "get": openapi_operation("anchorList", "List anchors", "List durable line anchors.", vec![], None, true),
                "post": openapi_operation("anchorCreate", "Create anchor", "Create a durable line anchor.", vec![], Some("AnchorCreateRequest"), true)
            },
            "/v1/anchors/{anchor_id}": {
                "get": openapi_operation("anchorResolve", "Resolve anchor", "Resolve a durable line anchor.", vec![
                    openapi_path_param("anchor_id", "string"),
                    openapi_query("branch", "string")
                ], None, true),
                "delete": openapi_operation("anchorDelete", "Delete anchor", "Delete a durable line anchor.", vec![
                    openapi_path_param("anchor_id", "string")
                ], None, true)
            },
            "/v1/merge-queue": {
                "get": openapi_operation("mergeQueueList", "List merge queue", "List merge queue entries.", vec![], None, true),
                "post": openapi_operation("mergeQueueAdd", "Queue merge", "Queue an agent or branch for serialized merge.", vec![], Some("MergeQueueAddRequest"), true)
            },
            "/v1/merge-queue/run": {
                "post": openapi_operation("mergeQueueRun", "Run merge queue", "Run queued merges serially.", vec![], Some("MergeQueueRunRequest"), true)
            },
            "/v1/merge-queue/{selector}": {
                "delete": openapi_operation("mergeQueueRemove", "Remove queue entry", "Cancel a queued or conflicted merge queue entry.", vec![
                    openapi_path_param("selector", "string")
                ], None, true)
            },
            "/v1/conflicts": {
                "get": openapi_operation("conflictList", "List conflicts", "List structured conflict sets.", vec![], None, true)
            },
            "/v1/conflicts/{conflict_set_id}": {
                "get": openapi_operation("conflictShow", "Show conflict", "Show one structured conflict set.", vec![
                    openapi_path_param("conflict_set_id", "string")
                ], None, true)
            },
            "/v1/conflicts/{conflict_set_id}/resolve": {
                "post": openapi_operation("conflictResolve", "Resolve conflict", "Resolve a conflict by taking source, target, or manual content.", vec![
                    openapi_path_param("conflict_set_id", "string")
                ], Some("ConflictResolveRequest"), true)
            },
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
            },
            "/v1/branches/{branch}/merge-agent": {
                "post": openapi_operation("branchMergeAgent", "Merge agent", "Merge an agent branch into a target branch.", vec![
                    openapi_path_param("branch", "string")
                ], Some("MergeAgentRequest"), true)
            },
            "/v1/agent/turns": {
                "post": openapi_operation("turnBegin", "Begin turn", "Start a durable agent turn.", vec![], Some("BeginTurnRequest"), true)
            },
            "/v1/agent/events": {
                "get": openapi_operation("eventList", "List trace events", "List recent agent trace events filtered by agent, session, turn, or type.", vec![
                    openapi_query("agent", "string"),
                    openapi_query("session", "string"),
                    openapi_query("turn_id", "string"),
                    openapi_query("turn", "string"),
                    openapi_query("event_type", "string"),
                    openapi_query("type", "string"),
                    openapi_query("limit", "integer")
                ], None, true)
            },
            "/v1/agent/spans": {
                "get": openapi_operation("spanList", "List trace spans", "List derived agent trace spans filtered by agent, session, turn, or trace.", vec![
                    openapi_query("agent", "string"),
                    openapi_query("session", "string"),
                    openapi_query("turn_id", "string"),
                    openapi_query("turn", "string"),
                    openapi_query("trace_id", "string"),
                    openapi_query("trace", "string"),
                    openapi_query("limit", "integer")
                ], None, true)
            },
            "/v1/agent/spans/summary": {
                "get": openapi_operation("spanSummary", "Summarize trace spans", "Summarize derived agent trace spans with status/type counts, open spans, failed spans, and slowest completed spans.", vec![
                    openapi_query("agent", "string"),
                    openapi_query("session", "string"),
                    openapi_query("turn_id", "string"),
                    openapi_query("turn", "string"),
                    openapi_query("trace_id", "string"),
                    openapi_query("trace", "string"),
                    openapi_query("slowest", "integer")
                ], None, true)
            },
            "/v1/agent/runs": {
                "get": openapi_operation("agentRunList", "List agent run states", "List durable paused/resumed agent run checkpoints, optionally scoped by agent and status.", vec![
                    openapi_query("agent", "string"),
                    openapi_query("status", "string")
                ], None, true),
                "post": openapi_operation("agentRunPause", "Pause agent run", "Persist a serialized paused agent run checkpoint for later resume.", vec![], Some("AgentRunPauseRequest"), true)
            },
            "/v1/agent/runs/{run_id}": {
                "get": openapi_operation("agentRunShow", "Show agent run state", "Show one durable agent run checkpoint.", vec![
                    openapi_path_param("run_id", "string")
                ], None, true)
            },
            "/v1/agent/runs/{run_id}/resume": {
                "post": openapi_operation("agentRunResume", "Resume agent run", "Mark a paused checkpoint resumed after any linked approval is approved.", vec![
                    openapi_path_param("run_id", "string")
                ], Some("AgentRunResumeRequest"), true)
            },
            "/v1/agent/spans/{span_id}": {
                "get": openapi_operation("spanShow", "Show trace span", "Show one derived agent trace span.", vec![
                    openapi_path_param("span_id", "string")
                ], None, true)
            },
            "/v1/agent/spans/{span_id}/end": {
                "post": openapi_operation("spanEnd", "End trace span", "End an agent trace span and attach result metadata.", vec![
                    openapi_path_param("span_id", "string")
                ], Some("EndSpanRequest"), true)
            },
            "/v1/agent/turns/{turn_id}": {
                "get": openapi_operation("turnShow", "Show turn", "Return a turn with messages, trace events, and operations.", vec![
                    openapi_path_param("turn_id", "string")
                ], None, true)
            },
            "/v1/agent/turns/{turn_id}/messages": {
                "post": openapi_operation("turnAddMessage", "Add turn message", "Attach a message to a durable turn.", vec![
                    openapi_path_param("turn_id", "string")
                ], Some("AddMessageRequest"), true)
            },
            "/v1/agent/turns/{turn_id}/events": {
                "post": openapi_operation("turnAddEvent", "Add trace event", "Attach a trace event to a durable turn.", vec![
                    openapi_path_param("turn_id", "string")
                ], Some("AddEventRequest"), true)
            },
            "/v1/agent/turns/{turn_id}/spans": {
                "post": openapi_operation("turnStartSpan", "Start trace span", "Start a parentable trace span under a durable turn.", vec![
                    openapi_path_param("turn_id", "string")
                ], Some("StartSpanRequest"), true)
            },
            "/v1/agent/turns/{turn_id}/patches": {
                "post": openapi_operation("turnApplyPatch", "Apply turn patch", "Apply a patch linked to a durable turn.", vec![
                    openapi_path_param("turn_id", "string")
                ], Some("PatchRequest"), true)
            },
            "/v1/agent/turns/{turn_id}/end": {
                "post": openapi_operation("turnEnd", "End turn", "End a durable agent turn.", vec![
                    openapi_path_param("turn_id", "string")
                ], Some("EndTurnRequest"), true)
            }
        },
        "components": {
            "securitySchemes": {
                "bearerAuth": {
                    "type": "http",
                    "scheme": "bearer",
                    "description": "Send Authorization: Bearer <token>."
                },
                "crabdbToken": {
                    "type": "apiKey",
                    "in": "header",
                    "name": "X-CrabDB-Token"
                }
            },
            "responses": {
                "Error": {
                    "description": "CrabDB error response",
                    "content": {
                        "application/json": {
                            "schema": { "$ref": "#/components/schemas/ErrorBody" }
                        }
                    }
                }
            },
            "schemas": openapi_schemas()
        }
    })
}

fn openapi_operation(
    operation_id: &str,
    summary: &str,
    description: &str,
    parameters: Vec<serde_json::Value>,
    request_schema: Option<&str>,
    authenticated: bool,
) -> Value {
    let mut operation = json!({
        "operationId": operation_id,
        "summary": summary,
        "description": description,
        "parameters": parameters,
        "responses": {
            "200": {
                "description": "Successful JSON response",
                "content": {
                    "application/json": {
                        "schema": { "$ref": "#/components/schemas/JsonValue" }
                    }
                }
            },
            "400": { "$ref": "#/components/responses/Error" },
            "401": { "$ref": "#/components/responses/Error" },
            "404": { "$ref": "#/components/responses/Error" }
        }
    });
    if let Some(schema) = request_schema {
        operation["requestBody"] = json!({
            "required": true,
            "content": {
                "application/json": {
                    "schema": { "$ref": format!("#/components/schemas/{schema}") }
                }
            }
        });
    }
    if !authenticated {
        operation["security"] = json!([]);
    }
    operation
}

fn openapi_query(name: &str, value_type: &str) -> Value {
    openapi_parameter(name, "query", false, value_type)
}

fn openapi_required_query(name: &str, value_type: &str) -> Value {
    openapi_parameter(name, "query", true, value_type)
}

fn openapi_path_param(name: &str, value_type: &str) -> Value {
    openapi_parameter(name, "path", true, value_type)
}

fn openapi_parameter(name: &str, location: &str, required: bool, value_type: &str) -> Value {
    json!({
        "name": name,
        "in": location,
        "required": required,
        "schema": { "type": value_type }
    })
}

fn openapi_schemas() -> Value {
    json!({
        "JsonValue": {
            "description": "CrabDB typed JSON report. See CLI reference for the concrete report shape.",
            "oneOf": [
                { "type": "object", "additionalProperties": true },
                { "type": "array", "items": true },
                { "type": "string" },
                { "type": "number" },
                { "type": "boolean" },
                { "type": "null" }
            ]
        },
        "ErrorBody": {
            "type": "object",
            "required": ["error"],
            "properties": {
                "error": {
                    "type": "object",
                    "required": ["message", "code"],
                    "properties": {
                        "message": { "type": "string" },
                        "code": { "type": "integer" }
                    }
                }
            }
        },
        "ConfigSetRequest": {
            "type": "object",
            "required": ["key", "value"],
            "additionalProperties": false,
            "properties": {
                "key": { "type": "string" },
                "value": { "type": "string" }
            }
        },
        "IgnorePatternRequest": {
            "type": "object",
            "required": ["pattern"],
            "additionalProperties": false,
            "properties": { "pattern": { "type": "string" } }
        },
        "IgnoreCheckRequest": {
            "type": "object",
            "required": ["path"],
            "additionalProperties": false,
            "properties": { "path": { "type": "string" } }
        },
        "GuardrailCheckRequest": {
            "type": "object",
            "required": ["action"],
            "additionalProperties": false,
            "properties": {
                "agent": { "type": "string" },
                "action": { "type": "string" },
                "summary": { "type": "string" },
                "payload": { "type": "object" },
                "paths": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            }
        },
        "SpawnAgentRequest": {
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": { "type": "string" },
                "from": { "type": "string" },
                "from_ref": { "type": "string" },
                "branch": { "type": "string" },
                "materialize": { "type": "boolean" },
                "workdir": { "type": "string" },
                "workdir_path": { "type": "string" },
                "provider": { "type": "string" },
                "model": { "type": "string" }
            }
        },
        "MergeAgentRequest": {
            "type": "object",
            "properties": {
                "agent_id": { "type": "string" },
                "agent": { "type": "string" },
                "name": { "type": "string" },
                "strategy": { "type": "string" },
                "dry_run": { "type": "boolean" },
                "dry-run": { "type": "boolean" }
            }
        },
        "BeginTurnRequest": {
            "type": "object",
            "required": ["agent"],
            "properties": {
                "agent": { "type": "string" },
                "branch": { "type": "string" },
                "session_title": { "type": "string" },
                "base_change": { "type": "string" }
            }
        },
        "AddMessageRequest": {
            "type": "object",
            "required": ["role"],
            "properties": {
                "role": { "type": "string" },
                "content": { "type": "string" },
                "text": { "type": "string" }
            }
        },
        "AddEventRequest": {
            "type": "object",
            "required": ["event_type"],
            "properties": {
                "event_type": { "type": "string" },
                "type": { "type": "string" },
                "payload": { "type": "object", "additionalProperties": true },
                "change_id": { "type": "string" },
                "message_id": { "type": "string" }
            }
        },
        "StartSpanRequest": {
            "type": "object",
            "required": ["span_type", "name"],
            "properties": {
                "span_type": { "type": "string" },
                "type": { "type": "string" },
                "name": { "type": "string" },
                "parent": { "type": "string" },
                "parent_span_id": { "type": "string" },
                "trace": { "type": "string" },
                "trace_id": { "type": "string" },
                "attributes": { "type": "object", "additionalProperties": true },
                "attributes_json": { "type": "object", "additionalProperties": true }
            }
        },
        "EndSpanRequest": {
            "type": "object",
            "properties": {
                "status": { "type": "string" },
                "result": { "type": "object", "additionalProperties": true },
                "result_json": { "type": "object", "additionalProperties": true }
            }
        },
        "EndTurnRequest": {
            "type": "object",
            "properties": {
                "status": { "type": "string", "enum": ["completed", "failed", "cancelled", "archived"] }
            }
        },
        "SessionStartRequest": {
            "type": "object",
            "required": ["agent"],
            "properties": {
                "agent": { "type": "string" },
                "title": { "type": "string" },
                "id": { "type": "string" }
            }
        },
        "SessionEndRequest": {
            "type": "object",
            "properties": {
                "status": { "type": "string", "enum": ["completed", "failed", "cancelled", "archived"] }
            }
        },
        "ApprovalRequest": {
            "type": "object",
            "required": ["agent", "action", "summary"],
            "properties": {
                "agent": { "type": "string" },
                "action": { "type": "string" },
                "summary": { "type": "string" },
                "payload": { "type": "object", "additionalProperties": true },
                "session_id": { "type": "string" },
                "turn_id": { "type": "string" },
                "turn": { "type": "string" }
            }
        },
        "ApprovalDecisionRequest": {
            "type": "object",
            "required": ["decision"],
            "properties": {
                "decision": { "type": "string", "enum": ["approved", "rejected", "cancelled"] },
                "reviewer": { "type": "string" },
                "note": { "type": "string" }
            }
        },
        "AgentRunPauseRequest": {
            "type": "object",
            "required": ["agent", "reason", "summary"],
            "properties": {
                "agent": { "type": "string" },
                "reason": { "type": "string" },
                "summary": { "type": "string" },
                "state": { "type": "object", "additionalProperties": true },
                "interruption": { "type": "object", "additionalProperties": true },
                "session_id": { "type": "string" },
                "turn_id": { "type": "string" },
                "turn": { "type": "string" }
            }
        },
        "AgentRunResumeRequest": {
            "type": "object",
            "properties": {
                "reviewer": { "type": "string" },
                "note": { "type": "string" }
            }
        },
        "LeaseAcquireRequest": {
            "type": "object",
            "required": ["agent"],
            "properties": {
                "agent": { "type": "string" },
                "path": { "type": "string" },
                "mode": { "type": "string", "enum": ["read", "write"] },
                "ttl_secs": { "type": "integer", "minimum": 1 },
                "ttl": { "type": "integer", "minimum": 1 }
            }
        },
        "AgentClaimRequest": {
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": { "type": "string" },
                "ttl_secs": { "type": "integer", "minimum": 1 },
                "ttl": { "type": "integer", "minimum": 1 }
            }
        },
        "AnchorCreateRequest": {
            "type": "object",
            "required": ["path_line", "label"],
            "properties": {
                "path_line": { "type": "string" },
                "label": { "type": "string" },
                "branch": { "type": "string" }
            }
        },
        "MergeQueueAddRequest": {
            "type": "object",
            "required": ["source", "target"],
            "properties": {
                "source": { "type": "string" },
                "target": { "type": "string" },
                "into": { "type": "string" },
                "target_branch": { "type": "string" },
                "priority": { "type": "integer" }
            }
        },
        "MergeQueueRunRequest": {
            "type": "object",
            "properties": { "limit": { "type": "integer", "minimum": 1 } }
        },
        "ConflictResolveRequest": {
            "type": "object",
            "properties": {
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
            }
        },
        "AgentTestRequest": {
            "type": "object",
            "required": ["command"],
            "properties": {
                "command": { "type": "array", "items": { "type": "string" } },
                "turn_id": { "type": "string" },
                "turn": { "type": "string" },
                "timeout_secs": { "type": "integer", "minimum": 1 },
                "timeout_seconds": { "type": "integer", "minimum": 1 },
                "suite": { "type": "string" },
                "score": { "type": "number" },
                "threshold": { "type": "number" }
            }
        },
        "SyncWorkdirRequest": {
            "type": "object",
            "properties": { "force": { "type": "boolean" } }
        },
        "PatchRequest": {
            "type": "object",
            "description": "Native CrabDB PatchDocument or design-style files patch.",
            "properties": {
                "base_change": { "type": "string" },
                "message": { "type": "string" },
                "session_id": { "type": "string" },
                "allow_ignored": { "type": "boolean" },
                "edits": { "type": "array", "items": { "type": "object", "additionalProperties": true } },
                "files": { "type": "array", "items": { "type": "object", "additionalProperties": true } }
            }
        }
    })
}
