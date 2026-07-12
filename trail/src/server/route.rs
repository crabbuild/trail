mod agent_hooks;
mod audit;
mod dispatch;
mod idempotency;
mod lane;
mod system;
mod utils;

pub(crate) use utils::error_response;

use super::transport::{HttpRequest, HttpResponse, ServerAuth};
use crate::Trail;

pub(crate) fn route_request(
    db: &mut Trail,
    request: HttpRequest,
    auth: &ServerAuth,
) -> HttpResponse {
    let audit = audit::HttpMutationAudit::from_request(&request);
    let idempotency = if utils::host_allowed(&request)
        && utils::origin_allowed(&request)
        && utils::authorized(&request, auth)
    {
        match idempotency::HttpIdempotency::from_request(&request) {
            Ok(idempotency) => idempotency,
            Err(err) => {
                let response = error_response(&err);
                if let Some(audit) = audit {
                    audit.record(db, &response);
                }
                return response;
            }
        }
    } else {
        None
    };
    if let Some(idempotency) = idempotency.as_ref() {
        match idempotency.replay(db) {
            Ok(Some(response)) => {
                if let Some(audit) = audit {
                    audit.record_idempotency_replay(db, &response);
                }
                return response;
            }
            Ok(None) => {}
            Err(err) => {
                let response = error_response(&err);
                if let Some(audit) = audit {
                    audit.record(db, &response);
                }
                return response;
            }
        }
    }
    let response = match dispatch::route_request_result(db, request, auth) {
        Ok(response) => response,
        Err(err) => error_response(&err),
    };
    if let Some(idempotency) = idempotency.as_ref() {
        idempotency.store(db, &response);
    }
    if let Some(audit) = audit {
        audit.record(db, &response);
    }
    response
}
