use crate::server::request_types::{AgentApplyRequest, AgentMarkReviewedRequest};
use crate::server::route::utils::json_response;
use crate::server::transport::{HttpRequest, HttpResponse};
use crate::{Result, Trail};

pub(super) fn handle_agent_route(
    db: &mut Trail,
    request: &HttpRequest,
    _path: &str,
    parts: &[&str],
) -> Result<Option<HttpResponse>> {
    if request.method != "POST" || parts.len() != 4 || parts[0] != "v1" || parts[1] != "agents" {
        return Ok(None);
    }

    match parts[3] {
        "reviewed" => {
            let body: AgentMarkReviewedRequest = serde_json::from_slice(&request.body)?;
            let report = db.agent_mark_reviewed(parts[2], body.note)?;
            Ok(Some(json_response(200, "OK", &report)?))
        }
        "apply" => {
            let body: AgentApplyRequest = serde_json::from_slice(&request.body)?;
            let report = db.agent_apply(parts[2], body.dry_run, body.message)?;
            Ok(Some(json_response(200, "OK", &report)?))
        }
        _ => Ok(None),
    }
}
