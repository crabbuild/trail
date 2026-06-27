mod approvals;
mod collaboration;
mod lanes;
mod sessions;
mod traces;
mod turns;

use crate::server::transport::{HttpRequest, HttpResponse};
use crate::{CrabDb, Result};

pub(super) fn handle_lane_route(
    db: &mut CrabDb,
    request: &HttpRequest,
    path: &str,
    query: &str,
    parts: &[&str],
) -> Result<Option<HttpResponse>> {
    if let Some(response) = lanes::handle_lane_resources(db, request, path, query, parts)? {
        return Ok(Some(response));
    }

    if let Some(response) = turns::handle_turn_routes(db, request, path, query, parts)? {
        return Ok(Some(response));
    }

    if let Some(response) = traces::handle_trace_routes(db, request, path, query, parts)? {
        return Ok(Some(response));
    }

    if let Some(response) = approvals::handle_approval_routes(db, request, path, query, parts)? {
        return Ok(Some(response));
    }

    if let Some(response) = sessions::handle_session_routes(db, request, path, query, parts)? {
        return Ok(Some(response));
    }

    if let Some(response) =
        collaboration::handle_collaboration_routes(db, request, path, query, parts)?
    {
        return Ok(Some(response));
    }

    Ok(None)
}
