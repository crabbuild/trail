use serde_json::{json, Value};

use crate::db::ExternalMutationAuditInput;
use crate::ids::ChangeId;
use crate::server::transport::{HttpRequest, HttpResponse};
use crate::Trail;

pub(super) struct HttpMutationAudit {
    actor: String,
    command: String,
    argument_lane: Option<String>,
    argument_turn_id: Option<String>,
    argument_target_ref: Option<String>,
}

impl HttpMutationAudit {
    pub(super) fn from_request(request: &HttpRequest) -> Option<Self> {
        if !matches!(request.method.as_str(), "POST" | "PUT" | "PATCH" | "DELETE") {
            return None;
        }
        let path = request_path_without_query(&request.path);
        let parts = path_parts(path);
        let body = request_body_json(request);
        Some(Self {
            actor: http_actor(request),
            command: format!("{} {}", request.method, path),
            argument_lane: http_request_lane(path, &parts, body.as_ref()),
            argument_turn_id: http_request_turn_id(path, body.as_ref()),
            argument_target_ref: http_request_target_ref(path, &parts, body.as_ref()),
        })
    }

    pub(super) fn record(self, db: &mut Trail, response: &HttpResponse) {
        self.record_with_context(db, response, None);
    }

    pub(super) fn record_idempotency_replay(self, db: &mut Trail, response: &HttpResponse) {
        self.record_with_context(
            db,
            response,
            Some(json!({
                "idempotency_replay": true
            })),
        );
    }

    fn record_with_context(self, db: &mut Trail, response: &HttpResponse, context: Option<Value>) {
        let body = serde_json::from_slice::<Value>(&response.body).ok();
        let lane_id = body
            .as_ref()
            .and_then(|value| first_string_for_keys(value, &["lane_id"]))
            .or(self.argument_lane);
        let turn_id = body
            .as_ref()
            .and_then(|value| first_string_for_keys(value, &["turn_id"]))
            .or(self.argument_turn_id);
        let target_ref = body
            .as_ref()
            .and_then(|value| first_string_for_keys(value, &["target_ref", "ref_name"]))
            .or(self.argument_target_ref);
        let change_id = body
            .as_ref()
            .and_then(|value| {
                first_string_for_keys(value, &["operation", "change_id", "result_change"])
            })
            .map(ChangeId);
        let mut summary = http_audit_summary(response.status, body.as_ref());
        merge_summary_context(&mut summary, context);
        let _ = db.record_external_mutation_audit(ExternalMutationAuditInput {
            actor: self.actor,
            surface: "http".to_string(),
            command: self.command,
            target_ref,
            lane_id,
            turn_id,
            status: if response.status < 400 { "ok" } else { "error" }.to_string(),
            status_code: Some(response.status as i64),
            change_id,
            summary: Some(summary),
        });
    }
}

fn http_actor(request: &HttpRequest) -> String {
    if request.headers.contains_key("authorization") {
        return "http:bearer".to_string();
    }
    if request.headers.contains_key("x-trail-token") {
        return "http:x-trail-token".to_string();
    }
    "http:no-auth".to_string()
}

fn http_request_lane(path: &str, parts: &[&str], body: Option<&Value>) -> Option<String> {
    if parts.len() >= 3 && parts[0] == "v1" && parts[1] == "lanes" {
        return Some(parts[2].to_string());
    }
    if parts.len() == 4 && parts[0] == "v1" && parts[1] == "branches" && parts[3] == "merge-lane" {
        return body.and_then(|value| top_level_string_for_keys(value, &["lane_id", "lane"]));
    }
    match path {
        "/v1/lane/turns" | "/v1/sessions" | "/v1/leases" | "/v1/approvals" | "/v1/lane/runs" => {
            body.and_then(|value| top_level_string_for_keys(value, &["lane"]))
        }
        "/v1/merge-queue" => body.and_then(|value| top_level_string_for_keys(value, &["source"])),
        _ => None,
    }
}

fn http_request_turn_id(path: &str, body: Option<&Value>) -> Option<String> {
    if let Some(turn_id) = turn_id_from_path(path) {
        return Some(turn_id.to_string());
    }
    body.and_then(|value| top_level_string_for_keys(value, &["turn_id", "turn"]))
}

fn http_request_target_ref(path: &str, parts: &[&str], body: Option<&Value>) -> Option<String> {
    if parts.len() == 4 && parts[0] == "v1" && parts[1] == "branches" && parts[3] == "merge-lane" {
        return Some(http_branch_ref(parts[2]));
    }
    if path == "/v1/merge-queue" {
        return body
            .and_then(|value| {
                top_level_string_for_keys(value, &["target", "target_branch", "into"])
            })
            .map(|target| http_branch_ref(&target));
    }
    None
}

fn turn_id_from_path(path: &str) -> Option<&str> {
    let parts = path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() >= 4 && parts[0] == "v1" && parts[1] == "lane" && parts[2] == "turns" {
        return Some(parts[3]);
    }
    None
}

fn request_path_without_query(path: &str) -> &str {
    path.split_once('?').map_or(path, |(path, _)| path)
}

fn path_parts(path: &str) -> Vec<&str> {
    path.split('/').filter(|part| !part.is_empty()).collect()
}

fn request_body_json(request: &HttpRequest) -> Option<Value> {
    serde_json::from_slice::<Value>(&request.body).ok()
}

fn http_branch_ref(branch: &str) -> String {
    if branch.starts_with("refs/") {
        branch.to_string()
    } else {
        format!("refs/heads/{branch}")
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

fn merge_summary_context(summary: &mut Value, context: Option<Value>) {
    let Some(Value::Object(context)) = context else {
        return;
    };
    let Value::Object(summary) = summary else {
        return;
    };
    for (key, value) in context {
        summary.insert(key, value);
    }
}

fn top_level_string_for_keys(value: &Value, keys: &[&str]) -> Option<String> {
    let Value::Object(map) = value else {
        return None;
    };
    for key in keys {
        if let Some(value) = map.get(*key).and_then(Value::as_str) {
            return Some(value.to_string());
        }
    }
    None
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
