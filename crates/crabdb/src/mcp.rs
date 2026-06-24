use std::collections::BTreeSet;
use std::io::{BufRead, Write};
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::model::ConflictManualResolution;
use crate::{AgentGateOptions, CrabDb, Error, PatchDocument, PatchEdit, Result};

const SERVER_NAME: &str = "crabdb";
const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
const RESOURCE_STATUS: &str = "crabdb://workspace/status";
const RESOURCE_DOCTOR: &str = "crabdb://workspace/doctor";
const RESOURCE_AGENTS: &str = "crabdb://workspace/agents";
const RESOURCE_MERGE_QUEUE: &str = "crabdb://workspace/merge-queue";
const RESOURCE_CONFLICTS: &str = "crabdb://workspace/conflicts";
const RESOURCE_OPENAPI: &str = "crabdb://workspace/openapi";
const RESOURCE_USER_GUIDE: &str = "crabdb://docs/user-guide";
const RESOURCE_AGENT_WORKFLOWS: &str = "crabdb://docs/agent-workflows";
const RESOURCE_CLI_REFERENCE: &str = "crabdb://docs/cli-reference";
const RESOURCE_AGENT_TEMPLATE: &str = "crabdb://workspace/agents/{agent}";
const RESOURCE_AGENT_STATUS_TEMPLATE: &str = "crabdb://workspace/agents/{agent}/status";
const RESOURCE_AGENT_CONTRIBUTION_TEMPLATE: &str = "crabdb://workspace/agents/{agent}/contribution";
const RESOURCE_AGENT_GATES_TEMPLATE: &str = "crabdb://workspace/agents/{agent}/gates";
const RESOURCE_AGENT_READINESS_TEMPLATE: &str = "crabdb://workspace/agents/{agent}/readiness";
const RESOURCE_AGENT_HANDOFF_TEMPLATE: &str = "crabdb://workspace/agents/{agent}/handoff";
const RESOURCE_AGENT_DIFF_TEMPLATE: &str = "crabdb://workspace/agents/{agent}/diff";
const RESOURCE_SESSION_TEMPLATE: &str = "crabdb://workspace/sessions/{session_id}";
const RESOURCE_TURN_TEMPLATE: &str = "crabdb://workspace/turns/{turn_id}";
const RESOURCE_CONFLICT_TEMPLATE: &str = "crabdb://workspace/conflicts/{conflict_set_id}";
const RESOURCE_APPROVAL_TEMPLATE: &str = "crabdb://workspace/approvals/{approval_id}";
const RESOURCE_SPAN_TEMPLATE: &str = "crabdb://workspace/spans/{span_id}";
const PROMPT_AGENT_TASK: &str = "crabdb.agent_task";
const PROMPT_REVIEW_AGENT: &str = "crabdb.review_agent";
const PROMPT_RESOLVE_CONFLICT: &str = "crabdb.resolve_conflict";

const USER_GUIDE_MD: &str = include_str!("../../../docs/USER_GUIDE.md");
const AGENT_WORKFLOWS_MD: &str = include_str!("../../../docs/AGENT_WORKFLOWS.md");
const CLI_REFERENCE_MD: &str = include_str!("../../../docs/CLI_REFERENCE.md");

#[derive(Debug, Deserialize)]
struct ToolCall {
    name: String,
    #[serde(default)]
    arguments: Value,
}

#[derive(Debug, Deserialize)]
struct ResourceReadArgs {
    uri: String,
}

#[derive(Debug, Deserialize)]
struct PromptGetArgs {
    name: String,
    #[serde(default)]
    arguments: Value,
}

#[derive(Debug, Deserialize)]
struct CompletionArgs {
    #[serde(rename = "ref")]
    reference: CompletionReference,
    argument: CompletionArgument,
}

