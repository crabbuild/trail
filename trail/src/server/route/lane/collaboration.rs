use crate::server::request_types::{
    default_lease_mode, AnchorCreateRequest, ConflictResolveRequest, LaneMergeQueueAddRequest,
    LaneMergeQueueRunRequest, LaneMergeRequest, LeaseAcquireRequest,
};
use crate::server::route::utils::{
    json_response, query_flag, query_usize, query_value, reject_unexpected_body,
    resolve_conflict_request, validate_merge_strategy,
};
use crate::server::transport::{HttpRequest, HttpResponse};
use crate::{Result, Trail};

pub(super) fn handle_collaboration_routes(
    db: &mut Trail,
    request: &HttpRequest,
    path: &str,
    query: &str,
    parts: &[&str],
) -> Result<Option<HttpResponse>> {
    if request.method == "GET" && path == "/v1/leases" {
        let leases = db.list_leases(query_flag(query, "all"))?;
        return Ok(Some(json_response(200, "OK", &leases)?));
    }

    if request.method == "POST" && path == "/v1/leases" {
        let body: LeaseAcquireRequest = serde_json::from_slice(&request.body)?;
        let mode = body.mode.unwrap_or_else(default_lease_mode);
        let report = db.acquire_lease(
            &body.lane,
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

    if request.method == "GET" && path == "/v1/lanes/merges/queue" {
        let entries = db.list_lane_merge_queue()?;
        return Ok(Some(json_response(200, "OK", &entries)?));
    }

    if request.method == "POST" && path == "/v1/lanes/merges/queue" {
        let body: LaneMergeQueueAddRequest = serde_json::from_slice(&request.body)?;
        let report = db.enqueue_lane_merge(&body.lane, &body.into, body.priority)?;
        return Ok(Some(json_response(201, "Created", &report)?));
    }

    if request.method == "POST" && path == "/v1/lanes/merges/queue/run" {
        let body: LaneMergeQueueRunRequest = if request.body.is_empty() {
            LaneMergeQueueRunRequest { limit: None }
        } else {
            serde_json::from_slice(&request.body)?
        };
        let report = db.run_lane_merge_queue(body.limit)?;
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if request.method == "GET" && path == "/v1/conflicts" {
        let conflicts = db.list_conflicts()?;
        return Ok(Some(json_response(200, "OK", &conflicts)?));
    }

    if parts.len() == 3 && parts[0] == "v1" && parts[1] == "leases" && request.method == "DELETE" {
        reject_unexpected_body(request, "DELETE /v1/leases/{lease_id}")?;
        let report = db.release_lease(parts[2])?;
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if parts.len() == 3 && parts[0] == "v1" && parts[1] == "anchors" {
        if request.method == "GET" {
            let report = db.resolve_anchor(parts[2], query_value(query, "branch"))?;
            return Ok(Some(json_response(200, "OK", &report)?));
        }
        if request.method == "DELETE" {
            reject_unexpected_body(request, "DELETE /v1/anchors/{anchor_id}")?;
            let report = db.delete_anchor(parts[2])?;
            return Ok(Some(json_response(200, "OK", &report)?));
        }
    }

    if parts.len() == 5
        && parts[0] == "v1"
        && parts[1] == "lanes"
        && parts[2] == "merges"
        && parts[3] == "queue"
        && request.method == "DELETE"
    {
        reject_unexpected_body(request, "DELETE /v1/lanes/merges/queue/{selector}")?;
        let report = db.remove_lane_merge_queue(parts[4])?;
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if parts.len() == 6
        && parts[0] == "v1"
        && parts[1] == "lanes"
        && parts[2] == "merges"
        && parts[3] == "queue"
        && parts[5] == "explain"
        && request.method == "GET"
    {
        let report = db.explain_lane_merge_queue(parts[4])?;
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if parts.len() == 3 && parts[0] == "v1" && parts[1] == "conflicts" && request.method == "GET" {
        let conflict = db.show_conflict_with_limit(parts[2], query_usize(query, "limit", 50)?)?;
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

    if parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "lanes"
        && parts[3] == "merge"
        && request.method == "POST"
    {
        let body: LaneMergeRequest = serde_json::from_slice(&request.body)?;
        validate_merge_strategy(body.strategy.as_deref())?;
        let lane = db.resolve_lane_handle(parts[2])?;
        let report =
            db.merge_lane_user_with_options(&lane, &body.into, body.dry_run, body.direct)?;
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    Ok(None)
}
