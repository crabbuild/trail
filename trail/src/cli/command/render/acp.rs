use super::*;

use trail::model::*;
use trail::Result;

pub(crate) fn render_acp_profiles(
    profiles: &[AcpProviderProfile],
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(profiles);
    }
    if profiles.is_empty() {
        return render_document(
            &TerminalDocument::new("No ACP providers", UiTone::Neutral),
            options,
        );
    }
    render_document(
        &TerminalDocument::new(
            format!("{} ACP provider(s)", profiles.len()),
            UiTone::Neutral,
        )
        .block(UiBlock::Table(UiTable::new(
            vec![
                UiColumn::left("PROVIDER", 0, 10),
                UiColumn::left("STATUS", 0, 9),
                UiColumn::left("ACP", 1, 4),
                UiColumn::left("MCP", 1, 4),
                UiColumn::left("TERMINAL", 1, 8),
                UiColumn::left("RELAY", 0, 18),
            ],
            profiles
                .iter()
                .map(|profile| {
                    vec![
                        profile.agent.clone(),
                        if profile.available {
                            "available"
                        } else {
                            "missing"
                        }
                        .to_string(),
                        profile.supports_acp.to_string(),
                        profile.supports_mcp.to_string(),
                        profile.supports_terminal.to_string(),
                        shell_join(&profile.relay_command),
                    ]
                })
                .collect(),
        ))),
        options,
    )
}

pub(crate) fn render_acp_setup(
    report: &trail::acp::AcpSetupReport,
    json: bool,
    options: &RenderOptions,
    print_only: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if print_only {
        return render_document(
            &TerminalDocument::empty().block(UiBlock::Patch {
                title: "ACP snippet".to_string(),
                text: report.snippet.clone(),
            }),
            options,
        );
    }
    let mut document = TerminalDocument::new(
        format!("ACP setup for {}", report.provider),
        if report.applied {
            UiTone::Success
        } else {
            UiTone::Attention
        },
    )
    .block(UiBlock::Metadata(vec![
        ("Editor".to_string(), report.editor.clone()),
        ("Action".to_string(), report.action.clone()),
        ("Applied".to_string(), report.applied.to_string()),
        ("Command".to_string(), shell_join(&report.command)),
    ]))
    .block(UiBlock::Patch {
        title: "Configuration snippet".to_string(),
        text: report.snippet.clone(),
    });
    if !report.warnings.is_empty() {
        document = document.block(UiBlock::Checklist(
            report
                .warnings
                .iter()
                .map(|warning| UiCheck::new(UiCheckState::Warn, "Install warning", warning))
                .collect(),
        ));
    }
    render_document(&document.pager_eligible(), options)
}

pub(crate) fn render_acp_doctor(
    report: &AcpDoctorReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut checks: Vec<_> = report
        .checks
        .iter()
        .map(|check| UiCheck::new(acp_check_state(&check.status), &check.name, &check.message))
        .collect();
    checks.extend(
        report
            .warnings
            .iter()
            .map(|warning| UiCheck::new(UiCheckState::Warn, "Warning", warning)),
    );
    render_document(
        &TerminalDocument::new(
            format!("ACP diagnostics: {}", report.status),
            if report.status == "ok" {
                UiTone::Success
            } else {
                UiTone::Attention
            },
        )
        .block(UiBlock::Checklist(checks)),
        options,
    )
}

pub(crate) fn render_acp_sessions(
    report: &AcpSessionListReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if report.sessions.is_empty() {
        return render_document(
            &TerminalDocument::new("No ACP sessions", UiTone::Neutral),
            options,
        );
    }
    render_document(
        &TerminalDocument::new(
            format!("{} ACP session(s)", report.sessions.len()),
            UiTone::Neutral,
        )
        .block(UiBlock::Table(UiTable::new(
            vec![
                UiColumn::left("STATUS", 0, 8),
                UiColumn::left("PROVIDER", 1, 10),
                UiColumn::left("TRAIL SESSION", 1, 12),
                UiColumn::left("ACP SESSION", 0, 14),
            ],
            report
                .sessions
                .iter()
                .map(|session| {
                    vec![
                        session.status.clone(),
                        session.provider.clone().unwrap_or_else(|| "—".to_string()),
                        session.trail_session_id.clone(),
                        session.acp_session_id.clone(),
                    ]
                })
                .collect(),
        ))),
        options,
    )
}

pub(crate) fn render_transcript(
    report: &TranscriptReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut document =
        TerminalDocument::new(format!("Transcript: {}", report.lane_name), UiTone::Neutral).block(
            UiBlock::Metadata(vec![
                ("Session".to_string(), report.session.session_id.clone()),
                ("Status".to_string(), report.session.status.clone()),
                ("Turns".to_string(), report.turns.len().to_string()),
            ]),
        );
    for turn in &report.turns {
        let mut blocks = vec![UiBlock::Metadata(vec![
            ("Status".to_string(), turn.turn.status.clone()),
            (
                "Checkpoint".to_string(),
                turn.checkpoint
                    .as_ref()
                    .map(|value| value.0.clone())
                    .unwrap_or_else(|| "—".to_string()),
            ),
        ])];
        if !turn.messages.is_empty() {
            blocks.push(UiBlock::Table(UiTable::new(
                vec![
                    UiColumn::left("ROLE", 0, 8),
                    UiColumn::left("MESSAGE", 0, 28),
                ],
                turn.messages
                    .iter()
                    .map(|message| vec![message.role.clone(), preview(&message.body, 160)])
                    .collect(),
            )));
        }
        if !turn.tool_summaries.is_empty() {
            blocks.push(UiBlock::Lines(
                turn.tool_summaries
                    .iter()
                    .cloned()
                    .map(|tool| (tool, UiTone::Muted))
                    .collect(),
            ));
        }
        document = document.block(UiBlock::section(
            format!("Turn {}", turn.turn.turn_id),
            blocks,
        ));
    }
    render_document(&document.pager_eligible(), options)
}

fn acp_check_state(status: &str) -> UiCheckState {
    match status.to_ascii_lowercase().as_str() {
        "ok" | "pass" | "healthy" => UiCheckState::Pass,
        "warn" | "warning" => UiCheckState::Warn,
        "pending" => UiCheckState::Pending,
        _ => UiCheckState::Fail,
    }
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
fn preview(value: &str, limit: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= limit {
        compact
    } else {
        format!(
            "{}…",
            compact
                .chars()
                .take(limit.saturating_sub(1))
                .collect::<String>()
        )
    }
}