#[derive(Debug, Deserialize)]
struct CompletionReference {
    #[serde(rename = "type")]
    reference_type: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    uri: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CompletionArgument {
    name: String,
    #[serde(default)]
    value: String,
}

#[derive(Debug, Deserialize)]
struct StatusArgs {
    #[serde(default)]
    branch: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DiffArgs {
    #[serde(default)]
    range: Option<String>,
    #[serde(default)]
    root: Option<String>,
    #[serde(default)]
    dirty: bool,
    #[serde(default)]
    patch: bool,
    #[serde(default, alias = "show-line-ids")]
    show_line_ids: bool,
}

#[derive(Debug, Deserialize)]
struct TimelineArgs {
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ConfigKeyArgs {
    key: String,
}

#[derive(Debug, Deserialize)]
struct ConfigSetArgs {
    key: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct SessionStartArgs {
    agent: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SessionListArgs {
    #[serde(default)]
    agent: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SessionCurrentArgs {
    #[serde(default)]
    agent: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SessionIdArgs {
    session_id: String,
}

#[derive(Debug, Deserialize)]
struct SessionContextArgs {
    session_id: String,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct SessionEndArgs {
    session_id: String,
    #[serde(default = "default_completed_status")]
    status: String,
}

#[derive(Debug, Deserialize)]
struct ApprovalRequestArgs {
    agent: String,
    action: String,
    summary: String,
    #[serde(default)]
    payload: Option<Value>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default, alias = "turn")]
    turn_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApprovalListArgs {
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApprovalShowArgs {
    approval_id: String,
}

#[derive(Debug, Deserialize)]
struct ApprovalDecideArgs {
    approval_id: String,
    decision: String,
    #[serde(default)]
    reviewer: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LeaseAcquireArgs {
    agent: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default, alias = "ttl")]
    ttl_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct LeaseListArgs {
    #[serde(default)]
    all: bool,
}

#[derive(Debug, Deserialize)]
struct LeaseReleaseArgs {
    lease_id: String,
}

#[derive(Debug, Deserialize)]
struct WhyArgs {
    #[serde(default)]
    path_line: Option<String>,
    #[serde(default)]
    line_id: Option<String>,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HistoryArgs {
    #[serde(default)]
    selector: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    file_id: Option<String>,
    #[serde(default)]
    line_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodeFromArgs {
    selector: String,
}

#[derive(Debug, Deserialize)]
struct AgentSpawnArgs {
    name: String,
    #[serde(default, alias = "from", alias = "branch")]
    from_ref: Option<String>,
    #[serde(default = "default_true")]
    materialize: bool,
    #[serde(default, alias = "workdir_path")]
    workdir: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AgentClaimArgs {
    agent: String,
    path: String,
    #[serde(default, alias = "ttl")]
    ttl_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct AgentHandleArgs {
    #[serde(alias = "agent_or_id", alias = "name")]
    agent: String,
}

#[derive(Debug, Deserialize)]
struct AgentContributionArgs {
    #[serde(alias = "agent_or_id", alias = "name")]
    agent: String,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct AgentRemoveArgs {
    #[serde(alias = "agent_or_id", alias = "name")]
    agent: String,
    #[serde(default)]
    force: bool,
}

#[derive(Debug, Deserialize)]
struct AnchorCreateArgs {
    path_line: String,
    label: String,
    #[serde(default)]
    branch: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnchorIdArgs {
    anchor_id: String,
    #[serde(default)]
    branch: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MergeQueueAddArgs {
    source: String,
    #[serde(alias = "into", alias = "target_branch")]
    target: String,
    #[serde(default)]
    priority: i64,
}

#[derive(Debug, Deserialize)]
struct MergeQueueRunArgs {
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct MergeQueueRemoveArgs {
    selector: String,
}

#[derive(Debug, Deserialize)]
struct ConflictIdArgs {
    conflict_set_id: String,
}

#[derive(Debug, Deserialize)]
struct ConflictResolveArgs {
    conflict_set_id: String,
    #[serde(default)]
    take: Option<String>,
    #[serde(default)]
    manual: Option<ConflictManualResolution>,
}

#[derive(Debug, Deserialize)]
struct BeginTurnArgs {
    agent: String,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    session_title: Option<String>,
    #[serde(default)]
    base_change: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TurnIdArgs {
    turn_id: String,
}

#[derive(Debug, Deserialize)]
struct AddMessageArgs {
    turn_id: String,
    role: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AddEventArgs {
    turn_id: String,
    #[serde(alias = "type")]
    event_type: String,
    #[serde(default)]
    payload: Option<Value>,
    #[serde(default)]
    change_id: Option<String>,
    #[serde(default)]
    message_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EventListArgs {
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    session: Option<String>,
    #[serde(default, alias = "turn")]
    turn_id: Option<String>,
    #[serde(default, alias = "type")]
    event_type: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct SpanStartArgs {
    turn_id: String,
    #[serde(alias = "type")]
    span_type: String,
    name: String,
    #[serde(default, alias = "parent_span_id")]
    parent: Option<String>,
    #[serde(default, alias = "trace_id")]
    trace: Option<String>,
    #[serde(default, alias = "attributes_json")]
    attributes: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct SpanEndArgs {
    span_id: String,
    #[serde(default = "default_completed_status")]
    status: String,
    #[serde(default, alias = "result_json")]
    result: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct SpanListArgs {
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    session: Option<String>,
    #[serde(default, alias = "turn")]
    turn_id: Option<String>,
    #[serde(default, alias = "trace")]
    trace_id: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct SpanSummaryArgs {
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    session: Option<String>,
    #[serde(default, alias = "turn")]
    turn_id: Option<String>,
    #[serde(default, alias = "trace")]
    trace_id: Option<String>,
    #[serde(default, alias = "slowest_limit")]
    slowest: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct SpanShowArgs {
    span_id: String,
}

#[derive(Debug, Deserialize)]
struct EndTurnArgs {
    turn_id: String,
    #[serde(default = "default_completed_status")]
    status: String,
}

#[derive(Debug, Deserialize)]
struct DiffAgentArgs {
    agent: String,
    #[serde(default)]
    patch: bool,
    #[serde(default, alias = "show-line-ids")]
    show_line_ids: bool,
}

#[derive(Debug, Deserialize)]
struct GateHistoryArgs {
    agent: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct RunTestArgs {
    agent: String,
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
struct SyncWorkdirArgs {
    agent: String,
    #[serde(default)]
    force: bool,
}

#[derive(Debug, Deserialize)]
struct IgnorePatternArgs {
    pattern: String,
}

#[derive(Debug, Deserialize)]
struct IgnoreCheckArgs {
    path: String,
}

#[derive(Debug, Deserialize)]
struct GuardrailCheckArgs {
    agent: Option<String>,
    action: String,
    summary: Option<String>,
    payload: Option<Value>,
    #[serde(default)]
    paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ApplyPatchArgs {
    turn_id: String,
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

fn default_completed_status() -> String {
    "completed".to_string()
}

fn default_lease_mode() -> String {
    "write".to_string()
}

fn default_true() -> bool {
    true
}

pub fn serve_stdio<R: BufRead, W: Write>(db: &mut CrabDb, input: R, output: &mut W) -> Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = serde_json::from_str(&line)?;
        if let Some(response) = handle_json_rpc(db, request) {
            serde_json::to_writer(&mut *output, &response)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}

pub fn handle_json_rpc(db: &mut CrabDb, request: Value) -> Option<Value> {
    let id = request.get("id").cloned();
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return id.map(|id| json_rpc_error(id, -32600, "invalid JSON-RPC request"));
    };
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    if id.is_none() {
        return None;
    }
    let id = id.unwrap();

    match method {
        "initialize" => Some(json_rpc_result(id, initialize_result())),
        "server/discover" => Some(json_rpc_result(id, discover_result())),
        "ping" => Some(json_rpc_result(id, json!({}))),
        "tools/list" => Some(json_rpc_result(id, tools_list_result())),
        "resources/list" => Some(json_rpc_result(id, resources_list_result())),
        "resources/templates/list" => Some(json_rpc_result(id, resources_templates_list_result())),
        "prompts/list" => Some(json_rpc_result(id, prompts_list_result())),
        "prompts/get" => Some(match handle_prompt_get(params) {
            Ok(result) => json_rpc_result(id, result),
            Err(err) => prompt_error_response(id, &err),
        }),
        "completion/complete" => Some(match handle_completion_complete(db, params) {
            Ok(result) => json_rpc_result(id, result),
            Err(err) => completion_error_response(id, &err),
        }),
        "resources/read" => Some(match handle_resource_read(db, params) {
            Ok(result) => json_rpc_result(id, result),
            Err(err) => resource_error_response(id, &err),
        }),
        "tools/call" => Some(match handle_tool_call(db, params) {
            Ok(result) => json_rpc_result(id, result),
            Err(err) => json_rpc_result(id, tool_error_result(&err)),
        }),
        _ => Some(json_rpc_error(
            id,
            -32601,
            &format!("method not found: {method}"),
        )),
    }
}

fn handle_tool_call(db: &mut CrabDb, params: Value) -> Result<Value> {
    let call: ToolCall = serde_json::from_value(params)?;
    match call.name.as_str() {
        "crabdb.doctor" => tool_result(db.doctor()?),
        "crabdb.status" => {
            let args: StatusArgs = from_arguments(call.arguments)?;
            tool_result(db.status(args.branch.as_deref())?)
        }
        "crabdb.diff" => {
            let args: DiffArgs = from_arguments(call.arguments)?;
            let forms = usize::from(args.range.is_some())
                + usize::from(args.root.is_some())
                + usize::from(args.dirty);
            if forms != 1 {
                return Err(Error::InvalidInput(
                    "diff requires exactly one of `range`, `root`, or `dirty`".to_string(),
                ));
            }
            let diff = if args.dirty {
                db.diff_dirty(args.patch, args.show_line_ids)?
            } else if let Some(root) = args.root {
                db.diff_roots(&root, args.patch, args.show_line_ids)?
            } else {
                db.diff_range_with_options(
                    args.range.as_deref().unwrap_or_default(),
                    args.patch,
                    args.show_line_ids,
                )?
            };
            tool_result(diff)
        }
        "crabdb.timeline" => {
            let args: TimelineArgs = from_arguments(call.arguments)?;
            tool_result(db.timeline_query(
                args.branch.as_deref(),
                args.session.as_deref(),
                args.agent.as_deref(),
                args.limit.unwrap_or(30),
            )?)
        }
        "crabdb.why" => {
            let args: WhyArgs = from_arguments(call.arguments)?;
            let at = args.at.as_deref().or(args.branch.as_deref());
            let result = match (args.path_line.as_deref(), args.line_id.as_deref()) {
                (Some(path_line), None) => db.why(path_line, at)?,
                (None, Some(line_id)) => db.why_line_id(line_id, at)?,
                (Some(_), Some(_)) => {
                    return Err(Error::InvalidInput(
                        "crabdb.why accepts either path_line or line_id, not both".to_string(),
                    ));
                }
                (None, None) => {
                    return Err(Error::InvalidInput(
                        "crabdb.why requires path_line or line_id".to_string(),
                    ));
                }
            };
            tool_result(result)
        }
        "crabdb.history" => {
            let args: HistoryArgs = from_arguments(call.arguments)?;
            let result = if let Some(line_id) = args.line_id {
                db.history_for_line_id(&line_id)?
            } else if let Some(file_id) = args.file_id {
                db.history_for_file_id(&file_id)?
            } else {
                let selector = args.path.or(args.selector).ok_or_else(|| {
                    Error::InvalidInput(
                        "history requires `path`, `selector`, `file_id`, or `line_id`".to_string(),
                    )
                })?;
                db.history_for_path(&selector)?
            };
            tool_result(result)
        }
        "crabdb.code_from" => {
            let args: CodeFromArgs = from_arguments(call.arguments)?;
            tool_result(db.code_from(&args.selector)?)
        }
        "crabdb.agent_spawn" => {
            let args: AgentSpawnArgs = from_arguments(call.arguments)?;
            tool_result(db.spawn_agent_with_workdir(
                &args.name,
                args.from_ref.as_deref(),
                args.materialize,
                args.provider,
                args.model,
                args.workdir.map(PathBuf::from),
            )?)
        }
        "crabdb.agent_claim" => {
            let args: AgentClaimArgs = from_arguments(call.arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.claim_agent_path(&agent, &args.path, args.ttl_secs.unwrap_or(600))?)
        }
        "crabdb.agent_list" => tool_result(db.list_agents()?),
        "crabdb.agent_show" => {
            let args: AgentHandleArgs = from_arguments(call.arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.agent_details(&agent)?)
        }
        "crabdb.agent_status" => {
            let args: AgentHandleArgs = from_arguments(call.arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.agent_status(&agent)?)
        }
        "crabdb.agent_contribution" => {
            let args: AgentContributionArgs = from_arguments(call.arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.agent_contribution(&agent, args.limit.unwrap_or(50))?)
        }
        "crabdb.agent_readiness" => {
            let args: AgentHandleArgs = from_arguments(call.arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.agent_readiness(&agent)?)
        }
        "crabdb.agent_handoff" => {
            let args: AgentContributionArgs = from_arguments(call.arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.agent_handoff(&agent, args.limit.unwrap_or(50))?)
        }
        "crabdb.agent_remove" => {
            let args: AgentRemoveArgs = from_arguments(call.arguments)?;
            let agent = db.resolve_agent_handle(&args.agent)?;
            tool_result(db.remove_agent(&agent, args.force)?)
        }
        "crabdb.config_list" => tool_result(db.config_entries()),
        "crabdb.config_get" => {
            let args: ConfigKeyArgs = from_arguments(call.arguments)?;
            tool_result(db.config_get(&args.key)?)
        }
        "crabdb.config_set" => {
            let args: ConfigSetArgs = from_arguments(call.arguments)?;
            tool_result(db.config_set(&args.key, &args.value)?)
        }
        "crabdb.session_start" => {
            let args: SessionStartArgs = from_arguments(call.arguments)?;
            tool_result(db.start_agent_session(&args.agent, args.title, args.id)?)
        }
        "crabdb.session_list" => {
            let args: SessionListArgs = from_arguments(call.arguments)?;
            tool_result(db.list_agent_sessions(args.agent.as_deref())?)
        }
        "crabdb.session_current" => {
            let args: SessionCurrentArgs = from_arguments(call.arguments)?;
            tool_result(db.current_agent_sessions(args.agent.as_deref())?)
        }
        "crabdb.session_show" => {
            let args: SessionIdArgs = from_arguments(call.arguments)?;
            tool_result(db.show_agent_session(&args.session_id)?)
        }
        "crabdb.session_context" => {
            let args: SessionContextArgs = from_arguments(call.arguments)?;
            tool_result(db.agent_session_context(&args.session_id, args.limit.unwrap_or(50))?)
        }
        "crabdb.session_end" => {
            let args: SessionEndArgs = from_arguments(call.arguments)?;
            tool_result(db.end_agent_session(&args.session_id, &args.status)?)
        }
        "crabdb.approval_request" => {
            let args: ApprovalRequestArgs = from_arguments(call.arguments)?;
            tool_result(db.request_agent_approval(
                &args.agent,
                &args.action,
                &args.summary,
                args.payload,
                args.session_id.as_deref(),
                args.turn_id.as_deref(),
            )?)
        }
        "crabdb.approval_list" => {
            let args: ApprovalListArgs = from_arguments(call.arguments)?;
            tool_result(db.list_agent_approvals(args.agent.as_deref(), args.status.as_deref())?)
        }
        "crabdb.approval_show" => {
            let args: ApprovalShowArgs = from_arguments(call.arguments)?;
            tool_result(db.show_agent_approval(&args.approval_id)?)
        }
        "crabdb.approval_decide" => {
            let args: ApprovalDecideArgs = from_arguments(call.arguments)?;
            tool_result(db.decide_agent_approval(
                &args.approval_id,
                &args.decision,
                args.reviewer,
                args.note,
            )?)
        }
        "crabdb.lease_acquire" => {
            let args: LeaseAcquireArgs = from_arguments(call.arguments)?;
            let mode = args.mode.unwrap_or_else(default_lease_mode);
            tool_result(db.acquire_lease(
                &args.agent,
                args.path.as_deref(),
                &mode,
                args.ttl_secs.unwrap_or(3600),
            )?)
        }
        "crabdb.lease_list" => {
            let args: LeaseListArgs = from_arguments(call.arguments)?;
            tool_result(db.list_leases(args.all)?)
        }
        "crabdb.lease_release" => {
            let args: LeaseReleaseArgs = from_arguments(call.arguments)?;
            tool_result(db.release_lease(&args.lease_id)?)
        }
        "crabdb.anchor_create" => {
            let args: AnchorCreateArgs = from_arguments(call.arguments)?;
            tool_result(db.create_anchor(&args.path_line, args.label, args.branch.as_deref())?)
        }
        "crabdb.anchor_list" => tool_result(db.list_anchors()?),
        "crabdb.anchor_resolve" => {
            let args: AnchorIdArgs = from_arguments(call.arguments)?;
            tool_result(db.resolve_anchor(&args.anchor_id, args.branch.as_deref())?)
        }
        "crabdb.anchor_delete" => {
            let args: AnchorIdArgs = from_arguments(call.arguments)?;
            tool_result(db.delete_anchor(&args.anchor_id)?)
        }
        "crabdb.merge_queue_add" => {
            let args: MergeQueueAddArgs = from_arguments(call.arguments)?;
            tool_result(db.enqueue_merge(&args.source, &args.target, args.priority)?)
        }
        "crabdb.merge_queue_list" => tool_result(db.list_merge_queue()?),
        "crabdb.merge_queue_run" => {
            let args: MergeQueueRunArgs = from_arguments(call.arguments)?;
            tool_result(db.run_merge_queue(args.limit)?)
        }
        "crabdb.merge_queue_remove" => {
            let args: MergeQueueRemoveArgs = from_arguments(call.arguments)?;
            tool_result(db.remove_merge_queue(&args.selector)?)
        }
        "crabdb.conflict_list" => tool_result(db.list_conflicts()?),
        "crabdb.conflict_show" => {
            let args: ConflictIdArgs = from_arguments(call.arguments)?;
            tool_result(db.show_conflict(&args.conflict_set_id)?)
        }
        "crabdb.conflict_resolve" => {
            let args: ConflictResolveArgs = from_arguments(call.arguments)?;
            match (args.take, args.manual) {
                (Some(take), None) => {
                    tool_result(db.resolve_conflict(&args.conflict_set_id, &take)?)
                }
                (None, Some(manual)) => {
                    tool_result(db.resolve_conflict_manual(&args.conflict_set_id, manual)?)
                }
                (Some(_), Some(_)) => Err(Error::InvalidInput(
                    "conflict_resolve requires only one of `take` or `manual`".to_string(),
                )),
                (None, None) => Err(Error::InvalidInput(
                    "conflict_resolve requires `take` or `manual`".to_string(),
                )),
            }
        }
        "crabdb.begin_turn" => {
            let args: BeginTurnArgs = from_arguments(call.arguments)?;
            tool_result(db.begin_agent_turn(
                &args.agent,
                args.branch.as_deref(),
                args.session_title,
                args.base_change.as_deref(),
            )?)
        }
        "crabdb.add_message" => {
            let args: AddMessageArgs = from_arguments(call.arguments)?;
            let text = args.content.or(args.text).ok_or_else(|| {
                Error::InvalidInput("add_message requires `content` or `text`".to_string())
            })?;
            tool_result(db.add_agent_turn_message(&args.turn_id, &args.role, &text)?)
        }
        "crabdb.add_event" => {
            let args: AddEventArgs = from_arguments(call.arguments)?;
            tool_result(db.add_agent_turn_event(
                &args.turn_id,
                &args.event_type,
                args.payload,
                args.change_id.as_deref(),
                args.message_id.as_deref(),
            )?)
        }
        "crabdb.event_list" => {
            let args: EventListArgs = from_arguments(call.arguments)?;
            tool_result(db.list_agent_events(
                args.agent.as_deref(),
                args.session.as_deref(),
                args.turn_id.as_deref(),
                args.event_type.as_deref(),
                args.limit.unwrap_or(50),
            )?)
        }
        "crabdb.span_start" => {
            let args: SpanStartArgs = from_arguments(call.arguments)?;
            tool_result(db.start_agent_trace_span(
                &args.turn_id,
                &args.span_type,
                &args.name,
                args.parent.as_deref(),
                args.trace.as_deref(),
                args.attributes,
            )?)
        }
        "crabdb.span_end" => {
            let args: SpanEndArgs = from_arguments(call.arguments)?;
            tool_result(db.end_agent_trace_span(&args.span_id, &args.status, args.result)?)
        }
        "crabdb.span_list" => {
            let args: SpanListArgs = from_arguments(call.arguments)?;
            tool_result(db.list_agent_trace_spans(
                args.agent.as_deref(),
                args.session.as_deref(),
                args.turn_id.as_deref(),
                args.trace_id.as_deref(),
                args.limit.unwrap_or(50),
            )?)
        }
        "crabdb.span_summary" => {
            let args: SpanSummaryArgs = from_arguments(call.arguments)?;
            tool_result(db.summarize_agent_trace_spans(
                args.agent.as_deref(),
                args.session.as_deref(),
                args.turn_id.as_deref(),
                args.trace_id.as_deref(),
                args.slowest.unwrap_or(5),
            )?)
        }
        "crabdb.span_show" => {
            let args: SpanShowArgs = from_arguments(call.arguments)?;
            tool_result(db.show_agent_trace_span(&args.span_id)?)
        }
        "crabdb.apply_patch" => {
            let args: ApplyPatchArgs = from_arguments(call.arguments)?;
            let turn_id = args.turn_id.clone();
            tool_result(db.apply_agent_turn_patch(&turn_id, patch_document_from_args(args))?)
        }
        "crabdb.end_turn" => {
            let args: EndTurnArgs = from_arguments(call.arguments)?;
            tool_result(db.end_agent_turn(&args.turn_id, &args.status)?)
        }
        "crabdb.show_turn" => {
            let args: TurnIdArgs = from_arguments(call.arguments)?;
            tool_result(db.show_agent_turn(&args.turn_id)?)
        }
        "crabdb.diff_agent" => {
            let args: DiffAgentArgs = from_arguments(call.arguments)?;
            tool_result(db.diff_agent_with_options(&args.agent, args.patch, args.show_line_ids)?)
        }
        "crabdb.gate_history" => {
            let args: GateHistoryArgs = from_arguments(call.arguments)?;
            tool_result(db.agent_gate_history(
                &args.agent,
                args.kind.as_deref(),
                args.limit.unwrap_or(50),
            )?)
        }
        "crabdb.run_test" => {
            let args: RunTestArgs = from_arguments(call.arguments)?;
            let options = AgentGateOptions {
                suite: args.suite,
                score: args.score,
                threshold: args.threshold,
            };
            tool_result(db.run_agent_test_with_options(
                &args.agent,
                args.command,
                args.turn_id.as_deref(),
                args.timeout_secs.unwrap_or(600),
                options,
            )?)
        }
        "crabdb.run_eval" => {
            let args: RunTestArgs = from_arguments(call.arguments)?;
            let options = AgentGateOptions {
                suite: args.suite,
                score: args.score,
                threshold: args.threshold,
            };
            tool_result(db.run_agent_eval_with_options(
                &args.agent,
                args.command,
                args.turn_id.as_deref(),
                args.timeout_secs.unwrap_or(600),
                options,
            )?)
        }
        "crabdb.sync_workdir" => {
            let args: SyncWorkdirArgs = from_arguments(call.arguments)?;
            tool_result(db.sync_agent_workdir(&args.agent, args.force)?)
        }
        "crabdb.ignore_list" => tool_result(db.ignore_list()?),
        "crabdb.ignore_add" => {
            let args: IgnorePatternArgs = from_arguments(call.arguments)?;
            tool_result(db.ignore_add(&args.pattern)?)
        }
        "crabdb.ignore_remove" => {
            let args: IgnorePatternArgs = from_arguments(call.arguments)?;
            tool_result(db.ignore_remove(&args.pattern)?)
        }
        "crabdb.ignore_check" => {
            let args: IgnoreCheckArgs = from_arguments(call.arguments)?;
            tool_result(db.ignore_check(&args.path)?)
        }
        "crabdb.guardrail_check" => {
            let args: GuardrailCheckArgs = from_arguments(call.arguments)?;
            tool_result(db.guardrail_check(
                args.agent.as_deref(),
                &args.action,
                args.summary.as_deref(),
                args.payload,
                &args.paths,
            )?)
        }
        _ => Err(Error::InvalidInput(format!(
            "unknown MCP tool `{}`",
            call.name
        ))),
    }
}

fn patch_document_from_args(args: ApplyPatchArgs) -> PatchDocument {
    let mut edits = args.edits;
    for file in args.files {
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
    PatchDocument {
        base_change: args.base_change,
        message: args.message,
        session_id: args.session_id,
        allow_ignored: args.allow_ignored,
        edits,
    }
}

fn from_arguments<T: for<'de> Deserialize<'de>>(arguments: Value) -> Result<T> {
    if arguments.is_null() {
        serde_json::from_value(json!({})).map_err(Error::from)
    } else {
        serde_json::from_value(arguments).map_err(Error::from)
    }
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "serverInfo": {
            "name": SERVER_NAME,
            "version": env!("CARGO_PKG_VERSION")
        },
        "capabilities": {
            "tools": {
                "listChanged": false
            },
            "resources": {
                "listChanged": false
            },
            "prompts": {
                "listChanged": false
            },
            "completions": {}
        }
    })
}

fn discover_result() -> Value {
    json!({
        "supportedVersions": [MCP_PROTOCOL_VERSION],
        "serverInfo": {
            "name": SERVER_NAME,
            "version": env!("CARGO_PKG_VERSION")
        },
        "capabilities": {
            "tools": {
                "listChanged": false
            },
            "resources": {
                "listChanged": false
            },
            "prompts": {
                "listChanged": false
            },
            "completions": {}
        }
    })
}

fn tools_list_result() -> Value {
    json!({
        "resultType": "complete",
        "tools": tools(),
        "ttlMs": 300000,
        "cacheScope": "public"
    })
}

fn resources_list_result() -> Value {
    json!({
        "resources": resources(),
        "ttlMs": 300000,
        "cacheScope": "public"
    })
}

fn resources_templates_list_result() -> Value {
    json!({
        "resourceTemplates": resource_templates()
    })
}

fn resource_templates() -> Value {
    json!([
        {
            "uriTemplate": RESOURCE_AGENT_TEMPLATE,
            "name": "agent",
            "title": "Agent Details",
            "description": "Read one agent record and branch state by agent name or id.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_STATUS_TEMPLATE,
            "name": "agent-status",
            "title": "Agent Status",
            "description": "Read one agent's branch, workdir, queue, and latest gate status.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_CONTRIBUTION_TEMPLATE,
            "name": "agent-contribution",
            "title": "Agent Contribution",
            "description": "Read one review bundle for an agent: status, changed paths, operations, sessions, events, and approvals.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_GATES_TEMPLATE,
            "name": "agent-gates",
            "title": "Agent Gate History",
            "description": "Read recent test/eval gate results for one agent.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_READINESS_TEMPLATE,
            "name": "agent-readiness",
            "title": "Agent Readiness",
            "description": "Read one merge-readiness report with blockers, warnings, conflicts, approvals, and latest gates.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_HANDOFF_TEMPLATE,
            "name": "agent-handoff",
            "title": "Agent Handoff",
            "description": "Read one transfer packet with readiness, current session context, recent events, spans, operations, and next steps.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_DIFF_TEMPLATE,
            "name": "agent-diff",
            "title": "Agent Diff",
            "description": "Read one agent branch diff summary without unified patches.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_SESSION_TEMPLATE,
            "name": "session",
            "title": "Agent Session",
            "description": "Read one durable agent session with turns, messages, events, and operations.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_TURN_TEMPLATE,
            "name": "turn",
            "title": "Agent Turn",
            "description": "Read one durable agent turn with messages, events, and operations.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_CONFLICT_TEMPLATE,
            "name": "conflict",
            "title": "Conflict Set",
            "description": "Read one structured merge conflict set.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_APPROVAL_TEMPLATE,
            "name": "approval",
            "title": "Approval Gate",
            "description": "Read one durable human approval gate.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_SPAN_TEMPLATE,
            "name": "trace-span",
            "title": "Trace Span",
            "description": "Read one derived agent trace span.",
            "mimeType": "application/json"
        }
    ])
}

fn prompts_list_result() -> Value {
    json!({
        "prompts": prompts(),
        "ttlMs": 300000,
        "cacheScope": "public"
    })
}

fn prompts() -> Value {
    json!([
        {
            "name": PROMPT_AGENT_TASK,
            "title": "Run a CrabDB Agent Task",
            "description": "Guide an MCP host through a safe CrabDB agent task with turn tracking, patching, gates, and merge handoff.",
            "arguments": [
                {
                    "name": "agent",
                    "description": "Agent branch name to use or create.",
                    "required": true
                },
                {
                    "name": "task",
                    "description": "User-visible task objective.",
                    "required": true
                },
                {
                    "name": "branch",
                    "description": "Base branch, defaulting to main.",
                    "required": false
                }
            ]
        },
        {
            "name": PROMPT_REVIEW_AGENT,
            "title": "Review a CrabDB Agent",
            "description": "Guide a host through reviewing an agent branch before merge.",
            "arguments": [
                {
                    "name": "agent",
                    "description": "Agent branch name or id to review.",
                    "required": true
                }
            ]
        },
        {
            "name": PROMPT_RESOLVE_CONFLICT,
            "title": "Resolve a CrabDB Conflict",
            "description": "Guide a host through inspecting and resolving a structured CrabDB merge conflict.",
            "arguments": [
                {
                    "name": "conflict_set_id",
                    "description": "Conflict set id from CrabDB.",
                    "required": true
                }
            ]
        }
    ])
}

fn resources() -> Value {
    json!([
        {
            "uri": RESOURCE_STATUS,
            "name": "status",
            "title": "Workspace Status",
            "description": "Current branch, worktree state, and changed paths.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_DOCTOR,
            "name": "doctor",
            "title": "Workspace Diagnostics",
            "description": "Read-only operational health checks for the CrabDB workspace.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_AGENTS,
            "name": "agents",
            "title": "Agents",
            "description": "Current agent branches and lifecycle metadata.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_MERGE_QUEUE,
            "name": "merge-queue",
            "title": "Merge Queue",
            "description": "Current serialized merge queue entries.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_CONFLICTS,
            "name": "conflicts",
            "title": "Open Conflicts",
            "description": "Structured merge conflict sets known to the workspace.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_OPENAPI,
            "name": "openapi",
            "title": "OpenAPI Contract",
            "description": "The local CrabDB HTTP API OpenAPI 3.1 document.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_USER_GUIDE,
            "name": "user-guide",
            "title": "CrabDB User Guide",
            "description": "End-user guide for common CrabDB workflows.",
            "mimeType": "text/markdown"
        },
        {
            "uri": RESOURCE_AGENT_WORKFLOWS,
            "name": "agent-workflows",
            "title": "CrabDB Agent Workflows",
            "description": "Guide for multi-agent coordinators and MCP hosts.",
            "mimeType": "text/markdown"
        },
        {
            "uri": RESOURCE_CLI_REFERENCE,
            "name": "cli-reference",
            "title": "CrabDB CLI Reference",
            "description": "Command reference for the CrabDB CLI and local API surfaces.",
            "mimeType": "text/markdown"
        }
    ])
}

fn handle_resource_read(db: &mut CrabDb, params: Value) -> Result<Value> {
    let args: ResourceReadArgs = from_arguments(params)?;
    let (mime_type, text) = match args.uri.as_str() {
        RESOURCE_STATUS => ("application/json", pretty_json(&db.status(None)?)?),
        RESOURCE_DOCTOR => ("application/json", pretty_json(&db.doctor()?)?),
        RESOURCE_AGENTS => ("application/json", pretty_json(&db.list_agents()?)?),
        RESOURCE_MERGE_QUEUE => ("application/json", pretty_json(&db.list_merge_queue()?)?),
        RESOURCE_CONFLICTS => ("application/json", pretty_json(&db.list_conflicts()?)?),
        RESOURCE_OPENAPI => (
            "application/json",
            serde_json::to_string_pretty(&crate::server::openapi_spec())?,
        ),
        RESOURCE_USER_GUIDE => ("text/markdown", USER_GUIDE_MD.to_string()),
        RESOURCE_AGENT_WORKFLOWS => ("text/markdown", AGENT_WORKFLOWS_MD.to_string()),
        RESOURCE_CLI_REFERENCE => ("text/markdown", CLI_REFERENCE_MD.to_string()),
        other => templated_resource(db, other)?,
    };
    Ok(json!({
        "contents": [
            {
                "uri": args.uri,
                "mimeType": mime_type,
                "text": text
            }
        ]
    }))
}

fn templated_resource(db: &mut CrabDb, uri: &str) -> Result<(&'static str, String)> {
    if let Some(agent) = template_uri_argument(
        uri,
        "crabdb://workspace/agents/",
        "/status",
        RESOURCE_AGENT_STATUS_TEMPLATE,
    )? {
        return Ok(("application/json", pretty_json(&db.agent_status(&agent)?)?));
    }
    if let Some(agent) = template_uri_argument(
        uri,
        "crabdb://workspace/agents/",
        "/contribution",
        RESOURCE_AGENT_CONTRIBUTION_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.agent_contribution(&agent, 50)?)?,
        ));
    }
    if let Some(agent) = template_uri_argument(
        uri,
        "crabdb://workspace/agents/",
        "/gates",
        RESOURCE_AGENT_GATES_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.agent_gate_history(&agent, None, 50)?)?,
        ));
    }
    if let Some(agent) = template_uri_argument(
        uri,
        "crabdb://workspace/agents/",
        "/readiness",
        RESOURCE_AGENT_READINESS_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.agent_readiness(&agent)?)?,
        ));
    }
    if let Some(agent) = template_uri_argument(
        uri,
        "crabdb://workspace/agents/",
        "/handoff",
        RESOURCE_AGENT_HANDOFF_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.agent_handoff(&agent, 50)?)?,
        ));
    }
    if let Some(agent) = template_uri_argument(
        uri,
        "crabdb://workspace/agents/",
        "/diff",
        RESOURCE_AGENT_DIFF_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.diff_agent_with_options(&agent, false, false)?)?,
        ));
    }
    if let Some(agent) = template_uri_argument(
        uri,
        "crabdb://workspace/agents/",
        "",
        RESOURCE_AGENT_TEMPLATE,
    )? {
        return Ok(("application/json", pretty_json(&db.agent_details(&agent)?)?));
    }
    if let Some(session_id) = template_uri_argument(
        uri,
        "crabdb://workspace/sessions/",
        "",
        RESOURCE_SESSION_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.show_agent_session(&session_id)?)?,
        ));
    }
    if let Some(turn_id) =
        template_uri_argument(uri, "crabdb://workspace/turns/", "", RESOURCE_TURN_TEMPLATE)?
    {
        return Ok((
            "application/json",
            pretty_json(&db.show_agent_turn(&turn_id)?)?,
        ));
    }
    if let Some(conflict_set_id) = template_uri_argument(
        uri,
        "crabdb://workspace/conflicts/",
        "",
        RESOURCE_CONFLICT_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.show_conflict(&conflict_set_id)?)?,
        ));
    }
    if let Some(approval_id) = template_uri_argument(
        uri,
        "crabdb://workspace/approvals/",
        "",
        RESOURCE_APPROVAL_TEMPLATE,
    )? {
        return Ok((
            "application/json",
            pretty_json(&db.show_agent_approval(&approval_id)?)?,
        ));
    }
    if let Some(span_id) =
        template_uri_argument(uri, "crabdb://workspace/spans/", "", RESOURCE_SPAN_TEMPLATE)?
    {
        return Ok((
            "application/json",
            pretty_json(&db.show_agent_trace_span(&span_id)?)?,
        ));
    }
    Err(Error::InvalidInput(format!(
        "MCP resource `{uri}` not found"
    )))
}

fn template_uri_argument(
    uri: &str,
    prefix: &str,
    suffix: &str,
    uri_template: &str,
) -> Result<Option<String>> {
    let Some(remainder) = uri.strip_prefix(prefix) else {
        return Ok(None);
    };
    if !remainder.ends_with(suffix) {
        return Ok(None);
    }
    let raw = &remainder[..remainder.len() - suffix.len()];
    if raw.is_empty() || raw.contains('/') {
        return Err(Error::InvalidInput(format!(
            "MCP resource `{uri}` does not match template `{uri_template}`"
        )));
    }
    let decoded = decode_uri_segment(raw)?;
    if decoded.trim().is_empty() || decoded.contains('/') {
        return Err(Error::InvalidInput(format!(
            "MCP resource `{uri}` does not match template `{uri_template}`"
        )));
    }
    Ok(Some(decoded))
}

fn decode_uri_segment(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(Error::InvalidInput(format!(
                    "invalid percent-encoding in URI segment `{value}`"
                )));
            }
            let high = hex_value(bytes[index + 1]).ok_or_else(|| {
                Error::InvalidInput(format!("invalid percent-encoding in URI segment `{value}`"))
            })?;
            let low = hex_value(bytes[index + 2]).ok_or_else(|| {
                Error::InvalidInput(format!("invalid percent-encoding in URI segment `{value}`"))
            })?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded)
        .map_err(|_| Error::InvalidInput(format!("URI segment `{value}` is not valid UTF-8")))
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn handle_prompt_get(params: Value) -> Result<Value> {
    let args: PromptGetArgs = from_arguments(params)?;
    match args.name.as_str() {
        PROMPT_AGENT_TASK => {
            let agent = prompt_arg(&args.arguments, "agent")?;
            let task = prompt_arg(&args.arguments, "task")?;
            let branch = prompt_arg_optional(&args.arguments, "branch")?
                .unwrap_or_else(|| "main".to_string());
            prompt_result(
                "Safe CrabDB agent task workflow",
                format!(
                    "Run this CrabDB task using the MCP tools and resources.\n\n\
Agent: `{agent}`\nBase branch: `{branch}`\nTask:\n{task}\n\n\
Workflow:\n\
1. Read `crabdb://workspace/status` and `crabdb://docs/agent-workflows` before mutating anything.\n\
2. Use `crabdb.agent_spawn` for `{agent}` from `{branch}` if it does not already exist.\n\
3. Start a turn with `crabdb.begin_turn`, then attach the user request with `crabdb.add_message`.\n\
4. Claim busy paths with `crabdb.agent_claim` when multiple agents may edit the same files, then prefer structured patches through `crabdb.apply_patch`; preflight risky shell, network, deploy, destructive, or ignored-path work with `crabdb.guardrail_check`.\n\
5. Record trace spans/events for tool calls, guardrails, and handoffs.\n\
6. Request human approval with `crabdb.approval_request` when `crabdb.guardrail_check` returns `approval_required`.\n\
7. Run `crabdb.run_test` and, when model/policy quality matters, `crabdb.run_eval`.\n\
8. End the turn with `crabdb.end_turn`, inspect `crabdb.agent_status`, `crabdb.agent_handoff`, and `crabdb.diff_agent`, then queue or merge only after review.\n\
9. If merge conflicts appear, use `crabdb.conflict_show` and `crabdb.conflict_resolve`; do not overwrite target changes silently."
                ),
                Some((RESOURCE_AGENT_WORKFLOWS, "text/markdown", AGENT_WORKFLOWS_MD)),
            )
        }
        PROMPT_REVIEW_AGENT => {
            let agent = prompt_arg(&args.arguments, "agent")?;
            prompt_result(
                "CrabDB agent review checklist",
                format!(
                    "Review CrabDB agent `{agent}` before accepting its work.\n\n\
Checklist:\n\
1. Read `crabdb://workspace/doctor`, `crabdb://workspace/agents`, and `crabdb://workspace/conflicts`.\n\
2. Call `crabdb.agent_contribution` for `{agent}` and inspect changed paths, operations, sessions, events, approvals, and latest gates.\n\
3. Call `crabdb.agent_handoff` for `{agent}` and use its current session context, trace spans, events, and next steps as the transfer packet.\n\
4. Call `crabdb.agent_readiness` for `{agent}` and treat blockers as stop conditions before merge.\n\
5. Call `crabdb.agent_status` for `{agent}` and confirm the branch/workdir state is clean enough to review.\n\
6. Call `crabdb.diff_agent` with patches and line ids; inspect provenance with `crabdb.why`, `crabdb.history`, and `crabdb.code_from` when a change is unclear.\n\
7. Confirm latest tests and evals passed or explain why warnings are acceptable.\n\
8. Use `crabdb.approval_request` for any unresolved human decision.\n\
9. Prefer `crabdb.merge_queue_add` plus `crabdb.merge_queue_run` for shared target branches; use direct `merge-agent` only for one-off merges.\n\
10. If conflicts exist, stop review and switch to the `{PROMPT_RESOLVE_CONFLICT}` prompt."
                ),
                Some((RESOURCE_CLI_REFERENCE, "text/markdown", CLI_REFERENCE_MD)),
            )
        }
        PROMPT_RESOLVE_CONFLICT => {
            let conflict_set_id = prompt_arg(&args.arguments, "conflict_set_id")?;
            prompt_result(
                "CrabDB conflict resolution workflow",
                format!(
                    "Resolve CrabDB conflict `{conflict_set_id}` safely.\n\n\
Workflow:\n\
1. Call `crabdb.conflict_show` with `conflict_set_id = {conflict_set_id}` and inspect every path in the conflict set.\n\
2. Read `crabdb://workspace/conflicts` and confirm this conflict is still open.\n\
3. Decide per conflicted path whether source, target, or manual content should win. Keep non-conflicting source changes merged.\n\
4. Use `crabdb.conflict_resolve` with either `take: source`, `take: target`, or `manual.files` covering every conflicted path and no unrelated paths.\n\
5. If manual content is used, preserve intended executable mode or set `delete: true` for intended deletions.\n\
6. After resolution, run status/diff plus the relevant `crabdb.run_test` or `crabdb.run_eval` gates before considering the merge complete.\n\
7. If CrabDB reports a stale branch, stop and re-run the merge from the current refs rather than forcing stale content."
                ),
                None,
            )
        }
        other => Err(Error::InvalidInput(format!(
            "MCP prompt `{other}` not found"
        ))),
    }
}

fn prompt_arg(arguments: &Value, name: &str) -> Result<String> {
    let object = arguments.as_object().ok_or_else(|| {
        Error::InvalidInput("prompts/get `arguments` must be an object".to_string())
    })?;
    let Some(value) = object.get(name) else {
        return Err(Error::InvalidInput(format!(
            "prompt requires argument `{name}`"
        )));
    };
    let Some(value) = value.as_str() else {
        return Err(Error::InvalidInput(format!(
            "prompt argument `{name}` must be a string"
        )));
    };
    if value.trim().is_empty() {
        return Err(Error::InvalidInput(format!(
            "prompt argument `{name}` must not be empty"
        )));
    }
    Ok(value.to_string())
}

fn prompt_arg_optional(arguments: &Value, name: &str) -> Result<Option<String>> {
    let object = arguments.as_object().ok_or_else(|| {
        Error::InvalidInput("prompts/get `arguments` must be an object".to_string())
    })?;
    let Some(value) = object.get(name) else {
        return Ok(None);
    };
    let Some(value) = value.as_str() else {
        return Err(Error::InvalidInput(format!(
            "prompt argument `{name}` must be a string"
        )));
    };
    if value.trim().is_empty() {
        return Err(Error::InvalidInput(format!(
            "prompt argument `{name}` must not be empty"
        )));
    }
    Ok(Some(value.to_string()))
}

fn prompt_result(
    description: &str,
    text: String,
    embedded_resource: Option<(&str, &str, &str)>,
) -> Result<Value> {
    let mut messages = vec![prompt_text_message(text)];
    if let Some((uri, mime_type, text)) = embedded_resource {
        messages.push(json!({
            "role": "user",
            "content": {
                "type": "resource",
                "resource": {
                    "uri": uri,
                    "mimeType": mime_type,
                    "text": text
                }
            }
        }));
    }
    Ok(json!({
        "description": description,
        "messages": messages
    }))
}

fn prompt_text_message(text: String) -> Value {
    json!({
        "role": "user",
        "content": {
            "type": "text",
            "text": text
        }
    })
}

fn handle_completion_complete(db: &mut CrabDb, params: Value) -> Result<Value> {
    let args: CompletionArgs = from_arguments(params)?;
    let candidates = match args.reference.reference_type.as_str() {
        "ref/prompt" => {
            let name = args.reference.name.as_deref().ok_or_else(|| {
                Error::InvalidInput("completion ref/prompt requires `name`".to_string())
            })?;
            prompt_completion_candidates(db, name, &args.argument.name)?
        }
        "ref/resource" => {
            let uri = args.reference.uri.as_deref().ok_or_else(|| {
                Error::InvalidInput("completion ref/resource requires `uri`".to_string())
            })?;
            resource_completion_candidates(db, uri, &args.argument.name)?
        }
        other => {
            return Err(Error::InvalidInput(format!(
                "unsupported MCP completion reference type `{other}`"
            )));
        }
    };
    Ok(completion_result(candidates, &args.argument.value))
}

fn prompt_completion_candidates(
    db: &CrabDb,
    prompt_name: &str,
    argument_name: &str,
) -> Result<Vec<String>> {
    match (prompt_name, argument_name) {
        (PROMPT_AGENT_TASK | PROMPT_REVIEW_AGENT, "agent") => agent_completion_candidates(db),
        (PROMPT_AGENT_TASK, "branch") => branch_completion_candidates(db),
        (PROMPT_RESOLVE_CONFLICT, "conflict_set_id") => conflict_completion_candidates(db),
        (PROMPT_AGENT_TASK, "task") => Ok(Vec::new()),
        (PROMPT_AGENT_TASK | PROMPT_REVIEW_AGENT | PROMPT_RESOLVE_CONFLICT, _) => Ok(Vec::new()),
        (other, _) => Err(Error::InvalidInput(format!(
            "MCP prompt `{other}` not found"
        ))),
    }
}

fn resource_completion_candidates(
    db: &CrabDb,
    uri_template: &str,
    argument_name: &str,
) -> Result<Vec<String>> {
    match (uri_template, argument_name) {
        (
            RESOURCE_AGENT_TEMPLATE
            | RESOURCE_AGENT_STATUS_TEMPLATE
            | RESOURCE_AGENT_CONTRIBUTION_TEMPLATE
            | RESOURCE_AGENT_GATES_TEMPLATE
            | RESOURCE_AGENT_READINESS_TEMPLATE
            | RESOURCE_AGENT_HANDOFF_TEMPLATE
            | RESOURCE_AGENT_DIFF_TEMPLATE,
            "agent",
        ) => agent_completion_candidates(db),
        (RESOURCE_SESSION_TEMPLATE, "session_id") => session_completion_candidates(db),
        (RESOURCE_TURN_TEMPLATE, "turn_id") => turn_completion_candidates(db),
        (RESOURCE_CONFLICT_TEMPLATE, "conflict_set_id") => conflict_completion_candidates(db),
        (RESOURCE_APPROVAL_TEMPLATE, "approval_id") => approval_completion_candidates(db),
        (RESOURCE_SPAN_TEMPLATE, "span_id") => span_completion_candidates(db),
        (
            RESOURCE_AGENT_TEMPLATE
            | RESOURCE_AGENT_STATUS_TEMPLATE
            | RESOURCE_AGENT_CONTRIBUTION_TEMPLATE
            | RESOURCE_AGENT_GATES_TEMPLATE
            | RESOURCE_AGENT_READINESS_TEMPLATE
            | RESOURCE_AGENT_HANDOFF_TEMPLATE
            | RESOURCE_AGENT_DIFF_TEMPLATE
            | RESOURCE_SESSION_TEMPLATE
            | RESOURCE_TURN_TEMPLATE
            | RESOURCE_CONFLICT_TEMPLATE
            | RESOURCE_APPROVAL_TEMPLATE
            | RESOURCE_SPAN_TEMPLATE,
            _,
        ) => Ok(Vec::new()),
        (other, _) => Err(Error::InvalidInput(format!(
            "MCP resource template `{other}` not found"
        ))),
    }
}

fn agent_completion_candidates(db: &CrabDb) -> Result<Vec<String>> {
    let mut values = BTreeSet::new();
    for agent in db.list_agents()? {
        values.insert(agent.record.name);
        values.insert(agent.record.agent_id);
    }
    Ok(values.into_iter().collect())
}

fn branch_completion_candidates(db: &CrabDb) -> Result<Vec<String>> {
    Ok(db
        .list_branches()?
        .into_iter()
        .map(|branch| branch.name)
        .collect())
}

fn session_completion_candidates(db: &CrabDb) -> Result<Vec<String>> {
    Ok(db
        .list_agent_sessions(None)?
        .into_iter()
        .map(|session| session.session_id)
        .collect())
}

fn turn_completion_candidates(db: &CrabDb) -> Result<Vec<String>> {
    let mut values = BTreeSet::new();
    for session in db.list_agent_sessions(None)? {
        for turn in db.show_agent_session(&session.session_id)?.turns {
            values.insert(turn.turn_id);
        }
    }
    for event in db.list_agent_events(None, None, None, None, 1000)? {
        if let Some(turn_id) = event.turn_id {
            values.insert(turn_id);
        }
    }
    Ok(values.into_iter().collect())
}

fn conflict_completion_candidates(db: &CrabDb) -> Result<Vec<String>> {
    Ok(db
        .list_conflicts()?
        .into_iter()
        .map(|conflict| conflict.conflict_set_id)
        .collect())
}

fn approval_completion_candidates(db: &CrabDb) -> Result<Vec<String>> {
    Ok(db
        .list_agent_approvals(None, None)?
        .into_iter()
        .map(|approval| approval.approval_id)
        .collect())
}

fn span_completion_candidates(db: &CrabDb) -> Result<Vec<String>> {
    Ok(db
        .list_agent_trace_spans(None, None, None, None, 1000)?
        .into_iter()
        .map(|span| span.span_id)
        .collect())
}

fn completion_result(candidates: Vec<String>, value: &str) -> Value {
    let needle = value.to_ascii_lowercase();
    let mut starts_with = Vec::new();
    let mut contains = Vec::new();
    let mut seen = BTreeSet::new();
    for candidate in candidates {
        if !seen.insert(candidate.clone()) {
            continue;
        }
        let candidate_lower = candidate.to_ascii_lowercase();
        if needle.is_empty() || candidate_lower.starts_with(&needle) {
            starts_with.push(candidate);
        } else if candidate_lower.contains(&needle) {
            contains.push(candidate);
        }
    }
    starts_with.sort();
    contains.sort();
    starts_with.extend(contains);
    let total = starts_with.len();
    let values = starts_with.into_iter().take(100).collect::<Vec<_>>();
    json!({
        "completion": {
            "values": values,
            "total": total,
            "hasMore": total > 100
        }
    })
}

fn tools() -> Value {
    let mut tools = json!([
        {
            "name": "crabdb.doctor",
            "title": "CrabDB Doctor",
            "description": "Run read-only operational diagnostics for workspace health, locks, fsck, approvals, leases, merge queue, conflicts, and agent workdirs.",
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
            "description": "Read recent operations, optionally scoped to one branch, session, or agent.",
            "inputSchema": object_schema(json!({
                "branch": { "type": "string" },
                "session": { "type": "string" },
                "agent": { "type": "string" },
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
            "description": "Find operations and changed paths produced by a change, message, session, or agent branch.",
            "inputSchema": object_schema(json!({
                "selector": { "type": "string" }
            }), vec!["selector"])
        },
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
            "name": "crabdb.session_start",
            "title": "Start Agent Session",
            "description": "Start an explicit durable session and attach it to an agent branch.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "title": { "type": "string" },
                "id": { "type": "string" }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.session_list",
            "title": "List Agent Sessions",
            "description": "List durable agent sessions, optionally scoped to one agent.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" }
            }), vec![])
        },
        {
            "name": "crabdb.session_current",
            "title": "Current Agent Session",
            "description": "Read current agent branch session attachments, optionally for one agent.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" }
            }), vec![])
        },
        {
            "name": "crabdb.session_show",
            "title": "Show Agent Session",
            "description": "Return a session with turns, messages, events, and operations.",
            "inputSchema": object_schema(json!({
                "session_id": { "type": "string" }
            }), vec!["session_id"])
        },
        {
            "name": "crabdb.session_context",
            "title": "Session Context",
            "description": "Return a bounded session context packet with total counts and recent messages, events, turns, and operations.",
            "inputSchema": object_schema(json!({
                "session_id": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 1000 }
            }), vec!["session_id"])
        },
        {
            "name": "crabdb.session_end",
            "title": "End Agent Session",
            "description": "End a durable agent session with completed, failed, cancelled, or archived status.",
            "inputSchema": object_schema(json!({
                "session_id": { "type": "string" },
                "status": { "type": "string", "enum": ["completed", "failed", "cancelled", "archived"] }
            }), vec!["session_id"])
        },
        {
            "name": "crabdb.approval_request",
            "title": "Request Human Approval",
            "description": "Create a durable pending approval for a sensitive agent action.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "action": { "type": "string" },
                "summary": { "type": "string" },
                "payload": { "type": "object" },
                "session_id": { "type": "string" },
                "turn_id": { "type": "string" }
            }), vec!["agent", "action", "summary"])
        },
        {
            "name": "crabdb.approval_list",
            "title": "List Human Approvals",
            "description": "List durable approval gates, optionally scoped by agent and status.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "status": { "type": "string", "enum": ["pending", "approved", "rejected", "cancelled", "all"] }
            }), vec![])
        },
        {
            "name": "crabdb.approval_show",
            "title": "Show Human Approval",
            "description": "Show one durable approval gate by id.",
            "inputSchema": object_schema(json!({
                "approval_id": { "type": "string" }
            }), vec!["approval_id"])
        },
        {
            "name": "crabdb.approval_decide",
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
            "name": "crabdb.lease_acquire",
            "title": "Acquire Path Lease",
            "description": "Acquire an advisory read or write lease for an agent path before editing.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "path": { "type": "string" },
                "mode": { "type": "string", "enum": ["read", "write"] },
                "ttl_secs": { "type": "integer", "minimum": 1 }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.lease_list",
            "title": "List Path Leases",
            "description": "List active advisory leases, or all leases when all is true.",
            "inputSchema": object_schema(json!({
                "all": { "type": "boolean" }
            }), vec![])
        },
        {
            "name": "crabdb.lease_release",
            "title": "Release Path Lease",
            "description": "Release an advisory path lease by lease id.",
            "inputSchema": object_schema(json!({
                "lease_id": { "type": "string" }
            }), vec!["lease_id"])
        },
        {
            "name": "crabdb.anchor_create",
            "title": "Create Line Anchor",
            "description": "Create a durable review anchor for a path:line selector on an optional branch.",
            "inputSchema": object_schema(json!({
                "path_line": { "type": "string" },
                "label": { "type": "string" },
                "branch": { "type": "string" }
            }), vec!["path_line", "label"])
        },
        {
            "name": "crabdb.anchor_list",
            "title": "List Line Anchors",
            "description": "List durable review anchors.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "crabdb.anchor_resolve",
            "title": "Resolve Line Anchor",
            "description": "Resolve a durable review anchor on an optional branch.",
            "inputSchema": object_schema(json!({
                "anchor_id": { "type": "string" },
                "branch": { "type": "string" }
            }), vec!["anchor_id"])
        },
        {
            "name": "crabdb.anchor_delete",
            "title": "Delete Line Anchor",
            "description": "Delete a durable review anchor by id.",
            "inputSchema": object_schema(json!({
                "anchor_id": { "type": "string" }
            }), vec!["anchor_id"])
        },
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
        },
        {
            "name": "crabdb.begin_turn",
            "title": "Begin Agent Turn",
            "description": "Create or reuse an agent branch and start a durable agent turn.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "branch": { "type": "string" },
                "session_title": { "type": "string" },
                "base_change": { "type": "string" }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.add_message",
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
            "name": "crabdb.add_event",
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
            "name": "crabdb.event_list",
            "title": "List Trace Events",
            "description": "List recent agent trace events across agents, sessions, turns, and event types.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "session": { "type": "string" },
                "turn_id": { "type": "string" },
                "event_type": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 1000 }
            }), vec![])
        },
        {
            "name": "crabdb.span_start",
            "title": "Start Trace Span",
            "description": "Start a parentable trace span for an agent, tool call, guardrail, handoff, or evaluation within a turn.",
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
            "name": "crabdb.span_end",
            "title": "End Trace Span",
            "description": "End a trace span with a status and optional result payload.",
            "inputSchema": object_schema(json!({
                "span_id": { "type": "string" },
                "status": { "type": "string" },
                "result": { "type": "object" }
            }), vec!["span_id"])
        },
        {
            "name": "crabdb.span_list",
            "title": "List Trace Spans",
            "description": "List derived trace spans across agents, sessions, turns, and trace ids.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "session": { "type": "string" },
                "turn_id": { "type": "string" },
                "trace_id": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 1000 }
            }), vec![])
        },
        {
            "name": "crabdb.span_summary",
            "title": "Summarize Trace Spans",
            "description": "Summarize derived trace spans with status/type counts, open spans, failed spans, and slowest completed spans.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "session": { "type": "string" },
                "turn_id": { "type": "string" },
                "trace_id": { "type": "string" },
                "slowest": { "type": "integer", "minimum": 1, "maximum": 50 }
            }), vec![])
        },
        {
            "name": "crabdb.span_show",
            "title": "Show Trace Span",
            "description": "Show a single derived trace span.",
            "inputSchema": object_schema(json!({
                "span_id": { "type": "string" }
            }), vec!["span_id"])
        },
        {
            "name": "crabdb.apply_patch",
            "title": "Apply Agent Patch",
            "description": "Apply a native CrabDB patch or design-style files patch to a turn's agent branch.",
            "inputSchema": object_schema(json!({
                "turn_id": { "type": "string" },
                "message": { "type": "string" },
                "base_change": { "type": "string" },
                "session_id": { "type": "string" },
                "allow_ignored": { "type": "boolean" },
                "edits": { "type": "array", "items": { "type": "object" } },
                "files": { "type": "array", "items": { "type": "object" } }
            }), vec!["turn_id"])
        },
        {
            "name": "crabdb.end_turn",
            "title": "End Agent Turn",
            "description": "Close a durable agent turn with completed, failed, cancelled, or archived status.",
            "inputSchema": object_schema(json!({
                "turn_id": { "type": "string" },
                "status": { "type": "string", "enum": ["completed", "failed", "cancelled", "archived"] }
            }), vec!["turn_id"])
        },
        {
            "name": "crabdb.show_turn",
            "title": "Show Agent Turn",
            "description": "Return a turn with its session, messages, trace events, and operations.",
            "inputSchema": object_schema(json!({
                "turn_id": { "type": "string" }
            }), vec!["turn_id"])
        },
        {
            "name": "crabdb.diff_agent",
            "title": "Diff Agent Branch",
            "description": "Show the changes from an agent branch base to its current head.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "patch": { "type": "boolean" },
                "show_line_ids": { "type": "boolean" },
                "show-line-ids": { "type": "boolean" }
            }), vec!["agent"])
        },
        {
            "name": "crabdb.run_test",
            "title": "Run Agent Test",
            "description": "Run a command in an agent workdir and record durable test_started/test_finished events plus stdout/stderr Blob output.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "command": { "type": "array", "items": { "type": "string" } },
                "turn_id": { "type": "string" },
                "timeout_secs": { "type": "integer", "minimum": 1 },
                "suite": { "type": "string" },
                "score": { "type": "number" },
                "threshold": { "type": "number" }
            }), vec!["agent", "command"])
        },
        {
            "name": "crabdb.run_eval",
            "title": "Run Agent Eval",
            "description": "Run an evaluation command in an agent workdir and record durable eval_started/eval_finished events plus stdout/stderr Blob output.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "command": { "type": "array", "items": { "type": "string" } },
                "turn_id": { "type": "string" },
                "timeout_secs": { "type": "integer", "minimum": 1 },
                "suite": { "type": "string" },
                "score": { "type": "number" },
                "threshold": { "type": "number" }
            }), vec!["agent", "command"])
        },
        {
            "name": "crabdb.sync_workdir",
            "title": "Sync Agent Workdir",
            "description": "Refresh an agent materialized workdir from its branch head, refusing dirty edits unless force is true.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
                "force": { "type": "boolean" }
            }), vec!["agent"])
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
            "description": "Preflight an agent action against CrabDB path policy, risky tool categories, and pending approvals. Returns allowed, approval_required, or blocked.",
            "inputSchema": object_schema(json!({
                "agent": { "type": "string" },
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
    ]);
    annotate_tools(&mut tools);
    tools
}

