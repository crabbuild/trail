use super::render_json;
use crate::cli::command::render::*;

use trail::model::*;
use trail::Result;

pub(crate) fn render_lane_status(
    report: &LaneStatusReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let stale = report
        .base_status
        .as_ref()
        .is_some_and(|base| base.stale || base.operations_behind.unwrap_or_default() > 0);
    let tone = if stale {
        UiTone::Attention
    } else {
        UiTone::Success
    };
    let mut document = TerminalDocument::new(
        format!(
            "Lane {} is {}",
            report.lane.record.name, report.lane.branch.status
        ),
        tone,
    )
    .context(format!(
        "{} changed path(s) · {} queued merge(s)",
        report.changed_paths.len(),
        report.queued_merges
    ))
    .block(lane_metadata(&report.lane));
    if let Some(base) = &report.base_status {
        let freshness = match base.operations_behind {
            Some(0) | None if !base.stale => "up to date".to_string(),
            Some(behind) => format!("{behind} operation(s) behind {}", base.target_branch),
            None => format!("stale against {}", base.target_branch),
        };
        document = document.block(UiBlock::Notice(format!("Base: {freshness}")));
    }
    if !report.changed_paths.is_empty() {
        document = document.block(UiBlock::Changes(change_list(&report.changed_paths)));
    }
    document = append_workdir(
        document,
        report.workdir_state.as_ref(),
        &report.workdir_changed_paths,
    );
    document = append_gate_summaries(
        document,
        report.latest_test.as_ref(),
        report.latest_eval.as_ref(),
    );
    if stale {
        document = document.next(
            format!(
                "trail lane refresh-preview {} <target>",
                report.lane.record.name
            ),
            "review the base update before refreshing this lane",
        );
    } else {
        document = document.next(
            format!("trail lane readiness {}", report.lane.record.name),
            "check merge readiness and any remaining gates",
        );
    }
    render_document(&document, options)
}

pub(crate) fn render_lane_contribution(
    report: &LaneContributionReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let status = &report.status;
    let pending_approvals = report
        .approvals
        .iter()
        .filter(|approval| approval.status == "pending")
        .count();
    let mut document = TerminalDocument::new(
        format!("Contribution for {}", status.lane.record.name),
        UiTone::Neutral,
    )
    .context(format!(
        "{} operation(s) · {} session(s) · {} event(s)",
        report.operations.len(),
        report.sessions.len(),
        report.recent_events.len()
    ))
    .block(lane_metadata(&status.lane));
    if pending_approvals > 0 {
        document = document.block(UiBlock::Checklist(vec![UiCheck::new(
            UiCheckState::Pending,
            "Approvals",
            format!("{pending_approvals} approval(s) pending"),
        )]));
    }
    if !status.changed_paths.is_empty() {
        document = document.block(UiBlock::Changes(change_list(&status.changed_paths)));
    }
    document = append_gate_summaries(
        document,
        status.latest_test.as_ref(),
        status.latest_eval.as_ref(),
    );
    if !report.operations.is_empty() {
        document = document.block(UiBlock::section(
            "Recent operations:",
            vec![UiBlock::Table(UiTable::new(
                vec![
                    UiColumn::left("WHEN", 2, 8),
                    UiColumn::left("KIND", 1, 8),
                    UiColumn::right("PATHS", 2, 5),
                    UiColumn::left("MESSAGE", 0, 16),
                ],
                report
                    .operations
                    .iter()
                    .map(|operation| {
                        vec![
                            format_timestamp(operation.created_at, options),
                            operation_kind_label(&operation.kind).to_string(),
                            operation.path_count.to_string(),
                            operation.message.clone().unwrap_or_else(|| "—".to_string()),
                        ]
                    })
                    .collect(),
            ))],
        ));
    }
    render_document(&document, options)
}

