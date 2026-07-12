use crate::cli::command::render::*;

use trail::model::*;
use trail::Result;

pub(crate) fn render_session_start(
    report: &LaneSessionStartReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new(
            format!("Started session {}", report.session.session_id),
            UiTone::Success,
        )
        .block(session_metadata(&report.session))
        .next(
            format!("trail session context {}", report.session.session_id),
            "inspect the growing session context",
        ),
        options,
    )
}

pub(crate) fn render_session_current(
    reports: &[LaneSessionCurrentReport],
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(&reports);
    }
    let active: Vec<_> = reports
        .iter()
        .filter_map(|report| report.session.as_ref().map(|session| (report, session)))
        .collect();
    if active.is_empty() {
        return render_document(
            &TerminalDocument::new("No active sessions", UiTone::Neutral),
            options,
        );
    }
    render_document(
        &TerminalDocument::new(
            format!("{} active session(s)", active.len()),
            UiTone::Success,
        )
        .block(UiBlock::Table(UiTable::new(
            vec![
                UiColumn::left("LANE", 0, 10),
                UiColumn::left("STATUS", 0, 8),
                UiColumn::left("TITLE", 0, 14),
                UiColumn::left("SESSION", 2, 12),
            ],
            active
                .iter()
                .map(|(report, session)| {
                    vec![
                        report.lane_name.clone(),
                        session.status.clone(),
                        session.title.clone().unwrap_or_else(|| "—".to_string()),
                        session.session_id.clone(),
                    ]
                })
                .collect(),
        ))),
        options,
    )
}

pub(crate) fn render_session_list(
    sessions: &[LaneSession],
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(&sessions);
    }
    if sessions.is_empty() {
        return render_document(
            &TerminalDocument::new("No sessions", UiTone::Neutral),
            options,
        );
    }
    render_document(
        &TerminalDocument::new(format!("{} session(s)", sessions.len()), UiTone::Neutral).block(
            UiBlock::Table(UiTable::new(
                vec![
                    UiColumn::left("STATUS", 0, 8),
                    UiColumn::left("LANE", 0, 10),
                    UiColumn::left("TITLE", 0, 14),
                    UiColumn::left("STARTED", 1, 10),
                    UiColumn::left("SESSION", 2, 12),
                ],
                sessions
                    .iter()
                    .map(|session| {
                        vec![
                            session.status.clone(),
                            session.lane_id.clone(),
                            session.title.clone().unwrap_or_else(|| "—".to_string()),
                            format_timestamp(session.started_at, options),
                            session.session_id.clone(),
                        ]
                    })
                    .collect(),
            )),
        ),
        options,
    )
}

pub(crate) fn render_session_details(
    details: &LaneSessionDetails,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(details);
    }
    let mut document = TerminalDocument::new(
        format!("Session {}", details.session.session_id),
        UiTone::Neutral,
    )
    .block(session_metadata(&details.session))
    .block(UiBlock::Metadata(vec![
        ("Turns".to_string(), details.turns.len().to_string()),
        ("Messages".to_string(), details.messages.len().to_string()),
        ("Events".to_string(), details.events.len().to_string()),
        (
            "Operations".to_string(),
            details.operations.len().to_string(),
        ),
    ]));
    if !details.turns.is_empty() {
        document = document.block(UiBlock::section(
            "Turns:",
            vec![UiBlock::Table(UiTable::new(
                vec![
                    UiColumn::left("STATUS", 0, 8),
                    UiColumn::left("CHECKPOINT", 1, 12),
                    UiColumn::left("TURN", 0, 12),
                ],
                details
                    .turns
                    .iter()
                    .map(|turn| {
                        vec![
                            turn.status.clone(),
                            turn.after_change
                                .as_ref()
                                .map(|change| change.0.clone())
                                .unwrap_or_else(|| "—".to_string()),
                            turn.turn_id.clone(),
                        ]
                    })
                    .collect(),
            ))],
        ));
    }
    document = document.next(
        format!("trail session context {}", details.session.session_id),
        "review recent messages, turns, and operations",
    );
    render_document(&document.pager_eligible(), options)
}

pub(crate) fn render_session_context(
    report: &LaneSessionContextReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut document = TerminalDocument::new(
        format!("Session context: {}", report.session.session_id),
        UiTone::Neutral,
    )
    .block(session_metadata(&report.session))
    .block(UiBlock::Metadata(vec![
        ("Messages".to_string(), report.message_count.to_string()),
        ("Events".to_string(), report.event_count.to_string()),
        ("Turns".to_string(), report.turn_count.to_string()),
        ("Operations".to_string(), report.operation_count.to_string()),
    ]));
    if !report.recent_messages.is_empty() {
        document = document.block(UiBlock::section(
            "Recent messages:",
            vec![UiBlock::Table(UiTable::new(
                vec![
                    UiColumn::left("ROLE", 0, 8),
                    UiColumn::left("MESSAGE", 0, 28),
                ],
                report
                    .recent_messages
                    .iter()
                    .map(|message| vec![message.role.clone(), preview(&message.body, 100)])
                    .collect(),
            ))],
        ));
    }
    if !report.recent_turns.is_empty() {
        document = document.block(UiBlock::section(
            "Recent turns:",
            vec![UiBlock::Table(UiTable::new(
                vec![
                    UiColumn::left("STATUS", 0, 8),
                    UiColumn::left("TURN", 0, 12),
                ],
                report
                    .recent_turns
                    .iter()
                    .map(|turn| vec![turn.status.clone(), turn.turn_id.clone()])
                    .collect(),
            ))],
        ));
    }
    render_document(&document.pager_eligible(), options)
}

pub(crate) fn render_session_end(
    report: &LaneSessionEndReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new(
            format!("Ended session {}", report.session.session_id),
            UiTone::Success,
        )
        .block(session_metadata(&report.session)),
        options,
    )
}

fn session_metadata(session: &LaneSession) -> UiBlock {
    let mut metadata = vec![
        ("Lane".to_string(), session.lane_id.clone()),
        ("Status".to_string(), session.status.clone()),
    ];
    if let Some(title) = &session.title {
        metadata.push(("Title".to_string(), title.clone()));
    }
    UiBlock::Metadata(metadata)
}

fn preview(value: &str, limit: usize) -> String {
    let mut preview = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if preview.chars().count() > limit {
        preview = preview.chars().take(limit.saturating_sub(1)).collect();
        preview.push('…');
    }
    preview
}
