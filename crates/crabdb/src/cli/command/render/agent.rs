use super::render_json;

use crabdb::model::*;
use crabdb::Result;

pub(crate) fn render_agent_setup(report: &AgentSetupReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent setup: {}", report.provider);
        println!("Editor: {}", report.editor);
        println!("Detected: {}", if report.detected { "yes" } else { "no" });
        println!("Command:");
        println!("  {}", shell_join(&report.command));
        if !report.warnings.is_empty() {
            println!("Warnings:");
            for warning in &report.warnings {
                println!("  {warning}");
            }
        }
        println!("Snippet:");
        println!("{}", report.snippet);
        println!("Next steps:");
        println!("  Paste the snippet into your editor's ACP custom-agent settings.");
        for suggestion in &report.suggestions {
            println!("  {}", suggestion.command);
            println!("  {}", suggestion.reason);
        }
    }
    Ok(())
}

pub(crate) fn render_agent_status(
    report: &AgentStatusReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent status: {:?}", report.status);
        if let Some(task) = &report.latest {
            print_agent_task_summary(task);
            if let Some(risk) = &report.risk {
                print_agent_risk_line(risk);
            }
        } else {
            println!("No agent tasks recorded");
        }
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_guide(report: &AgentGuideReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent guide: {:?}", report.status);
        println!("{}", report.headline);
        println!("{}", report.current_state);
        if let Some(task) = &report.task {
            println!("Task: {}", agent_task_display_title(task));
            print_agent_task_id_if_needed(task);
            print_agent_task_workdir(task);
        }
        println!("Do this next:");
        println!("  {}", report.primary.command);
        println!("  {}", report.primary.reason);
        if !report.steps.is_empty() {
            println!("Workflow:");
            for (idx, step) in report.steps.iter().enumerate() {
                println!("  {}. {}", idx + 1, step.label);
                println!("     {}", step.command);
                println!("     {}", step.reason);
                println!("     When: {}", step.when);
            }
        }
        if !report.concepts.is_empty() {
            println!("Mental model:");
            for concept in &report.concepts {
                println!("  {}: {}", concept.name, concept.meaning);
            }
        }
    }
    Ok(())
}

pub(crate) fn render_agent_dashboard(
    report: &AgentDashboardReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent dashboard: {:?}", report.status);
        println!("{}", report.summary);
        if let Some(task) = &report.task {
            println!("Task: {}", agent_task_display_title(task));
            print_agent_task_id_if_needed(task);
            print_agent_task_workdir(task);
            println!(
                "Changed files: {}  Turns: {}  Tool events: {}",
                task.changed_paths.len(),
                task.turns,
                task.tool_events
            );
            if let Some(checkpoint) = &task.latest_checkpoint {
                println!("Last checkpoint: {}", checkpoint.0);
            }
        }
        if let Some(ready) = &report.ready {
            print_agent_risk_line(&ready.risk);
            println!(
                "Readiness: {}  Apply: {}  Ready: {}",
                ready.readiness_status, ready.status, ready.ready
            );
        }
        if let Some(validation) = &report.validation {
            println!("Validation: {}", validation.status);
            if validation.needs_test || validation.needs_eval {
                println!("  {}", validation.next.command);
                println!("  {}", validation.next.reason);
            }
        }
        if let Some(focus) = &report.focus {
            println!("Focus:");
            println!("  {}", focus.path);
            println!("  {}", focus.summary);
            if let Some(command) = &focus.open_command {
                println!("  Open: {command}");
            } else {
                println!("  Inspect: crabdb agent focus {}", focus.task.lane);
            }
        }
        if let Some(changes) = &report.changes {
            if changes.total_changed_paths.is_empty() {
                println!("Changed files: none");
            } else {
                println!("Changed files:");
                for change in changes.total_changed_paths.iter().take(5) {
                    println!(
                        "  {:?} {} (+{} -{})",
                        change.kind, change.path, change.additions, change.deletions
                    );
                }
                if changes.total_changed_paths.len() > 5 {
                    println!(
                        "  ... {} more; run `crabdb agent changes {} --by-file`",
                        changes.total_changed_paths.len() - 5,
                        changes.lane
                    );
                }
            }
        }
        println!("Next:");
        println!("  {}", report.next.command);
        println!("  {}", report.next.reason);
        if let Some(task) = &report.task {
            println!("Actions:");
            println!("  crabdb agent action {}", task.lane);
            println!("  show runnable review, validation, apply, and recovery actions");
        }
        if report.suggestions.len() > 1 {
            println!("Useful commands:");
            for suggestion in report.suggestions.iter().skip(1) {
                println!("  {}", suggestion.command);
                println!("  {}", suggestion.reason);
            }
        }
    }
    Ok(())
}

