use crate::cli::command::render::*;

use trail::model::*;
use trail::Result;

pub(crate) fn render_approval_request(
    report: &LaneApprovalRequestReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut document = TerminalDocument::new(
        format!("Approval requested: {}", report.approval.action),
        UiTone::Attention,
    )
    .block(approval_metadata(&report.approval));
    if let Some(run) = &report.run_state {
        document = document.block(UiBlock::Notice(format!("Paused run: {}", run.run_id)));
    }
    document = document.next(
        format!("trail approvals show {}", report.approval.approval_id),
        "review the evidence before recording a decision",
    );
    render_document(&document, options)
}

pub(crate) fn render_approval_list(
    approvals: &[LaneApproval],
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(&approvals);
    }
    if approvals.is_empty() {
        return render_document(
            &TerminalDocument::new("No approvals", UiTone::Success),
            options,
        );
    }
    render_document(
        &TerminalDocument::new(format!("{} approval(s)", approvals.len()), UiTone::Neutral).block(
            UiBlock::Table(UiTable::new(
                vec![
                    UiColumn::left("STATUS", 0, 8),
                    UiColumn::left("LANE", 0, 10),
                    UiColumn::left("ACTION", 0, 12),
                    UiColumn::left("SUMMARY", 0, 18),
                    UiColumn::left("APPROVAL", 2, 12),
                ],
                approvals
                    .iter()
                    .map(|approval| {
                        vec![
                            approval.status.clone(),
                            approval.lane_id.clone(),
                            approval.action.clone(),
                            approval.summary.clone(),
                            approval.approval_id.clone(),
                        ]
                    })
                    .collect(),
            )),
        ),
        options,
    )
}

pub(crate) fn render_approval(
    approval: &LaneApproval,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(approval);
    }
    let pending = approval.status.eq_ignore_ascii_case("pending");
    let mut document = TerminalDocument::new(
        format!("Approval {} is {}", approval.approval_id, approval.status),
        if pending {
            UiTone::Attention
        } else {
            UiTone::Neutral
        },
    )
    .block(approval_metadata(approval));
    if pending {
        document = document.next(
            format!("trail approvals decide {} <decision>", approval.approval_id),
            "record a reviewed decision for this confirmation-sensitive action",
        );
    }
    render_document(&document, options)
}

pub(crate) fn render_approval_decision(
    report: &LaneApprovalDecisionReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut document = TerminalDocument::new(
        format!(
            "Recorded {} for approval {}",
            report.decision, report.approval.approval_id
        ),
        UiTone::Success,
    )
    .block(approval_metadata(&report.approval));
    if !report.run_states.is_empty() {
        document = document.block(UiBlock::Notice(format!(
            "{} linked run(s) updated",
            report.run_states.len()
        )));
    }
    render_document(&document, options)
}

fn approval_metadata(approval: &LaneApproval) -> UiBlock {
    let mut metadata = vec![
        ("Lane".to_string(), approval.lane_id.clone()),
        ("Status".to_string(), approval.status.clone()),
        ("Action".to_string(), approval.action.clone()),
        ("Summary".to_string(), approval.summary.clone()),
    ];
    if let Some(session) = &approval.session_id {
        metadata.push(("Session".to_string(), session.clone()));
    }
    if let Some(turn) = &approval.turn_id {
        metadata.push(("Turn".to_string(), turn.clone()));
    }
    if let Some(reviewer) = &approval.reviewer {
        metadata.push(("Reviewer".to_string(), reviewer.clone()));
    }
    if let Some(note) = &approval.note {
        metadata.push(("Note".to_string(), note.clone()));
    }
    UiBlock::Metadata(metadata)
}
