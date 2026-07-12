use crate::cli::command::render::*;

use trail::model::*;
use trail::Result;

pub(crate) fn render_lane_record(
    report: &LaneRecordReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let Some(operation) = &report.operation else {
        return render_document(
            &TerminalDocument::new("No lane workdir changes to record", UiTone::Neutral),
            options,
        );
    };
    let mut document =
        TerminalDocument::new(format!("Recorded lane {}", report.lane_id), UiTone::Success)
            .context(format!("{} changed path(s)", report.changed_paths.len()));
    if !report.changed_paths.is_empty() {
        document = document.block(UiBlock::Changes(change_list(&report.changed_paths)));
    }
    if options.verbose {
        document = document.block(UiBlock::Metadata(vec![
            ("Operation".to_string(), operation.0.clone()),
            ("Root".to_string(), report.root_id.0.clone()),
        ]));
    }
    render_document(&document, options)
}

pub(crate) fn render_workspace_checkpoint(
    report: &WorkspaceCheckpointReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut document = TerminalDocument::new(
        format!("Checkpointed workspace view {}", report.view_id),
        UiTone::Success,
    )
    .block(UiBlock::Metadata(vec![
        (
            "Journal sequence".to_string(),
            report.journal_sequence.to_string(),
        ),
        (
            "Source paths".to_string(),
            report.source_paths.len().to_string(),
        ),
        (
            "Generated dirty paths".to_string(),
            report.generated_dirty_paths.to_string(),
        ),
    ]));
    if options.verbose {
        document = document.block(UiBlock::Metadata(vec![
            ("Root".to_string(), report.root_id.0.clone()),
            (
                "Operation".to_string(),
                report
                    .operation
                    .as_ref()
                    .map(|operation| operation.0.clone())
                    .unwrap_or_else(|| "—".to_string()),
            ),
        ]));
    }
    render_document(&document, options)
}

pub(crate) fn render_workspace_space(
    report: &WorkspaceSpaceReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new(
            format!("Workspace space for view {}", report.view_id),
            UiTone::Neutral,
        )
        .block(UiBlock::Metadata(vec![
            (
                "Logical visible".to_string(),
                byte_count(report.logical_visible_bytes),
            ),
            (
                "Shared physical".to_string(),
                byte_count(report.shared_physical_bytes),
            ),
            (
                "Lane-exclusive".to_string(),
                byte_count(report.lane_exclusive_physical_bytes),
            ),
            (
                "Uncheckpointed".to_string(),
                byte_count(report.uncheckpointed_source_bytes),
            ),
            ("Accounting".to_string(), report.physical_accounting.clone()),
        ])),
        options,
    )
}

pub(crate) fn render_workspace_exec(
    report: &WorkspaceExecReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let success = report.exit_code == 0;
    let mut document = TerminalDocument::new(
        if success {
            format!("Workspace command completed for {}", report.lane_id)
        } else {
            format!("Workspace command exited with {}", report.exit_code)
        },
        if success {
            UiTone::Success
        } else {
            UiTone::Failure
        },
    )
    .block(UiBlock::Metadata(vec![
        ("Command".to_string(), report.command.join(" ")),
        ("View".to_string(), report.view_id.clone()),
        ("Backend".to_string(), report.backend.clone()),
        ("Generation".to_string(), report.generation.to_string()),
    ]));
    if options.verbose {
        document = document.block(UiBlock::Metadata(vec![(
            "Source root".to_string(),
            report.source_root.0.clone(),
        )]));
    }
    render_document(&document, options)
}

pub(crate) fn render_workspace_mount(
    report: &WorkspaceMountReport,
    state: &str,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let tone = if report.healthy {
        UiTone::Success
    } else {
        UiTone::Attention
    };
    render_document(
        &TerminalDocument::new(format!("Workspace view {} {}", report.view_id, state), tone).block(
            UiBlock::Metadata(vec![
                ("Mountpoint".to_string(), report.mountpoint.clone()),
                ("Backend".to_string(), report.backend.clone()),
                ("Generation".to_string(), report.generation.to_string()),
                (
                    "Health".to_string(),
                    if report.healthy {
                        "healthy"
                    } else {
                        "needs attention"
                    }
                    .to_string(),
                ),
            ]),
        ),
        options,
    )
}

