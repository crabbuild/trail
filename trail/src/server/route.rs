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
    let metrics_requested = auth.daemon_identity.is_some()
        && request
            .headers
            .get("x-trail-operation-metrics")
            .is_some_and(|value| value == "1")
        && utils::authorized(&request, auth);
    let metrics_generation = metrics_requested
        .then(|| db.operation_metrics_generation())
        .flatten();
    let mut response = route_request_inner(db, request, auth);
    if let Some(generation) = metrics_generation {
        if let Some(report) = db.operation_metrics_json_after(generation) {
            response
                .extra_headers
                .push(("X-Trail-Operation-Metrics", report));
        }
    }
    response
}

fn route_request_inner(db: &mut Trail, request: HttpRequest, auth: &ServerAuth) -> HttpResponse {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InitImportMode;
    use std::collections::BTreeMap;

    #[test]
    fn authenticated_daemon_route_returns_only_its_new_operation_report() {
        let workspace = tempfile::tempdir().unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::Empty, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let auth = ServerAuth::bearer("route-metrics-token")
            .unwrap()
            .with_daemon_identity(super::super::transport::DaemonServerIdentity::new(
                "owner",
                "workspace",
                "executable",
                "process",
            ));
        let request = HttpRequest {
            method: "GET".into(),
            path: "/v1/status".into(),
            headers: BTreeMap::from([
                ("host".into(), "localhost".into()),
                ("authorization".into(), "Bearer route-metrics-token".into()),
                ("x-trail-operation-metrics".into(), "1".into()),
            ]),
            body: Vec::new(),
        };
        let response = route_request(&mut db, request, &auth);
        assert_eq!(response.status, 200);
        let reports = response
            .extra_headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("x-trail-operation-metrics"))
            .collect::<Vec<_>>();
        assert_eq!(reports.len(), 1);
        let report: serde_json::Value = serde_json::from_str(&reports[0].1).unwrap();
        assert_eq!(report["generation"], 1);
        assert_eq!(report["operation"], "status");
        assert_eq!(report["outcome"], "success");
    }
}
