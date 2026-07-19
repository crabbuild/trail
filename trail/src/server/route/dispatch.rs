use crate::server::transport::{HttpRequest, HttpResponse, ServerAuth};
use crate::{Error, Result};

use super::utils;
use super::{agent_hooks, lane, system};

pub(crate) fn route_request_result(
    db: &mut crate::Trail,
    request: HttpRequest,
    auth: &ServerAuth,
) -> Result<HttpResponse> {
    let raw_path = request.path.trim_end_matches('/');
    let (path, query) = raw_path.split_once('?').unwrap_or((raw_path, ""));

    if !utils::host_allowed(&request) {
        return Ok(utils::forbidden_host_response());
    }

    if !utils::origin_allowed(&request) {
        return Ok(utils::forbidden_origin_response());
    }

    if request.method == "GET" && path == "/v1/health" {
        return utils::json_response(
            200,
            "OK",
            &serde_json::json!({
                "ok": true,
                "service": "trail",
                "version": env!("CARGO_PKG_VERSION")
            }),
        );
    }

    if !utils::authorized(&request, auth) {
        return Ok(utils::unauthorized_response());
    }

    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();

    if request.method == "POST"
        && parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "agent-hooks"
        && !auth.is_required()
    {
        return Ok(utils::unauthorized_response());
    }

    if let Some(response) = agent_hooks::handle_agent_hook_route(db, &request, path, query, &parts)?
    {
        return Ok(response);
    }

    if let Some(response) = system::handle_system_route(db, &request, auth, path, query, &parts)? {
        return Ok(response);
    }

    if let Some(response) = lane::handle_lane_route(db, &request, path, query, &parts)? {
        return Ok(response);
    }

    Err(Error::InvalidInput(format!(
        "unknown API endpoint `{}`",
        request.path
    )))
}