fn annotate_tools(tools: &mut Value) {
    let Some(tools) = tools.as_array_mut() else {
        return;
    };
    for tool in tools {
        let Some(name) = tool.get("name").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        if let Some(object) = tool.as_object_mut() {
            object.insert("annotations".to_string(), tool_annotations(&name));
        }
    }
}

fn tool_annotations(name: &str) -> Value {
    match tool_risk_class(name) {
        ToolRiskClass::ReadOnly => json!({
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        }),
        ToolRiskClass::Write => json!({
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": false,
            "openWorldHint": false
        }),
        ToolRiskClass::IdempotentWrite => json!({
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        }),
        ToolRiskClass::DestructiveWrite => json!({
            "readOnlyHint": false,
            "destructiveHint": true,
            "idempotentHint": false,
            "openWorldHint": false
        }),
        ToolRiskClass::OpenWorldWrite => json!({
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": false,
            "openWorldHint": true
        }),
    }
}

#[derive(Clone, Copy)]
enum ToolRiskClass {
    ReadOnly,
    Write,
    IdempotentWrite,
    DestructiveWrite,
    OpenWorldWrite,
}

fn tool_risk_class(name: &str) -> ToolRiskClass {
    match name {
        "crabdb.doctor"
        | "crabdb.status"
        | "crabdb.diff"
        | "crabdb.timeline"
        | "crabdb.why"
        | "crabdb.history"
        | "crabdb.code_from"
        | "crabdb.agent_list"
        | "crabdb.agent_show"
        | "crabdb.agent_status"
        | "crabdb.agent_contribution"
        | "crabdb.gate_history"
        | "crabdb.agent_readiness"
        | "crabdb.agent_handoff"
        | "crabdb.config_list"
        | "crabdb.config_get"
        | "crabdb.session_list"
        | "crabdb.session_current"
        | "crabdb.session_show"
        | "crabdb.session_context"
        | "crabdb.approval_list"
        | "crabdb.approval_show"
        | "crabdb.lease_list"
        | "crabdb.anchor_list"
        | "crabdb.anchor_resolve"
        | "crabdb.merge_queue_list"
        | "crabdb.conflict_list"
        | "crabdb.conflict_show"
        | "crabdb.event_list"
        | "crabdb.span_list"
        | "crabdb.span_summary"
        | "crabdb.span_show"
        | "crabdb.show_turn"
        | "crabdb.diff_agent"
        | "crabdb.ignore_list"
        | "crabdb.ignore_check"
        | "crabdb.guardrail_check" => ToolRiskClass::ReadOnly,
        "crabdb.config_set" | "crabdb.ignore_add" | "crabdb.ignore_remove" => {
            ToolRiskClass::IdempotentWrite
        }
        "crabdb.agent_remove"
        | "crabdb.anchor_delete"
        | "crabdb.merge_queue_remove"
        | "crabdb.conflict_resolve"
        | "crabdb.apply_patch"
        | "crabdb.sync_workdir" => ToolRiskClass::DestructiveWrite,
        "crabdb.run_test" | "crabdb.run_eval" => ToolRiskClass::OpenWorldWrite,
        _ => ToolRiskClass::Write,
    }
}