pub(crate) fn render_lane_review_packet(
    report: &LaneReviewPacketReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let readiness = &report.readiness;
    let mut document = TerminalDocument::new(
        format!(
            "Lane {} is {} for review",
            report.lane.record.name,
            if readiness.ready {
                "ready"
            } else {
                "not ready"
            }
        ),
        readiness_tone(readiness.ready),
    )
    .context(format!(
        "{} blocker(s) · {} warning(s) · {} changed path(s)",
        readiness.blockers.len(),
        readiness.warnings.len(),
        report.changed_paths.len()
    ))
    .block(lane_metadata(&report.lane));
    document = append_readiness(document, readiness);
    document = append_gate_summaries(
        document,
        report.latest_test.as_ref(),
        report.latest_eval.as_ref(),
    );
    if !report.changed_paths.is_empty() {
        document = document.block(UiBlock::Changes(change_list(&report.changed_paths)));
    }
    document = document.block(UiBlock::Metadata(vec![
        (
            "Operations".to_string(),
            report.evidence_summary.operations.to_string(),
        ),
        (
            "Sessions".to_string(),
            report.evidence_summary.sessions.to_string(),
        ),
        (
            "Approvals".to_string(),
            report.evidence_summary.approvals.to_string(),
        ),
        (
            "Gates".to_string(),
            report.evidence_summary.gates.to_string(),
        ),
        (
            "Conflicts".to_string(),
            report.evidence_summary.conflicts.to_string(),
        ),
    ]));
    if !report.conflicts.is_empty() {
        document = document.block(UiBlock::section(
            "Conflicts:",
            vec![UiBlock::Lines(
                report
                    .conflicts
                    .iter()
                    .map(|conflict| {
                        (
                            format!("{} {}", conflict.conflict_set_id, conflict.status),
                            UiTone::Blocked,
                        )
                    })
                    .collect(),
            )],
        ));
    }
    document = append_next_steps(document, &report.next_steps);
    render_document(&document.pager_eligible(), options)
}

pub(crate) fn render_lane_gate_history(
    report: &LaneGateHistoryReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let document = TerminalDocument::new(
        format!(
            "{} gate history for {}",
            report.kind, report.lane.record.name
        ),
        UiTone::Neutral,
    )
    .context(format!(
        "{} result(s), limit {}",
        report.gates.len(),
        report.limit
    ))
    .block(UiBlock::Table(UiTable::new(
        vec![
            UiColumn::left("WHEN", 1, 8),
            UiColumn::left("STATUS", 0, 8),
            UiColumn::left("SUITE", 2, 8),
            UiColumn::left("SCORE", 2, 6),
            UiColumn::right("TIME", 1, 6),
            UiColumn::left("COMMAND", 0, 16),
        ],
        report
            .gates
            .iter()
            .map(|gate| {
                vec![
                    format_timestamp(gate.created_at, options),
                    gate.status.clone(),
                    gate.suite.clone().unwrap_or_else(|| "—".to_string()),
                    score_summary(gate),
                    format!("{} ms", gate.duration_ms),
                    gate.command.join(" "),
                ]
            })
            .collect(),
    )))
    .next(
        format!("trail lane readiness {}", report.lane.record.name),
        "see whether the most recent gates satisfy merge readiness",
    )
    .pager_eligible();
    render_document(&document, options)
}

pub(crate) fn render_lane_readiness(
    report: &LaneReadinessReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut document = TerminalDocument::new(
        format!(
            "Lane {} is {}",
            report.lane.record.name,
            if report.ready {
                "ready to merge"
            } else {
                "not ready to merge"
            }
        ),
        readiness_tone(report.ready),
    )
    .context(format!(
        "{} blocker(s) · {} warning(s) · {} pending approval(s)",
        report.blockers.len(),
        report.warnings.len(),
        report.pending_approvals.len()
    ))
    .block(lane_metadata(&report.lane));
    document = append_readiness(document, report);
    document = append_gate_summaries(
        document,
        report.latest_test.as_ref(),
        report.latest_eval.as_ref(),
    );
    if !report.changed_paths.is_empty() {
        document = document.block(UiBlock::Changes(change_list(&report.changed_paths)));
    }
    document = append_workdir(
        document,
        report.workdir_state.as_ref(),
        &report.workdir_changed_paths,
    );
    if report.ready {
        document = document.next(
            format!("trail lane merge {}", report.lane.record.name),
            "merge this lane, or use --dry-run to preview it",
        );
    }
    render_document(&document, options)
}

pub(crate) fn render_lane_refresh_preview(
    report: &LaneRefreshPreviewReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let tone = if report.conflicted {
        UiTone::Blocked
    } else if report.clean {
        UiTone::Success
    } else {
        UiTone::Attention
    };
    let mut document =
        TerminalDocument::new(format!("Refresh preview for {}", report.ref_name), tone)
            .context(format!("onto {}", report.target_ref))
            .block(UiBlock::Metadata(vec![
                ("Clean".to_string(), report.clean.to_string()),
                ("Conflicted".to_string(), report.conflicted.to_string()),
                (
                    "Behind".to_string(),
                    report
                        .operations_behind
                        .map(|behind| format!("{behind} operation(s)"))
                        .unwrap_or_else(|| "unknown".to_string()),
                ),
            ]));
    if !report.conflicts.is_empty() {
        document = document.block(UiBlock::Checklist(
            report
                .conflicts
                .iter()
                .map(|conflict| UiCheck::new(UiCheckState::Blocked, "Conflict", conflict))
                .collect(),
        ));
    }
    if !report.changed_paths.is_empty() {
        document = document.block(UiBlock::Changes(change_list(&report.changed_paths)));
    }
    document = append_next_steps(document, &report.next_steps);
    render_document(&document, options)
}

