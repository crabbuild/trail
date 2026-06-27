mod dispatch;
mod lane;
mod system;
mod utils;

pub(crate) use utils::error_response;

use super::transport::{HttpRequest, HttpResponse, ServerAuth};
use crate::CrabDb;

pub(crate) fn route_request(
    db: &mut CrabDb,
    request: HttpRequest,
    auth: &ServerAuth,
) -> HttpResponse {
    match dispatch::route_request_result(db, request, auth) {
        Ok(response) => response,
        Err(err) => error_response(&err),
    }
}
