use std::time::Duration;

use super::super::DiffArgs;
use super::*;

use trail::model::*;
use trail::{Error, Result, Trail};

pub(crate) fn diff_from_args(db: &mut Trail, args: &DiffArgs) -> Result<DiffSummary> {
    validate_diff_view(
        args.patch,
        args.stat,
        args.show_line_ids,
        args.name_only,
        args.name_status,
    )?;
    let forms = usize::from(args.range.is_some())
        + usize::from(args.root.is_some())
        + usize::from(args.dirty);
    if forms != 1 {
        return Err(Error::InvalidInput(
            "diff requires exactly one of RANGE, --root ROOT..ROOT, or --dirty".to_string(),
        ));
    }
    if let Some(range) = &args.range {
        db.diff_range_with_options(range, args.patch, args.show_line_ids)
    } else if let Some(root_range) = &args.root {
        db.diff_roots(root_range, args.patch, args.show_line_ids)
    } else {
        db.diff_dirty(args.patch, args.show_line_ids)
    }
}

pub(crate) fn validate_diff_view(
    patch: bool,
    stat: bool,
    show_line_ids: bool,
    name_only: bool,
    name_status: bool,
) -> Result<()> {
    if name_only && name_status {
        return Err(Error::InvalidInput(
            "diff accepts only one of --name-only or --name-status".to_string(),
        ));
    }
    if (name_only || name_status) && (patch || stat || show_line_ids) {
        return Err(Error::InvalidInput(
            "--name-only and --name-status cannot be combined with --patch, --stat, or --show-line-ids"
                .to_string(),
        ));
    }
    Ok(())
}

pub(crate) fn watch_interval(interval_secs: u64, debounce_ms: Option<u64>) -> Result<Duration> {
    if let Some(ms) = debounce_ms {
        if ms == 0 {
            return Err(Error::InvalidInput(
                "watch debounce must be greater than 0ms".to_string(),
            ));
        }
        return Ok(Duration::from_millis(ms));
    }
    if interval_secs == 0 {
        return Err(Error::InvalidInput(
            "watch interval must be greater than 0 seconds".to_string(),
        ));
    }
    Ok(Duration::from_secs(interval_secs))
}

pub(crate) fn render_diff(
    summary: &DiffSummary,
    json: bool,
    options: &RenderOptions,
    patch_requested: bool,
    stat: bool,
    name_only: bool,
    name_status: bool,
) -> Result<()> {
    render_diff_with_title(
        summary,
        json,
        options,
        patch_requested,
        stat,
        name_only,
        name_status,
        None,
    )
}

pub(crate) fn render_diff_with_title(
    summary: &DiffSummary,
    json: bool,
    options: &RenderOptions,
    patch_requested: bool,
    stat: bool,
    name_only: bool,
    name_status: bool,
    title: Option<&str>,
) -> Result<()> {
    if json {
        return render_json(summary);
    }
    if summary.files.is_empty() {
        return render_document(
            &TerminalDocument::new(
                format!("No changes between {} and {}", summary.from, summary.to),
                UiTone::Success,
            ),
            options,
        );
    }
    if name_only {
        return render_document(
            &TerminalDocument::empty().block(UiBlock::Lines(
                summary
                    .files
                    .iter()
                    .map(|file| (file.path.clone(), UiTone::Neutral))
                    .collect(),
            )),
            options,
        );
    }
    if name_status {
        return render_document(
            &TerminalDocument::empty().block(UiBlock::Lines(
                summary
                    .files
                    .iter()
                    .map(|file| {
                        (
                            format!("{}\t{}", file_change_marker(&file.kind), file.path),
                            UiTone::Neutral,
                        )
                    })
                    .collect(),
            )),
            options,
        );
    }
    let lead = title
        .map(str::to_string)
        .unwrap_or_else(|| format!("Changes from {} to {}", summary.from, summary.to));
    let mut document = TerminalDocument::new(lead, UiTone::Neutral)
        .block(UiBlock::Changes(diff_change_list(&summary.files)));
    if patch_requested && !stat {
        for file in &summary.files {
            if let Some(patch) = &file.patch {
                document = document.block(UiBlock::Patch {
                    title: display_path(file, options),
                    text: patch.clone(),
                });
            } else {
                document = document.block(UiBlock::Notice(format!(
                    "{}: no textual patch is available (binary, opaque, or missing source content).",
                    display_path(file, options)
                )));
            }
            if options.verbose && !file.line_changes.is_empty() {
                document = document.block(UiBlock::Paragraph {
                    text: line_change_summary(file),
                    tone: UiTone::Muted,
                });
            }
        }
    }
    if patch_requested && !stat && summary.files.iter().any(|file| file.patch.is_some()) {
        document = document.pager_eligible();
    }
    render_document(&document, options)
}

fn diff_change_list(files: &[FileDiffSummary]) -> UiChangeList {
    UiChangeList::new(
        files
            .iter()
            .map(|file| UiChange {
                marker: file_change_marker(&file.kind),
                path: file.path.clone(),
                old_path: file.old_path.clone(),
                additions: file.additions,
                deletions: file.deletions,
                detail: if file.patch.is_none() {
                    Some("text patch unavailable".to_string())
                } else {
                    None
                },
            })
            .collect(),
    )
}

