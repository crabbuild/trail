use super::*;

use serde::Serialize;
use serde_json::Value;
use trail::model::*;
use trail::Result;

macro_rules! agent_renderer {
    ($name:ident, $report:ty, $title:literal) => {
        pub(crate) fn $name(report: &$report, json: bool, options: &RenderOptions) -> Result<()> {
            render_agent_value($title, report, json, options)
        }
    };
}

agent_renderer!(render_agent_setup, AgentSetupReport, "Agent setup");
agent_renderer!(render_agent_status, AgentStatusReport, "Agent status");
agent_renderer!(render_agent_guide, AgentGuideReport, "Agent guide");
agent_renderer!(
    render_agent_dashboard,
    AgentDashboardReport,
    "Agent dashboard"
);
agent_renderer!(
    render_agent_review_data,
    AgentReviewDataReport,
    "Agent review data"
);
agent_renderer!(
    render_agent_action_palette,
    AgentReviewDataReport,
    "Agent actions"
);
agent_renderer!(
    render_agent_review_flow,
    AgentReviewFlowReport,
    "Agent review flow"
);
agent_renderer!(render_agent_inbox, AgentInboxReport, "Agent inbox");
agent_renderer!(render_agent_board, AgentBoardReport, "Agent board");
agent_renderer!(render_agent_stack, AgentStackReport, "Agent stack");
agent_renderer!(render_agent_next, AgentNextReport, "Agent next");
agent_renderer!(render_agent_list, AgentTaskListReport, "Agent tasks");
agent_renderer!(render_agent_brief, AgentBriefReport, "Agent brief");
agent_renderer!(render_agent_summary, AgentSummaryReport, "Agent summary");
agent_renderer!(
    render_agent_validate,
    AgentValidationReport,
    "Agent validation"
);
agent_renderer!(
    render_agent_test_plan,
    AgentTestPlanReport,
    "Agent test plan"
);
agent_renderer!(render_agent_receipt, AgentReceiptReport, "Agent receipt");
agent_renderer!(render_agent_handoff, AgentHandoffReport, "Agent handoff");
agent_renderer!(render_agent_story, AgentStoryReport, "Agent story");
agent_renderer!(render_agent_tools, AgentToolsReport, "Agent tools");
agent_renderer!(render_agent_impact, AgentImpactReport, "Agent impact");
agent_renderer!(
    render_agent_review_map,
    AgentReviewMapReport,
    "Agent review map"
);
agent_renderer!(render_agent_risk, AgentRiskReport, "Agent risk");
agent_renderer!(
    render_agent_confidence,
    AgentConfidenceReport,
    "Agent confidence"
);
agent_renderer!(render_agent_ready, AgentReadyReport, "Agent readiness");
agent_renderer!(
    render_agent_diagnose,
    AgentDiagnosisReport,
    "Agent diagnosis"
);
agent_renderer!(render_agent_compare, AgentCompareReport, "Agent comparison");
agent_renderer!(render_agent_workdir, AgentWorkdirReport, "Agent workdir");
agent_renderer!(render_agent_view, AgentTaskViewReport, "Agent task");
agent_renderer!(render_agent_changes, AgentChangesReport, "Agent changes");
agent_renderer!(render_agent_delta, AgentDeltaReport, "Agent delta");
agent_renderer!(render_agent_new, AgentNewReport, "Created agent task");
agent_renderer!(
    render_agent_mark_reviewed,
    AgentMarkReviewedReport,
    "Recorded review"
);
agent_renderer!(
    render_agent_mark_file_reviewed,
    AgentMarkFileReviewedReport,
    "Recorded file review"
);
agent_renderer!(
    render_agent_archive,
    AgentArchiveReport,
    "Updated agent archive"
);
agent_renderer!(render_agent_timeline, AgentTimelineReport, "Agent timeline");
agent_renderer!(
    render_agent_change_set,
    AgentChangeSetReport,
    "Agent change set"
);
agent_renderer!(render_agent_files, AgentFilesReport, "Agent files");
agent_renderer!(render_agent_file, AgentFileReport, "Agent file");
agent_renderer!(
    render_agent_checkpoints,
    AgentCheckpointReport,
    "Agent checkpoints"
);
agent_renderer!(render_agent_why, AgentWhyReport, "Agent provenance");
agent_renderer!(render_agent_turn, AgentTurnReport, "Agent turn");
agent_renderer!(render_agent_review, AgentReviewReport, "Agent review");
agent_renderer!(render_agent_focus, AgentFocusReport, "Agent focus");
agent_renderer!(render_agent_apply, AgentApplyReport, "Agent apply");
agent_renderer!(render_agent_finish, AgentFinishReport, "Agent finish");
agent_renderer!(render_agent_run, AgentRunReport, "Agent run");
agent_renderer!(
    render_agent_continue,
    AgentContinueReport,
    "Agent follow-up"
);

