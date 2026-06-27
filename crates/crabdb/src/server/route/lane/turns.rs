use crate::server::request_types::{
    default_completed_status, AddEventRequest, AddMessageRequest, BeginTurnRequest, EndTurnRequest,
    StartSpanRequest,
};
use crate::server::route::utils::{json_response, parse_patch_request, query_usize, query_value};
use crate::server::transport::{HttpRequest, HttpResponse};
use crate::{CrabDb, Error, Result};

pub(super) fn handle_turn_routes(
    db: &mut CrabDb,
    request: &HttpRequest,
    path: &str,
    query: &str,
    parts: &[&str],
) -> Result<Option<HttpResponse>> {
    if request.method == "POST" && path == "/v1/lane/turns" {
        let body: BeginTurnRequest = serde_json::from_slice(&request.body)?;
        let report = db.begin_lane_turn(
            &body.lane,
            body.branch.as_deref(),
            body.session_title,
            body.base_change.as_deref(),
        )?;
        return Ok(Some(json_response(201, "Created", &report)?));
    }

    if request.method == "GET" && path == "/v1/lane/events" {
        let limit = query_usize(query, "limit", 50)?;
        let events = db.list_lane_events(
            query_value(query, "lane"),
            query_value(query, "session"),
            query_value(query, "turn_id").or_else(|| query_value(query, "turn")),
            query_value(query, "event_type").or_else(|| query_value(query, "type")),
            limit,
        )?;
        return Ok(Some(json_response(200, "OK", &events)?));
    }

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "lane"
        && parts[2] == "turns"
        && request.method == "GET"
    {
        let details = db.show_lane_turn(parts[3])?;
        return Ok(Some(json_response(200, "OK", &details)?));
    }

    if parts.len() == 5
        && parts[0] == "v1"
        && parts[1] == "lane"
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
                let report = db.add_lane_turn_message(turn_id, &body.role, &text)?;
                json_response(201, "Created", &report)?
            }
            "events" => {
                let body: AddEventRequest = serde_json::from_slice(&request.body)?;
                let report = db.add_lane_turn_event(
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
                let report = db.start_lane_trace_span(
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
                let report = db.apply_lane_turn_patch(turn_id, patch)?;
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
                let report = db.end_lane_turn(turn_id, &body.status)?;
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
