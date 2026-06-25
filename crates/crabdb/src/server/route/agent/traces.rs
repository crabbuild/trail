use crate::server::request_types::{default_completed_status, EndSpanRequest};
use crate::server::route::utils::{json_response, query_usize, query_value};
use crate::server::transport::{HttpRequest, HttpResponse};
use crate::{CrabDb, Result};

pub(super) fn handle_trace_routes(
    db: &mut CrabDb,
    request: &HttpRequest,
    path: &str,
    query: &str,
    parts: &[&str],
) -> Result<Option<HttpResponse>> {
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

    Ok(None)
}
