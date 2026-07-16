use super::*;

use trail::model::*;
use trail::Result;

pub(crate) fn render_show(result: &ShowResult, json: bool, options: &RenderOptions) -> Result<()> {
    if json {
        return render_json(result);
    }
    let document = match result {
        ShowResult::Operation { value } => {
            let operation = &value.operation;
            let mut document = TerminalDocument::new(
                format!("Operation {}", operation.change_id.0),
                UiTone::Neutral,
            )
            .context(operation_kind_label(&operation.kind))
            .block(UiBlock::Metadata(vec![
                ("Branch".to_string(), operation.branch.clone()),
                ("Actor".to_string(), operation.actor.id.clone()),
            ]));
            if let Some(message) = &operation.message {
                document = document.block(UiBlock::paragraph(message));
            }
            if !value.changed_paths.is_empty() {
                document = document.block(UiBlock::section(
                    "Changes:",
                    vec![UiBlock::Changes(crate::cli::command::render::change_list(
                        &value.changed_paths,
                    ))],
                ));
            }
            if options.verbose {
                let mut metadata = vec![("After root".to_string(), operation.after_root.0.clone())];
                if let Some(before_root) = &operation.before_root {
                    metadata.push(("Before root".to_string(), before_root.0.clone()));
                }
                document = document.block(UiBlock::Metadata(metadata));
                if !operation.parents.is_empty() {
                    document = document.block(UiBlock::section(
                        "Parents:",
                        vec![UiBlock::Lines(
                            operation
                                .parents
                                .iter()
                                .map(|parent| (parent.0.clone(), UiTone::Muted))
                                .collect(),
                        )],
                    ));
                }
            }
            document
        }
        ShowResult::Message { value } => {
            TerminalDocument::new(format!("Message {}", value.id.0), UiTone::Neutral)
                .block(UiBlock::paragraph(&value.body))
                .block(UiBlock::Metadata(message_metadata(value)))
        }
        ShowResult::Ref { value } => {
            TerminalDocument::new(format!("Ref {}", value.name), UiTone::Neutral).block(
                UiBlock::Metadata(vec![
                    ("Change".to_string(), value.change_id.0.clone()),
                    ("Generation".to_string(), value.generation.to_string()),
                    ("Root".to_string(), value.root_id.0.clone()),
                ]),
            )
        }
        ShowResult::Lane { value } => TerminalDocument::new(
            format!("Lane {}", value.lane_id),
            status_tone(&value.status),
        )
        .context(value.status.clone())
        .block(UiBlock::Metadata(lane_metadata(value, options))),
        ShowResult::Object { value } => {
            TerminalDocument::new(format!("Object {}", value.object_id.0), UiTone::Neutral).block(
                UiBlock::Metadata(vec![
                    ("Kind".to_string(), value.kind.clone()),
                    ("Version".to_string(), value.version.to_string()),
                    ("Size".to_string(), format!("{} bytes", value.size_bytes)),
                ]),
            )
        }
    };
    render_document(&document, options)
}

pub(crate) fn render_object_inspect(
    report: &ObjectInspectReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut document = TerminalDocument::new(
        format!("Object {}", report.info.object_id.0),
        UiTone::Neutral,
    )
    .block(UiBlock::Metadata(vec![
        ("Kind".to_string(), report.info.kind.clone()),
        ("Version".to_string(), report.info.version.to_string()),
        (
            "Size".to_string(),
            format!("{} bytes", report.info.size_bytes),
        ),
        (
            "Created".to_string(),
            format_timestamp(report.info.created_at, options),
        ),
    ]));
    if report
        .summary
        .as_object()
        .map(|summary| !summary.is_empty())
        .unwrap_or(true)
    {
        let summary = serde_json::to_string_pretty(&report.summary)?;
        document = document
            .block(UiBlock::section(
                "Summary:",
                vec![UiBlock::Patch {
                    title: "Object metadata".to_string(),
                    text: summary,
                }],
            ))
            .pager_eligible();
    }
    render_document(&document, options)
}

