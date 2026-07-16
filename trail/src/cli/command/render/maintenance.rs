use super::*;

use trail::model::*;
use trail::Result;

pub(crate) fn render_doctor(
    report: &DoctorReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let checks = report
        .checks
        .iter()
        .map(|check| {
            UiCheck::new(
                doctor_check_state(&check.status),
                &check.name,
                &check.message,
            )
        })
        .collect();
    let tone = if report.status.eq_ignore_ascii_case("ok")
        || report.status.eq_ignore_ascii_case("healthy")
        || report.status.eq_ignore_ascii_case("pass")
    {
        UiTone::Success
    } else {
        UiTone::Attention
    };
    render_document(
        &TerminalDocument::new(format!("Trail diagnostics: {}", report.status), tone)
            .block(UiBlock::Checklist(checks)),
        options,
    )
}

fn doctor_check_state(status: &str) -> UiCheckState {
    match status.to_ascii_lowercase().as_str() {
        "ok" | "pass" | "healthy" | "ready" => UiCheckState::Pass,
        "warning" | "warn" => UiCheckState::Warn,
        "blocked" => UiCheckState::Blocked,
        "pending" | "running" => UiCheckState::Pending,
        "skip" | "skipped" => UiCheckState::Skip,
        _ => UiCheckState::Fail,
    }
}

pub(crate) fn render_backup_create(
    report: &BackupCreateReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut document =
        TerminalDocument::new("Created backup", UiTone::Success).block(UiBlock::Metadata(vec![
            ("Path".to_string(), report.path.clone()),
            ("Branch".to_string(), report.branch.clone()),
            ("Refs".to_string(), report.ref_count.to_string()),
            ("Operations".to_string(), report.operation_count.to_string()),
            (
                "SQLite".to_string(),
                format!("{} bytes", report.sqlite_bytes),
            ),
        ]));
    if !report.fsck_errors.is_empty() {
        document = document.block(UiBlock::Checklist(
            report
                .fsck_errors
                .iter()
                .map(|error| UiCheck::new(UiCheckState::Warn, "Backup integrity", error))
                .collect(),
        ));
    }
    document = document.next(
        format!("trail backup verify {}", report.path),
        "verify the backup before relying on it",
    );
    render_document(&document, options)
}

pub(crate) fn render_backup_verify(
    report: &BackupVerifyReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut document = TerminalDocument::new(
        if report.valid {
            "Backup verification passed"
        } else {
            "Backup verification failed"
        },
        if report.valid {
            UiTone::Success
        } else {
            UiTone::Failure
        },
    )
    .block(UiBlock::Metadata(vec![
        ("Path".to_string(), report.path.clone()),
        ("Refs".to_string(), report.checked_refs.to_string()),
        ("Roots".to_string(), report.checked_roots.to_string()),
        ("Text objects".to_string(), report.checked_texts.to_string()),
    ]));
    if !report.errors.is_empty() {
        document = document.block(UiBlock::Checklist(
            report
                .errors
                .iter()
                .map(|error| UiCheck::new(UiCheckState::Fail, "Verification error", error))
                .collect(),
        ));
    }
    render_document(&document, options)
}

pub(crate) fn render_backup_restore(
    report: &BackupRestoreReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new("Restored backup", UiTone::Success).block(UiBlock::Metadata(vec![
            ("Backup".to_string(), report.backup_path.clone()),
            ("Workspace".to_string(), report.workspace.clone()),
            ("Branch".to_string(), report.branch.clone()),
            (
                "Replaced existing DB".to_string(),
                report.replaced_existing.to_string(),
            ),
            (
                "Rewritten workdirs".to_string(),
                report.rewritten_workdirs.to_string(),
            ),
        ])),
        options,
    )
}

pub(crate) fn render_fsck(report: &FsckReport, json: bool, options: &RenderOptions) -> Result<()> {
    if json {
        return render_json(report);
    }
    let valid = report.errors.is_empty();
    let mut document = TerminalDocument::new(
        if valid {
            "Integrity check passed".to_string()
        } else {
            format!("Integrity check found {} error(s)", report.errors.len())
        },
        if valid {
            UiTone::Success
        } else {
            UiTone::Failure
        },
    )
    .block(UiBlock::Metadata(vec![
        ("Refs".to_string(), report.checked_refs.to_string()),
        ("Roots".to_string(), report.checked_roots.to_string()),
        ("Text objects".to_string(), report.checked_texts.to_string()),
    ]));
    if !valid {
        document = document.block(UiBlock::Checklist(
            report
                .errors
                .iter()
                .map(|error| UiCheck::new(UiCheckState::Fail, "Integrity error", error))
                .collect(),
        ));
    }
    render_document(&document, options)
}

pub(crate) fn render_index_rebuild(
    report: &IndexRebuildReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(&index_rebuild_document(report), options)
}

