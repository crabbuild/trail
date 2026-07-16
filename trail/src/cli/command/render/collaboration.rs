use super::*;

use trail::model::*;
use trail::Result;

mod coordination;
mod merge;

pub(crate) use coordination::*;
pub(crate) use merge::*;

pub(crate) fn render_branch(
    report: &BranchReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new(
            format!("Created Trail branch {}", report.name),
            UiTone::Success,
        )
        .context(format!("From {}", report.from.0)),
        options,
    )
}

pub(crate) fn render_branch_list(
    entries: &[BranchListEntry],
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    render_document(
        &TerminalDocument::new(
            format!("{} Trail branch(es)", entries.len()),
            UiTone::Neutral,
        )
        .block(UiBlock::Table(UiTable::new(
            vec![
                UiColumn::left("BRANCH", 0, 12),
                UiColumn::left("STATE", 1, 7),
                UiColumn::right("GEN", 2, 3),
            ],
            entries
                .iter()
                .map(|entry| {
                    vec![
                        entry.name.clone(),
                        if entry.is_current { "current" } else { "" }.to_string(),
                        entry.generation.to_string(),
                    ]
                })
                .collect(),
        ))),
        options,
    )
}

pub(crate) fn render_branch_delete(
    report: &BranchDeleteReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new(
            format!("Deleted Trail branch {}", report.name),
            UiTone::Success,
        ),
        options,
    )
}

pub(crate) fn render_branch_rename(
    report: &BranchRenameReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new(
            format!("Renamed Trail branch {}", report.old_name),
            UiTone::Success,
        )
        .context(format!("New name: {}", report.new_name)),
        options,
    )
}
