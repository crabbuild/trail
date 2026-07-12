use super::*;

use trail::model::*;
use trail::Result;

pub(crate) fn render_guardrail_check(
    report: &GuardrailCheckReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let state = check_state_from_status(&report.decision);
    let mut document = TerminalDocument::new(
        format!("Guardrail decision: {}", report.decision),
        check_tone(state),
    )
    .context(format!("Action: {}", report.action));
    if let Some(lane) = &report.lane {
        document = document.block(UiBlock::Metadata(vec![(
            "Lane".to_string(),
            lane.record.name.clone(),
        )]));
    }
    if !report.reasons.is_empty() {
        document = document.block(UiBlock::section(
            "Checks:",
            vec![UiBlock::Checklist(
                report
                    .reasons
                    .iter()
                    .map(|reason| check_for_status(&reason.severity, &reason.code, &reason.message))
                    .collect(),
            )],
        ));
    }
    if !report.path_checks.is_empty() {
        document = document.block(UiBlock::section(
            "Paths:",
            vec![UiBlock::Table(UiTable::new(
                vec![
                    UiColumn::left("PATH", 0, 12),
                    UiColumn::left("DECISION", 1, 8),
                    UiColumn::left("RULE", 2, 12),
                ],
                report
                    .path_checks
                    .iter()
                    .map(|check| {
                        vec![
                            check.path.clone(),
                            if check.ignored { "ignored" } else { "allowed" }.to_string(),
                            check.source.clone().unwrap_or_else(|| "—".to_string()),
                        ]
                    })
                    .collect(),
            ))],
        ));
    }
    if let Some(approval) = &report.approval_request {
        document = document.block(UiBlock::Notice(format!(
            "Approval required: {}",
            approval.summary
        )));
    }
    if !report.satisfied_approvals.is_empty() {
        document = document.block(UiBlock::section(
            "Satisfied approvals:",
            vec![UiBlock::Lines(
                report
                    .satisfied_approvals
                    .iter()
                    .map(|approval| {
                        (
                            format!("{} {}", approval.approval_id, approval.action),
                            UiTone::Success,
                        )
                    })
                    .collect(),
            )],
        ));
    }
    render_document(&document, options)
}

fn check_tone(state: UiCheckState) -> UiTone {
    match state {
        UiCheckState::Pass => UiTone::Success,
        UiCheckState::Warn | UiCheckState::Pending => UiTone::Attention,
        UiCheckState::Blocked | UiCheckState::Fail => UiTone::Blocked,
        UiCheckState::Skip => UiTone::Neutral,
    }
}
