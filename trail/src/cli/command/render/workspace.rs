use super::*;

use trail::model::*;
use trail::{Result, WorktreeState};

pub(crate) fn render_init(report: &InitReport, json: bool, options: &RenderOptions) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut document = TerminalDocument::new("Initialized Trail workspace", UiTone::Success)
        .context(format!("Trail branch {}", report.branch))
        .block(UiBlock::paragraph(format!(
            "Imported {} file{}: {} text, {} opaque, {} binary",
            report.imported.files,
            plural(report.imported.files),
            report.imported.text,
            report.imported.opaque,
            report.imported.binary
        )));
    if options.verbose {
        document = document.block(UiBlock::Metadata(vec![
            ("Workspace".to_string(), report.workspace_id.0.clone()),
            ("Initial operation".to_string(), report.operation.0.clone()),
            ("Root".to_string(), report.root_id.0.clone()),
        ]));
    }
    render_document(&document, options)
}

pub(crate) fn render_status(
    report: &StatusReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(&status_document(report, options), options)
}

fn status_document(report: &StatusReport, options: &RenderOptions) -> TerminalDocument {
    let changed = report.changed_paths.len();
    let (lead, tone, context) = match report.worktree_state {
        WorktreeState::Clean => (
            "Worktree clean".to_string(),
            UiTone::Success,
            format!("Trail branch {}", report.branch),
        ),
        WorktreeState::DirtyTracked => (
            format!(
                "Worktree has {changed} unrecorded change{}",
                plural(changed as u64)
            ),
            UiTone::Attention,
            format!("Trail branch {}", report.branch),
        ),
        WorktreeState::DirtyUntracked => (
            format!(
                "Worktree has {changed} unrecorded change{}, including untracked paths",
                plural(changed as u64)
            ),
            UiTone::Attention,
            format!("Trail branch {}", report.branch),
        ),
    };
    let mut document = TerminalDocument::new(lead, tone).context(context);
    if !report.changed_paths.is_empty() {
        document = document.block(UiBlock::section(
            "Changes:",
            vec![UiBlock::Changes(change_list(&report.changed_paths))],
        ));
    }
    if options.verbose {
        document = document.block(UiBlock::Metadata(vec![
            ("Head".to_string(), report.head.change_id.0.clone()),
            ("Root".to_string(), report.head.root_id.0.clone()),
            ("Ref".to_string(), report.head.name.clone()),
        ]));
    }
    if !matches!(report.worktree_state, WorktreeState::Clean) {
        let primary = report
            .suggestions
            .first()
            .map(|suggestion| (suggestion.command.clone(), suggestion.reason.clone()));
        let (command, reason) = primary.unwrap_or_else(|| {
            (
                "trail diff --dirty".to_string(),
                "Review the unrecorded changes.".to_string(),
            )
        });
        document = document.next(command, reason);
        for suggestion in report.suggestions.iter().skip(1) {
            document = document.more(suggestion.command.clone(), suggestion.reason.clone());
        }
    }
    document
}

pub(crate) fn render_record(
    report: &RecordReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let document = match &report.operation {
        Some(operation) => {
            let mut document = TerminalDocument::new(
                format!(
                    "Recorded {} change{} on {}",
                    report.changed_paths.len(),
                    plural(report.changed_paths.len() as u64),
                    report.branch
                ),
                UiTone::Success,
            )
            .block(UiBlock::Changes(change_list(&report.changed_paths)));
            if options.verbose {
                document = document.block(UiBlock::Metadata(vec![
                    ("Operation".to_string(), operation.0.clone()),
                    ("Root".to_string(), report.root_id.0.clone()),
                ]));
            }
            document
        }
        None => TerminalDocument::new("No changes to record", UiTone::Neutral),
    };
    render_document(&document, options)
}

pub(crate) fn render_timeline(
    entries: &[TimelineEntry],
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if entries.is_empty() {
        return render_document(
            &TerminalDocument::new("No operations recorded", UiTone::Neutral),
            options,
        );
    }
    let rows = entries
        .iter()
        .map(|entry| {
            vec![
                format_timestamp(entry.created_at, options),
                operation_kind_label(&entry.kind).to_string(),
                entry.branch.clone(),
                entry.path_count.to_string(),
                entry.message.clone().unwrap_or_else(|| "—".to_string()),
            ]
        })
        .collect();
    let mut document = TerminalDocument::new(
        format!(
            "{} recent operation{}",
            entries.len(),
            plural(entries.len() as u64)
        ),
        UiTone::Neutral,
    )
    .block(UiBlock::Table(UiTable::new(
        vec![
            UiColumn::left("WHEN", 2, 20),
            UiColumn::left("KIND", 0, 15),
            UiColumn::left("BRANCH", 1, 8),
            UiColumn::right("PATHS", 2, 5),
            UiColumn::left("MESSAGE", 3, 16),
        ],
        rows,
    )));
    if options.verbose {
        document = document.block(UiBlock::section(
            "Operation selectors:",
            vec![UiBlock::Lines(
                entries
                    .iter()
                    .map(|entry| (entry.change_id.0.clone(), UiTone::Muted))
                    .collect(),
            )],
        ));
    }
    render_document(&document, options)
}