pub(crate) fn render_root_inspect(
    report: &RootInspectReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut metadata = vec![
        ("Created by".to_string(), report.root.created_by.0.clone()),
        ("Files".to_string(), report.root.file_count.to_string()),
        (
            "Text".to_string(),
            format!("{} bytes", report.root.total_text_bytes),
        ),
    ];
    if let Some(path_root) = &report.root.path_map_root {
        metadata.push(("Path map".to_string(), path_root.clone()));
    }
    if let Some(file_root) = &report.root.file_index_map_root {
        metadata.push(("File index".to_string(), file_root.clone()));
    }
    let rows = report
        .files
        .iter()
        .map(|file| {
            vec![
                file_kind_label(&file.kind).to_string(),
                file.path.clone(),
                file.file_id.clone(),
                format!("{} bytes", file.size_bytes),
            ]
        })
        .collect();
    render_document(
        &TerminalDocument::new(format!("Root {}", report.root_id.0), UiTone::Neutral)
            .block(UiBlock::Metadata(metadata))
            .block(UiBlock::section(
                "Files:",
                vec![UiBlock::Table(UiTable::new(
                    vec![
                        UiColumn::left("KIND", 2, 6),
                        UiColumn::left("PATH", 0, 12),
                        UiColumn::left("FILE", 1, 12),
                        UiColumn::right("SIZE", 2, 8),
                    ],
                    rows,
                ))],
            ))
            .pager_eligible(),
        options,
    )
}

pub(crate) fn render_text_inspect(
    report: &TextInspectReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let rows = report
        .lines
        .iter()
        .map(|line| {
            Ok(vec![
                line.line_number.to_string(),
                line.line_id.clone(),
                newline_kind_label(&line.newline).to_string(),
                serde_json::to_string(&line.text)?,
            ])
        })
        .collect::<Result<Vec<_>>>()?;
    let mut document = TerminalDocument::new(format!("Text {}", report.text_id.0), UiTone::Neutral)
        .block(UiBlock::Metadata(vec![
            (
                "Content hash".to_string(),
                report.content.content_hash.clone(),
            ),
            (
                "Lines".to_string(),
                format!(
                    "{} (showing {})",
                    report.content.line_count,
                    report.lines.len()
                ),
            ),
            ("Bytes".to_string(), report.content.byte_count.to_string()),
        ]))
        .block(UiBlock::Table(UiTable::new(
            vec![
                UiColumn::right("LINE", 0, 4),
                UiColumn::left("LINE ID", 1, 12),
                UiColumn::left("ENDING", 2, 5),
                UiColumn::left("TEXT", 0, 16),
            ],
            rows,
        )));
    if report.truncated {
        document = document.block(UiBlock::Notice(
            "Output truncated; pass --limit 0 to show all lines.".to_string(),
        ));
    }
    render_document(&document.pager_eligible(), options)
}

fn file_kind_label(kind: &FileKind) -> &'static str {
    match kind {
        FileKind::Text => "text",
        FileKind::OpaqueText => "opaque text",
        FileKind::Binary => "binary",
    }
}

fn newline_kind_label(kind: &NewlineKind) -> &'static str {
    match kind {
        NewlineKind::None => "none",
        NewlineKind::Lf => "LF",
        NewlineKind::Crlf => "CRLF",
    }
}

pub(crate) fn render_map_range(
    report: &MapRangeReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let rows = report
        .entries
        .iter()
        .map(|entry| {
            Ok(vec![
                render_map_key(&entry.key),
                render_map_value_summary(&entry.value)?,
            ])
        })
        .collect::<Result<Vec<_>>>()?;
    let mut document = TerminalDocument::new(format!("Map {}", report.map_id), UiTone::Neutral)
        .context(report.map_type.clone())
        .block(UiBlock::Table(UiTable::new(
            vec![UiColumn::left("KEY", 0, 12), UiColumn::left("VALUE", 0, 20)],
            rows,
        )));
    if report.truncated {
        document = document.block(UiBlock::Notice(
            "Output truncated; pass --limit 0 to show all entries.".to_string(),
        ));
    }
    render_document(&document.pager_eligible(), options)
}

