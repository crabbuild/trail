use crate::server::request_types::{
    default_completed_status, SessionEndRequest, SessionStartRequest,
};
use crate::server::route::utils::{json_response, query_usize, query_value};
use crate::server::transport::{HttpRequest, HttpResponse};
use crate::{CrabDb, Result};

pub(super) fn handle_session_routes(
    db: &mut CrabDb,
    request: &HttpRequest,
    path: &str,
    query: &str,
    parts: &[&str],
) -> Result<Option<HttpResponse>> {
    if request.method == "GET" && path == "/v1/sessions/current" {
        let reports = db.current_lane_sessions(query_value(query, "lane"))?;
        return Ok(Some(json_response(200, "OK", &reports)?));
    }

    if request.method == "GET" && path == "/v1/sessions" {
        let sessions = db.list_lane_sessions(query_value(query, "lane"))?;
        return Ok(Some(json_response(200, "OK", &sessions)?));
    }

    if request.method == "POST" && path == "/v1/sessions" {
        let body: SessionStartRequest = serde_json::from_slice(&request.body)?;
        let report = db.start_lane_session(&body.lane, body.title, body.id)?;
        return Ok(Some(json_response(201, "Created", &report)?));
    }

    if parts.len() == 3 && parts[0] == "v1" && parts[1] == "sessions" && request.method == "GET" {
        let details = db.show_lane_session(parts[2])?;
        return Ok(Some(json_response(200, "OK", &details)?));
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "sessions"
        && parts[3] == "context"
        && request.method == "GET"
    {
        let report = db.lane_session_context(parts[2], query_usize(query, "limit", 50)?)?;
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
        let report = db.end_lane_session(parts[2], &body.status)?;
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    Ok(None)
}