pub(crate) fn render_agent_review_data(
    report: &AgentReviewDataReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Agent review data: {}",
            agent_task_display_title(&report.task)
        );
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("{}", report.summary);
        println!(
            "Files: {}/{} reviewed  Needs review: {}",
            report.reviewed_files, report.total_files, report.needs_review_files
        );
        println!(
            "Verdict: {}  Confidence: {}/100  Validation: {}  Apply: {}",
            report.confidence_verdict,
            report.confidence_score,
            report.validation_status,
            report.readiness_status
        );
        println!("Risk: {:?}", report.risk_level);
        if let Some(focus) = &report.focus {
            println!("Focus:");
            println!("  {}", focus.path);
            println!("  {}", focus.summary);
            if let Some(command) = &focus.open_command {
                println!("  Open: {command}");
            }
        }
        if !report.review_map.areas.is_empty() {
            println!("Review areas:");
            for area in &report.review_map.areas {
                println!(
                    "  {}: {} file(s), {}",
                    area.label,
                    area.files.len(),
                    area.state
                );
            }
        }
        println!("Next:");
        println!("  {}", report.next.command);
        println!("  {}", report.next.reason);
        if !report.actions.is_empty() {
            println!("Actions:");
            for action in &report.actions {
                let state = if action.enabled {
                    "enabled"
                } else {
                    "disabled"
                };
                let confirmation = if action.requires_confirmation {
                    ", confirmation"
                } else {
                    ""
                };
                println!(
                    "  {} ({}) [{}: {}{}]",
                    action.label, action.id, state, action.safety, confirmation
                );
                println!("    {}", action.command);
                println!("    {}", action.reason);
                if let Some(reason) = &action.disabled_reason {
                    println!("    Disabled: {reason}");
                }
            }
        }
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_action_palette(
    report: &AgentReviewDataReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(&serde_json::json!({
            "task": &report.task,
            "summary": &report.summary,
            "next": &report.next,
            "actions": &report.actions
        }));
    }
    if !quiet {
        println!("Agent actions: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        println!("{}", report.summary);
        if report.actions.is_empty() {
            println!("No actions are currently available.");
        } else {
            println!("Actions:");
            for action in &report.actions {
                let state = if action.enabled {
                    "enabled"
                } else {
                    "disabled"
                };
                let confirmation = if action.requires_confirmation {
                    ", confirmation"
                } else {
                    ""
                };
                println!(
                    "  {}  {} [{}: {}{}]",
                    action.id, action.label, state, action.safety, confirmation
                );
                println!(
                    "    Run: crabdb agent action {} {}",
                    report.task.lane, action.id
                );
                println!(
                    "    Print: crabdb agent action {} {} --print",
                    report.task.lane, action.id
                );
                println!("    {}", action.reason);
                if let Some(reason) = &action.disabled_reason {
                    println!("    Disabled: {reason}");
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn render_agent_empty_action_palette(json: bool, quiet: bool) -> Result<()> {
    let summary = "No agent task is recorded yet. Set up an editor, verify the provider, or start a terminal task.";
    let next = StatusSuggestion {
        command: "crabdb agent setup".to_string(),
        reason: "print a stable editor config that creates fresh CrabDB tasks automatically"
            .to_string(),
    };
    let actions = agent_empty_action_palette_actions();
    if json {
        return render_json(&serde_json::json!({
            "status": "empty",
            "task": null,
            "summary": summary,
            "next": &next,
            "actions": &actions
        }));
    }
    if !quiet {
        println!("Agent actions: no agent task yet");
        println!("{summary}");
        println!("Next:");
        println!("  {}", next.command);
        println!("  {}", next.reason);
        println!("Actions:");
        for action in &actions {
            let confirmation = if action.requires_confirmation {
                ", confirmation"
            } else {
                ""
            };
            println!(
                "  {}  {} [enabled: {}{}]",
                action.id, action.label, action.safety, confirmation
            );
            println!("    Command: {}", action.command);
            println!("    {}", action.reason);
        }
    }
    Ok(())
}

pub(crate) fn render_agent_empty_task_hint(requested: &str, json: bool, quiet: bool) -> Result<()> {
    let summary = match requested {
        "changes" => "No agent task is recorded yet, so there are no agent changes to inspect. Set up an editor, verify the provider, or start a terminal task.".to_string(),
        "apply" => "No agent task is recorded yet, so there is nothing to apply. Set up an editor, verify the provider, or start a terminal task.".to_string(),
        "view" => "No agent task is recorded yet, so there is no task to view. Set up an editor, verify the provider, or start a terminal task.".to_string(),
        _ => format!(
            "No agent task is recorded yet. Set up an editor, verify the provider, or start a terminal task before running `{requested}`."
        ),
    };
    let next = StatusSuggestion {
        command: "crabdb agent setup".to_string(),
        reason: "print a stable editor config that creates fresh CrabDB tasks automatically"
            .to_string(),
    };
    let actions = agent_empty_action_palette_actions();
    if json {
        return render_json(&serde_json::json!({
            "status": "empty",
            "task": null,
            "requested": requested,
            "summary": summary,
            "next": &next,
            "actions": &actions
        }));
    }
    if !quiet {
        println!("Agent {requested}: no agent task yet");
        println!("{summary}");
        println!("Next:");
        println!("  {}", next.command);
        println!("  {}", next.reason);
        println!("Other useful commands:");
        for action in &actions {
            println!("  {}", action.command);
            println!("  {}", action.reason);
        }
    }
    Ok(())
}

pub(crate) fn agent_empty_action_palette_actions() -> Vec<AgentReviewAction> {
    vec![
        agent_static_action(
            "setup_vscode",
            "Set up VS Code",
            "setup",
            "crabdb agent setup",
            "print a copyable ACP editor config that creates fresh task lanes automatically",
            "read_only",
            false,
        ),
        agent_static_action(
            "doctor_claude_code",
            "Check Claude Code",
            "doctor",
            "crabdb agent doctor --provider claude-code",
            "verify CrabDB workspace readiness and provider availability",
            "read_only",
            false,
        ),
        agent_static_action(
            "setup_codex_vscode",
            "Set up VS Code for Codex",
            "setup",
            "crabdb agent setup --provider codex",
            "print a copyable Codex ACP editor config that creates fresh task lanes automatically",
            "read_only",
            false,
        ),
        agent_static_action(
            "doctor_codex",
            "Check Codex",
            "doctor",
            "crabdb agent doctor --provider codex",
            "verify CrabDB workspace readiness and provider availability",
            "read_only",
            false,
        ),
        agent_static_action(
            "start_terminal_task",
            "Start terminal task",
            "start",
            "crabdb agent start --provider claude-code",
            "launch a fresh materialized terminal task when you are not using an editor",
            "open_world",
            true,
        ),
    ]
}

fn agent_static_action(
    id: &str,
    label: &str,
    kind: &str,
    command: &str,
    reason: &str,
    safety: &str,
    requires_confirmation: bool,
) -> AgentReviewAction {
    AgentReviewAction {
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
    }
}

pub(crate) fn render_agent_review_flow(
    report: &AgentReviewFlowReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Agent review flow: {}",
            agent_task_display_title(&report.task)
        );
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("{}", report.summary);
        println!(
            "Review: {}  New files: {}  New lines: {}",
            report.review_status, report.new_changed_paths, report.new_changed_lines
        );
        println!(
            "Validation: {}  Apply: {}  Ready: {}",
            report.validation.status, report.ready.status, report.ready.ready
        );
        if let Some(checkpoint) = &report.task.latest_checkpoint {
            println!("Last checkpoint: {}", checkpoint.0);
        }
        if let Some(marker) = &report.reviewed {
            println!("Reviewed checkpoint: {}", marker.checkpoint.0);
        } else {
            println!("Reviewed checkpoint: none");
        }
        if let Some(focus) = &report.focus {
            println!("Focus: {}", focus.path);
        }
        println!("Checklist:");
        for (idx, step) in report.steps.iter().enumerate() {
            println!("  {}. [{}] {}", idx + 1, step.state, step.label);
            println!("     {}", step.command);
            println!("     {}", step.reason);
        }
        println!("Next:");
        println!("  {}", report.next.command);
        println!("  {}", report.next.reason);
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_inbox(report: &AgentInboxReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent inbox");
        println!(
            "Tasks: {}  Need attention: {}  Archived: {}",
            report.total, report.attention_count, report.archived_count
        );
        if report.include_archived {
            println!("Showing: active and archived tasks");
        }
        if report.groups.is_empty() {
            println!("No agent tasks recorded");
        }
        for group in &report.groups {
            print_agent_inbox_group(group);
        }
        println!("Next command:");
        println!("  {}", report.next.command);
        println!("  {}", report.next.reason);
        if report.suggestions.len() > 1 {
            println!("Other useful commands:");
            for suggestion in report.suggestions.iter().skip(1) {
                println!("  {}", suggestion.command);
                println!("  {}", suggestion.reason);
            }
        }
    }
    Ok(())
}

pub(crate) fn render_agent_board(report: &AgentBoardReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent board");
        println!(
            "Tasks: {}  Need attention: {}  Ready: {}  Running: {}  Blocked: {}  Conflicted: {}  Applied: {}  Archived: {}",
            report.total,
            report.attention_count,
            report.ready_count,
            report.active_count,
            report.blocked_count,
            report.conflicted_count,
            report.applied_count,
            report.archived_count
        );
        if report.include_archived {
            println!("Showing: active and archived tasks");
        }
        if report.columns.is_empty() {
            println!("No agent tasks recorded");
        }
        for column in &report.columns {
            print_agent_board_column(column);
        }
        println!("Next command:");
        println!("  {}", report.next.command);
        println!("  {}", report.next.reason);
        if report.suggestions.len() > 1 {
            println!("Other useful commands:");
            for suggestion in report.suggestions.iter().skip(1) {
                println!("  {}", suggestion.command);
                println!("  {}", suggestion.reason);
            }
        }
    }
    Ok(())
}

pub(crate) fn render_agent_stack(report: &AgentStackReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent stack");
        println!("{}", report.summary);
        println!(
            "Tasks: {}  Ready: {}  Blocked: {}  Shared paths: {}",
            report.total, report.ready_count, report.blocked_count, report.overlap_count
        );
        if report.include_archived {
            println!("Showing: active and archived tasks");
        }
        if !report.shared_paths.is_empty() {
            println!("Shared files:");
            for shared in &report.shared_paths {
                println!("  {} ({})", shared.path, shared.task_titles.join(", "));
            }
        }
        if !report.apply_order.is_empty() {
            println!("Apply order:");
            for (idx, lane) in report.apply_order.iter().enumerate() {
                if let Some(item) = report.items.iter().find(|item| &item.task.lane == lane) {
                    println!(
                        "  {}. {}  risk {:?} ({}/100), {} file(s)",
                        idx + 1,
                        agent_task_display_title(&item.task),
                        item.risk.level,
                        item.risk.score,
                        item.task.changed_paths.len()
                    );
                }
            }
        }
        if !report.items.is_empty() {
            println!("Tasks:");
            for item in &report.items {
                let shared = if item.shared_paths.is_empty() {
                    "no shared files".to_string()
                } else {
                    format!("shared: {}", item.shared_paths.join(", "))
                };
                println!(
                    "  {}. {}  {}  risk {:?} ({}/100), {}",
                    item.rank,
                    agent_task_display_title(&item.task),
                    item.status,
                    item.risk.level,
                    item.risk.score,
                    shared
                );
                println!("     {}", item.next.command);
                println!("     {}", item.next.reason);
            }
        }
        println!("Next command:");
        println!("  {}", report.next.command);
        println!("  {}", report.next.reason);
        if report.suggestions.len() > 1 {
            println!("Other useful commands:");
            for suggestion in report.suggestions.iter().skip(1) {
                println!("  {}", suggestion.command);
                println!("  {}", suggestion.reason);
            }
        }
    }
    Ok(())
}

pub(crate) fn render_agent_next(report: &AgentNextReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent next: {:?}", report.status);
        println!("{}", report.summary);
        if let Some(task) = &report.task {
            println!("Task: {}", agent_task_display_title(task));
            print_agent_task_id_if_needed(task);
            print_agent_task_workdir(task);
            println!("Changed files: {}", task.changed_paths.len());
            println!("Turns: {}  Tool events: {}", task.turns, task.tool_events);
        }
        println!("Next command:");
        println!("  {}", report.primary.command);
        println!("  {}", report.primary.reason);
        if !report.suggestions.is_empty() {
            println!("Other useful commands:");
            for suggestion in &report.suggestions {
                println!("  {}", suggestion.command);
                println!("  {}", suggestion.reason);
            }
        }
    }
    Ok(())
}

pub(crate) fn render_agent_list(
    report: &AgentTaskListReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.tasks.is_empty() {
            println!("No agent tasks");
        }
        for task in &report.tasks {
            println!(
                "{} {:?}{} {} changed path(s)",
                agent_task_display_title(task),
                task.status,
                if task.archived { " archived" } else { "" },
                task.changed_paths.len()
            );
            print_agent_task_id_if_needed(task);
            print_agent_task_workdir(task);
        }
    }
    Ok(())
}