pub(crate) fn render_change_ledger_reconcile(
    report: &ChangeLedgerReconcileReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new("Reconciled changed-path ledger", UiTone::Success).block(
            UiBlock::Metadata(vec![
                ("Scope".to_string(), report.scope_id.clone()),
                ("Scope kind".to_string(), report.scope_kind.clone()),
                ("Previous state".to_string(), report.previous_state.clone()),
                ("Reason".to_string(), report.reason.clone()),
                (
                    "Observed paths".to_string(),
                    report.observed_paths.to_string(),
                ),
                ("Candidates".to_string(), report.candidates.to_string()),
                (
                    "Resulting epoch".to_string(),
                    report.resulting_epoch.to_string(),
                ),
                (
                    "Resulting state".to_string(),
                    report.resulting_state.clone(),
                ),
            ]),
        ),
        options,
    )
}

fn index_rebuild_document(report: &IndexRebuildReport) -> TerminalDocument {
    let mut document =
        TerminalDocument::new("Rebuilt indexes", UiTone::Success).block(UiBlock::Metadata(vec![
            ("Operations".to_string(), report.operations.to_string()),
            ("Parents".to_string(), report.operation_parents.to_string()),
            (
                "File rows".to_string(),
                report.file_history_rows.to_string(),
            ),
            (
                "Line rows".to_string(),
                report.line_history_rows.to_string(),
            ),
            ("Messages".to_string(), report.messages.to_string()),
            (
                "Path roots repaired".to_string(),
                report.path_index_repaired_roots.len().to_string(),
            ),
            (
                "Live refs repaired".to_string(),
                report.path_index_repaired_refs.len().to_string(),
            ),
        ]));
    if !report.path_index_repaired_refs.is_empty() {
        let refs = report
            .path_index_repaired_refs
            .iter()
            .map(|repair| repair.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        document = document.block(UiBlock::Notice(format!(
            "Upgraded persistent path-invariant indexes for: {refs}"
        )));
    }
    if report.rich_text_hydrated > 0 {
        document = document.block(UiBlock::Notice(format!(
            "Hydrated {} lazy text object(s)",
            report.rich_text_hydrated
        )));
    }
    if !report.errors.is_empty() {
        document = document.block(UiBlock::Checklist(
            report
                .errors
                .iter()
                .map(|error| UiCheck::new(UiCheckState::Warn, "Index warning", error))
                .collect(),
        ));
    }
    document
}

pub(crate) fn render_worktree_index(
    report: &WorktreeIndexReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new("Refreshed worktree index", UiTone::Success).block(
            UiBlock::Metadata(vec![
                ("Files".to_string(), report.files.to_string()),
                (
                    "Cached entries".to_string(),
                    report.indexed_entries.to_string(),
                ),
                ("Duration".to_string(), format!("{} ms", report.duration_ms)),
            ]),
        ),
        options,
    )
}

pub(crate) fn render_gc(report: &GcReport, json: bool, options: &RenderOptions) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut document = TerminalDocument::new(
        if report.dry_run {
            "GC dry run"
        } else {
            "GC complete"
        },
        UiTone::Success,
    )
    .block(UiBlock::Metadata(vec![
        (
            if report.dry_run { "Prunable" } else { "Pruned" }.to_string(),
            if report.dry_run {
                report.prunable_objects
            } else {
                report.pruned_objects
            }
            .to_string(),
        ),
        (
            "Reachable".to_string(),
            report.reachable_objects.to_string(),
        ),
        (
            "Unknown preserved".to_string(),
            report.preserved_unknown_objects.to_string(),
        ),
    ]));
    if !report.errors.is_empty() {
        document = document.block(UiBlock::Checklist(
            report
                .errors
                .iter()
                .map(|error| UiCheck::new(UiCheckState::Warn, "GC warning", error))
                .collect(),
        ));
    }
    if report.dry_run {
        document = document.next(
            "trail gc",
            "remove the objects identified by this reviewed dry run",
        );
    }
    render_document(&document, options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use trail::{ChangeId, ObjectId};

    #[test]
    fn index_rebuild_document_reports_path_index_repairs() {
        let report = IndexRebuildReport {
            path_index_repaired_roots: vec![PathIndexRootRepair {
                old_root: ObjectId("old-root".to_string()),
                new_root: ObjectId("new-root".to_string()),
                case_fold_map_root: "fold-root".to_string(),
            }],
            path_index_repaired_refs: vec![PathIndexRefRepair {
                name: "refs/branches/main".to_string(),
                old_change: ChangeId("old-change".to_string()),
                new_change: ChangeId("new-change".to_string()),
                old_root: ObjectId("old-root".to_string()),
                new_root: ObjectId("new-root".to_string()),
            }],
            ..IndexRebuildReport::default()
        };

        let document = index_rebuild_document(&report);

        let metadata = document
            .blocks
            .iter()
            .find_map(|block| match block {
                UiBlock::Metadata(values) => Some(values),
                _ => None,
            })
            .unwrap();
        assert!(metadata.contains(&("Path roots repaired".to_string(), "1".to_string())));
        assert!(metadata.contains(&("Live refs repaired".to_string(), "1".to_string())));
        assert!(document.blocks.iter().any(|block| {
            matches!(block, UiBlock::Notice(text) if text.contains("refs/branches/main"))
        }));
    }
}
