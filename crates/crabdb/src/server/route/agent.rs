use std::path::PathBuf;

use super::utils::{
    json_response, parse_patch_request, query_flag, query_line_ids_flag, query_usize, query_value,
    resolve_conflict_request, validate_merge_strategy,
};
use crate::model::AgentGateOptions;
use crate::server::request_types::{
    default_completed_status, default_lease_mode, AddEventRequest, AddMessageRequest,
    AgentClaimRequest, AgentRunPauseRequest, AgentRunResumeRequest, AgentTestRequest,
    AnchorCreateRequest, ApprovalDecisionRequest, ApprovalRequest, BeginTurnRequest,
    ConflictResolveRequest, EndSpanRequest, EndTurnRequest, LeaseAcquireRequest, MergeAgentRequest,
    MergeQueueAddRequest, MergeQueueRunRequest, SessionEndRequest, SessionStartRequest,
    SpawnAgentRequest, StartSpanRequest, SyncWorkdirRequest,
};
use crate::server::transport::{HttpRequest, HttpResponse};
use crate::{CrabDb, Error, Result};

pub(super) fn handle_agent_route(
    db: &mut CrabDb,
    request: &HttpRequest,
    path: &str,
    query: &str,
    parts: &[&str],
) -> Result<Option<HttpResponse>> {
    if request.method == "GET" && path == "/v1/agents" {
        let agents = db.list_agents()?;
        return Ok(Some(json_response(200, "OK", &agents)?));
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
        return Ok(Some(json_response(201, "Created", &report)?));
    }

    if request.method == "POST" && path == "/v1/agent/turns" {
        let body: BeginTurnRequest = serde_json::from_slice(&request.body)?;
        let report = db.begin_agent_turn(
            &body.agent,
            body.branch.as_deref(),
            body.session_title,
            body.base_change.as_deref(),
        )?;
        return Ok(Some(json_response(201, "Created", &report)?));
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
        return Ok(Some(json_response(200, "OK", &events)?));
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
        return Ok(Some(json_response(200, "OK", &spans)?));
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
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if request.method == "GET" && path == "/v1/agent/runs" {
        let run_states =
            db.list_agent_run_states(query_value(query, "agent"), query_value(query, "status"))?;
        return Ok(Some(json_response(200, "OK", &run_states)?));
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
        return Ok(Some(json_response(201, "Created", &report)?));
    }

    if request.method == "GET" && path == "/v1/sessions/current" {
        let reports = db.current_agent_sessions(query_value(query, "agent"))?;
        return Ok(Some(json_response(200, "OK", &reports)?));
    }

    if request.method == "GET" && path == "/v1/sessions" {
        let sessions = db.list_agent_sessions(query_value(query, "agent"))?;
        return Ok(Some(json_response(200, "OK", &sessions)?));
    }

    if request.method == "POST" && path == "/v1/sessions" {
        let body: SessionStartRequest = serde_json::from_slice(&request.body)?;
        let report = db.start_agent_session(&body.agent, body.title, body.id)?;
        return Ok(Some(json_response(201, "Created", &report)?));
    }

    if request.method == "GET" && path == "/v1/approvals" {
        let approvals =
            db.list_agent_approvals(query_value(query, "agent"), query_value(query, "status"))?;
        return Ok(Some(json_response(200, "OK", &approvals)?));
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
        return Ok(Some(json_response(201, "Created", &report)?));
    }

    if request.method == "GET" && path == "/v1/leases" {
        let leases = db.list_leases(query_flag(query, "all"))?;
        return Ok(Some(json_response(200, "OK", &leases)?));
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
        return Ok(Some(json_response(201, "Created", &report)?));
    }

    if request.method == "GET" && path == "/v1/anchors" {
        let anchors = db.list_anchors()?;
        return Ok(Some(json_response(200, "OK", &anchors)?));
    }

    if request.method == "POST" && path == "/v1/anchors" {
        let body: AnchorCreateRequest = serde_json::from_slice(&request.body)?;
        let report = db.create_anchor(&body.path_line, body.label, body.branch.as_deref())?;
        return Ok(Some(json_response(201, "Created", &report)?));
    }

    if request.method == "GET" && path == "/v1/merge-queue" {
        let entries = db.list_merge_queue()?;
        return Ok(Some(json_response(200, "OK", &entries)?));
    }

    if request.method == "POST" && path == "/v1/merge-queue" {
        let body: MergeQueueAddRequest = serde_json::from_slice(&request.body)?;
        let report = db.enqueue_merge(&body.source, &body.target, body.priority)?;
        return Ok(Some(json_response(201, "Created", &report)?));
    }

    if request.method == "POST" && path == "/v1/merge-queue/run" {
        let body: MergeQueueRunRequest = if request.body.is_empty() {
            MergeQueueRunRequest { limit: None }
        } else {
            serde_json::from_slice(&request.body)?
        };
        let report = db.run_merge_queue(body.limit)?;
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if request.method == "GET" && path == "/v1/conflicts" {
        let conflicts = db.list_conflicts()?;
        return Ok(Some(json_response(200, "OK", &conflicts)?));
    }

    if parts.len() == 3 && parts[0] == "v1" && parts[1] == "sessions" && request.method == "GET" {
        let details = db.show_agent_session(parts[2])?;
        return Ok(Some(json_response(200, "OK", &details)?));
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "sessions"
        && parts[3] == "context"
        && request.method == "GET"
    {
        let report = db.agent_session_context(parts[2], query_usize(query, "limit", 50)?)?;
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if parts.len() == 3 && parts[0] == "v1" && parts[1] == "approvals" && request.method == "GET" {
        let approval = db.show_agent_approval(parts[2])?;
        return Ok(Some(json_response(200, "OK", &approval)?));
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "agent"
        && parts[2] == "runs"
        && request.method == "GET"
    {
        let run_state = db.show_agent_run_state(parts[3])?;
        return Ok(Some(json_response(200, "OK", &run_state)?));
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
        return Ok(Some(json_response(200, "OK", &report)?));
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
        return Ok(Some(json_response(200, "OK", &report)?));
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
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if parts.len() == 3 && parts[0] == "v1" && parts[1] == "leases" && request.method == "DELETE" {
        let report = db.release_lease(parts[2])?;
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if parts.len() == 3 && parts[0] == "v1" && parts[1] == "anchors" {
        if request.method == "GET" {
            let report = db.resolve_anchor(parts[2], query_value(query, "branch"))?;
            return Ok(Some(json_response(200, "OK", &report)?));
        }
        if request.method == "DELETE" {
            let report = db.delete_anchor(parts[2])?;
            return Ok(Some(json_response(200, "OK", &report)?));
        }
    }

    if parts.len() == 3
        && parts[0] == "v1"
        && parts[1] == "merge-queue"
        && request.method == "DELETE"
    {
        let report = db.remove_merge_queue(parts[2])?;
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if parts.len() == 3 && parts[0] == "v1" && parts[1] == "conflicts" && request.method == "GET" {
        let conflict = db.show_conflict(parts[2])?;
        return Ok(Some(json_response(200, "OK", &conflict)?));
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "conflicts"
        && parts[3] == "resolve"
        && request.method == "POST"
    {
        let body: ConflictResolveRequest = serde_json::from_slice(&request.body)?;
        let report = resolve_conflict_request(db, parts[2], body)?;
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if parts.len() == 3 && parts[0] == "v1" && parts[1] == "agents" {
        let agent = db.resolve_agent_handle(parts[2])?;
        if request.method == "GET" {
            let details = db.agent_details(&agent)?;
            return Ok(Some(json_response(200, "OK", &details)?));
        }
        if request.method == "DELETE" {
            let report = db.remove_agent(&agent, query_flag(query, "force"))?;
            return Ok(Some(json_response(200, "OK", &report)?));
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
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if parts.len() == 4 && parts[0] == "v1" && parts[1] == "agents" && request.method == "GET" {
        let agent = db.resolve_agent_handle(parts[2])?;
        return Ok(Some(match parts[3] {
            "status" => {
                let report = db.agent_status(&agent)?;
                json_response(200, "OK", &report)?
            }
            "contribution" => {
                let report = db.agent_contribution(&agent, query_usize(query, "limit", 50)?)?;
                json_response(200, "OK", &report)?
            }
            "gates" => {
                let report = db.agent_gate_history(
                    &agent,
                    query_value(query, "kind"),
                    query_usize(query, "limit", 50)?,
                )?;
                json_response(200, "OK", &report)?
            }
            "readiness" => {
                let report = db.agent_readiness(&agent)?;
                json_response(200, "OK", &report)?
            }
            "handoff" => {
                let report = db.agent_handoff(&agent, query_usize(query, "limit", 50)?)?;
                json_response(200, "OK", &report)?
            }
            "diff" => {
                let diff = db.diff_agent_with_options(
                    &agent,
                    query_flag(query, "patch"),
                    query_line_ids_flag(query),
                )?;
                json_response(200, "OK", &diff)?
            }
            _ => {
                return Err(Error::InvalidInput(format!(
                    "unknown API endpoint `{}`",
                    request.path
                )))
            }
        }));
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
        return Ok(Some(json_response(200, "OK", &report)?));
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
        return Ok(Some(json_response(200, "OK", &report)?));
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
        return Ok(Some(json_response(200, "OK", &report)?));
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
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "agent"
        && parts[2] == "spans"
        && request.method == "GET"
    {
        let span = db.show_agent_trace_span(parts[3])?;
        return Ok(Some(json_response(200, "OK", &span)?));
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
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "agent"
        && parts[2] == "turns"
        && request.method == "GET"
    {
        let details = db.show_agent_turn(parts[3])?;
        return Ok(Some(json_response(200, "OK", &details)?));
    }

    if parts.len() == 5
        && parts[0] == "v1"
        && parts[1] == "agent"
        && parts[2] == "turns"
        && request.method == "POST"
    {
        let turn_id = parts[3];
        return Ok(Some(match parts[4] {
            "messages" => {
                let body: AddMessageRequest = serde_json::from_slice(&request.body)?;
                let text = body.content.or(body.text).ok_or_else(|| {
                    Error::InvalidInput("message body requires `content` or `text`".to_string())
                })?;
                let report = db.add_agent_turn_message(turn_id, &body.role, &text)?;
                json_response(201, "Created", &report)?
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
                json_response(201, "Created", &report)?
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
                json_response(201, "Created", &report)?
            }
            "patches" => {
                let patch = parse_patch_request(&request.body)?;
                let report = db.apply_agent_turn_patch(turn_id, patch)?;
                json_response(200, "OK", &report)?
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
                json_response(200, "OK", &report)?
            }
            _ => {
                return Err(Error::InvalidInput(format!(
                    "unknown API endpoint `{}`",
                    request.path
                )))
            }
        }));
    }

    Ok(None)
}