pub(crate) fn render_agent_brief(report: &AgentBriefReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent brief: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!(
            "Status: {:?}  Ready: {}  Changed files: {}",
            report.task.status,
            report.ready_to_apply,
            report.changed_paths.len()
        );
        print_agent_risk_line(&report.risk);
        println!("Next:");
        println!("  {}", report.next.primary.command);
        println!("  {}", report.next.primary.reason);
        if !report.blockers.is_empty() {
            println!("Blockers:");
            for blocker in &report.blockers {
                println!("  {}: {}", blocker.code, blocker.message);
            }
        }
        if !report.warnings.is_empty() {
            println!("Warnings:");
            for warning in &report.warnings {
                println!("  {}: {}", warning.code, warning.message);
            }
        }
        if !report.changed_paths.is_empty() {
            println!("Changed files:");
            print_changed_paths(&report.changed_paths, "  ");
        }
        if !report.groups.is_empty() {
            println!("Changes:");
            for group in &report.groups {
                print_agent_brief_group(group);
            }
        }
        if let Some(diff) = &report.latest_change_diff {
            print_agent_brief_diff(diff);
        }
        if !report.tool_summaries.is_empty() {
            println!("Tools:");
            for tool in &report.tool_summaries {
                println!("  {tool}");
            }
        }
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_summary(
    report: &AgentSummaryReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent summary: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("{}", report.summary);
        println!(
            "Ready: {}  Status: {}  Readiness: {}",
            report.ready, report.ready_status, report.readiness_status
        );
        print_agent_risk_line(&report.risk);
        println!(
            "Changed files: {}  Turns: {}  Tool events: {}",
            report.changed_paths.len(),
            report.task.turns,
            report.task.tool_events
        );
        if let Some(checkpoint) = &report.latest_checkpoint {
            println!("Last checkpoint: {}", checkpoint.0);
        }
        if report.validation.is_empty() {
            println!("Validation: no test or eval gate recorded");
        } else {
            println!("Validation:");
            for gate in &report.validation {
                let suite = gate.suite.as_deref().unwrap_or("default");
                let result = if gate.success { "passed" } else { "failed" };
                println!(
                    "  {} `{}` {} ({})",
                    gate.kind,
                    suite,
                    result,
                    shell_join(&gate.command)
                );
            }
        }
        if !report.blockers.is_empty() {
            println!("Blockers:");
            for blocker in &report.blockers {
                println!("  {}: {}", blocker.code, blocker.message);
            }
        }
        if !report.warnings.is_empty() {
            println!("Warnings:");
            for warning in &report.warnings {
                println!("  {}: {}", warning.code, warning.message);
            }
        }
        if let Some(error) = &report.apply_error {
            println!("Git preflight: failed");
            println!("  {error}");
        } else if let Some(preview) = &report.apply_preview {
            println!("Git preflight: {}", preview.status);
            if let Some(branch) = &preview.git_apply_plan.git_branch {
                println!("Into Git branch: {branch}");
            }
        }
        println!("PR title:");
        println!("  {}", report.pr_title);
        println!("Next:");
        println!("  {}", report.next.command);
        println!("  {}", report.next.reason);
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_validate(
    report: &AgentValidationReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Agent validation: {}",
            agent_task_display_title(&report.task)
        );
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("Status: {}", report.status);
        println!("{}", report.summary);
        println!(
            "Needs test: {}  Needs eval: {}  Changed files: {}",
            report.needs_test,
            report.needs_eval,
            report.changed_paths.len()
        );
        if let Some(test) = &report.latest_test {
            println!("Latest test:");
            print_agent_gate_summary(test, "  ");
        } else {
            println!("Latest test: none recorded");
        }
        if let Some(eval) = &report.latest_eval {
            println!("Latest eval:");
            print_agent_gate_summary(eval, "  ");
        } else {
            println!("Latest eval: none recorded");
        }
        if !report.recent_gates.is_empty() {
            println!("Recent gates:");
            for gate in &report.recent_gates {
                print_agent_gate_summary(gate, "  ");
            }
        }
        println!("Next:");
        println!("  {}", report.next.command);
        println!("  {}", report.next.reason);
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_test_plan(
    report: &AgentTestPlanReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Agent test plan: {}",
            agent_task_display_title(&report.task)
        );
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("Status: {}", report.status);
        println!("{}", report.summary);
        print_agent_risk_line(&report.risk);
        println!("Validation: {}", report.validation.status);
        println!("Next:");
        println!("  {}", report.next.command);
        println!("  {}", report.next.reason);
        if report.steps.is_empty() {
            println!("Steps: none");
        } else {
            println!("Steps:");
            for step in &report.steps {
                let required = if step.required {
                    "required"
                } else {
                    "optional"
                };
                println!(
                    "  {}. {} [{}; {}]",
                    step.rank, step.label, step.state, required
                );
                println!("     {}", step.command);
                println!("     {}", step.reason);
                if let Some(area) = &step.area_label {
                    println!("     Area: {area}");
                }
                if !step.paths.is_empty() {
                    print_changed_paths(&step.paths, "     ");
                }
                if let Some(gate) = &step.latest_gate {
                    println!("     Latest gate:");
                    print_agent_gate_summary(gate, "       ");
                }
            }
        }
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_report(
    report: &AgentReviewBundleReport,
    json: bool,
    quiet: bool,
    markdown: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if markdown {
        print!("{}", report.markdown);
        return Ok(());
    }
    if !quiet {
        println!("Agent report: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("{}", report.summary);
        println!(
            "Readiness: {}  Ready: {}",
            report.readiness_status, report.ready_to_apply
        );
        print_agent_risk_line(&report.risk);
        println!("Next:");
        println!("  {}", report.next.command);
        println!("  {}", report.next.reason);
        if !report.review.readiness.blockers.is_empty() {
            println!("Blockers:");
            for blocker in &report.review.readiness.blockers {
                println!("  {}: {}", blocker.code, blocker.message);
            }
        }
        if !report.review.readiness.warnings.is_empty() {
            println!("Warnings:");
            for warning in &report.review.readiness.warnings {
                println!("  {}: {}", warning.code, warning.message);
            }
        }
        if !report.story.turn_summaries.is_empty() {
            println!("Turns:");
            for turn in &report.story.turn_summaries {
                print_agent_story_turn(turn);
            }
        }
        if !report.task.changed_paths.is_empty() {
            println!("Changed files:");
            print_changed_paths(&report.task.changed_paths, "  ");
        }
        println!("Markdown:");
        println!("  crabdb agent report {} --markdown", report.task.lane);
        if report.suggestions.len() > 1 {
            println!("Other useful commands:");
            for suggestion in report.suggestions.iter().skip(1) {
                println!("  {}", suggestion.command);
                println!("  {}", suggestion.reason);
            }
        }
    }
    Ok(())
}

pub(crate) fn render_agent_receipt(
    report: &AgentReceiptReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        print!("{}", report.markdown);
    }
    Ok(())
}

pub(crate) fn render_agent_handoff(
    report: &AgentHandoffReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        print!("{}", report.markdown);
    }
    Ok(())
}

pub(crate) fn render_agent_pr(
    report: &AgentPrDraftReport,
    json: bool,
    quiet: bool,
    title_only: bool,
    body_only: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if title_only {
            println!("{}", report.title);
        } else if body_only {
            print!("{}", report.body);
        } else {
            println!("{}", report.title);
            println!();
            print!("{}", report.body);
        }
    }
    Ok(())
}

