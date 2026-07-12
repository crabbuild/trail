use crate::cli::command::render::*;

use trail;
use trail::model::*;
use trail::Result;

pub(crate) fn render_lane_message(
    report: &LaneMessageReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new(format!("Added {} message", report.role), UiTone::Success).block(
            UiBlock::Metadata(vec![("Message".to_string(), report.message_id.0.clone())]),
        ),
        options,
    )
}

pub(crate) fn render_lane_turn_start(
    report: &trail::LaneTurnStartReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new(
            format!("Started turn {}", report.turn.turn_id),
            UiTone::Success,
        )
        .block(turn_metadata(&report.turn))
        .block(UiBlock::Metadata(vec![
            ("Session".to_string(), report.session.session_id.clone()),
            ("Base root".to_string(), report.base_root.0.clone()),
        ])),
        options,
    )
}

pub(crate) fn render_lane_turn_details(
    details: &trail::LaneTurnDetails,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(details);
    }
    let mut document =
        TerminalDocument::new(format!("Turn {}", details.turn.turn_id), UiTone::Neutral)
            .block(turn_metadata(&details.turn))
            .block(UiBlock::Metadata(vec![
                ("Messages".to_string(), details.messages.len().to_string()),
                ("Events".to_string(), details.events.len().to_string()),
                (
                    "Operations".to_string(),
                    details.operations.len().to_string(),
                ),
            ]));
    if let Some(envelope) = &details.turn_envelope {
        let mut metadata = Vec::new();
        if let Some(provider) = &envelope.provider {
            metadata.push(("Provider".to_string(), provider.clone()));
        }
        if let Some(model) = &envelope.model {
            metadata.push(("Model".to_string(), model.clone()));
        }
        if envelope.outcome.no_changes {
            metadata.push(("Outcome".to_string(), "no changes".to_string()));
        }
        if !metadata.is_empty() {
            document = document.block(UiBlock::section(
                "Agent outcome:",
                vec![UiBlock::Metadata(metadata)],
            ));
        }
    }
    if !details.events.is_empty() {
        document = document.block(UiBlock::section(
            "Events:",
            vec![UiBlock::Table(UiTable::new(
                vec![
                    UiColumn::left("WHEN", 1, 8),
                    UiColumn::left("TYPE", 0, 12),
                    UiColumn::left("EVENT", 2, 12),
                ],
                details
                    .events
                    .iter()
                    .map(|event| {
                        vec![
                            format_timestamp(event.created_at, options),
                            event.event_type.clone(),
                            event.event_id.clone(),
                        ]
                    })
                    .collect(),
            ))],
        ));
    }
    render_document(&document.pager_eligible(), options)
}

pub(crate) fn render_lane_turn_event(
    report: &trail::LaneTurnEventReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new(
            format!("Recorded {} event", report.event.event_type),
            UiTone::Success,
        )
        .block(event_metadata(&report.event)),
        options,
    )
}

pub(crate) fn render_lane_events(
    events: &[LaneEventRecord],
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(events);
    }
    if events.is_empty() {
        return render_document(
            &TerminalDocument::new("No events", UiTone::Neutral),
            options,
        );
    }
    render_document(
        &TerminalDocument::new(format!("{} event(s)", events.len()), UiTone::Neutral)
            .block(UiBlock::Table(UiTable::new(
                vec![
                    UiColumn::left("WHEN", 1, 8),
                    UiColumn::left("TYPE", 0, 12),
                    UiColumn::left("LANE", 0, 10),
                    UiColumn::left("SESSION", 2, 12),
                    UiColumn::left("TURN", 2, 12),
                ],
                events
                    .iter()
                    .map(|event| {
                        vec![
                            format_timestamp(event.created_at, options),
                            event.event_type.clone(),
                            event.lane_id.clone(),
                            event.session_id.clone().unwrap_or_else(|| "—".to_string()),
                            event.turn_id.clone().unwrap_or_else(|| "—".to_string()),
                        ]
                    })
                    .collect(),
            )))
            .pager_eligible(),
        options,
    )
}

pub(crate) fn render_lane_turn_end(
    report: &trail::LaneTurnEndReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new(
            format!(
                "Ended turn {} as {}",
                report.turn.turn_id, report.turn.status
            ),
            UiTone::Success,
        )
        .block(turn_metadata(&report.turn)),
        options,
    )
}

fn turn_metadata(turn: &LaneTurn) -> UiBlock {
    let mut metadata = vec![
        ("Lane".to_string(), turn.lane_id.clone()),
        ("Status".to_string(), turn.status.clone()),
        ("Before".to_string(), turn.before_change.0.clone()),
    ];
    if let Some(after) = &turn.after_change {
        metadata.push(("After".to_string(), after.0.clone()));
    }
    if let Some(session) = &turn.session_id {
        metadata.push(("Session".to_string(), session.clone()));
    }
    UiBlock::Metadata(metadata)
}

fn event_metadata(event: &LaneEventRecord) -> UiBlock {
    UiBlock::Metadata(vec![
        ("Event".to_string(), event.event_id.clone()),
        ("Lane".to_string(), event.lane_id.clone()),
        (
            "Turn".to_string(),
            event.turn_id.clone().unwrap_or_else(|| "—".to_string()),
        ),
    ])
}
