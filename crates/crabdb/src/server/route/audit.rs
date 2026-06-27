use serde_json::{json, Value};

use crate::db::ExternalMutationAuditInput;
use crate::ids::ChangeId;
use crate::server::transport::{HttpRequest, HttpResponse};
use crate::CrabDb;

pub(super) struct HttpMutationAudit {
    command: String,
}

impl HttpMutationAudit {
    pub(super) fn from_request(request: &HttpRequest) -> Option<Self> {
        if !matches!(request.method.as_str(), "POST" | "PUT" | "PATCH" | "DELETE") {
            return None;
        }
        Some(Self {
            command: format!(
                "{} {}",
                request.method,
                request
                    .path
                    .split_once('?')
                    .map_or(request.path.as_str(), |(path, _)| path)
            ),
        })
    }

    pub(super) fn record(self, db: &mut CrabDb, response: &HttpResponse) {
        if matches!(response.status, 401 | 403) {
            return;
        }
        let body = serde_json::from_slice::<Value>(&response.body).ok();
        let lane_id = body
            .as_ref()
            .and_then(|value| first_string_for_keys(value, &["lane_id"]));
        let target_ref = body
            .as_ref()
            .and_then(|value| first_string_for_keys(value, &["target_ref", "ref_name"]));
        let change_id = body
            .as_ref()
            .and_then(|value| {
                first_string_for_keys(value, &["operation", "change_id", "result_change"])
            })
            .map(ChangeId);
        let summary = http_audit_summary(response.status, body.as_ref());
        let _ = db.record_external_mutation_audit(ExternalMutationAuditInput {
            surface: "http".to_string(),
            command: self.command,
            target_ref,
            lane_id,
            status: if response.status < 400 { "ok" } else { "error" }.to_string(),
            status_code: Some(response.status as i64),
            change_id,
            summary: Some(summary),
        });
    }
}

fn http_audit_summary(status: u16, body: Option<&Value>) -> Value {
    let mut summary = json!({
        "status_code": status,
    });
    if let Some(error) = body.and_then(|value| value.pointer("/error/message")) {
        summary["error"] = error.clone();
    }
    if let Some(change_id) = body.and_then(|value| {
        first_string_for_keys(value, &["operation", "change_id", "result_change"])
    }) {
        summary["change_id"] = Value::String(change_id);
    }
    if let Some(target_ref) =
        body.and_then(|value| first_string_for_keys(value, &["target_ref", "ref_name"]))
    {
        summary["target_ref"] = Value::String(target_ref);
    }
    summary
}

fn first_string_for_keys(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(value) = map.get(*key).and_then(Value::as_str) {
                    return Some(value.to_string());
                }
            }
            map.values()
                .find_map(|value| first_string_for_keys(value, keys))
        }
        Value::Array(values) => values
            .iter()
            .find_map(|value| first_string_for_keys(value, keys)),
        _ => None,
    }
}
