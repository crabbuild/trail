use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::model::ConflictManualResolution;
use crate::{AgentGateOptions, CrabDb, Error, PatchDocument, PatchEdit, Result};

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct ServerAuth {
    token: Option<String>,
}

impl ServerAuth {
    pub fn disabled() -> Self {
        Self { token: None }
    }

    pub fn bearer(token: impl Into<String>) -> Result<Self> {
        let token = token.into();
        if token.trim().is_empty() {
            return Err(Error::InvalidInput(
                "daemon auth token cannot be empty".to_string(),
            ));
        }
        Ok(Self { token: Some(token) })
    }

    pub fn is_required(&self) -> bool {
        self.token.is_some()
    }
}

#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    reason: &'static str,
    body: Vec<u8>,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: ErrorDetails,
}

#[derive(Debug, Serialize)]
struct ErrorDetails {
    message: String,
    code: i32,
}

#[derive(Debug, Deserialize)]
struct SpawnAgentRequest {
    name: String,
    #[serde(default, alias = "from_ref", alias = "branch")]
    from: Option<String>,
    #[serde(default)]
    materialize: Option<bool>,
    #[serde(default, alias = "workdir_path")]
    workdir: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MergeAgentRequest {
    #[serde(default, alias = "agent", alias = "name")]
    agent_id: Option<String>,
    #[serde(default)]
    strategy: Option<String>,
    #[serde(default, alias = "dry-run")]
    dry_run: bool,
}

#[derive(Debug, Deserialize)]
struct BeginTurnRequest {
    agent: String,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    session_title: Option<String>,
    #[serde(default)]
    base_change: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AddMessageRequest {
    role: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AddEventRequest {
    #[serde(alias = "type")]
    event_type: String,
    #[serde(default)]
    payload: Option<serde_json::Value>,
    #[serde(default)]
    change_id: Option<String>,
    #[serde(default)]
    message_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StartSpanRequest {
    #[serde(alias = "type")]
    span_type: String,
    name: String,
    #[serde(default, alias = "parent_span_id")]
    parent: Option<String>,
    #[serde(default, alias = "trace_id")]
    trace: Option<String>,
    #[serde(default, alias = "attributes_json")]
    attributes: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct EndSpanRequest {
    #[serde(default = "default_completed_status")]
    status: String,
    #[serde(default, alias = "result_json")]
    result: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct EndTurnRequest {
    #[serde(default = "default_completed_status")]
    status: String,
}

#[derive(Debug, Deserialize)]
struct AgentTestRequest {
    command: Vec<String>,
    #[serde(default, alias = "turn")]
    turn_id: Option<String>,
    #[serde(default, alias = "timeout_seconds")]
    timeout_secs: Option<u64>,
    #[serde(default)]
    suite: Option<String>,
    #[serde(default)]
    score: Option<f64>,
    #[serde(default)]
    threshold: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct SyncWorkdirRequest {
    #[serde(default)]
    force: bool,
}

#[derive(Debug, Deserialize)]
struct IgnorePatternRequest {
    pattern: String,
}

#[derive(Debug, Deserialize)]
struct IgnoreCheckRequest {
    path: String,
}

#[derive(Debug, Deserialize)]
struct GuardrailCheckRequest {
    agent: Option<String>,
    action: String,
    summary: Option<String>,
    payload: Option<serde_json::Value>,
    #[serde(default)]
    paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ConfigSetRequest {
    key: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct SessionStartRequest {
    agent: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SessionEndRequest {
    #[serde(default = "default_completed_status")]
    status: String,
}

#[derive(Debug, Deserialize)]
struct ApprovalRequest {
    agent: String,
    action: String,
    summary: String,
    #[serde(default)]
    payload: Option<serde_json::Value>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default, alias = "turn")]
    turn_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApprovalDecisionRequest {
    decision: String,
    #[serde(default)]
    reviewer: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AgentRunPauseRequest {
    agent: String,
    reason: String,
    summary: String,
    #[serde(default)]
    state: Option<serde_json::Value>,
    #[serde(default)]
    interruption: Option<serde_json::Value>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default, alias = "turn")]
    turn_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AgentRunResumeRequest {
    #[serde(default)]
    reviewer: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LeaseAcquireRequest {
    agent: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default, alias = "ttl")]
    ttl_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct AgentClaimRequest {
    path: String,
    #[serde(default, alias = "ttl")]
    ttl_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct AnchorCreateRequest {
    path_line: String,
    label: String,
    #[serde(default)]
    branch: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MergeQueueAddRequest {
    source: String,
    #[serde(alias = "into", alias = "target_branch")]
    target: String,
    #[serde(default)]
    priority: i64,
}

#[derive(Debug, Deserialize)]
struct MergeQueueRunRequest {
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ConflictResolveRequest {
    #[serde(default)]
    take: Option<String>,
    #[serde(default)]
    manual: Option<ConflictManualResolution>,
}

#[derive(Debug, Deserialize)]
struct ApiPatchRequest {
    #[serde(default)]
    base_change: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    allow_ignored: bool,
    #[serde(default)]
    edits: Vec<PatchEdit>,
    #[serde(default)]
    files: Vec<ApiPatchFile>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ApiPatchFile {
    AddText {
        path: String,
        content: String,
        #[serde(default)]
        executable: bool,
    },
    ModifyText {
        path: String,
        edits: Vec<ApiTextEdit>,
    },
    WriteBytes {
        path: String,
        bytes_hex: String,
        #[serde(default)]
        executable: bool,
    },
    Delete {
        path: String,
    },
    Rename {
        from: String,
        to: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ApiTextEdit {
    ModifyLine {
        line_id: String,
        #[serde(default)]
        expected_text: Option<String>,
        new_text: String,
    },
}

pub fn openapi_spec() -> serde_json::Value {
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
) -> serde_json::Value {
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

fn openapi_query(name: &str, value_type: &str) -> serde_json::Value {
    openapi_parameter(name, "query", false, value_type)
}

fn openapi_required_query(name: &str, value_type: &str) -> serde_json::Value {
    openapi_parameter(name, "query", true, value_type)
}

fn openapi_path_param(name: &str, value_type: &str) -> serde_json::Value {
    openapi_parameter(name, "path", true, value_type)
}

fn openapi_parameter(
    name: &str,
    location: &str,
    required: bool,
    value_type: &str,
) -> serde_json::Value {
    json!({
        "name": name,
        "in": location,
        "required": required,
        "schema": { "type": value_type }
    })
}

fn openapi_schemas() -> serde_json::Value {
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

fn default_completed_status() -> String {
    "completed".to_string()
}

fn default_lease_mode() -> String {
    "write".to_string()
}

pub fn serve_listener(
    mut db: CrabDb,
    listener: TcpListener,
    max_requests: Option<usize>,
) -> Result<()> {
    serve_listener_with_auth(&mut db, listener, max_requests, ServerAuth::disabled())
}

pub fn serve_listener_with_auth(
    db: &mut CrabDb,
    listener: TcpListener,
    max_requests: Option<usize>,
    auth: ServerAuth,
) -> Result<()> {
    let mut handled = 0usize;
    loop {
        if max_requests.is_some_and(|max| handled >= max) {
            break;
        }
        let (stream, _) = listener.accept()?;
        handle_connection(db, stream, &auth)?;
        handled += 1;
    }
    Ok(())
}

pub fn handle_http_request(db: &mut CrabDb, raw: &[u8]) -> HttpResponse {
    handle_http_request_with_auth(db, raw, &ServerAuth::disabled())
}

pub fn handle_http_request_with_auth(
    db: &mut CrabDb,
    raw: &[u8],
    auth: &ServerAuth,
) -> HttpResponse {
    match parse_request(raw) {
        Ok(request) => route_request(db, request, auth),
        Err(err) => error_response(&err),
    }
}

fn handle_connection(db: &mut CrabDb, mut stream: TcpStream, auth: &ServerAuth) -> Result<()> {
    let request = read_request(&mut stream)?;
    let response = route_request(db, request, auth);
    stream.write_all(&response.to_http_bytes())?;
    stream.flush()?;
    Ok(())
}

fn read_request(stream: &mut TcpStream) -> Result<HttpRequest> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut first_line = String::new();
    reader.read_line(&mut first_line)?;
    if first_line.trim().is_empty() {
        return Err(Error::InvalidInput("empty HTTP request".to_string()));
    }
    let mut content_length = 0usize;
    let mut headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().map_err(|_| {
                    Error::InvalidInput("invalid Content-Length header".to_string())
                })?;
            }
        }
    }
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body)?;
    parse_request_parts(&first_line, headers, body)
}

fn parse_request(raw: &[u8]) -> Result<HttpRequest> {
    let raw = String::from_utf8_lossy(raw);
    let Some((head, body)) = raw.split_once("\r\n\r\n") else {
        return Err(Error::InvalidInput("malformed HTTP request".to_string()));
    };
    let mut lines = head.lines();
    let first_line = lines
        .next()
        .ok_or_else(|| Error::InvalidInput("empty HTTP request".to_string()))?;
    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    parse_request_parts(first_line, headers, body.as_bytes().to_vec())
}

fn parse_request_parts(
    first_line: &str,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
) -> Result<HttpRequest> {
    let mut parts = first_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| Error::InvalidInput("missing HTTP method".to_string()))?;
    let path = parts
        .next()
        .ok_or_else(|| Error::InvalidInput("missing HTTP path".to_string()))?;
    Ok(HttpRequest {
        method: method.to_string(),
        path: path.to_string(),
        headers,
        body,
    })
}

fn route_request(db: &mut CrabDb, request: HttpRequest, auth: &ServerAuth) -> HttpResponse {
    match route_request_result(db, request, auth) {
        Ok(response) => response,
        Err(err) => error_response(&err),
    }
}

fn route_request_result(
    db: &mut CrabDb,
    request: HttpRequest,
    auth: &ServerAuth,
) -> Result<HttpResponse> {
    let raw_path = request.path.trim_end_matches('/');
    let (path, query) = raw_path.split_once('?').unwrap_or((raw_path, ""));
    if request.method == "GET" && path == "/v1/health" {
        return json_response(
            200,
            "OK",
            &json!({
                "ok": true,
                "service": "crabdb",
                "version": env!("CARGO_PKG_VERSION")
            }),
        );
    }

    if !authorized(&request, auth) {
        return Ok(unauthorized_response());
    }

    if request.method == "GET" && path == "/v1/openapi.json" {
        return json_response(200, "OK", &openapi_spec());
    }

    if request.method == "GET" && path == "/v1/doctor" {
        let report = db.doctor()?;
        return json_response(200, "OK", &report);
    }

    if request.method == "GET" && path == "/v1/status" {
        let report = db.status(None)?;
        return json_response(200, "OK", &report);
    }

    if request.method == "GET" && path == "/v1/diff" {
        let patch = query_flag(query, "patch");
        let line_ids = query_line_ids_flag(query);
        let diff = if query_flag(query, "dirty") {
            if query_value(query, "range").is_some() || query_value(query, "root").is_some() {
                return Err(Error::InvalidInput(
                    "diff accepts only one of `range`, `root`, or `dirty`".to_string(),
                ));
            }
            db.diff_dirty(patch, line_ids)?
        } else if let Some(root) = query_value(query, "root") {
            if query_value(query, "range").is_some() {
                return Err(Error::InvalidInput(
                    "diff accepts only one of `range`, `root`, or `dirty`".to_string(),
                ));
            }
            db.diff_roots(root, patch, line_ids)?
        } else {
            let range = required_query(query, "range")?;
            db.diff_range_with_options(range, patch, line_ids)?
        };
        return json_response(200, "OK", &diff);
    }

    if request.method == "GET" && path == "/v1/config" {
        let entries = db.config_entries();
        return json_response(200, "OK", &entries);
    }

    if request.method == "POST" && path == "/v1/config" {
        let body: ConfigSetRequest = serde_json::from_slice(&request.body)?;
        let report = db.config_set(&body.key, &body.value)?;
        return json_response(200, "OK", &report);
    }

    if request.method == "GET" && path == "/v1/timeline" {
        let limit = query_usize(query, "limit", 30)?;
        let entries = db.timeline_query(
            query_value(query, "branch"),
            query_value(query, "session"),
            query_value(query, "agent"),
            limit,
        )?;
        return json_response(200, "OK", &entries);
    }

    if request.method == "GET" && path == "/v1/why" {
        let at = query_value(query, "at").or_else(|| query_value(query, "branch"));
        let result = match (
            query_value(query, "path_line"),
            query_value(query, "line_id"),
        ) {
            (Some(path_line), None) => db.why(path_line, at)?,
            (None, Some(line_id)) => db.why_line_id(line_id, at)?,
            (Some(_), Some(_)) => {
                return Err(Error::InvalidInput(
                    "why accepts either `path_line` or `line_id`, not both".to_string(),
                ));
            }
            (None, None) => {
                return Err(Error::InvalidInput(
                    "why requires `path_line` or `line_id`".to_string(),
                ));
            }
        };
        return json_response(200, "OK", &result);
    }

    if request.method == "GET" && path == "/v1/history" {
        let result = if let Some(line_id) = query_value(query, "line_id") {
            db.history_for_line_id(line_id)?
        } else if let Some(file_id) = query_value(query, "file_id") {
            db.history_for_file_id(file_id)?
        } else {
            let selector = query_value(query, "path")
                .or_else(|| query_value(query, "selector"))
                .ok_or_else(|| {
                    Error::InvalidInput(
                        "history requires `path`, `selector`, `file_id`, or `line_id`".to_string(),
                    )
                })?;
            db.history_for_path(selector)?
        };
        return json_response(200, "OK", &result);
    }

    if request.method == "GET" && path == "/v1/code-from" {
        let selector = required_query(query, "selector")?;
        let result = db.code_from(selector)?;
        return json_response(200, "OK", &result);
    }

    if request.method == "GET" && path == "/v1/ignore" {
        let report = db.ignore_list()?;
        return json_response(200, "OK", &report);
    }

    if request.method == "POST" && path == "/v1/ignore/patterns" {
        let body: IgnorePatternRequest = serde_json::from_slice(&request.body)?;
        let report = db.ignore_add(&body.pattern)?;
        return json_response(200, "OK", &report);
    }

    if request.method == "DELETE" && path == "/v1/ignore/patterns" {
        let body: IgnorePatternRequest = serde_json::from_slice(&request.body)?;
        let report = db.ignore_remove(&body.pattern)?;
        return json_response(200, "OK", &report);
    }

    if request.method == "POST" && path == "/v1/ignore/check" {
        let body: IgnoreCheckRequest = serde_json::from_slice(&request.body)?;
        let report = db.ignore_check(&body.path)?;
        return json_response(200, "OK", &report);
    }

    if request.method == "POST" && path == "/v1/guardrails/check" {
        let body: GuardrailCheckRequest = serde_json::from_slice(&request.body)?;
        let report = db.guardrail_check(
            body.agent.as_deref(),
            &body.action,
            body.summary.as_deref(),
            body.payload,
            &body.paths,
        )?;
        return json_response(200, "OK", &report);
    }

    if request.method == "GET" && path == "/v1/agents" {
        let agents = db.list_agents()?;
        return json_response(200, "OK", &agents);
    }

    if request.method == "POST" && path == "/v1/agents" {
        let body: SpawnAgentRequest = serde_json::from_slice(&request.body)?;
        let report = db.spawn_agent_with_workdir(
            &body.name,
            body.from.as_deref(),
            body.materialize.unwrap_or(true),
            body.provider,
            body.model,
            body.workdir.map(PathBuf::from),
        )?;
        return json_response(201, "Created", &report);
    }

    if request.method == "POST" && path == "/v1/agent/turns" {
        let body: BeginTurnRequest = serde_json::from_slice(&request.body)?;
        let report = db.begin_agent_turn(
            &body.agent,
            body.branch.as_deref(),
            body.session_title,
            body.base_change.as_deref(),
        )?;
        return json_response(201, "Created", &report);
    }

    if request.method == "GET" && path == "/v1/agent/events" {
        let limit = query_usize(query, "limit", 50)?;
        let events = db.list_agent_events(
            query_value(query, "agent"),
            query_value(query, "session"),
            query_value(query, "turn_id").or_else(|| query_value(query, "turn")),
            query_value(query, "event_type").or_else(|| query_value(query, "type")),
            limit,
        )?;
        return json_response(200, "OK", &events);
    }

    if request.method == "GET" && path == "/v1/agent/spans" {
        let limit = query_usize(query, "limit", 50)?;
        let spans = db.list_agent_trace_spans(
            query_value(query, "agent"),
            query_value(query, "session"),
            query_value(query, "turn_id").or_else(|| query_value(query, "turn")),
            query_value(query, "trace_id").or_else(|| query_value(query, "trace")),
            limit,
        )?;
        return json_response(200, "OK", &spans);
    }

    if request.method == "GET" && path == "/v1/agent/spans/summary" {
        let slowest_limit = query_usize(query, "slowest", 5)?;
        let report = db.summarize_agent_trace_spans(
            query_value(query, "agent"),
            query_value(query, "session"),
            query_value(query, "turn_id").or_else(|| query_value(query, "turn")),
            query_value(query, "trace_id").or_else(|| query_value(query, "trace")),
            slowest_limit,
        )?;
        return json_response(200, "OK", &report);
    }

    if request.method == "GET" && path == "/v1/agent/runs" {
        let run_states =
            db.list_agent_run_states(query_value(query, "agent"), query_value(query, "status"))?;
        return json_response(200, "OK", &run_states);
    }

    if request.method == "POST" && path == "/v1/agent/runs" {
        let body: AgentRunPauseRequest = serde_json::from_slice(&request.body)?;
        let report = db.pause_agent_run(
            &body.agent,
            &body.reason,
            &body.summary,
            body.state,
            body.interruption,
            body.session_id.as_deref(),
            body.turn_id.as_deref(),
        )?;
        return json_response(201, "Created", &report);
    }

    if request.method == "GET" && path == "/v1/sessions/current" {
        let reports = db.current_agent_sessions(query_value(query, "agent"))?;
        return json_response(200, "OK", &reports);
    }

    if request.method == "GET" && path == "/v1/sessions" {
        let sessions = db.list_agent_sessions(query_value(query, "agent"))?;
        return json_response(200, "OK", &sessions);
    }

    if request.method == "POST" && path == "/v1/sessions" {
        let body: SessionStartRequest = serde_json::from_slice(&request.body)?;
        let report = db.start_agent_session(&body.agent, body.title, body.id)?;
        return json_response(201, "Created", &report);
    }

    if request.method == "GET" && path == "/v1/approvals" {
        let approvals =
            db.list_agent_approvals(query_value(query, "agent"), query_value(query, "status"))?;
        return json_response(200, "OK", &approvals);
    }

    if request.method == "POST" && path == "/v1/approvals" {
        let body: ApprovalRequest = serde_json::from_slice(&request.body)?;
        let report = db.request_agent_approval(
            &body.agent,
            &body.action,
            &body.summary,
            body.payload,
            body.session_id.as_deref(),
            body.turn_id.as_deref(),
        )?;
        return json_response(201, "Created", &report);
    }

    if request.method == "GET" && path == "/v1/leases" {
        let leases = db.list_leases(query_flag(query, "all"))?;
        return json_response(200, "OK", &leases);
    }

    if request.method == "POST" && path == "/v1/leases" {
        let body: LeaseAcquireRequest = serde_json::from_slice(&request.body)?;
        let mode = body.mode.unwrap_or_else(default_lease_mode);
        let report = db.acquire_lease(
            &body.agent,
            body.path.as_deref(),
            &mode,
            body.ttl_secs.unwrap_or(3600),
        )?;
        return json_response(201, "Created", &report);
    }

    if request.method == "GET" && path == "/v1/anchors" {
        let anchors = db.list_anchors()?;
        return json_response(200, "OK", &anchors);
    }

    if request.method == "POST" && path == "/v1/anchors" {
        let body: AnchorCreateRequest = serde_json::from_slice(&request.body)?;
        let report = db.create_anchor(&body.path_line, body.label, body.branch.as_deref())?;
        return json_response(201, "Created", &report);
    }

    if request.method == "GET" && path == "/v1/merge-queue" {
        let entries = db.list_merge_queue()?;
        return json_response(200, "OK", &entries);
    }

    if request.method == "POST" && path == "/v1/merge-queue" {
        let body: MergeQueueAddRequest = serde_json::from_slice(&request.body)?;
        let report = db.enqueue_merge(&body.source, &body.target, body.priority)?;
        return json_response(201, "Created", &report);
    }

    if request.method == "POST" && path == "/v1/merge-queue/run" {
        let body: MergeQueueRunRequest = if request.body.is_empty() {
            MergeQueueRunRequest { limit: None }
        } else {
            serde_json::from_slice(&request.body)?
        };
        let report = db.run_merge_queue(body.limit)?;
        return json_response(200, "OK", &report);
    }

    if request.method == "GET" && path == "/v1/conflicts" {
        let conflicts = db.list_conflicts()?;
        return json_response(200, "OK", &conflicts);
    }

    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    if parts.len() == 3 && parts[0] == "v1" && parts[1] == "config" && request.method == "GET" {
        let entry = db.config_get(parts[2])?;
        return json_response(200, "OK", &entry);
    }

    if parts.len() == 3 && parts[0] == "v1" && parts[1] == "sessions" && request.method == "GET" {
        let details = db.show_agent_session(parts[2])?;
        return json_response(200, "OK", &details);
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "sessions"
        && parts[3] == "context"
        && request.method == "GET"
    {
        let report = db.agent_session_context(parts[2], query_usize(query, "limit", 50)?)?;
        return json_response(200, "OK", &report);
    }

    if parts.len() == 3 && parts[0] == "v1" && parts[1] == "approvals" && request.method == "GET" {
        let approval = db.show_agent_approval(parts[2])?;
        return json_response(200, "OK", &approval);
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "agent"
        && parts[2] == "runs"
        && request.method == "GET"
    {
        let run_state = db.show_agent_run_state(parts[3])?;
        return json_response(200, "OK", &run_state);
    }

    if parts.len() == 5
        && parts[0] == "v1"
        && parts[1] == "agent"
        && parts[2] == "runs"
        && parts[4] == "resume"
        && request.method == "POST"
    {
        let body: AgentRunResumeRequest = if request.body.is_empty() {
            AgentRunResumeRequest {
                reviewer: None,
                note: None,
            }
        } else {
            serde_json::from_slice(&request.body)?
        };
        let report = db.resume_agent_run(parts[3], body.reviewer, body.note)?;
        return json_response(200, "OK", &report);
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "approvals"
        && parts[3] == "decision"
        && request.method == "POST"
    {
        let body: ApprovalDecisionRequest = serde_json::from_slice(&request.body)?;
        let report =
            db.decide_agent_approval(parts[2], &body.decision, body.reviewer, body.note)?;
        return json_response(200, "OK", &report);
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "sessions"
        && parts[3] == "end"
        && request.method == "POST"
    {
        let body: SessionEndRequest = if request.body.is_empty() {
            SessionEndRequest {
                status: default_completed_status(),
            }
        } else {
            serde_json::from_slice(&request.body)?
        };
        let report = db.end_agent_session(parts[2], &body.status)?;
        return json_response(200, "OK", &report);
    }

    if parts.len() == 3 && parts[0] == "v1" && parts[1] == "leases" && request.method == "DELETE" {
        let report = db.release_lease(parts[2])?;
        return json_response(200, "OK", &report);
    }

    if parts.len() == 3 && parts[0] == "v1" && parts[1] == "anchors" {
        if request.method == "GET" {
            let report = db.resolve_anchor(parts[2], query_value(query, "branch"))?;
            return json_response(200, "OK", &report);
        }
        if request.method == "DELETE" {
            let report = db.delete_anchor(parts[2])?;
            return json_response(200, "OK", &report);
        }
    }

    if parts.len() == 3
        && parts[0] == "v1"
        && parts[1] == "merge-queue"
        && request.method == "DELETE"
    {
        let report = db.remove_merge_queue(parts[2])?;
        return json_response(200, "OK", &report);
    }

    if parts.len() == 3 && parts[0] == "v1" && parts[1] == "conflicts" && request.method == "GET" {
        let conflict = db.show_conflict(parts[2])?;
        return json_response(200, "OK", &conflict);
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "conflicts"
        && parts[3] == "resolve"
        && request.method == "POST"
    {
        let body: ConflictResolveRequest = serde_json::from_slice(&request.body)?;
        let report = resolve_conflict_request(db, parts[2], body)?;
        return json_response(200, "OK", &report);
    }

    if parts.len() == 3 && parts[0] == "v1" && parts[1] == "agents" {
        let agent = db.resolve_agent_handle(parts[2])?;
        if request.method == "GET" {
            let details = db.agent_details(&agent)?;
            return json_response(200, "OK", &details);
        }
        if request.method == "DELETE" {
            let report = db.remove_agent(&agent, query_flag(query, "force"))?;
            return json_response(200, "OK", &report);
        }
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "agents"
        && parts[3] == "claims"
        && request.method == "POST"
    {
        let agent = db.resolve_agent_handle(parts[2])?;
        let body: AgentClaimRequest = serde_json::from_slice(&request.body)?;
        let report = db.claim_agent_path(&agent, &body.path, body.ttl_secs.unwrap_or(600))?;
        return json_response(200, "OK", &report);
    }

    if parts.len() == 4 && parts[0] == "v1" && parts[1] == "agents" && request.method == "GET" {
        let agent = db.resolve_agent_handle(parts[2])?;
        return match parts[3] {
            "status" => {
                let report = db.agent_status(&agent)?;
                json_response(200, "OK", &report)
            }
            "contribution" => {
                let report = db.agent_contribution(&agent, query_usize(query, "limit", 50)?)?;
                json_response(200, "OK", &report)
            }
            "gates" => {
                let report = db.agent_gate_history(
                    &agent,
                    query_value(query, "kind"),
                    query_usize(query, "limit", 50)?,
                )?;
                json_response(200, "OK", &report)
            }
            "readiness" => {
                let report = db.agent_readiness(&agent)?;
                json_response(200, "OK", &report)
            }
            "handoff" => {
                let report = db.agent_handoff(&agent, query_usize(query, "limit", 50)?)?;
                json_response(200, "OK", &report)
            }
            "diff" => {
                let diff = db.diff_agent_with_options(
                    &agent,
                    query_flag(query, "patch"),
                    query_line_ids_flag(query),
                )?;
                json_response(200, "OK", &diff)
            }
            _ => Err(Error::InvalidInput(format!(
                "unknown API endpoint `{}`",
                request.path
            ))),
        };
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "agents"
        && parts[3] == "sync-workdir"
        && request.method == "POST"
    {
        let agent = db.resolve_agent_handle(parts[2])?;
        let body: SyncWorkdirRequest = if request.body.is_empty() {
            SyncWorkdirRequest { force: false }
        } else {
            serde_json::from_slice(&request.body)?
        };
        let report = db.sync_agent_workdir(&agent, body.force)?;
        return json_response(200, "OK", &report);
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "agents"
        && (parts[3] == "tests" || parts[3] == "evals")
        && request.method == "POST"
    {
        let agent = db.resolve_agent_handle(parts[2])?;
        let body: AgentTestRequest = serde_json::from_slice(&request.body)?;
        let options = AgentGateOptions {
            suite: body.suite,
            score: body.score,
            threshold: body.threshold,
        };
        let report = if parts[3] == "evals" {
            db.run_agent_eval_with_options(
                &agent,
                body.command,
                body.turn_id.as_deref(),
                body.timeout_secs.unwrap_or(600),
                options,
            )?
        } else {
            db.run_agent_test_with_options(
                &agent,
                body.command,
                body.turn_id.as_deref(),
                body.timeout_secs.unwrap_or(600),
                options,
            )?
        };
        return json_response(200, "OK", &report);
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "agents"
        && parts[3] == "patches"
        && request.method == "POST"
    {
        let agent = db.resolve_agent_handle(parts[2])?;
        let patch = parse_patch_request(&request.body)?;
        let report = db.apply_agent_patch(&agent, patch)?;
        return json_response(200, "OK", &report);
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "branches"
        && parts[3] == "merge-agent"
        && request.method == "POST"
    {
        let body: MergeAgentRequest = serde_json::from_slice(&request.body)?;
        validate_merge_strategy(body.strategy.as_deref())?;
        let agent = body.agent_id.ok_or_else(|| {
            Error::InvalidInput("merge-agent request requires `agent_id`".to_string())
        })?;
        let agent = db.resolve_agent_handle(&agent)?;
        let report = db.merge_agent_with_options(&agent, parts[2], body.dry_run)?;
        return json_response(200, "OK", &report);
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "agent"
        && parts[2] == "spans"
        && request.method == "GET"
    {
        let span = db.show_agent_trace_span(parts[3])?;
        return json_response(200, "OK", &span);
    }

    if parts.len() == 5
        && parts[0] == "v1"
        && parts[1] == "agent"
        && parts[2] == "spans"
        && parts[4] == "end"
        && request.method == "POST"
    {
        let body: EndSpanRequest = if request.body.is_empty() {
            EndSpanRequest {
                status: default_completed_status(),
                result: None,
            }
        } else {
            serde_json::from_slice(&request.body)?
        };
        let report = db.end_agent_trace_span(parts[3], &body.status, body.result)?;
        return json_response(200, "OK", &report);
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "agent"
        && parts[2] == "turns"
        && request.method == "GET"
    {
        let details = db.show_agent_turn(parts[3])?;
        return json_response(200, "OK", &details);
    }

    if parts.len() == 5
        && parts[0] == "v1"
        && parts[1] == "agent"
        && parts[2] == "turns"
        && request.method == "POST"
    {
        let turn_id = parts[3];
        return match parts[4] {
            "messages" => {
                let body: AddMessageRequest = serde_json::from_slice(&request.body)?;
                let text = body.content.or(body.text).ok_or_else(|| {
                    Error::InvalidInput("message body requires `content` or `text`".to_string())
                })?;
                let report = db.add_agent_turn_message(turn_id, &body.role, &text)?;
                json_response(201, "Created", &report)
            }
            "events" => {
                let body: AddEventRequest = serde_json::from_slice(&request.body)?;
                let report = db.add_agent_turn_event(
                    turn_id,
                    &body.event_type,
                    body.payload,
                    body.change_id.as_deref(),
                    body.message_id.as_deref(),
                )?;
                json_response(201, "Created", &report)
            }
            "spans" => {
                let body: StartSpanRequest = serde_json::from_slice(&request.body)?;
                let report = db.start_agent_trace_span(
                    turn_id,
                    &body.span_type,
                    &body.name,
                    body.parent.as_deref(),
                    body.trace.as_deref(),
                    body.attributes,
                )?;
                json_response(201, "Created", &report)
            }
            "patches" => {
                let patch = parse_patch_request(&request.body)?;
                let report = db.apply_agent_turn_patch(turn_id, patch)?;
                json_response(200, "OK", &report)
            }
            "end" => {
                let body: EndTurnRequest = if request.body.is_empty() {
                    EndTurnRequest {
                        status: default_completed_status(),
                    }
                } else {
                    serde_json::from_slice(&request.body)?
                };
                let report = db.end_agent_turn(turn_id, &body.status)?;
                json_response(200, "OK", &report)
            }
            _ => Err(Error::InvalidInput(format!(
                "unknown API endpoint `{}`",
                request.path
            ))),
        };
    }

    Err(Error::InvalidInput(format!(
        "unknown API endpoint `{}`",
        request.path
    )))
}

fn resolve_conflict_request(
    db: &mut CrabDb,
    conflict_set_id: &str,
    body: ConflictResolveRequest,
) -> Result<crate::model::ConflictResolveReport> {
    match (body.take, body.manual) {
        (Some(take), None) => db.resolve_conflict(conflict_set_id, &take),
        (None, Some(manual)) => db.resolve_conflict_manual(conflict_set_id, manual),
        (Some(_), Some(_)) => Err(Error::InvalidInput(
            "conflict resolve request must include only one of `take` or `manual`".to_string(),
        )),
        (None, None) => Err(Error::InvalidInput(
            "conflict resolve request requires `take` or `manual`".to_string(),
        )),
    }
}

fn parse_patch_request(body: &[u8]) -> Result<PatchDocument> {
    let request: ApiPatchRequest = serde_json::from_slice(body)?;
    let mut edits = request.edits;
    for file in request.files {
        match file {
            ApiPatchFile::AddText {
                path,
                content,
                executable,
            } => edits.push(PatchEdit::Write {
                path,
                content,
                executable,
            }),
            ApiPatchFile::ModifyText {
                path,
                edits: file_edits,
            } => {
                for edit in file_edits {
                    match edit {
                        ApiTextEdit::ModifyLine {
                            line_id,
                            expected_text,
                            new_text,
                        } => edits.push(PatchEdit::ReplaceLine {
                            path: path.clone(),
                            line_id,
                            expected_text,
                            new_text,
                        }),
                    }
                }
            }
            ApiPatchFile::WriteBytes {
                path,
                bytes_hex,
                executable,
            } => edits.push(PatchEdit::WriteBytes {
                path,
                bytes_hex,
                executable,
            }),
            ApiPatchFile::Delete { path } => edits.push(PatchEdit::Delete { path }),
            ApiPatchFile::Rename { from, to } => edits.push(PatchEdit::Rename { from, to }),
        }
    }
    Ok(PatchDocument {
        base_change: request.base_change,
        message: request.message,
        session_id: request.session_id,
        allow_ignored: request.allow_ignored,
        edits,
    })
}

fn query_flag(query: &str, key: &str) -> bool {
    query.split('&').any(|part| {
        let Some((candidate, value)) = part.split_once('=') else {
            return part == key;
        };
        candidate == key && matches!(value, "1" | "true" | "yes")
    })
}

fn query_line_ids_flag(query: &str) -> bool {
    query_flag(query, "show_line_ids") || query_flag(query, "show-line-ids")
}

fn validate_merge_strategy(value: Option<&str>) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    match value {
        "conservative" | "line-id-aware" | "line_id_aware" => Ok(()),
        other => Err(Error::InvalidInput(format!(
            "merge strategy must be conservative, line-id-aware, or line_id_aware, got `{other}`"
        ))),
    }
}

fn query_value<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|part| {
        let (candidate, value) = part.split_once('=')?;
        (candidate == key && !value.is_empty()).then_some(value)
    })
}

fn required_query<'a>(query: &'a str, key: &str) -> Result<&'a str> {
    query_value(query, key)
        .ok_or_else(|| Error::InvalidInput(format!("missing `{key}` query value")))
}

fn query_usize(query: &str, key: &str, default: usize) -> Result<usize> {
    let Some(value) = query_value(query, key) else {
        return Ok(default);
    };
    value
        .parse()
        .map_err(|_| Error::InvalidInput(format!("invalid `{key}` query value `{value}`")))
}

fn json_response<T: Serialize>(
    status: u16,
    reason: &'static str,
    value: &T,
) -> Result<HttpResponse> {
    Ok(HttpResponse {
        status,
        reason,
        body: serde_json::to_vec(value)?,
    })
}

fn authorized(request: &HttpRequest, auth: &ServerAuth) -> bool {
    let Some(expected) = auth.token.as_deref() else {
        return true;
    };
    if let Some(value) = request.headers.get("authorization") {
        if let Some((scheme, token)) = value.split_once(' ') {
            if scheme.eq_ignore_ascii_case("bearer")
                && constant_time_eq(token.trim().as_bytes(), expected.as_bytes())
            {
                return true;
            }
        }
    }
    request
        .headers
        .get("x-crabdb-token")
        .is_some_and(|token| constant_time_eq(token.trim().as_bytes(), expected.as_bytes()))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut diff = left.len() ^ right.len();
    let max_len = left.len().max(right.len());
    for idx in 0..max_len {
        let l = left.get(idx).copied().unwrap_or(0);
        let r = right.get(idx).copied().unwrap_or(0);
        diff |= (l ^ r) as usize;
    }
    diff == 0
}

fn unauthorized_response() -> HttpResponse {
    let body = serde_json::to_vec(&ErrorBody {
        error: ErrorDetails {
            message: "unauthorized: missing or invalid CrabDB daemon token".to_string(),
            code: 11,
        },
    })
    .unwrap_or_else(|_| b"{\"error\":{\"message\":\"unauthorized\",\"code\":11}}".to_vec());
    HttpResponse {
        status: 401,
        reason: "Unauthorized",
        body,
    }
}

fn error_response(err: &Error) -> HttpResponse {
    let status = match err {
        Error::RefNotFound(_) | Error::OperationNotFound(_) | Error::RootNotFound(_) => 404,
        Error::Conflict(_)
        | Error::DirtyWorktree
        | Error::DirtyWorktreeWithMessage(_)
        | Error::PatchRejected(_)
        | Error::StaleBranch(_)
        | Error::WorkspaceLocked(_) => 409,
        Error::InvalidInput(_) | Error::InvalidPath { .. } | Error::IgnoredPath(_) => 400,
        _ => 500,
    };
    let reason = match status {
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        _ => "Internal Server Error",
    };
    let body = serde_json::to_vec(&ErrorBody {
        error: ErrorDetails {
            message: err.to_string(),
            code: err.exit_code(),
        },
    })
    .unwrap_or_else(|_| b"{\"error\":{\"message\":\"serialization failed\",\"code\":1}}".to_vec());
    HttpResponse {
        status,
        reason,
        body,
    }
}

impl HttpResponse {
    pub fn body_json<T: for<'de> Deserialize<'de>>(&self) -> Result<T> {
        serde_json::from_slice(&self.body).map_err(Error::from)
    }

    fn to_http_bytes(&self) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.status,
            self.reason,
            self.body.len()
        )
        .into_bytes();
        out.extend_from_slice(&self.body);
        out
    }
}
