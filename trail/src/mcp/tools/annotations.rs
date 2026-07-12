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
        ToolRiskClass::OpenWorldDestructiveWrite => json!({
            "readOnlyHint": false,
            "destructiveHint": true,
            "idempotentHint": true,
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
    OpenWorldDestructiveWrite,
}

fn tool_risk_class(name: &str) -> ToolRiskClass {
    classified_tool_risk_class(name).unwrap_or(ToolRiskClass::Write)
}

fn classified_tool_risk_class(name: &str) -> Option<ToolRiskClass> {
    match name {
        "trail.doctor"
        | "trail.status"
        | "trail.diff"
        | "trail.timeline"
        | "trail.why"
        | "trail.history"
        | "trail.code_from"
        | "trail.agent_status"
        | "trail.agent_inbox"
        | "trail.agent_board"
        | "trail.agent_stack"
        | "trail.agent_next"
        | "trail.agent_guide"
        | "trail.agent_dashboard"
        | "trail.agent_review_data"
        | "trail.agent_review_flow"
        | "trail.agent_ask"
        | "trail.agent_view"
        | "trail.agent_brief"
        | "trail.agent_summary"
        | "trail.agent_validate"
        | "trail.agent_test_plan"
        | "trail.agent_report"
        | "trail.agent_handoff"
        | "trail.agent_receipt"
        | "trail.agent_pr"
        | "trail.agent_story"
        | "trail.agent_tools"
        | "trail.agent_risk"
        | "trail.agent_impact"
        | "trail.agent_review_map"
        | "trail.agent_confidence"
        | "trail.agent_ready"
        | "trail.agent_diagnose"
        | "trail.agent_workdir"
        | "trail.agent_changes"
        | "trail.agent_delta"
        | "trail.agent_new"
        | "trail.agent_change"
        | "trail.agent_timeline"
        | "trail.agent_files"
        | "trail.agent_file"
        | "trail.agent_checkpoints"
        | "trail.agent_why"
        | "trail.agent_turn"
        | "trail.agent_compare"
        | "trail.agent_diff"
        | "trail.agent_review"
        | "trail.agent_focus"
        | "trail.agent_integrations"
        | "trail.agent_hook_installations"
        | "trail.agent_hook_receipts"
        | "trail.agent_capture_runs"
        | "trail.agent_artifacts"
        | "trail.agent_provenance"
        | "trail.agent_attestations"
        | "trail.agent_attestation_verify"
        | "trail.agent_learnings"
        | "trail.agent_git_links"
        | "trail.agent_trace"
        | "trail.lane_list"
        | "trail.lane_show"
        | "trail.lane_status"
        | "trail.lane_review"
        | "trail.lane_contribution"
        | "trail.gate_history"
        | "trail.lane_readiness"
        | "trail.lane_refresh_preview"
        | "trail.lane_handoff"
        | "trail.lane_workspace"
        | "trail.lane_space"
        | "trail.deps_status"
        | "trail.env_adapters"
        | "trail.env_status"
        | "trail.env_discover"
        | "trail.env_graph"
        | "trail.env_generation"
        | "trail.env_runtime_status"
        | "trail.env_explain"
        | "trail.env_plan"
        | "trail.cache_list"
        | "trail.cache_inspect"
        | "trail.cache_verify"
        | "trail.config_list"
        | "trail.config_get"
        | "trail.session_list"
        | "trail.session_current"
        | "trail.session_show"
        | "trail.session_context"
        | "trail.approval_list"
        | "trail.approval_show"
        | "trail.run_list"
        | "trail.run_show"
        | "trail.lease_list"
        | "trail.anchor_list"
        | "trail.anchor_resolve"
        | "trail.merge_queue_list"
        | "trail.merge_queue_explain"
        | "trail.conflict_list"
        | "trail.conflict_show"
        | "trail.event_list"
        | "trail.span_list"
        | "trail.span_summary"
        | "trail.span_show"
        | "trail.show_turn"
        | "trail.diff_lane"
        | "trail.ignore_list"
        | "trail.ignore_check"
        | "trail.guardrail_check" => Some(ToolRiskClass::ReadOnly),
        "trail.config_set" | "trail.ignore_add" | "trail.ignore_remove" | "trail.lane_unmount" => {
            Some(ToolRiskClass::IdempotentWrite)
        }
        "trail.session_start"
        | "trail.session_end"
        | "trail.agent_mark_reviewed"
        | "trail.agent_mark_file_reviewed"
        | "trail.agent_archive"
        | "trail.agent_unarchive"
        | "trail.approval_request"
        | "trail.approval_decide"
        | "trail.run_pause"
        | "trail.run_resume"
        | "trail.lease_acquire"
        | "trail.lease_release"
        | "trail.anchor_create"
        | "trail.lane_spawn"
        | "trail.lane_claim"
        | "trail.lane_checkpoint"
        | "trail.lane_update"
        | "trail.lane_mount"
        | "trail.merge_queue_add"
        | "trail.begin_turn"
        | "trail.add_message"
        | "trail.add_event"
        | "trail.span_start"
        | "trail.span_end"
        | "trail.end_turn" => Some(ToolRiskClass::Write),
        "trail.lane_remove"
        | "trail.agent_apply"
        | "trail.agent_finish"
        | "trail.agent_rewind"
        | "trail.agent_undo"
        | "trail.lane_rewind"
        | "trail.anchor_delete"
        | "trail.merge_queue_run"
        | "trail.merge_queue_remove"
        | "trail.conflict_resolve"
        | "trail.apply_patch"
        | "trail.read_file"
        | "trail.lane_hydrate"
        | "trail.sync_workdir" => Some(ToolRiskClass::DestructiveWrite),
        "trail.cache_gc" => Some(ToolRiskClass::DestructiveWrite),
        "trail.agent_test"
        | "trail.agent_eval"
        | "trail.run_test"
        | "trail.run_eval"
        | "trail.lane_exec"
        | "trail.deps_sync"
        | "trail.env_sync"
        | "trail.env_sync_all"
        | "trail.env_runtime_reconcile" => Some(ToolRiskClass::OpenWorldWrite),
        "trail.env_runtime_stop" => Some(ToolRiskClass::OpenWorldDestructiveWrite),
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