fn display_path(file: &FileDiffSummary, options: &RenderOptions) -> String {
    match &file.old_path {
        Some(old_path) => format!("{} {} {}", old_path, options.unicode("→", "->"), file.path),
        None => file.path.clone(),
    }
}

fn line_change_summary(file: &FileDiffSummary) -> String {
    let mut added = 0_u64;
    let mut modified = 0_u64;
    let mut deleted = 0_u64;
    let mut moved = 0_u64;
    for line in &file.line_changes {
        match line.kind {
            LineChangeKind::Added => added += 1,
            LineChangeKind::Modified => modified += 1,
            LineChangeKind::Deleted => deleted += 1,
            LineChangeKind::Moved => moved += 1,
        }
    }
    format!("Stable line changes: +{added} ~{modified} -{deleted} moved {moved}")
}

pub(crate) fn render_history(
    result: &HistoryResult,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(result);
    }
    if result.file_history.is_empty() && result.line_history.is_empty() {
        return render_document(
            &TerminalDocument::new(
                format!("No history found for {}", result.selector),
                UiTone::Neutral,
            ),
            options,
        );
    }
    let mut rows = Vec::new();
    rows.extend(result.file_history.iter().map(|entry| {
        vec![
            entry.change_id.0.clone(),
            format_timestamp(entry.created_at, options),
            file_change_label(&entry.kind).to_string(),
            entry.path.clone(),
        ]
    }));
    rows.extend(result.line_history.iter().map(|entry| {
        let location = entry
            .line_number
            .map(|line| format!("{}:{line}", entry.path))
            .unwrap_or_else(|| entry.path.clone());
        vec![
            entry.change_id.0.clone(),
            format_timestamp(entry.created_at, options),
            line_change_label(&entry.kind).to_string(),
            location,
        ]
    }));
    let document =
        TerminalDocument::new(format!("History for {}", result.selector), UiTone::Neutral)
            .block(UiBlock::Table(UiTable::new(
                vec![
                    UiColumn::left("OPERATION", 0, 16),
                    UiColumn::left("WHEN", 2, 20),
                    UiColumn::left("CHANGE", 1, 9),
                    UiColumn::left("PATH", 0, 12),
                ],
                rows,
            )))
            .pager_eligible();
    render_document(&document, options)
}

pub(crate) fn render_code_from(
    result: &CodeFromResult,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(result);
    }
    if result.operations.is_empty() {
        return render_document(
            &TerminalDocument::new(
                format!("No source operations found for {}", result.selector),
                UiTone::Neutral,
            ),
            options,
        );
    }
    let mut blocks = Vec::new();
    for operation in &result.operations {
        let paths = operation
            .changed_paths
            .iter()
            .map(|path| UiChange {
                marker: file_change_marker(&path.kind),
                path: path.path.clone(),
                old_path: path.old_path.clone(),
                additions: path.additions,
                deletions: path.deletions,
                detail: None,
            })
            .collect();
        let title = format!(
            "{} · {} · {}",
            operation.change_id.0,
            operation_kind_label(&operation.kind),
            operation.branch
        );
        let mut operation_blocks = Vec::new();
        if let Some(message) = &operation.message {
            operation_blocks.push(UiBlock::paragraph(message));
        }
        if !operation.changed_paths.is_empty() {
            operation_blocks.push(UiBlock::Changes(UiChangeList::new(paths)));
        }
        blocks.push(UiBlock::section(title, operation_blocks));
    }
    render_document(
        &TerminalDocument::new(
            format!("Source operations for {}", result.selector),
            UiTone::Neutral,
        )
        .block(UiBlock::section("Operations:", blocks))
        .pager_eligible(),
        options,
    )
}

pub(crate) fn render_why(result: &WhyResult, json: bool, options: &RenderOptions) -> Result<()> {
    if json {
        return render_json(result);
    }
    let mut document = TerminalDocument::new(
        format!("Why {}:{}", result.path, result.line_number),
        UiTone::Neutral,
    )
    .block(UiBlock::paragraph(&result.current_text))
    .block(UiBlock::Metadata(vec![
        ("Line".to_string(), result.line_id.alias()),
        ("Introduced by".to_string(), result.introduced_by.0.clone()),
        (
            "Last content change".to_string(),
            result.last_content_change.0.clone(),
        ),
    ]));
    if !result.history.is_empty() {
        let rows = result
            .history
            .iter()
            .map(|entry| {
                vec![
                    entry.change_id.0.clone(),
                    line_change_label(&entry.kind).to_string(),
                    entry.path.clone(),
                    entry
                        .line_number
                        .map(|line| line.to_string())
                        .unwrap_or_else(|| "—".to_string()),
                ]
            })
            .collect();
        document = document.block(UiBlock::section(
            "Line history:",
            vec![UiBlock::Table(UiTable::new(
                vec![
                    UiColumn::left("OPERATION", 0, 16),
                    UiColumn::left("CHANGE", 1, 9),
                    UiColumn::left("PATH", 0, 12),
                    UiColumn::right("LINE", 2, 4),
                ],
                rows,
            ))],
        ));
    }
    render_document(&document, options)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_projections_are_mutually_exclusive() {
        assert!(validate_diff_view(false, false, false, true, false).is_ok());
        assert!(validate_diff_view(false, false, false, false, true).is_ok());
        assert!(validate_diff_view(false, false, false, true, true).is_err());
        assert!(validate_diff_view(true, false, false, true, false).is_err());
    }
}
