use super::super::*;

use trail::model::*;
use trail::Result;

pub(crate) fn render_merge(
    report: &MergeReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let conflicted = !report.conflicts.is_empty();
    let mut document = TerminalDocument::new(
        if report.dry_run {
            format!(
                "Merge preview: {} into {}",
                report.source_ref, report.target_ref
            )
        } else {
            format!("Merged {} into {}", report.source_ref, report.target_ref)
        },
        if conflicted {
            UiTone::Blocked
        } else {
            UiTone::Success
        },
    )
    .context(format!("{} changed path(s)", report.changed_paths.len()));
    if conflicted {
        document = document.block(UiBlock::Checklist(
            report
                .conflicts
                .iter()
                .map(|conflict| UiCheck::new(UiCheckState::Blocked, "Conflict", conflict))
                .collect(),
        ));
        document = document.next(
            "trail conflicts list",
            "inspect and resolve the merge conflicts",
        );
    } else if report.dry_run {
        document = document.next(
            format!(
                "trail merge {} --into {}",
                report.source_ref, report.target_ref
            ),
            "apply this reviewed merge",
        );
    }
    if !report.changed_paths.is_empty() {
        document = document.block(UiBlock::Changes(change_list(&report.changed_paths)));
    }
    if options.verbose {
        document = document.block(UiBlock::Metadata(vec![
            ("Operation".to_string(), report.operation.0.clone()),
            ("Root".to_string(), report.root_id.0.clone()),
        ]));
    }
    render_document(&document, options)
}

pub(crate) fn render_merge_queue_add(
    report: &LaneMergeQueueAddReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new(
            format!(
                "Queued {} into {}",
                report.entry.lane, report.entry.target_ref
            ),
            UiTone::Success,
        )
        .block(queue_entry_block(&report.entry))
        .next(
            "trail merge-queue explain <queue-id>",
            "check readiness before processing this merge",
        ),
        options,
    )
}

pub(crate) fn render_merge_queue_list(
    entries: &[LaneMergeQueueEntry],
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if entries.is_empty() {
        return render_document(
            &TerminalDocument::new("Merge queue is empty", UiTone::Neutral),
            options,
        );
    }
    render_document(
        &TerminalDocument::new(
            format!("{} queued merge(s)", entries.len()),
            UiTone::Neutral,
        )
        .block(UiBlock::Table(UiTable::new(
            vec![
                UiColumn::left("STATUS", 0, 8),
                UiColumn::right("PRIORITY", 1, 8),
                UiColumn::left("SOURCE", 0, 12),
                UiColumn::left("TARGET", 0, 12),
                UiColumn::left("ID", 2, 10),
            ],
            entries
                .iter()
                .map(|entry| {
                    vec![
                        entry.status.clone(),
                        entry.priority.to_string(),
                        entry.lane.clone(),
                        entry.target_ref.clone(),
                        entry.queue_id.clone(),
                    ]
                })
                .collect(),
        )))
        .next(
            "trail merge-queue explain <queue-id>",
            "inspect blockers and the dry-run before running the queue",
        ),
        options,
    )
}

pub(crate) fn render_merge_queue_run(
    report: &LaneMergeQueueRunReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let tone = if report.stopped_on_conflict {
        UiTone::Blocked
    } else if report.stopped_on_failure {
        UiTone::Failure
    } else {
        UiTone::Success
    };
    let mut document = TerminalDocument::new(
        if report.processed.is_empty() {
            "Merge queue is empty".to_string()
        } else {
            format!("Processed {} queued merge(s)", report.processed.len())
        },
        tone,
    );
    if !report.processed.is_empty() {
        document = document.block(UiBlock::Table(UiTable::new(
            vec![
                UiColumn::left("STATUS", 0, 8),
                UiColumn::left("SOURCE", 0, 12),
                UiColumn::left("TARGET", 0, 12),
                UiColumn::left("RESULT", 1, 16),
            ],
            report
                .processed
                .iter()
                .map(|item| {
                    vec![
                        item.status.clone(),
                        item.lane.clone(),
                        item.target_ref.clone(),
                        item.error
                            .clone()
                            .or_else(|| item.operation.as_ref().map(|id| format!("operation {id}")))
                            .unwrap_or_else(|| "—".to_string()),
                    ]
                })
                .collect(),
        )));
    }
    if report.stopped_on_conflict {
        document = document.next(
            "trail conflicts list",
            "resolve the conflict before continuing the queue",
        );
    } else if report.stopped_on_failure {
        document = document.next(
            "trail merge-queue list",
            "inspect the failed queue item before retrying",
        );
    }
    render_document(&document, options)
}

