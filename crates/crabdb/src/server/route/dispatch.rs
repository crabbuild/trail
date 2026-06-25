use crate::server::transport::{HttpRequest, HttpResponse, ServerAuth};
use crate::{Error, Result};

use super::utils;
use super::{agent, system};

pub(crate) fn route_request_result(
    db: &mut crate::CrabDb,
    request: HttpRequest,
    auth: &ServerAuth,
) -> Result<HttpResponse> {
    let raw_path = request.path.trim_end_matches('/');
    let (path, query) = raw_path.split_once('?').unwrap_or((raw_path, ""));

    if request.method == "GET" && path == "/v1/health" {
        return Ok(utils::json_response(
            200,
            "OK",
            &serde_json::json!({
                "ok": true,
                "service": "crabdb",
                "version": env!("CARGO_PKG_VERSION")
            }),
        )?);
    }

    if !utils::authorized(&request, auth) {
        return Ok(utils::unauthorized_response());
    }

    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();

    if let Some(response) = system::handle_system_route(db, &request, path, query, &parts)? {
        return Ok(response);
    }

    if let Some(response) = agent::handle_agent_route(db, &request, path, query, &parts)? {
        return Ok(response);
    }

    Err(Error::InvalidInput(format!(
        "unknown API endpoint `{}`",
        request.path
    )))
}