pub(crate) fn render_map_diff(
    report: &MapDiffReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let rows = report
        .changes
        .iter()
        .map(|change| {
            Ok(vec![
                change.kind.clone(),
                render_map_key(&change.key),
                change
                    .old_value
                    .as_ref()
                    .map(render_map_value_summary)
                    .transpose()?
                    .unwrap_or_else(|| "—".to_string()),
                change
                    .new_value
                    .as_ref()
                    .map(render_map_value_summary)
                    .transpose()?
                    .unwrap_or_else(|| "—".to_string()),
            ])
        })
        .collect::<Result<Vec<_>>>()?;
    let mut document = TerminalDocument::new(
        format!("Map diff {} to {}", report.left_map_id, report.right_map_id),
        UiTone::Neutral,
    )
    .context(report.map_type.clone())
    .block(UiBlock::Table(UiTable::new(
        vec![
            UiColumn::left("CHANGE", 1, 6),
            UiColumn::left("KEY", 0, 12),
            UiColumn::left("OLD", 2, 12),
            UiColumn::left("NEW", 0, 12),
        ],
        rows,
    )));
    if report.truncated {
        document = document.block(UiBlock::Notice(
            "Output truncated; pass --limit 0 to show all changes.".to_string(),
        ));
    }
    render_document(&document.pager_eligible(), options)
}

pub(crate) fn render_map_key(key: &MapKeyInspect) -> String {
    key.text
        .clone()
        .unwrap_or_else(|| format!("hex:{}", key.hex))
}

pub(crate) fn render_map_value_summary(value: &MapValueInspect) -> Result<String> {
    if let Some(text) = &value.text {
        if value.summary == serde_json::json!({ "bytes": value.bytes }) {
            return Ok(format!("{text:?}"));
        }
    }
    let summary = serde_json::to_string(&value.summary)?;
    if value.truncated {
        Ok(format!(
            "{summary} ({} bytes, hex preview truncated)",
            value.bytes
        ))
    } else {
        Ok(format!("{summary} ({} bytes)", value.bytes))
    }
}

fn message_metadata(value: &Message) -> Vec<(String, String)> {
    let mut metadata = vec![("Role".to_string(), value.role.clone())];
    if let Some(lane_id) = &value.lane_id {
        metadata.push(("Lane".to_string(), lane_id.clone()));
    }
    if let Some(session_id) = &value.session_id {
        metadata.push(("Session".to_string(), session_id.clone()));
    }
    if let Some(change_id) = &value.change_id {
        metadata.push(("Change".to_string(), change_id.0.clone()));
    }
    metadata
}

fn lane_metadata(value: &LaneBranch, options: &RenderOptions) -> Vec<(String, String)> {
    let mut metadata = vec![("Ref".to_string(), value.ref_name.clone())];
    if options.verbose {
        metadata.push(("Base".to_string(), value.base_change.0.clone()));
        metadata.push(("Head".to_string(), value.head_change.0.clone()));
    }
    if let Some(workdir) = &value.workdir {
        metadata.push(("Workdir".to_string(), workdir.clone()));
    }
    metadata
}

fn status_tone(status: &str) -> UiTone {
    match status.to_ascii_lowercase().as_str() {
        "ready" | "active" | "open" => UiTone::Success,
        "blocked" | "conflicted" | "failed" => UiTone::Blocked,
        "paused" | "stale" | "pending" => UiTone::Attention,
        _ => UiTone::Neutral,
    }
}