pub(crate) fn render_agent_empty_action_palette(json: bool, options: &RenderOptions) -> Result<()> {
    let actions = agent_empty_action_palette_actions();
    if json {
        return render_json(&actions);
    }
    render_document(
        &TerminalDocument::new("No agent task yet", UiTone::Neutral)
            .context("Set up a provider or start a task to see review actions.")
            .block(UiBlock::Table(actions_table(&actions)))
            .next(
                "trail agent setup",
                "print a copyable provider configuration",
            ),
        options,
    )
}

pub(crate) fn render_agent_empty_task_hint(
    requested: &str,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(&serde_json::json!({"requested": requested, "status": "empty"}));
    }
    render_document(
        &TerminalDocument::new(
            format!("No agent task available for {requested}"),
            UiTone::Neutral,
        )
        .next(
            "trail agent start --provider claude-code",
            "start an isolated terminal task",
        ),
        options,
    )
}

pub(crate) fn render_agent_report(
    report: &AgentReviewBundleReport,
    json: bool,
    options: &RenderOptions,
    markdown: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if markdown {
        return render_raw_content(&report.markdown, options);
    }
    render_agent_value("Agent report", report, false, options)
}

pub(crate) fn render_agent_pr(
    report: &AgentPrDraftReport,
    json: bool,
    options: &RenderOptions,
    title_only: bool,
    body_only: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if title_only {
        return render_raw_content(&report.title, options);
    }
    if body_only {
        return render_raw_content(&report.body, options);
    }
    render_agent_value("Agent pull request draft", report, false, options)
}

pub(crate) fn render_agent_diff(
    report: &AgentDiffReport,
    json: bool,
    options: &RenderOptions,
    stat: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_diff_with_title(
        &report.diff,
        false,
        options,
        true,
        stat,
        false,
        false,
        Some("Agent diff"),
    )
}

pub(crate) fn agent_empty_action_palette_actions() -> Vec<AgentReviewAction> {
    [
        (
            "setup_vscode",
            "Set up VS Code",
            "setup",
            "trail agent setup",
            "print a copyable ACP editor config that creates fresh task lanes automatically",
            "read_only",
            false,
        ),
        (
            "doctor_claude_code",
            "Check Claude Code",
            "doctor",
            "trail agent doctor --provider claude-code",
            "verify Trail workspace readiness and provider availability",
            "read_only",
            false,
        ),
        (
            "setup_codex_vscode",
            "Set up VS Code for Codex",
            "setup",
            "trail agent setup --provider codex",
            "print a copyable Codex ACP editor config that creates fresh task lanes automatically",
            "read_only",
            false,
        ),
        (
            "doctor_codex",
            "Check Codex",
            "doctor",
            "trail agent doctor --provider codex",
            "verify Trail workspace readiness and provider availability",
            "read_only",
            false,
        ),
        (
            "setup_cursor_vscode",
            "Set up VS Code for Cursor",
            "setup",
            "trail agent setup --provider cursor",
            "print a copyable Cursor ACP editor config that creates fresh task lanes automatically",
            "read_only",
            false,
        ),
        (
            "doctor_cursor",
            "Check Cursor",
            "doctor",
            "trail agent doctor --provider cursor",
            "verify Trail workspace readiness and provider availability",
            "read_only",
            false,
        ),
        (
            "start_terminal_task",
            "Start terminal task",
            "start",
            "trail agent start --provider claude-code",
            "launch a fresh materialized terminal task when you are not using an editor",
            "open_world",
            true,
        ),
        (
            "start_gemini_task",
            "Start Gemini task",
            "start",
            "trail agent start --provider gemini",
            "launch Gemini CLI in a fresh materialized Trail task lane",
            "open_world",
            true,
        ),
    ]
    .into_iter()
    .map(
        |(id, label, kind, command, reason, safety, requires_confirmation)| AgentReviewAction {
            id: id.to_string(),
            label: label.to_string(),
            kind: kind.to_string(),
            command: command.to_string(),
            reason: reason.to_string(),
            enabled: true,
            disabled_reason: None,
            safety: safety.to_string(),
            requires_confirmation,
            path: None,
            open_path: None,
            mcp_tool: None,
            mcp_arguments: None,
        },
    )
    .collect()
}

