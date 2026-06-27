use serde_json::{json, Value};

use crate::db::ExternalMutationAuditInput;
use crate::ids::ChangeId;
use crate::CrabDb;

use super::tools::tool_is_read_only;

pub(crate) struct McpMutationAudit {
    command: String,
    argument_lane: Option<String>,
    argument_target_ref: Option<String>,
}

impl McpMutationAudit {
    pub(crate) fn from_tool_call_params(params: &Value) -> Option<Self> {
        let name = params.get("name").and_then(Value::as_str)?;
        if tool_is_read_only(name) {
            return None;
        }
        let arguments = params.get("arguments").unwrap_or(&Value::Null);
        Some(Self {
            command: name.to_string(),
            argument_lane: first_string_for_keys(arguments, &["lane", "lane_or_id"]),
            argument_target_ref: first_string_for_keys(
                arguments,
                &["target_ref", "target_branch", "target", "into", "branch"],
            ),
        })
    }

    pub(crate) fn record(self, db: &mut CrabDb, tool_result: &Value) {
        let structured = tool_result.get("structuredContent");
        let is_error = tool_result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let lane_id = structured
            .and_then(|value| first_string_for_keys(value, &["lane_id"]))
            .or(self.argument_lane);
        let target_ref = structured
            .and_then(|value| first_string_for_keys(value, &["target_ref", "ref_name"]))
            .or(self.argument_target_ref);
        let change_id = structured
            .and_then(|value| {
                first_string_for_keys(value, &["operation", "change_id", "result_change"])
            })
            .map(ChangeId);
        let summary = mcp_audit_summary(tool_result, structured, is_error);
        let _ = db.record_external_mutation_audit(ExternalMutationAuditInput {
            surface: "mcp".to_string(),
            command: self.command,
            target_ref,
            lane_id,
            status: if is_error { "error" } else { "ok" }.to_string(),
            status_code: None,
            change_id,
            summary: Some(summary),
        });
    }
}

fn mcp_audit_summary(tool_result: &Value, structured: Option<&Value>, is_error: bool) -> Value {
    let mut summary = json!({
        "is_error": is_error,
    });
    if let Some(error) = tool_result
        .pointer("/content/0/text")
        .and_then(Value::as_str)
        .filter(|_| is_error)
    {
        summary["error"] = Value::String(error.to_string());
    }
    if let Some(change_id) = structured.and_then(|value| {
        first_string_for_keys(value, &["operation", "change_id", "result_change"])
    }) {
        summary["change_id"] = Value::String(change_id);
    }
    if let Some(target_ref) =
        structured.and_then(|value| first_string_for_keys(value, &["target_ref", "ref_name"]))
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