pub(crate) fn render_agent_story(report: &AgentStoryReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent story: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("{}", report.summary);
        if !report.turn_summaries.is_empty() {
            println!("Turns:");
            for turn in &report.turn_summaries {
                print_agent_story_turn(turn);
            }
        }
        if !report.changed_files.is_empty() {
            println!("Changed files:");
            print_changed_paths(&report.changed_files, "  ");
        }
        if !report.tool_summaries.is_empty() {
            println!("Tools:");
            for tool in &report.tool_summaries {
                println!("  {tool}");
            }
        }
        if !report.risk_notes.is_empty() {
            println!("Notes:");
            for note in &report.risk_notes {
                println!("  {note}");
            }
        }
        println!("Next:");
        println!("  {}", report.next.command);
        println!("  {}", report.next.reason);
        if report.suggestions.len() > 1 {
            println!("Other useful commands:");
            for suggestion in report.suggestions.iter().skip(1) {
                println!("  {}", suggestion.command);
                println!("  {}", suggestion.reason);
            }
        }
    }
    Ok(())
}

pub(crate) fn render_agent_tools(report: &AgentToolsReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent tools: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("{}", report.summary);
        println!(
            "Tool events: {}  Tools: {}  Turns with tools: {}",
            report.total_tool_events, report.unique_tools, report.turns_with_tools
        );
        if report.available_commands.is_empty() {
            println!("Available commands: none captured");
        } else {
            println!("Available commands:");
            for command in report.available_commands.iter().take(20) {
                println!("  {command}");
            }
            if report.available_commands.len() > 20 {
                println!("  ... {} more", report.available_commands.len() - 20);
            }
        }
        if report.tools.is_empty() {
            println!("Tool usage: none captured");
        } else {
            println!("Tool usage:");
            for tool in &report.tools {
                println!();
                println!(
                    "  {}. {}{}  {} event(s) across {} turn(s)",
                    tool.rank,
                    tool.name,
                    tool.kind
                        .as_ref()
                        .map(|kind| format!(" ({kind})"))
                        .unwrap_or_default(),
                    tool.event_count,
                    tool.turn_count
                );
                if !tool.statuses.is_empty() {
                    let statuses = tool
                        .statuses
                        .iter()
                        .map(|(status, count)| format!("{status}:{count}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!("     Statuses: {statuses}");
                }
                if !tool.event_types.is_empty() {
                    println!("     Events: {}", tool.event_types.join(", "));
                }
                println!(
                    "     Changed files around this tool: {}",
                    tool.changed_paths.len()
                );
                print_changed_paths(&tool.changed_paths, "       ");
                println!("     Turns:");
                for turn in &tool.turns {
                    let prompt = turn
                        .prompt_preview
                        .as_deref()
                        .map(|value| single_line_preview(value, 120))
                        .unwrap_or_else(|| "no prompt preview".to_string());
                    println!("       turn {} {} - {}", turn.index, turn.status, prompt);
                    if let Some(checkpoint) = &turn.checkpoint {
                        println!("         checkpoint: {}", checkpoint.0);
                    }
                    println!("         inspect: {}", turn.turn_command);
                    if let Some(command) = &turn.diff_command {
                        println!("         diff: {command}");
                    }
                }
            }
        }
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_impact(
    report: &AgentImpactReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent impact: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("{}", report.summary);
        println!(
            "Changed files: {}  Changed lines: {}  Highest impact: {}",
            report.changed_paths.len(),
            report.changed_lines,
            report.highest_impact
        );
        print_agent_risk_line(&report.risk);
        println!("Validation: {}", report.validation.status);
        if report.areas.is_empty() {
            println!("Impact areas: none");
        } else {
            println!("Impact areas:");
            for area in &report.areas {
                println!();
                println!(
                    "  {} ({})  {} file(s), {} changed line(s)",
                    area.label,
                    area.severity,
                    area.changed_paths.len(),
                    area.changed_lines
                );
                if !area.reasons.is_empty() {
                    println!("     Why: {}", area.reasons.join(", "));
                }
                print_changed_paths(&area.changed_paths, "     ");
                println!("     Review: {}", area.review_command);
                if let Some(command) = &area.diff_command {
                    println!("     Patch: {command}");
                }
            }
        }
        if !report.recommendations.is_empty() {
            println!("Recommended checks:");
            for recommendation in &report.recommendations {
                println!("  {}", recommendation.command);
                println!("  {}", recommendation.reason);
            }
        }
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_review_map(
    report: &AgentReviewMapReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Agent review map: {}",
            agent_task_display_title(&report.task)
        );
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("{}", report.summary);
        println!(
            "Review: {}  Changed files: {}  Changed lines: {}  Highest impact: {}",
            report.review_status,
            report.changed_paths.len(),
            report.changed_lines,
            report.highest_impact
        );
        print_agent_risk_line(&report.risk);
        println!("Validation: {}", report.validation.status);
        println!("Next:");
        println!("  {}", report.next.command);
        println!("  {}", report.next.reason);
        if report.areas.is_empty() {
            println!("Review areas: none");
        } else {
            println!("Review areas:");
            for area in &report.areas {
                println!();
                println!(
                    "  {} ({}, {})  {} file(s), {} changed line(s)",
                    area.label,
                    area.severity,
                    area.state,
                    area.files.len(),
                    area.changed_lines
                );
                if !area.reasons.is_empty() {
                    println!("     Why: {}", area.reasons.join(", "));
                }
                println!("     Start: {}", area.review_command);
                if let Some(command) = &area.patch_command {
                    println!("     Patch: {command}");
                }
                for file in &area.files {
                    println!(
                        "     {}. {} [{}] +{} -{} score {}",
                        file.rank,
                        file.path,
                        file.state,
                        file.change.additions,
                        file.change.deletions,
                        file.score
                    );
                    if let Some(marker) = &file.reviewed {
                        println!("        Reviewed at: {}", marker.checkpoint.0);
                    }
                    if !file.reasons.is_empty() {
                        println!("        Reasons: {}", file.reasons.join(", "));
                    }
                    println!("        Review: {}", file.review_command);
                    if let Some(command) = &file.open_command {
                        println!("        Open: {command}");
                    }
                    println!("        Why: {}", file.why_command);
                    if let Some(command) = &file.diff_command {
                        println!("        Patch: {command}");
                    }
                }
            }
        }
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_risk(report: &AgentRiskReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent risk: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("Risk: {:?} ({}/100)", report.level, report.score);
        println!("{}", report.summary);
        if report.reasons.is_empty() {
            println!("Reasons: none");
        } else {
            println!("Reasons:");
            for reason in &report.reasons {
                println!(
                    "  [{}] {}: {}",
                    reason.severity, reason.code, reason.message
                );
            }
        }
        if !report.recommendations.is_empty() {
            println!("Recommendations:");
            for recommendation in &report.recommendations {
                println!("  {}", recommendation.command);
                println!("  {}", recommendation.reason);
            }
        }
    }
    Ok(())
}

pub(crate) fn render_agent_confidence(
    report: &AgentConfidenceReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Agent confidence: {}",
            agent_task_display_title(&report.task)
        );
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("Verdict: {}  Score: {}/100", report.verdict, report.score);
        println!("{}", report.summary);
        println!(
            "Review: {}  Validation: {}  Apply: {}",
            report.review_status, report.validation.status, report.ready.status
        );
        print_agent_risk_line(&report.risk);
        if let Some(marker) = &report.reviewed {
            println!("Reviewed checkpoint: {}", marker.checkpoint.0);
        }
        if report.factors.is_empty() {
            println!("Factors: none");
        } else {
            println!("Factors:");
            for factor in &report.factors {
                println!(
                    "  [{}] {} ({})",
                    factor.state, factor.name, factor.score_delta
                );
                println!("      {}", factor.message);
                if let Some(command) = &factor.command {
                    println!("      {command}");
                }
            }
        }
        println!("Next:");
        println!("  {}", report.next.command);
        println!("  {}", report.next.reason);
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_ready(report: &AgentReadyReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent ready: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("Ready: {}", report.ready);
        println!("Status: {}", report.status);
        println!("Readiness: {}", report.readiness_status);
        print_agent_risk_line(&report.risk);
        println!("Default commit message: {}", report.default_apply_message);
        println!("{}", report.summary);
        if !report.blockers.is_empty() {
            println!("Blockers:");
            for blocker in &report.blockers {
                println!("  {}: {}", blocker.code, blocker.message);
            }
        }
        if !report.warnings.is_empty() {
            println!("Warnings:");
            for warning in &report.warnings {
                println!("  {}: {}", warning.code, warning.message);
            }
        }
        if let Some(error) = &report.apply_error {
            println!("Git preflight: failed");
            println!("  {error}");
        } else if let Some(preview) = &report.apply_preview {
            println!("Git preflight: {}", preview.status);
            if let Some(branch) = &preview.git_apply_plan.git_branch {
                println!("Into Git branch: {branch}");
            }
            if let Some(range) = &preview.git_apply_plan.range {
                println!("CrabDB range: {range}");
            }
            if preview.git_apply_plan.would_record {
                println!("Would record task workdir before applying");
            }
            if preview.git_apply_plan.would_create_git_commit {
                println!("Would create Git commit");
            }
            if preview.git_apply_plan.would_fast_forward {
                println!("Would fast-forward current Git branch");
            }
        }
        println!("Next:");
        println!("  {}", report.next.command);
        println!("  {}", report.next.reason);
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_diagnose(
    report: &AgentDiagnosisReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent diagnose: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("Status: {}  Severity: {}", report.status, report.severity);
        println!(
            "Ready: {}  Readiness: {}",
            report.ready, report.readiness_status
        );
        print_agent_risk_line(&report.risk);
        println!("{}", report.summary);
        println!("Likely issue:");
        println!("  {}", report.likely_issue);
        if !report.evidence.is_empty() {
            println!("Evidence:");
            for item in &report.evidence {
                println!("  {item}");
            }
        }
        if !report.blockers.is_empty() {
            println!("Blockers:");
            for blocker in &report.blockers {
                println!("  {}: {}", blocker.code, blocker.message);
            }
        }
        if !report.warnings.is_empty() {
            println!("Warnings:");
            for warning in &report.warnings {
                println!("  {}: {}", warning.code, warning.message);
            }
        }
        if !report.checkpoints.is_empty() {
            println!("Recent recovery targets:");
            for checkpoint in &report.checkpoints {
                print_agent_checkpoint_entry(checkpoint);
            }
        }
        if !report.recovery_options.is_empty() {
            println!("Recovery options:");
            for option in &report.recovery_options {
                println!("  {}", option.command);
                println!("  {}", option.reason);
            }
        }
        println!("Next:");
        println!("  {}", report.next.command);
        println!("  {}", report.next.reason);
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_compare(
    report: &AgentCompareReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent compare");
        println!("{}", report.summary);
        println!();
        print_agent_compare_task("Left", &report.left, &report.left_risk);
        print_agent_compare_task("Right", &report.right, &report.right_risk);
        println!();
        println!("Shared changed files: {}", report.shared_paths.len());
        if !report.shared_paths.is_empty() {
            for path in &report.shared_paths {
                println!(
                    "  {}  left {:?} (+{} -{})  right {:?} (+{} -{})",
                    path.path,
                    path.left.kind,
                    path.left.additions,
                    path.left.deletions,
                    path.right.kind,
                    path.right.additions,
                    path.right.deletions
                );
            }
        }
        println!("Left only: {}", report.left_only_paths.len());
        print_changed_paths(&report.left_only_paths, "  ");
        println!("Right only: {}", report.right_only_paths.len());
        print_changed_paths(&report.right_only_paths, "  ");
        println!("Recommendation:");
        println!("  {}", report.recommendation.command);
        println!("  {}", report.recommendation.reason);
        if report.suggestions.len() > 1 {
            println!("Other useful commands:");
            for suggestion in report.suggestions.iter().skip(1) {
                println!("  {}", suggestion.command);
                println!("  {}", suggestion.reason);
            }
        }
    }
    Ok(())
}

pub(crate) fn render_agent_workdir(
    report: &AgentWorkdirReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent workdir: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        if let Some(workdir) = &report.workdir {
            println!("Workdir: {workdir}");
            if let Some(command) = &report.cd_command {
                println!("Command:");
                println!("  {command}");
            }
        } else {
            println!("No materialized workdir recorded for this task");
        }
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_view(
    report: &AgentTaskViewReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        print_agent_task_summary(&report.task);
        if !report.task.changed_paths.is_empty() {
            println!("Changed paths:");
            for path in &report.task.changed_paths {
                println!("  {:?} {}", path.kind, path.path);
            }
        }
        if let Some(transcript) = &report.transcript {
            println!("Transcript: {} turn(s)", transcript.turns.len());
            for turn in &transcript.turns {
                println!("  Turn {} {}", turn.turn.turn_id, turn.turn.status);
                for message in &turn.messages {
                    println!(
                        "    {}: {}",
                        message.role,
                        single_line_preview(&message.body, 120)
                    );
                }
                for tool in &turn.tool_summaries {
                    println!("    tool: {tool}");
                }
            }
        }
        print_suggestions(&report.task.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_changes(
    report: &AgentChangesReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent changes: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("Status: {:?}", report.task.status);
        println!("{}", report.summary);
        println!("Range: {}..{}", report.base_change.0, report.head_change.0);
        println!(
            "Grouping: {}  Total changed files: {}",
            report.grouping,
            report.total_changed_paths.len()
        );
        println!("Next:");
        println!("  {}", report.next.command);
        println!("  {}", report.next.reason);
        if !report.cards.is_empty() {
            println!("Change cards:");
            for card in &report.cards {
                print_agent_change_card(card);
            }
        }
        if report.groups.is_empty() {
            println!("No recorded changes");
        } else {
            println!("Details:");
            for group in &report.groups {
                print_agent_change_group(group);
            }
        }
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_delta(report: &AgentDeltaReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent delta: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("Mode: {}  Matched: {}", report.mode, report.matched);
        if let Some(path) = &report.file_filter {
            println!("File: {path}");
        }
        println!("{}", report.summary);
        if let Some(group) = &report.group {
            println!("Newest delta:");
            print_agent_change_group(group);
        } else {
            println!("Newest delta: none");
        }
        println!("Changed files: {}", report.changed_paths.len());
        print_changed_paths(&report.changed_paths, "  ");
        if let Some(diff) = &report.diff {
            if diff.diff.files.iter().any(|file| file.patch.is_some()) {
                println!("Patch:");
                print_diff_files(&diff.diff.files, true);
            }
        }
        println!("Next:");
        println!("  {}", report.next.command);
        println!("  {}", report.next.reason);
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_new(report: &AgentNewReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent new: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("Status: {}  Matched: {}", report.status, report.matched);
        if let Some(marker) = &report.reviewed {
            println!(
                "Reviewed: {} at {}",
                marker.checkpoint.0, marker.reviewed_at
            );
        } else {
            println!("Reviewed: not marked yet");
        }
        if let Some(path) = &report.file_filter {
            println!("File: {path}");
        }
        println!("Range: {}..{}", report.base_change.0, report.head_change.0);
        println!("{}", report.summary);
        if report.new_groups.is_empty() {
            println!("New turns or operations: none");
        } else {
            println!("New turns or operations:");
            for group in &report.new_groups {
                print_agent_change_group(group);
            }
        }
        println!("Changed files: {}", report.changed_paths.len());
        print_changed_paths(&report.changed_paths, "  ");
        if let Some(diff) = &report.diff {
            if diff.diff.files.iter().any(|file| file.patch.is_some()) {
                println!("Patch:");
                print_diff_files(&diff.diff.files, true);
            }
        }
        println!("Next:");
        println!("  {}", report.next.command);
        println!("  {}", report.next.reason);
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_mark_reviewed(
    report: &AgentMarkReviewedReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent reviewed: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("Checkpoint: {}", report.marker.checkpoint.0);
        println!("Changed files: {}", report.marker.changed_paths);
        if let Some(previous) = &report.previous {
            println!("Previous reviewed: {}", previous.checkpoint.0);
        }
        if let Some(note) = &report.marker.note {
            println!("Note: {note}");
        }
        println!("{}", report.summary);
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_mark_file_reviewed(
    report: &AgentMarkFileReviewedReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent file reviewed: {}", report.path);
        println!("Task: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("Checkpoint: {}", report.marker.checkpoint.0);
        if let Some(note) = &report.marker.note {
            println!("Note: {note}");
        }
        if let Some(previous) = &report.previous {
            println!("Previous file marker: {}", previous.checkpoint.0);
        }
        println!("{}", report.summary);
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_archive(
    report: &AgentArchiveReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent archive: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!(
            "Archived: {}  Previous: {}",
            report.archived, report.previous_archived
        );
        if let Some(note) = &report.note {
            println!("Note: {note}");
        }
        println!("{}", report.summary);
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_timeline(
    report: &AgentTimelineReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent timeline: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("Status: {:?}", report.task.status);
        println!("{}", report.summary);
        println!("Range: {}..{}", report.base_change.0, report.head_change.0);
        println!(
            "Mode: {}  Items: {}  Changed files: {}",
            report.mode,
            report.items.len(),
            report.task.changed_paths.len()
        );
        if report.items.is_empty() {
            println!("No recorded timeline items");
        } else {
            println!("Timeline:");
            for item in &report.items {
                print_agent_timeline_item(item);
            }
        }
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_change_set(
    report: &AgentChangeSetReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Agent change set: {}",
            agent_task_display_title(&report.task)
        );
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!(
            "Card: {}. {} ({})  {:?}",
            report.card.rank, report.card.title, report.card.key, report.card.risk
        );
        println!("{}", report.summary);
        if !report.card.reasons.is_empty() {
            println!("Review notes: {}", report.card.reasons.join(", "));
        }
        println!("Files:");
        if report.files.is_empty() {
            println!("  No changed files recorded");
        } else {
            for file in &report.files {
                print_agent_file_entry(file);
            }
        }
        if !report.groups.is_empty() {
            println!("Changed in:");
            for group in &report.groups {
                print_agent_change_group(group);
            }
        }
        if !report.diffs.is_empty() {
            println!("Focused patches:");
            for diff in &report.diffs {
                println!();
                println!(
                    "  {} {}",
                    diff.target_kind,
                    diff.file_filter.as_deref().unwrap_or(&diff.target)
                );
                print_diff_files(&diff.diff.files, true);
            }
        }
        println!("Next:");
        println!("  {}", report.next.command);
        println!("  {}", report.next.reason);
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_files(report: &AgentFilesReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent files: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!(
            "Grouping: {}  Changed files: {}",
            report.grouping,
            report.files.len()
        );
        if report.files.is_empty() {
            println!("No changed files recorded");
        } else {
            for file in &report.files {
                print_agent_file_entry(file);
            }
        }
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_file(report: &AgentFileReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent file: {}", report.path);
        println!("Task: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("Matched: {}", report.matched);
        println!("{}", report.summary);
        if let Some(file) = &report.file {
            println!("File entry:");
            print_agent_file_entry(file);
        }
        if !report.change_cards.is_empty() {
            println!("Change sets:");
            for card in &report.change_cards {
                print_agent_change_card(card);
            }
        }
        if !report.groups.is_empty() {
            println!("Changed in:");
            for group in &report.groups {
                print_agent_change_group(group);
            }
        }
        if let Some(diff) = &report.diff {
            println!("Focused patch:");
            print_diff_files(&diff.diff.files, true);
        }
        println!("Next:");
        println!("  {}", report.next.command);
        println!("  {}", report.next.reason);
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_checkpoints(
    report: &AgentCheckpointReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Agent checkpoints: {}",
            agent_task_display_title(&report.task)
        );
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("Base: {}", report.base_change.0);
        println!("Head: {}", report.head_change.0);
        if report.entries.is_empty() {
            println!("No turn or operation checkpoints recorded");
        } else {
            println!("Targets:");
            for entry in &report.entries {
                print_agent_checkpoint_entry(entry);
            }
        }
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_why(report: &AgentWhyReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent why: {}", report.path);
        println!("Task: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("{}", report.summary);
        if let Some(change) = &report.task_change {
            println!(
                "Task change: {:?} {} (+{} -{})",
                change.kind, change.path, change.additions, change.deletions
            );
            if let Some(old_path) = &change.old_path {
                println!("  Old path: {old_path}");
            }
        }
        if report.groups.is_empty() {
            println!("No prompt, turn, or operation changed this file in the selected task.");
        } else {
            println!("Changed in:");
            for group in &report.groups {
                print_agent_change_group(group);
            }
        }
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_turn(report: &AgentTurnReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Agent turn {}: {}",
            report.index,
            agent_task_display_title(&report.task)
        );
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("Status: {}", report.status);
        println!("Turn id: {}", report.turn_id);
        if let Some(prompt) = &report.prompt_preview {
            println!("Prompt: {prompt}");
        }
        if let Some(assistant) = &report.assistant_preview {
            println!("Assistant: {assistant}");
        }
        println!(
            "Messages: {}  Events: {}  Tools: {}",
            report.messages.len(),
            report.event_count,
            report.tool_summaries.len()
        );
        println!("Before: {}", report.before_change.0);
        if let Some(checkpoint) = &report.checkpoint {
            println!("Checkpoint: {}", checkpoint.0);
        } else {
            println!("Checkpoint: none");
        }
        println!("Changed files: {}", report.changed_paths.len());
        print_changed_paths(&report.changed_paths, "  ");
        for tool in &report.tool_summaries {
            println!("  tool: {tool}");
        }
        if let Some(diff) = &report.diff {
            if diff.file_filter.is_some() || diff.diff.files.iter().any(|file| file.patch.is_some())
            {
                println!("Diff:");
                if let Some(path) = &diff.file_filter {
                    println!("  File: {path}");
                }
                print_diff_files(&diff.diff.files, false);
            }
        }
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_diff(
    report: &AgentDiffReport,
    json: bool,
    quiet: bool,
    stat: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent diff: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("Target: {} {}", report.target_kind, report.target);
        if let Some(turn_id) = &report.turn_id {
            println!("Turn: {turn_id}");
        }
        if let Some(operation_id) = &report.operation_id {
            println!("Operation: {}", operation_id.0);
        }
        if let Some(path) = &report.file_filter {
            println!("File: {path}");
        }
        println!(
            "Range: {}..{}",
            report.before_change.0, report.after_change.0
        );
        print_diff_files(&report.diff.files, stat);
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_review(
    report: &AgentReviewReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent review: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!(
            "Status: {:?}  Ready: {}  Changed files: {}",
            report.task.status,
            report.ready_to_apply,
            report.task.changed_paths.len()
        );
        println!(
            "Transcript: {} turn(s)  Tool events: {}",
            report.transcript_turns, report.tool_events
        );
        if let Some(checkpoint) = &report.latest_checkpoint {
            println!("Last checkpoint: {}", checkpoint.0);
        }
        print_agent_risk_line(&report.risk);
        println!("{}", report.summary);
        println!("Next:");
        println!("  {}", report.next.command);
        println!("  {}", report.next.reason);
        if !report.blockers.is_empty() {
            println!("Blockers:");
            for blocker in &report.blockers {
                println!("  {}: {}", blocker.code, blocker.message);
            }
        }
        if !report.warnings.is_empty() {
            println!("Warnings:");
            for warning in &report.warnings {
                println!("  {}: {}", warning.code, warning.message);
            }
        }
        if report.priorities.is_empty() {
            println!("Review priority: no changed files recorded");
        } else {
            println!("Review priority:");
            for priority in report.priorities.iter().take(8) {
                print_agent_review_priority(priority);
            }
            if report.priorities.len() > 8 {
                println!(
                    "  ... {} more file(s); run `crabdb agent files {}`",
                    report.priorities.len() - 8,
                    report.task.lane
                );
            }
        }
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_focus(report: &AgentFocusReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent focus: {}", report.path);
        println!("Task: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        if let Some(open_path) = &report.open_path {
            println!("Open path: {open_path}");
        }
        if let Some(open_command) = &report.open_command {
            println!("Open command:");
            println!("  {open_command}");
        }
        println!("Source: {}", report.source);
        println!("{}", report.summary);
        if let Some(priority) = &report.priority {
            println!("Review priority:");
            print_agent_review_priority(priority);
        }
        if report.why.groups.is_empty() {
            println!("Changed in: no captured turn or operation");
        } else {
            println!("Changed in:");
            for group in &report.why.groups {
                print_agent_change_group(group);
            }
        }
        println!("Focused diff:");
        println!(
            "  Target: {} {}",
            report.diff.target_kind, report.diff.target
        );
        println!(
            "  Range: {}..{}",
            report.diff.before_change.0, report.diff.after_change.0
        );
        print_diff_files(&report.diff.diff.files, true);
        println!("Next:");
        println!("  {}", report.next.command);
        println!("  {}", report.next.reason);
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_apply(report: &AgentApplyReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.dry_run {
            println!(
                "Would apply agent task: {}",
                agent_task_display_title(&report.task)
            );
        } else {
            println!(
                "Applied agent task: {}",
                agent_task_display_title(&report.task)
            );
        }
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        if let Some(branch) = &report.git_apply_plan.git_branch {
            println!("Into Git branch: {branch}");
        }
        println!("CrabDB base: {}", report.git_apply_plan.crab_branch);
        println!("Changed files: {}", report.task.changed_paths.len());
        println!("Result: {}", report.status);
        if report.git_apply_plan.would_record {
            println!("Would record lane workdir before applying");
        }
        if let Some(range) = &report.git_apply_plan.range {
            println!("CrabDB range: {range}");
        }
        if let Some(export) = &report.git_export {
            println!("Git commit: {}", export.commit);
        }
        if report.fast_forwarded {
            println!("Fast-forwarded current Git branch");
        }
        for warning in &report.warnings {
            println!("warning: {warning}");
        }
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_finish(
    report: &AgentFinishReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.dry_run {
            println!(
                "Would finish agent task: {}",
                agent_task_display_title(&report.task)
            );
        } else {
            println!(
                "Finished agent task: {}",
                agent_task_display_title(&report.task)
            );
        }
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("Apply: {}", report.apply.status);
        if let Some(export) = &report.apply.git_export {
            println!("Git commit: {}", export.commit);
        }
        if report.apply.fast_forwarded {
            println!("Fast-forwarded current Git branch");
        }
        if report.dry_run {
            println!("Archive after apply: {}", report.would_archive);
        } else if let Some(archive) = &report.archive {
            println!("Archived: {}", archive.archived);
        } else if report.task.archived {
            println!("Archived: already");
        } else {
            println!("Archived: no");
        }
        println!("Result: {}", report.status);
        for warning in &report.apply.warnings {
            println!("warning: {warning}");
        }
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_run(report: &AgentRunReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent task: {}", agent_task_display_title(&report.task));
        print_agent_task_id_if_needed(&report.task);
        print_agent_task_workdir(&report.task);
        println!("Status: {}", report.status);
        if let Some(code) = report.exit_code {
            println!("Exit code: {code}");
        }
        if let Some(recorded) = &report.recorded {
            if let Some(operation) = &recorded.operation {
                println!("Checkpoint: {}", operation.0);
            }
        }
        print_suggestions(&report.task.suggestions);
    }
    Ok(())
}

pub(crate) fn render_agent_continue(
    report: &AgentContinueReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Agent follow-up: {}",
            agent_task_display_title(&report.run.task)
        );
        println!(
            "From: {} ({})",
            agent_task_display_title(&report.source_task),
            report.from_change.0
        );
        print_agent_task_id_if_needed(&report.run.task);
        print_agent_task_workdir(&report.run.task);
        println!("Status: {}", report.run.status);
        if let Some(code) = report.run.exit_code {
            println!("Exit code: {code}");
        }
        if let Some(recorded) = &report.run.recorded {
            if let Some(operation) = &recorded.operation {
                println!("Checkpoint: {}", operation.0);
            }
        }
        print_suggestions(&report.suggestions);
    }
    Ok(())
}

fn print_agent_task_summary(task: &AgentTaskReport) {
    println!("Agent task: {}", agent_task_display_title(task));
    print_agent_task_id_if_needed(task);
    print_agent_task_workdir(task);
    println!("Status: {:?}", task.status);
    if task.archived {
        println!("Archived: yes");
    }
    if let Some(provider) = &task.provider {
        println!("Provider: {provider}");
    }
    println!("Changed files: {}", task.changed_paths.len());
    println!("Turns: {}  Tool events: {}", task.turns, task.tool_events);
    if let Some(checkpoint) = &task.latest_checkpoint {
        println!("Last checkpoint: {}", checkpoint.0);
    }
}

fn print_agent_inbox_group(group: &AgentInboxGroup) {
    println!();
    println!("{}: {}", group.label, group.items.len());
    for item in &group.items {
        let task = &item.task;
        let checkpoint = task
            .latest_checkpoint
            .as_ref()
            .map(|change| change.0.as_str())
            .unwrap_or("-");
        println!(
            "  {}{}  {} file(s)  {} turn(s)  checkpoint {}",
            agent_task_display_title(task),
            if task.archived { " [archived]" } else { "" },
            task.changed_paths.len(),
            task.turns,
            checkpoint
        );
        if agent_task_display_title(task) != task.name {
            println!("    Task id: {}", task.name);
        }
        if let Some(workdir) = &task.workdir {
            println!("    Workdir: {workdir}");
        }
        println!("    Attention: {} - {}", item.attention, item.detail);
        if item.new_changed_paths > 0 {
            println!(
                "    New since review: {} file(s), {} line(s)",
                item.new_changed_paths, item.new_changed_lines
            );
        }
        if let Some(target) = &item.review_first {
            println!("    Review first: {} ({})", target.path, target.reason);
            println!("      {}", target.command);
        }
        println!("    Next: {}", item.next.command);
    }
    if let Some(next) = &group.next {
        println!("  Group next: {}", next.command);
        println!("  {}", next.reason);
    }
}

fn print_agent_board_column(column: &AgentBoardColumn) {
    println!();
    println!("{}: {}", column.label, column.items.len());
    println!("  {}", column.summary);
    for item in &column.items {
        let task = &item.task;
        let checkpoint = task
            .latest_checkpoint
            .as_ref()
            .map(|change| change.0.as_str())
            .unwrap_or("-");
        println!(
            "  {}{}  [{}]  {} file(s)  {} turn(s)  checkpoint {}",
            agent_task_display_title(task),
            if task.archived { " [archived]" } else { "" },
            item.status_label,
            item.changed_paths,
            item.turns,
            checkpoint
        );
        if agent_task_display_title(task) != task.name {
            println!("    Task id: {}", task.name);
        }
        println!("    {}", item.detail);
        if let Some(target) = &item.review_first {
            println!("    Review first: {} ({})", target.path, target.reason);
        }
        println!("    Next: {}", item.next.command);
    }
    if let Some(next) = &column.next {
        println!("  Column next: {}", next.command);
        println!("  {}", next.reason);
    }
}

fn agent_task_display_title(task: &AgentTaskReport) -> &str {
    if task.title.trim().is_empty() {
        &task.name
    } else {
        &task.title
    }
}

fn print_agent_task_id_if_needed(task: &AgentTaskReport) {
    if agent_task_display_title(task) != task.name {
        println!("Task id: {}", task.name);
    }
}

fn print_agent_task_workdir(task: &AgentTaskReport) {
    if let Some(workdir) = &task.workdir {
        println!("Workdir: {workdir}");
    }
}

fn print_agent_risk_line(risk: &AgentRiskReport) {
    let reason_codes = risk
        .reasons
        .iter()
        .take(3)
        .map(|reason| reason.code.as_str())
        .collect::<Vec<_>>();
    if reason_codes.is_empty() {
        println!("Risk: {:?} ({}/100)", risk.level, risk.score);
    } else {
        println!(
            "Risk: {:?} ({}/100) {}",
            risk.level,
            risk.score,
            reason_codes.join(", ")
        );
    }
}

fn print_agent_compare_task(label: &str, task: &AgentTaskReport, risk: &AgentRiskReport) {
    println!(
        "{label}: {}  {:?}  {} file(s)  {} turn(s)",
        agent_task_display_title(task),
        task.status,
        task.changed_paths.len(),
        task.turns
    );
    if agent_task_display_title(task) != task.name {
        println!("  Task id: {}", task.name);
    }
    println!("  Lane: {}", task.lane);
    println!("  Risk: {:?} ({}/100)", risk.level, risk.score);
    if let Some(workdir) = &task.workdir {
        println!("  Workdir: {workdir}");
    }
}

fn print_agent_change_group(group: &AgentChangeGroup) {
    match group.kind.as_str() {
        "turn" => {
            let prompt = group
                .prompt_preview
                .as_deref()
                .unwrap_or("(no prompt captured)");
            println!();
            println!("Turn {}: {prompt}", group.index);
        }
        _ => {
            println!();
            println!("Operation {}: {}", group.index, group.id);
        }
    }
    if let Some(status) = &group.status {
        println!("  Status: {status}");
    }
    if let Some(checkpoint) = &group.checkpoint {
        println!("  Checkpoint: {}", checkpoint.0);
    }
    if let Some(assistant) = &group.assistant_preview {
        println!("  Assistant: {assistant}");
    }
    if !group.operations.is_empty() {
        let operations = group
            .operations
            .iter()
            .map(|change| change.0.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        println!("  Operations: {operations}");
    }
    println!("  Changed files: {}", group.changed_paths.len());
    print_changed_paths(&group.changed_paths, "    ");
    for tool in &group.tool_summaries {
        println!("    tool: {tool}");
    }
}

fn print_agent_change_card(card: &AgentChangeCard) {
    println!();
    println!("  {}. {}  {:?}", card.rank, card.title, card.risk);
    println!("     {}", card.summary);
    if !card.reasons.is_empty() {
        println!("     Review notes: {}", card.reasons.join(", "));
    }
    if !card.touched_by.is_empty() {
        let touched_by = card
            .touched_by
            .iter()
            .map(|touch| match touch.kind.as_str() {
                "turn" => format!("turn {}", touch.index),
                _ => format!("operation {}", touch.index),
            })
            .collect::<Vec<_>>()
            .join(", ");
        println!("     Touched by: {touched_by}");
    }
    println!("     Files:");
    print_changed_paths(&card.changed_paths, "       ");
    for tool in &card.tool_summaries {
        println!("       tool: {tool}");
    }
    println!("     Review: {}", card.review_command);
    if let Some(command) = &card.focus_command {
        println!("     Focus: {command}");
    }
    if let Some(command) = &card.why_command {
        println!("     Why: {command}");
    }
    if let Some(command) = &card.diff_command {
        println!("     Diff: {command}");
    }
}

fn print_agent_timeline_item(item: &AgentTimelineItem) {
    println!();
    println!("  {}", item.title);
    if let Some(status) = &item.status {
        println!("    Status: {status}");
    }
    if let Some(assistant) = &item.assistant_preview {
        println!("    Assistant: {assistant}");
    }
    println!(
        "    Messages: {}  Events: {}  Tools: {}",
        item.message_count,
        item.event_count,
        item.tool_summaries.len()
    );
    if let Some(before) = &item.before_change {
        println!("    Before: {}", before.0);
    }
    if let Some(checkpoint) = &item.checkpoint {
        println!("    Checkpoint: {}", checkpoint.0);
    }
    if !item.operations.is_empty() {
        let operations = item
            .operations
            .iter()
            .map(|change| change.0.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        println!("    Operations: {operations}");
    }
    println!("    Changed files: {}", item.changed_paths.len());
    print_changed_paths(&item.changed_paths, "      ");
    for tool in &item.tool_summaries {
        println!("      tool: {tool}");
    }
    if let Some(command) = &item.view_command {
        println!("    View: {command}");
    }
    if let Some(command) = &item.diff_command {
        println!("    Diff: {command}");
    }
    if let Some(command) = &item.rewind_before_command {
        println!("    Rewind before: {command}");
    }
}

fn print_agent_checkpoint_entry(entry: &AgentCheckpointEntry) {
    let label = entry
        .prompt_preview
        .as_deref()
        .map(|prompt| format!("{}: {prompt}", entry.label))
        .unwrap_or_else(|| entry.label.clone());
    println!();
    println!("  {label}");
    if let Some(before) = &entry.before_change {
        let target = entry.before_target.as_deref().unwrap_or(before.0.as_str());
        println!("    Before: {}  target `{target}`", before.0);
    }
    if let Some(checkpoint) = &entry.checkpoint {
        let target = entry
            .checkpoint_target
            .as_deref()
            .unwrap_or(checkpoint.0.as_str());
        println!("    Checkpoint: {}  target `{target}`", checkpoint.0);
    }
    println!("    Changed files: {}", entry.changed_paths.len());
    if let Some(command) = &entry.diff_command {
        println!("    Diff: {command}");
    }
    if let Some(command) = &entry.rewind_before_command {
        println!("    Rewind before: {command}");
    }
    if let Some(command) = &entry.rewind_checkpoint_command {
        println!("    Rewind to checkpoint: {command}");
    }
}

fn print_agent_file_entry(file: &AgentFileEntry) {
    println!();
    println!(
        "  {:?} {} (+{} -{})",
        file.change.kind, file.change.path, file.change.additions, file.change.deletions
    );
    if let Some(old_path) = &file.change.old_path {
        println!("    Old path: {old_path}");
    }
    if file.touched_by.is_empty() {
        println!("    Touched by: no captured turn or operation");
    } else {
        println!("    Touched by:");
        for touch in &file.touched_by {
            let label = match touch.kind.as_str() {
                "turn" => format!("turn {}", touch.index),
                _ => format!("operation {}", touch.index),
            };
            if let Some(prompt) = &touch.prompt_preview {
                println!("      {label}: {prompt}");
            } else {
                println!("      {label}: {}", touch.id);
            }
            if let Some(checkpoint) = &touch.checkpoint {
                println!("        checkpoint: {}", checkpoint.0);
            }
        }
    }
    println!("    Why: {}", file.why_command);
    if let Some(command) = &file.diff_command {
        println!("    Diff: {command}");
    }
}

fn print_agent_review_priority(priority: &AgentReviewPriority) {
    println!();
    println!(
        "  {}. {:?} {} (+{} -{})  score {}",
        priority.rank,
        priority.change.kind,
        priority.change.path,
        priority.change.additions,
        priority.change.deletions,
        priority.score
    );
    if !priority.reasons.is_empty() {
        println!("     Why review: {}", priority.reasons.join(", "));
    }
    if !priority.touched_by.is_empty() {
        let touches = priority
            .touched_by
            .iter()
            .map(|touch| match touch.kind.as_str() {
                "turn" => format!("turn {}", touch.index),
                _ => format!("operation {}", touch.index),
            })
            .collect::<Vec<_>>()
            .join(", ");
        println!("     Touched by: {touches}");
    }
    println!("     Why: {}", priority.why_command);
    if let Some(command) = &priority.diff_command {
        println!("     Diff: {command}");
    }
}

fn print_agent_brief_group(group: &AgentChangeGroup) {
    let label = match group.kind.as_str() {
        "turn" => format!("Turn {}", group.index),
        _ => format!("Operation {}", group.index),
    };
    let summary = group
        .prompt_preview
        .as_deref()
        .or(group.assistant_preview.as_deref())
        .unwrap_or(&group.id);
    println!(
        "  {label}: {summary} ({} changed file(s))",
        group.changed_paths.len()
    );
}

fn print_agent_story_turn(turn: &AgentStoryTurn) {
    let label = turn
        .prompt_preview
        .as_deref()
        .or(turn.outcome_preview.as_deref())
        .unwrap_or(&turn.id);
    println!(
        "  {}. {} ({} changed file(s))",
        turn.index,
        label,
        turn.changed_paths.len()
    );
    if let Some(checkpoint) = &turn.checkpoint {
        println!("     Checkpoint: {}", checkpoint.0);
    }
    for tool in &turn.tool_summaries {
        println!("     tool: {tool}");
    }
}

fn print_agent_brief_diff(diff: &DiffSummary) {
    let additions: u64 = diff.files.iter().map(|file| file.additions).sum();
    let deletions: u64 = diff.files.iter().map(|file| file.deletions).sum();
    println!(
        "Latest change: {} file(s), +{} -{}",
        diff.files.len(),
        additions,
        deletions
    );
}

fn print_changed_paths(paths: &[FileDiffSummary], indent: &str) {
    for path in paths {
        println!(
            "{}{:?} {} (+{} -{})",
            indent, path.kind, path.path, path.additions, path.deletions
        );
    }
}

fn print_diff_files(files: &[FileDiffSummary], stat: bool) {
    let mut total_additions = 0;
    let mut total_deletions = 0;
    if files.is_empty() {
        println!("No file changes");
    }
    for file in files {
        total_additions += file.additions;
        total_deletions += file.deletions;
        println!(
            "  {:?} {} (+{} -{})",
            file.kind, file.path, file.additions, file.deletions
        );
        if let Some(patch) = &file.patch {
            print!("{patch}");
        }
    }
    if stat {
        println!(
            "{} files changed, {} additions, {} deletions",
            files.len(),
            total_additions,
            total_deletions
        );
    }
}

fn print_suggestions(suggestions: &[StatusSuggestion]) {
    if !suggestions.is_empty() {
        println!("Next step:");
        for suggestion in suggestions {
            println!("  {}", suggestion.command);
            println!("  {}", suggestion.reason);
        }
    }
}

fn print_agent_gate_summary(gate: &LaneTestSummary, indent: &str) {
    let suite = gate.suite.as_deref().unwrap_or("default");
    let result = if gate.success { "passed" } else { "failed" };
    println!(
        "{indent}{} `{}` {} ({})",
        gate.kind,
        suite,
        result,
        shell_join(&gate.command)
    );
}

fn shell_join(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| {
            if part.chars().all(|ch| {
                ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '/' | '.' | '@' | ':')
            }) {
                part.clone()
            } else {
                format!("'{}'", part.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn single_line_preview(value: &str, limit: usize) -> String {
    let mut preview = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if preview.len() > limit {
        preview.truncate(limit.saturating_sub(3));
        preview.push_str("...");
    }
    preview
}