fn object_schema(properties: Value, required: Vec<&str>) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

fn tool_result<T: serde::Serialize>(value: T) -> Result<Value> {
    let structured = serde_json::to_value(value)?;
    Ok(json!({
        "resultType": "complete",
        "content": [
            {
                "type": "text",
                "text": serde_json::to_string_pretty(&structured)?
            }
        ],
        "structuredContent": structured,
        "isError": false
    }))
}

fn tool_error_result(err: &Error) -> Value {
    let structured = json!({
        "message": err.to_string(),
        "code": err.exit_code()
    });
    json!({
        "resultType": "complete",
        "content": [
            {
                "type": "text",
                "text": err.to_string()
            }
        ],
        "structuredContent": structured,
        "isError": true
    })
}

fn pretty_json<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_string_pretty(value).map_err(Error::from)
}

fn resource_error_response(id: Value, err: &Error) -> Value {
    let code = match err {
        Error::InvalidInput(_) => -32002,
        _ => -32603,
    };
    json_rpc_error(id, code, &err.to_string())
}

fn prompt_error_response(id: Value, err: &Error) -> Value {
    let code = match err {
        Error::InvalidInput(_) | Error::Json(_) => -32602,
        _ => -32603,
    };
    json_rpc_error(id, code, &err.to_string())
}

fn completion_error_response(id: Value, err: &Error) -> Value {
    let code = match err {
        Error::InvalidInput(_) | Error::Json(_) => -32602,
        _ => -32603,
    };
    json_rpc_error(id, code, &err.to_string())
}

fn json_rpc_result(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

fn json_rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
}