pub(crate) fn render_lane_record_preview(
    report: &LaneRecordPreviewReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let tone = if !report.policy.allowed {
        UiTone::Blocked
    } else if report.clean {
        UiTone::Neutral
    } else {
        UiTone::Attention
    };
    let mut document = TerminalDocument::new(
        if report.clean {
            format!("No lane workdir changes to record for {}", report.lane_id)
        } else {
            format!("Record preview for lane {}", report.lane_id)
        },
        tone,
    )
    .block(UiBlock::Metadata(vec![
        ("Workdir".to_string(), report.workdir.clone()),
        (
            "Policy".to_string(),
            if report.policy.allowed {
                "allowed"
            } else {
                "blocked"
            }
            .to_string(),
        ),
        (
            "Changed paths".to_string(),
            report.changed_paths.len().to_string(),
        ),
    ]));
    let mut checks = Vec::new();
    if let Some(error) = &report.policy.error {
        checks.push(UiCheck::new(UiCheckState::Blocked, "Record policy", error));
    }
    checks.extend(
        report
            .policy
            .warnings
            .iter()
            .map(|warning| UiCheck::new(UiCheckState::Warn, "Record policy", warning)),
    );
    checks.extend(report.risky_paths.iter().map(|path| {
        UiCheck::new(
            UiCheckState::Warn,
            format!("Risky {}", path.kind),
            format!("{}: {}", path.path, path.message),
        )
    }));
    if !checks.is_empty() {
        document = document.block(UiBlock::Checklist(checks));
    }
    if !report.changed_paths.is_empty() {
        document = document.block(UiBlock::Changes(change_list(&report.changed_paths)));
    }
    if !report.oversized_files.is_empty() {
        document = document.block(UiBlock::section(
            "Oversized files:",
            vec![UiBlock::Table(UiTable::new(
                vec![
                    UiColumn::left("PATH", 0, 16),
                    UiColumn::right("SIZE", 1, 8),
                    UiColumn::right("LIMIT", 1, 8),
                ],
                report
                    .oversized_files
                    .iter()
                    .map(|file| {
                        vec![
                            file.path.clone(),
                            byte_count(file.size_bytes),
                            byte_count(file.limit_bytes),
                        ]
                    })
                    .collect(),
            ))],
        ));
    }
    if !report.policy.allowed {
        document = document.next(
            format!("trail lane record {} --preview", report.lane_id),
            "resolve the listed policy blockers before recording",
        );
    } else if !report.clean {
        document = document.next(
            format!("trail lane record {}", report.lane_id),
            "record the reviewed lane changes",
        );
    }
    render_document(&document, options)
}

pub(crate) fn render_lane_rewind(
    report: &LaneRewindReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut document = TerminalDocument::new(
        format!("Rewound lane {} to {}", report.lane_id, report.target),
        UiTone::Success,
    )
    .block(UiBlock::Metadata(vec![
        ("Target change".to_string(), report.target_change.0.clone()),
        (
            "Workdir".to_string(),
            if report.workdir_synced {
                "synced"
            } else {
                "not synced"
            }
            .to_string(),
        ),
    ]));
    if !report.changed_paths.is_empty() {
        document = document.block(UiBlock::Changes(change_list(&report.changed_paths)));
    }
    if let Some(recorded) = &report.recorded_current {
        document = document.block(UiBlock::Notice(format!(
            "Recorded current workdir as {} before rewinding.",
            recorded.0
        )));
    }
    render_document(&document, options)
}

pub(crate) fn render_lane_watch(
    report: &LaneWatchReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut document =
        TerminalDocument::new(format!("Watched lane {}", report.lane_id), UiTone::Success).context(
            format!(
                "{} iteration(s) · {} recorded operation(s)",
                report.iterations,
                report.recorded_operations.len()
            ),
        );
    if !report.changed_paths.is_empty() {
        document = document.block(UiBlock::Changes(change_list(&report.changed_paths)));
    }
    render_document(&document, options)
}