pub(crate) fn render_semantic_report<T: Serialize>(
    title: &str,
    report: &T,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let value = serde_json::to_value(report)?;
    let document = projected_document(title, &value);
    render_document(&document.pager_eligible(), options)
}

fn render_agent_value<T: Serialize>(
    title: &str,
    report: &T,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    render_semantic_report(title, report, json, options)
}

fn projected_document(title: &str, value: &Value) -> TerminalDocument {
    let Some(object) = value.as_object() else {
        return TerminalDocument::new(title, UiTone::Neutral).block(UiBlock::Metadata(vec![(
            "Value".to_string(),
            scalar(value),
        )]));
    };
    let mut metadata = Vec::new();
    let status = object
        .get("status")
        .or_else(|| object.get("state"))
        .or_else(|| object.get("readiness_status"))
        .or_else(|| object.get("validation_status"))
        .or_else(|| object.get("decision"))
        .and_then(Value::as_str);
    let lead = status
        .map(|status| format!("{title}: {}", state_label(status)))
        .unwrap_or_else(|| title.to_string());
    let mut document = TerminalDocument::new(lead, document_tone(object));
    if let Some(context) = ["headline", "summary", "current_state"]
        .into_iter()
        .find_map(|key| object.get(key).and_then(Value::as_str))
    {
        document = document.context(context);
    }
    if let Some(next) = ["primary", "next"]
        .into_iter()
        .find_map(|key| object.get(key).and_then(action_from_value))
        .or_else(|| {
            object
                .get("suggestions")
                .and_then(Value::as_array)
                .and_then(|suggestions| suggestions.first())
                .and_then(action_from_value)
        })
    {
        document.next = Some(next);
    }
    for (key, value) in object {
        if matches!(
            key.as_str(),
            "headline" | "summary" | "current_state" | "primary" | "next"
        ) {
            continue;
        }
        match value {
            Value::Null => {}
            Value::Bool(_) | Value::Number(_) | Value::String(_) => {
                let rendered = if matches!(
                    key.as_str(),
                    "status" | "state" | "readiness_status" | "validation_status" | "decision"
                ) {
                    value
                        .as_str()
                        .map(state_label)
                        .unwrap_or_else(|| scalar(value))
                } else {
                    scalar(value)
                };
                metadata.push((label(key), rendered));
            }
            Value::Array(values)
                if values
                    .iter()
                    .all(|value| value.is_string() || value.is_number() || value.is_boolean()) =>
            {
                document = document.block(UiBlock::section(
                    label(key),
                    vec![UiBlock::Lines(
                        values
                            .iter()
                            .map(|value| (scalar(value), UiTone::Neutral))
                            .collect(),
                    )],
                ));
            }
            Value::Array(values) => {
                document = document.block(UiBlock::section(label(key), vec![array_block(values)]));
            }
            Value::Object(values) => {
                document = document.block(UiBlock::section(label(key), object_blocks(values)));
            }
        }
    }
    if !metadata.is_empty() {
        document.blocks.insert(0, UiBlock::Metadata(metadata));
    }
    document
}

fn action_from_value(value: &Value) -> Option<UiNextAction> {
    let object = value.as_object()?;
    let command = object.get("command")?.as_str()?.trim();
    if command.is_empty() {
        return None;
    }
    let reason = object
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("continue with the displayed workflow")
        .to_string();
    Some(UiNextAction {
        command: command.to_string(),
        reason,
    })
}

fn document_tone(object: &serde_json::Map<String, Value>) -> UiTone {
    let state = [
        "status",
        "state",
        "decision",
        "readiness_status",
        "validation_status",
    ]
    .into_iter()
    .find_map(|key| object.get(key).and_then(Value::as_str))
    .unwrap_or_default()
    .to_ascii_lowercase();
    if matches!(
        state.as_str(),
        "ok" | "pass" | "passed" | "ready" | "complete" | "completed"
    ) {
        UiTone::Success
    } else if matches!(
        state.as_str(),
        "blocked" | "fail" | "failed" | "conflicted" | "conflict"
    ) {
        UiTone::Blocked
    } else if matches!(state.as_str(), "warning" | "warn" | "pending" | "dirty") {
        UiTone::Attention
    } else {
        UiTone::Neutral
    }
}

