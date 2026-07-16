use crate::cli::command::render::*;

use trail::model::*;
use trail::Result;

pub(crate) fn render_lane_run_pause(
    report: &LaneRunPauseReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut document = TerminalDocument::new(
        format!("Paused run {}", report.run_state.run_id),
        UiTone::Attention,
    )
    .block(run_metadata(&report.run_state));
    if let Some(approval) = &report.run_state.approval_id {
        document = document.next(
            format!("trail approvals show {approval}"),
            "review or decide the approval required to resume",
        );
    } else {
        document = document.next(
            format!("trail lane run resume {}", report.run_state.run_id),
            "resume when the interruption has been handled",
        );
    }
    render_document(&document, options)
}

pub(crate) fn render_lane_run_resume(
    report: &LaneRunResumeReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new(
            format!("Resumed run {}", report.run_state.run_id),
            UiTone::Success,
        )
        .block(run_metadata(&report.run_state)),
        options,
    )
}

pub(crate) fn render_lane_run_list(
    run_states: &[LaneRunState],
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(run_states);
    }
    if run_states.is_empty() {
        return render_document(
            &TerminalDocument::new("No lane runs", UiTone::Neutral),
            options,
        );
    }
    render_document(
        &TerminalDocument::new(format!("{} lane run(s)", run_states.len()), UiTone::Neutral).block(
            UiBlock::Table(UiTable::new(
                vec![
                    UiColumn::left("STATUS", 0, 8),
                    UiColumn::left("LANE", 0, 10),
                    UiColumn::left("REASON", 0, 12),
                    UiColumn::left("APPROVAL", 2, 12),
                    UiColumn::left("RUN", 2, 12),
                ],
                run_states
                    .iter()
                    .map(|run| {
                        vec![
                            run.status.clone(),
                            run.lane_id.clone(),
                            run.reason.clone(),
                            run.approval_id.clone().unwrap_or_else(|| "—".to_string()),
                            run.run_id.clone(),
                        ]
                    })
                    .collect(),
            )),
        ),
        options,
    )
}

pub(crate) fn render_lane_run_state(
    run_state: &LaneRunState,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(run_state);
    }
    let paused = run_state.status.eq_ignore_ascii_case("paused");
    let mut document = TerminalDocument::new(
        format!("Run {} is {}", run_state.run_id, run_state.status),
        if paused {
            UiTone::Attention
        } else {
            UiTone::Neutral
        },
    )
    .block(run_metadata(run_state));
    if paused {
        if let Some(approval) = &run_state.approval_id {
            document = document.next(
                format!("trail approvals show {approval}"),
                "resolve the dependent approval before resuming",
            );
        } else {
            document = document.next(
                format!("trail lane run resume {}", run_state.run_id),
                "resume this paused run when ready",
            );
        }
    }
    render_document(&document, options)
}

fn run_metadata(run: &LaneRunState) -> UiBlock {
    let mut metadata = vec![
        ("Lane".to_string(), run.lane_id.clone()),
        ("Reason".to_string(), run.reason.clone()),
        ("Summary".to_string(), run.summary.clone()),
    ];
    if let Some(session) = &run.session_id {
        metadata.push(("Session".to_string(), session.clone()));
    }
    if let Some(turn) = &run.turn_id {
        metadata.push(("Turn".to_string(), turn.clone()));
    }
    if let Some(approval) = &run.approval_id {
        metadata.push(("Approval".to_string(), approval.clone()));
    }
    if let Some(reviewer) = &run.reviewer {
        metadata.push(("Reviewer".to_string(), reviewer.clone()));
    }
    if let Some(note) = &run.note {
        metadata.push(("Note".to_string(), note.clone()));
    }
    UiBlock::Metadata(metadata)
}
