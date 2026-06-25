use std::path::PathBuf;

use crate::model::AgentGateOptions;
use crate::server::request_types::{
    AgentClaimRequest, AgentReadFileRequest, AgentTestRequest, SpawnAgentRequest,
    SyncWorkdirRequest,
};
use crate::server::route::utils::{
    json_response, parse_patch_request, query_flag, query_line_ids_flag, query_usize, query_value,
};
use crate::server::transport::{HttpRequest, HttpResponse};
use crate::{CrabDb, Error, Result};

pub(super) fn handle_agent_resources(
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
        let materialize = body.materialize.unwrap_or(
            body.workdir.is_some() || !body.paths.is_empty() || db.default_agent_materialize(),
        );
        let report = db.spawn_agent_with_workdir_paths_and_neighbors(
            &body.name,
            body.from.as_deref(),
            materialize,
            body.provider,
            body.model,
            body.workdir.map(PathBuf::from),
            &body.paths,
            body.include_neighbors,
        )?;
        return Ok(Some(json_response(201, "Created", &report)?));
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
        && parts[3] == "read-file"
        && request.method == "POST"
    {
        let agent = db.resolve_agent_handle(parts[2])?;
        let body: AgentReadFileRequest = serde_json::from_slice(&request.body)?;
        let report = db.read_agent_file_with_hydration(
            &agent,
            &body.path,
            body.hydrate,
            body.force,
            body.include_neighbors,
        )?;
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "agents"
        && parts[3] == "sync-workdir"
        && request.method == "POST"
    {
        let agent = db.resolve_agent_handle(parts[2])?;
        let body: SyncWorkdirRequest = if request.body.is_empty() {
            SyncWorkdirRequest {
                force: false,
                paths: Vec::new(),
                include_neighbors: false,
            }
        } else {
            serde_json::from_slice(&request.body)?
        };
        let report = db.sync_agent_workdir_with_paths_and_neighbors(
            &agent,
            body.force,
            &body.paths,
            body.include_neighbors,
        )?;
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

    Ok(None)
}