pub(crate) fn render_lane_test(
    report: &LaneTestReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut document = TerminalDocument::new(
        format!(
            "{} {} for lane {}",
            report.kind,
            if report.success { "passed" } else { "failed" },
            report.lane_id
        ),
        if report.success {
            UiTone::Success
        } else {
            UiTone::Failure
        },
    )
    .block(UiBlock::Metadata(vec![
        ("Command".to_string(), report.command.join(" ")),
        ("Turn".to_string(), report.turn_id.clone()),
        ("Status".to_string(), report.status.clone()),
        (
            "Duration".to_string(),
            super::super::format_duration(report.duration_ms, options),
        ),
        (
            "Exit".to_string(),
            report
                .exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| {
                    if report.timed_out {
                        "timed out"
                    } else {
                        "unavailable"
                    }
                    .to_string()
                }),
        ),
    ]));
    if let Some(suite) = &report.suite {
        document = document.block(UiBlock::Metadata(vec![(
            "Suite".to_string(),
            suite.clone(),
        )]));
    }
    if report.score.is_some() || report.threshold.is_some() {
        document = document.block(UiBlock::Metadata(vec![(
            "Score".to_string(),
            format!(
                "{} / threshold {}",
                report
                    .score
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "n/a".to_string()),
                report
                    .threshold
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "n/a".to_string())
            ),
        )]));
    }
    if !report.stdout_preview.is_empty() {
        document = document.block(UiBlock::Patch {
            title: if report.stdout_truncated {
                "Stdout (truncated)"
            } else {
                "Stdout"
            }
            .to_string(),
            text: report.stdout_preview.clone(),
        });
    }
    if !report.stderr_preview.is_empty() {
        document = document.block(UiBlock::Patch {
            title: if report.stderr_truncated {
                "Stderr (truncated)"
            } else {
                "Stderr"
            }
            .to_string(),
            text: report.stderr_preview.clone(),
        });
    }
    render_document(&document.pager_eligible(), options)
}

pub(crate) fn render_lane_workdir(
    report: &LaneWorkdirReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let Some(workdir) = &report.workdir else {
        return render_document(
            &TerminalDocument::new(
                format!("Lane {} has no materialized workdir", report.lane_id),
                UiTone::Neutral,
            ),
            options,
        );
    };
    render_document(
        &TerminalDocument::new(
            format!("Workdir for lane {}", report.lane_id),
            UiTone::Success,
        )
        .block(UiBlock::Metadata(vec![
            ("Path".to_string(), workdir.clone()),
            ("Mode".to_string(), report.workdir_mode.as_str().to_string()),
            (
                "Transparent COW available".to_string(),
                report.transparent_cow_available.to_string(),
            ),
        ])),
        options,
    )
}

pub(crate) fn render_lane_file_read(
    report: &LaneFileReadReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    // File reads are intentionally raw so callers can inspect or pipe exact
    // content. This is the documented raw-content exception for the CLI.
    render_raw_content(&report.content, options)
}

pub(crate) fn render_lane_workdir_sync(
    report: &LaneWorkdirSyncReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut document = TerminalDocument::new(
        format!("Synced workdir for lane {}", report.lane_id),
        UiTone::Success,
    )
    .block(UiBlock::Metadata(vec![
        ("Workdir".to_string(), report.workdir.clone()),
        ("Head".to_string(), report.head_change.0.clone()),
        ("Forced".to_string(), report.forced.to_string()),
    ]));
    if let Some(rescue) = &report.rescue_workdir {
        document = document.block(UiBlock::Notice(format!("Rescue workdir: {rescue}")));
    }
    if !report.changed_paths.is_empty() {
        document = document.block(UiBlock::Changes(change_list(&report.changed_paths)));
    }
    render_document(&document, options)
}

pub(crate) fn render_lane_patch(
    report: &LanePatchReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut document = TerminalDocument::new(
        format!("Applied patch to lane {}", report.lane_id),
        UiTone::Success,
    )
    .context(format!("{} changed path(s)", report.changed_paths.len()));
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

pub(crate) fn render_lane_remove(
    report: &LaneRemoveReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut document =
        TerminalDocument::new(format!("Removed lane {}", report.lane_id), UiTone::Success).block(
            UiBlock::Metadata(vec![
                ("Ref".to_string(), report.ref_name.clone()),
                ("Forced".to_string(), report.forced.to_string()),
            ]),
        );
    if let Some(workdir) = &report.removed_workdir {
        document = document.block(UiBlock::Notice(format!("Removed workdir: {workdir}")));
    }
    render_document(&document, options)
}

fn byte_count(value: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    match value {
        value if value >= GIB => format!("{:.1} GiB", value as f64 / GIB as f64),
        value if value >= MIB => format!("{:.1} MiB", value as f64 / MIB as f64),
        value if value >= KIB => format!("{:.1} KiB", value as f64 / KIB as f64),
        _ => format!("{value} B"),
    }
}
