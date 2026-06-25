use crate::server::request_types::{
    AgentRunPauseRequest, AgentRunResumeRequest, ApprovalDecisionRequest, ApprovalRequest,
};
use crate::server::route::utils::{json_response, query_value};
use crate::server::transport::{HttpRequest, HttpResponse};
use crate::{CrabDb, Result};

pub(super) fn handle_approval_routes(
    db: &mut CrabDb,
    request: &HttpRequest,
    path: &str,
    query: &str,
    parts: &[&str],
) -> Result<Option<HttpResponse>> {
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

    Ok(None)
}