pub(crate) fn render_lane_handoff(
    report: &LaneHandoffReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut document = TerminalDocument::new(
        format!("Handoff for {}", report.lane.record.name),
        readiness_tone(report.readiness.ready),
    )
    .context(format!(
        "{} session(s) · {} event(s) · {} operation(s)",
        report.recent_sessions.len(),
        report.recent_events.len(),
        report.recent_operations.len()
    ))
    .block(lane_metadata(&report.lane));
    document = append_readiness(document, &report.readiness);
    if let Some(session) = &report.current_session {
        document = document.block(UiBlock::section(
            "Current session:",
            vec![UiBlock::Metadata(vec![
                ("ID".to_string(), session.session.session_id.clone()),
                ("Status".to_string(), session.session.status.clone()),
                ("Turns".to_string(), session.turns.len().to_string()),
                ("Messages".to_string(), session.messages.len().to_string()),
                ("Events".to_string(), session.events.len().to_string()),
            ])],
        ));
    }
    document = append_next_steps(document, &report.next_steps);
    render_document(&document.pager_eligible(), options)
}

fn lane_metadata(lane: &LaneDetails) -> UiBlock {
    UiBlock::Metadata(vec![
        ("Ref".to_string(), lane.branch.ref_name.clone()),
        ("Base".to_string(), lane.branch.base_change.0.clone()),
        ("Head".to_string(), lane.branch.head_change.0.clone()),
    ])
}

fn append_readiness(
    mut document: TerminalDocument,
    report: &LaneReadinessReport,
) -> TerminalDocument {
    let mut checks: Vec<_> = report
        .blockers
        .iter()
        .map(|issue| UiCheck::new(UiCheckState::Blocked, &issue.code, &issue.message))
        .collect();
    checks.extend(
        report
            .warnings
            .iter()
            .map(|issue| UiCheck::new(UiCheckState::Warn, &issue.code, &issue.message)),
    );
    checks.extend(report.pending_approvals.iter().map(|approval| {
        UiCheck::new(
            UiCheckState::Pending,
            "Approval",
            format!("{} ({})", approval.approval_id, approval.action),
        )
    }));
    if !checks.is_empty() {
        document = document.block(UiBlock::Checklist(checks));
    }
    document
}

fn append_workdir(
    mut document: TerminalDocument,
    state: Option<&WorktreeState>,
    paths: &[FileDiffSummary],
) -> TerminalDocument {
    if let Some(state) = state {
        document = document.block(UiBlock::Notice(format!(
            "Workdir: {}",
            worktree_state_label(state)
        )));
    }
    if !paths.is_empty() {
        document = document.block(UiBlock::section(
            "Unrecorded workdir changes:",
            vec![UiBlock::Changes(change_list(paths))],
        ));
    }
    document
}

fn append_gate_summaries(
    mut document: TerminalDocument,
    test: Option<&LaneTestSummary>,
    eval: Option<&LaneTestSummary>,
) -> TerminalDocument {
    let mut checks = Vec::new();
    if let Some(test) = test {
        checks.push(gate_check("Test", test));
    }
    if let Some(eval) = eval {
        checks.push(gate_check("Eval", eval));
    }
    if !checks.is_empty() {
        document = document.block(UiBlock::Checklist(checks));
    }
    document
}

fn gate_check(label: &str, gate: &LaneTestSummary) -> UiCheck {
    UiCheck::new(
        if gate.success {
            UiCheckState::Pass
        } else {
            UiCheckState::Fail
        },
        label,
        format!(
            "{} · {} ms · {}",
            gate.status,
            gate.duration_ms,
            gate.command.join(" ")
        ),
    )
}

fn append_next_steps(mut document: TerminalDocument, steps: &[String]) -> TerminalDocument {
    for (index, step) in steps.iter().enumerate() {
        if index == 0 {
            document = document.next(step, "recommended next step");
        } else {
            document = document.more(step, "alternative next step");
        }
    }
    document
}

fn readiness_tone(ready: bool) -> UiTone {
    if ready {
        UiTone::Success
    } else {
        UiTone::Blocked
    }
}

fn score_summary(gate: &LaneTestSummary) -> String {
    match (gate.score, gate.threshold) {
        (Some(score), Some(threshold)) => format!("{score} / {threshold}"),
        (Some(score), None) => score.to_string(),
        (None, Some(threshold)) => format!("threshold {threshold}"),
        (None, None) => "—".to_string(),
    }
}
