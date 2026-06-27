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

pub(crate) fn render_agent_inbox(report: &AgentInboxReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Agent inbox");
        println!(
            "Tasks: {}  Need attention: {}",
            report.total, report.attention_count
        );
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
                "{} {:?} {} changed path(s)",
                agent_task_display_title(task),
                task.status,
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

fn print_agent_task_summary(task: &AgentTaskReport) {
    println!("Agent task: {}", agent_task_display_title(task));
    print_agent_task_id_if_needed(task);
    print_agent_task_workdir(task);
    println!("Status: {:?}", task.status);
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
            "  {}  {} file(s)  {} turn(s)  checkpoint {}",
            agent_task_display_title(task),
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