pub(crate) fn render_merge_queue_explain(
    report: &LaneMergeQueueExplainReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let blocked = !report.blockers.is_empty() || report.error.is_some();
    let mut document = TerminalDocument::new(
        format!(
            "Queue item {} is {}",
            report.entry.queue_id,
            if blocked { "blocked" } else { "ready" }
        ),
        if blocked {
            UiTone::Blocked
        } else {
            UiTone::Success
        },
    )
    .block(queue_entry_block(&report.entry));
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
    if let Some(error) = &report.error {
        checks.push(UiCheck::new(UiCheckState::Fail, "Preflight", error));
    }
    if !checks.is_empty() {
        document = document.block(UiBlock::Checklist(checks));
    }
    if let Some(dry_run) = &report.dry_run {
        if !dry_run.changed_paths.is_empty() {
            document = document.block(UiBlock::section(
                "Dry-run changes:",
                vec![UiBlock::Changes(change_list(&dry_run.changed_paths))],
            ));
        }
    }
    document = append_next_steps(document, &report.next_steps);
    render_document(&document, options)
}

pub(crate) fn render_merge_queue_remove(
    report: &LaneMergeQueueRemoveReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new(
            format!("Removed queued merge {}", report.entry.queue_id),
            UiTone::Success,
        )
        .block(queue_entry_block(&report.entry)),
        options,
    )
}

pub(crate) fn render_conflicts(
    entries: &[ConflictSetSummary],
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if entries.is_empty() {
        return render_document(
            &TerminalDocument::new("No unresolved conflicts", UiTone::Success),
            options,
        );
    }
    render_document(
        &TerminalDocument::new(
            format!("{} unresolved conflict set(s)", entries.len()),
            UiTone::Blocked,
        )
        .block(UiBlock::Table(UiTable::new(
            vec![
                UiColumn::left("STATUS", 0, 8),
                UiColumn::left("SOURCE", 1, 12),
                UiColumn::left("TARGET", 1, 12),
                UiColumn::left("CONFLICT ID", 0, 14),
            ],
            entries
                .iter()
                .map(|entry| {
                    vec![
                        entry.status.clone(),
                        entry.source_ref.clone().unwrap_or_else(|| "—".to_string()),
                        entry.target_ref.clone().unwrap_or_else(|| "—".to_string()),
                        entry.conflict_set_id.clone(),
                    ]
                })
                .collect(),
        )))
        .next(
            "trail conflicts show <conflict-id>",
            "review evidence and the proposed resolutions",
        ),
        options,
    )
}

pub(crate) fn render_conflict(
    entry: &ConflictSetSummary,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(entry);
    }
    let mut document = TerminalDocument::new(
        format!("Conflict {}", entry.conflict_set_id),
        UiTone::Blocked,
    )
    .block(UiBlock::Metadata(vec![
        ("Status".to_string(), entry.status.clone()),
        (
            "Source".to_string(),
            entry.source_ref.clone().unwrap_or_else(|| "—".to_string()),
        ),
        (
            "Target".to_string(),
            entry.target_ref.clone().unwrap_or_else(|| "—".to_string()),
        ),
    ]));
    if !entry.details.is_empty() {
        document = document.block(UiBlock::section(
            "Details:",
            vec![UiBlock::Lines(
                entry
                    .details
                    .iter()
                    .cloned()
                    .map(|detail| (detail, UiTone::Attention))
                    .collect(),
            )],
        ));
    }
    if let Some(explanation) = &entry.explanation {
        document = document.block(UiBlock::section(
            "Paths:",
            vec![UiBlock::Table(UiTable::new(
                vec![
                    UiColumn::left("PATH", 0, 14),
                    UiColumn::left("CLASS", 1, 8),
                    UiColumn::left("RECOMMENDATION", 0, 14),
                    UiColumn::left("WHY", 0, 18),
                ],
                explanation
                    .paths
                    .iter()
                    .map(|path| {
                        vec![
                            path.path.clone(),
                            path.conflict_class.clone(),
                            format!(
                                "{} ({})",
                                path.recommendation.resolution, path.recommendation.confidence
                            ),
                            path.recommendation.reason.clone(),
                        ]
                    })
                    .collect(),
            ))],
        ));
        document = append_next_steps(document, &explanation.next_steps);
    }
    document = document.next(
        format!(
            "trail conflicts resolve {} --take <side>",
            entry.conflict_set_id
        ),
        "apply a reviewed resolution, or use --manual for edited content",
    );
    render_document(&document.pager_eligible(), options)
}

pub(crate) fn render_conflict_resolve(
    report: &ConflictResolveReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut document = TerminalDocument::new(
        format!("Resolved conflict {}", report.conflict_set_id),
        UiTone::Success,
    )
    .context(format!("using {}", report.resolution));
    if !report.changed_paths.is_empty() {
        document = document.block(UiBlock::Changes(change_list(&report.changed_paths)));
    }
    document = document.next(
        "trail conflicts list",
        "confirm no unresolved conflicts remain",
    );
    render_document(&document, options)
}

fn queue_entry_block(entry: &LaneMergeQueueEntry) -> UiBlock {
    UiBlock::Metadata(vec![
        ("ID".to_string(), entry.queue_id.clone()),
        ("Status".to_string(), entry.status.clone()),
        ("Priority".to_string(), entry.priority.to_string()),
        ("Lane".to_string(), entry.lane.clone()),
        ("Target".to_string(), entry.target_ref.clone()),
    ])
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
