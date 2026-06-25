use serde_json::{json, Value};

pub(super) fn annotate_tools(tools: &mut Value) {
    let Some(tools) = tools.as_array_mut() else {
        return;
    };
    for tool in tools {
        let Some(name) = tool.get("name").and_then(Value::as_str).map(str::to_string) else {
            continue;
        };
        if let Some(object) = tool.as_object_mut() {
            object.insert("annotations".to_string(), tool_annotations(&name));
        }
    }
}

fn tool_annotations(name: &str) -> Value {
    match tool_risk_class(name) {
        ToolRiskClass::ReadOnly => json!({
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        }),
        ToolRiskClass::Write => json!({
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": false,
            "openWorldHint": false
        }),
        ToolRiskClass::IdempotentWrite => json!({
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        }),
        ToolRiskClass::DestructiveWrite => json!({
            "readOnlyHint": false,
            "destructiveHint": true,
            "idempotentHint": false,
            "openWorldHint": false
        }),
        ToolRiskClass::OpenWorldWrite => json!({
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": false,
            "openWorldHint": true
        }),
    }
}

#[derive(Clone, Copy)]
enum ToolRiskClass {
    ReadOnly,
    Write,
    IdempotentWrite,
    DestructiveWrite,
    OpenWorldWrite,
}

fn tool_risk_class(name: &str) -> ToolRiskClass {
    match name {
        "crabdb.doctor"
        | "crabdb.status"
        | "crabdb.diff"
        | "crabdb.timeline"
        | "crabdb.why"
        | "crabdb.history"
        | "crabdb.code_from"
        | "crabdb.agent_list"
        | "crabdb.agent_show"
        | "crabdb.agent_status"
        | "crabdb.agent_contribution"
        | "crabdb.gate_history"
        | "crabdb.agent_readiness"
        | "crabdb.agent_handoff"
        | "crabdb.config_list"
        | "crabdb.config_get"
        | "crabdb.session_list"
        | "crabdb.session_current"
        | "crabdb.session_show"
        | "crabdb.session_context"
        | "crabdb.approval_list"
        | "crabdb.approval_show"
        | "crabdb.run_list"
        | "crabdb.run_show"
        | "crabdb.lease_list"
        | "crabdb.anchor_list"
        | "crabdb.anchor_resolve"
        | "crabdb.merge_queue_list"
        | "crabdb.conflict_list"
        | "crabdb.conflict_show"
        | "crabdb.event_list"
        | "crabdb.span_list"
        | "crabdb.span_summary"
        | "crabdb.span_show"
        | "crabdb.show_turn"
        | "crabdb.diff_agent"
        | "crabdb.ignore_list"
        | "crabdb.ignore_check"
        | "crabdb.guardrail_check" => ToolRiskClass::ReadOnly,
        "crabdb.config_set" | "crabdb.ignore_add" | "crabdb.ignore_remove" => {
            ToolRiskClass::IdempotentWrite
        }
        "crabdb.agent_remove"
        | "crabdb.anchor_delete"
        | "crabdb.merge_queue_remove"
        | "crabdb.conflict_resolve"
        | "crabdb.apply_patch"
        | "crabdb.sync_workdir" => ToolRiskClass::DestructiveWrite,
        "crabdb.run_test" | "crabdb.run_eval" => ToolRiskClass::OpenWorldWrite,
        _ => ToolRiskClass::Write,
    }
}
