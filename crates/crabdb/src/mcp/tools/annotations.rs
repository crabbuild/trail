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

pub(crate) fn tool_is_read_only(name: &str) -> bool {
    matches!(tool_risk_class(name), ToolRiskClass::ReadOnly)
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
    classified_tool_risk_class(name).unwrap_or(ToolRiskClass::Write)
}

fn classified_tool_risk_class(name: &str) -> Option<ToolRiskClass> {
    match name {
        "crabdb.doctor"
        | "crabdb.status"
        | "crabdb.diff"
        | "crabdb.timeline"
        | "crabdb.why"
        | "crabdb.history"
        | "crabdb.code_from"
        | "crabdb.agent_status"
        | "crabdb.agent_inbox"
        | "crabdb.agent_board"
        | "crabdb.agent_stack"
        | "crabdb.agent_next"
        | "crabdb.agent_guide"
        | "crabdb.agent_dashboard"
        | "crabdb.agent_review_data"
        | "crabdb.agent_review_flow"
        | "crabdb.agent_ask"
        | "crabdb.agent_view"
        | "crabdb.agent_brief"
        | "crabdb.agent_summary"
        | "crabdb.agent_validate"
        | "crabdb.agent_test_plan"
        | "crabdb.agent_report"
        | "crabdb.agent_handoff"
        | "crabdb.agent_receipt"
        | "crabdb.agent_pr"
        | "crabdb.agent_story"
        | "crabdb.agent_tools"
        | "crabdb.agent_risk"
        | "crabdb.agent_impact"
        | "crabdb.agent_review_map"
        | "crabdb.agent_confidence"
        | "crabdb.agent_ready"
        | "crabdb.agent_diagnose"
        | "crabdb.agent_workdir"
        | "crabdb.agent_changes"
        | "crabdb.agent_delta"
        | "crabdb.agent_new"
        | "crabdb.agent_change"
        | "crabdb.agent_timeline"
        | "crabdb.agent_files"
        | "crabdb.agent_file"
        | "crabdb.agent_checkpoints"
        | "crabdb.agent_why"
        | "crabdb.agent_turn"
        | "crabdb.agent_compare"
        | "crabdb.agent_diff"
        | "crabdb.agent_review"
        | "crabdb.agent_focus"
        | "crabdb.lane_list"
        | "crabdb.lane_show"
        | "crabdb.lane_status"
        | "crabdb.lane_review"
        | "crabdb.lane_contribution"
        | "crabdb.gate_history"
        | "crabdb.lane_readiness"
        | "crabdb.lane_refresh_preview"
        | "crabdb.lane_handoff"
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
        | "crabdb.merge_queue_explain"
        | "crabdb.conflict_list"
        | "crabdb.conflict_show"
        | "crabdb.event_list"
        | "crabdb.span_list"
        | "crabdb.span_summary"
        | "crabdb.span_show"
        | "crabdb.show_turn"
        | "crabdb.diff_lane"
        | "crabdb.ignore_list"
        | "crabdb.ignore_check"
        | "crabdb.guardrail_check" => Some(ToolRiskClass::ReadOnly),
        "crabdb.config_set" | "crabdb.ignore_add" | "crabdb.ignore_remove" => {
            Some(ToolRiskClass::IdempotentWrite)
        }
        "crabdb.session_start"
        | "crabdb.session_end"
        | "crabdb.agent_mark_reviewed"
        | "crabdb.agent_mark_file_reviewed"
        | "crabdb.agent_archive"
        | "crabdb.agent_unarchive"
        | "crabdb.approval_request"
        | "crabdb.approval_decide"
        | "crabdb.run_pause"
        | "crabdb.run_resume"
        | "crabdb.lease_acquire"
        | "crabdb.lease_release"
        | "crabdb.anchor_create"
        | "crabdb.lane_spawn"
        | "crabdb.lane_claim"
        | "crabdb.merge_queue_add"
        | "crabdb.begin_turn"
        | "crabdb.add_message"
        | "crabdb.add_event"
        | "crabdb.span_start"
        | "crabdb.span_end"
        | "crabdb.end_turn" => Some(ToolRiskClass::Write),
        "crabdb.lane_remove"
        | "crabdb.agent_apply"
        | "crabdb.agent_finish"
        | "crabdb.agent_rewind"
        | "crabdb.agent_undo"
        | "crabdb.lane_rewind"
        | "crabdb.anchor_delete"
        | "crabdb.merge_queue_run"
        | "crabdb.merge_queue_remove"
        | "crabdb.conflict_resolve"
        | "crabdb.apply_patch"
        | "crabdb.read_file"
        | "crabdb.sync_workdir" => Some(ToolRiskClass::DestructiveWrite),
        "crabdb.agent_test" | "crabdb.agent_eval" | "crabdb.run_test" | "crabdb.run_eval" => {
            Some(ToolRiskClass::OpenWorldWrite)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_annotations_match_enforcement_for_all_declared_tools() {
        let tools = crate::mcp::tools::tools();
        let tools = tools.as_array().expect("tools must be an array");
        assert!(!tools.is_empty());

        for tool in tools {
            let name = tool["name"].as_str().expect("tool has a name");
            let read_only_hint = tool["annotations"]["readOnlyHint"]
                .as_bool()
                .unwrap_or_else(|| panic!("tool {name} missing readOnlyHint"));
            assert_eq!(
                read_only_hint,
                tool_is_read_only(name),
                "tool {name} advertises readOnlyHint={read_only_hint} but enforcement is {}",
                tool_is_read_only(name)
            );
        }
    }

    #[test]
    fn all_declared_tools_have_explicit_risk_classification() {
        let tools = crate::mcp::tools::tools();
        let tools = tools.as_array().expect("tools must be an array");
        assert!(!tools.is_empty());

        for tool in tools {
            let name = tool["name"].as_str().expect("tool has a name");
            assert!(
                classified_tool_risk_class(name).is_some(),
                "tool {name} is declared but falls back to generic write annotations"
            );
        }
    }

    #[test]
    fn all_declared_tools_have_unique_names() {
        let tools = crate::mcp::tools::tools();
        let tools = tools.as_array().expect("tools must be an array");
        assert!(!tools.is_empty());

        let mut seen = std::collections::BTreeSet::new();
        for tool in tools {
            let name = tool["name"].as_str().expect("tool has a name");
            assert!(seen.insert(name), "duplicate MCP tool declaration `{name}`");
        }
    }
}