pub(crate) fn render_checkout(
    report: &CheckoutReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let lead = if report.dry_run {
        format!(
            "Checkout preview: {} path{} would change",
            report.changed_paths.len(),
            plural(report.changed_paths.len() as u64)
        )
    } else {
        format!(
            "Checked out {} file{}",
            report.written_files,
            plural(report.written_files)
        )
    };
    let mut document = TerminalDocument::new(
        lead,
        if report.dry_run {
            UiTone::Attention
        } else {
            UiTone::Success
        },
    );
    if !report.changed_paths.is_empty() {
        document = document.block(UiBlock::Changes(change_list(&report.changed_paths)));
    }
    if let Some(output_root) = &report.output_root {
        document = document.block(UiBlock::Metadata(vec![(
            "Output".to_string(),
            output_root.clone(),
        )]));
    }
    if let Some(recorded) = &report.recorded_dirty {
        document = document.block(UiBlock::Notice(format!(
            "Recorded existing worktree changes as {} before checkout.",
            recorded.0
        )));
    }
    if options.verbose {
        document = document.block(UiBlock::Metadata(vec![
            ("Operation".to_string(), report.change_id.0.clone()),
            ("Root".to_string(), report.root_id.0.clone()),
        ]));
    }
    render_document(&document, options)
}

pub(crate) fn change_list(paths: &[FileDiffSummary]) -> UiChangeList {
    UiChangeList::new(
        paths
            .iter()
            .map(|path| UiChange {
                marker: file_change_marker(&path.kind),
                path: path.path.clone(),
                old_path: path.old_path.clone(),
                additions: path.additions,
                deletions: path.deletions,
                detail: None,
            })
            .collect(),
    )
}

fn plural(value: u64) -> &'static str {
    if value == 1 {
        ""
    } else {
        "s"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use trail::{ChangeId, ObjectId};

    fn status_fixture(state: WorktreeState, paths: Vec<FileDiffSummary>) -> StatusReport {
        StatusReport {
            branch: "main".to_string(),
            head: RefRecord {
                name: "refs/heads/main".to_string(),
                change_id: ChangeId("change_fixture".to_string()),
                root_id: ObjectId("root_fixture".to_string()),
                operation_id: ObjectId("operation_fixture".to_string()),
                generation: 1,
                updated_at: 0,
            },
            worktree_state: state,
            changed_paths: paths,
            suggestions: vec![StatusSuggestion {
                command: "trail diff --dirty".to_string(),
                reason: "Review the unrecorded changes.".to_string(),
            }],
        }
    }

    fn changed_path(kind: FileChangeKind) -> FileDiffSummary {
        FileDiffSummary {
            path: "src/login.rs".to_string(),
            old_path: None,
            kind,
            before_hash: None,
            after_hash: None,
            additions: 3,
            deletions: 1,
            line_changes: Vec::new(),
            patch: None,
        }
    }

    #[test]
    fn typed_status_fixtures_cover_clean_and_dirty_workspace_states() {
        let options = RenderOptions::test(RenderMode::Plain, 80);
        let clean = status_document(&status_fixture(WorktreeState::Clean, Vec::new()), &options);
        let dirty = status_document(
            &status_fixture(
                WorktreeState::DirtyTracked,
                vec![changed_path(FileChangeKind::Modified)],
            ),
            &options,
        );
        let untracked = status_document(
            &status_fixture(
                WorktreeState::DirtyUntracked,
                vec![changed_path(FileChangeKind::Added)],
            ),
            &options,
        );
        assert_eq!(clean.lead.as_ref().unwrap().text, "Worktree clean");
        assert!(dirty.lead.as_ref().unwrap().text.contains("unrecorded"));
        assert!(untracked.lead.as_ref().unwrap().text.contains("untracked"));
        assert!(dirty.next.is_some());
        assert!(untracked.next.is_some());
    }
}