fn object_blocks(object: &serde_json::Map<String, Value>) -> Vec<UiBlock> {
    let scalar_entries = object
        .iter()
        .filter_map(|(key, value)| {
            value
                .is_boolean()
                .then_some((key, value))
                .or_else(|| value.is_number().then_some((key, value)))
                .or_else(|| value.is_string().then_some((key, value)))
        })
        .map(|(key, value)| (label(key), scalar(value)))
        .collect::<Vec<_>>();
    let mut blocks = Vec::new();
    if !scalar_entries.is_empty() {
        blocks.push(UiBlock::Metadata(scalar_entries));
    }
    for (key, value) in object {
        if !value.is_null() && !value.is_boolean() && !value.is_number() && !value.is_string() {
            let nested = match value {
                Value::Array(values) => array_block(values),
                Value::Object(values) => UiBlock::section(label(key), object_blocks(values)),
                _ => continue,
            };
            blocks.push(UiBlock::section(label(key), vec![nested]));
        }
    }
    blocks
}

fn array_block(values: &[Value]) -> UiBlock {
    if values.is_empty() {
        return UiBlock::Notice("No items.".to_string());
    }
    let object_rows = values
        .iter()
        .filter_map(Value::as_object)
        .collect::<Vec<_>>();
    if object_rows.len() == values.len() {
        let mut keys = Vec::new();
        for object in &object_rows {
            for (key, value) in *object {
                if (value.is_boolean() || value.is_number() || value.is_string())
                    && !keys.contains(key)
                {
                    keys.push(key.clone());
                }
            }
        }
        if !keys.is_empty() {
            let columns = keys
                .iter()
                .enumerate()
                .map(|(index, key)| UiColumn::left(label(key), index.min(3) as u8, 10))
                .collect();
            let rows = object_rows
                .iter()
                .map(|object| {
                    keys.iter()
                        .map(|key| {
                            object
                                .get(key)
                                .map(scalar)
                                .unwrap_or_else(|| "-".to_string())
                        })
                        .collect()
                })
                .collect();
            return UiBlock::Table(UiTable::new(columns, rows));
        }
    }
    UiBlock::Lines(
        values
            .iter()
            .map(|value| (compact_value(value), UiTone::Neutral))
            .collect(),
    )
}

fn compact_value(value: &Value) -> String {
    match value {
        Value::Object(object) => object
            .iter()
            .filter(|(_, value)| value.is_boolean() || value.is_number() || value.is_string())
            .map(|(key, value)| format!("{}: {}", label(key), scalar(value)))
            .collect::<Vec<_>>()
            .join("; "),
        Value::Array(values) => format!("{} items", values.len()),
        _ => scalar(value),
    }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}
fn label(key: &str) -> String {
    key.replace('_', " ")
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}
fn actions_table(actions: &[AgentReviewAction]) -> UiTable {
    UiTable::new(
        vec![
            UiColumn::left("ACTION", 0, 14),
            UiColumn::left("WHY", 0, 18),
            UiColumn::left("COMMAND", 1, 20),
        ],
        actions
            .iter()
            .map(|a| vec![a.label.clone(), a.reason.clone(), a.command.clone()])
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_projection_prioritizes_state_focus_and_safe_next_action() {
        let document = projected_document(
            "Agent status",
            &serde_json::json!({
                "status": "dirty",
                "summary": "fix-login needs one recorded checkpoint",
                "primary": {
                    "command": "trail agent mark-reviewed latest",
                    "reason": "record the reviewed checkpoint before applying work"
                },
                "tasks": [
                    {"title": "Fix login", "state": "dirty", "changed_paths": 2},
                    {"title": "Update docs", "state": "ready", "changed_paths": 1}
                ]
            }),
        );
        assert_eq!(document.lead.unwrap().text, "Agent status: needs record");
        assert_eq!(
            document.context.as_deref(),
            Some("fix-login needs one recorded checkpoint")
        );
        assert_eq!(
            document.next.unwrap().command,
            "trail agent mark-reviewed latest"
        );
        assert!(document
            .blocks
            .iter()
            .any(|block| matches!(block, UiBlock::Section { title, .. } if title == "Tasks")));
    }

    #[test]
    fn agent_state_fixture_matrix_keeps_recovery_states_distinct() {
        for (state, expected, tone) in [
            ("running", "running", UiTone::Neutral),
            ("dirty", "needs record", UiTone::Attention),
            ("blocked", "blocked", UiTone::Blocked),
            ("conflicted", "conflicted", UiTone::Blocked),
            ("ready", "ready", UiTone::Success),
            ("archived", "archived", UiTone::Neutral),
        ] {
            let document = projected_document("Agent task", &serde_json::json!({"status": state}));
            let lead = document.lead.unwrap();
            assert_eq!(lead.text, format!("Agent task: {expected}"));
            assert_eq!(lead.tone, tone);
        }
    }
}
