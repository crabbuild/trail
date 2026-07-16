use super::*;

use trail::model::*;
use trail::Result;

pub(crate) fn render_git_import_update(
    report: &GitImportReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let Some(operation) = &report.operation else {
        return render_document(
            &TerminalDocument::new("No Git-tracked changes to import", UiTone::Neutral),
            options,
        );
    };
    let mut document = TerminalDocument::new(
        format!("Imported Git update {}", operation.0),
        UiTone::Success,
    )
    .block(UiBlock::Metadata(vec![
        ("Files".to_string(), report.imported.files.to_string()),
        ("Text".to_string(), report.imported.text.to_string()),
        ("Opaque".to_string(), report.imported.opaque.to_string()),
        ("Binary".to_string(), report.imported.binary.to_string()),
    ]));
    if !report.changed_paths.is_empty() {
        document = document.block(UiBlock::Changes(change_list(&report.changed_paths)));
    }
    render_document(&document, options)
}

pub(crate) fn render_git_export(
    report: &GitExportReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut metadata = vec![
        ("Commit".to_string(), report.commit.clone()),
        ("Range".to_string(), report.range.clone()),
        ("Trail operation".to_string(), report.operation.0.clone()),
        ("Root".to_string(), report.root_id.0.clone()),
    ];
    if let Some(parent) = &report.parent {
        metadata.push(("Parent".to_string(), parent.clone()));
    }
    if let Some(mapping) = &report.mapping {
        metadata.push(("Mapping".to_string(), mapping.mapping_id.clone()));
    }
    render_document(
        &TerminalDocument::new("Created Git commit", UiTone::Success)
            .block(UiBlock::Metadata(metadata)),
        options,
    )
}

pub(crate) fn render_git_mappings(
    entries: &[GitMapping],
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if entries.is_empty() {
        return render_document(
            &TerminalDocument::new("No Git mappings", UiTone::Neutral),
            options,
        );
    }
    render_document(
        &TerminalDocument::new(format!("{} Git mapping(s)", entries.len()), UiTone::Neutral).block(
            UiBlock::Table(UiTable::new(
                vec![
                    UiColumn::left("DIRECTION", 0, 9),
                    UiColumn::left("HEAD", 1, 12),
                    UiColumn::left("BRANCH", 0, 12),
                    UiColumn::left("STATE", 1, 7),
                    UiColumn::left("TRAIL CHANGE", 2, 12),
                ],
                entries
                    .iter()
                    .map(|entry| {
                        vec![
                            entry.direction.clone(),
                            entry
                                .git_head
                                .clone()
                                .map(|head| head.chars().take(12).collect())
                                .unwrap_or_else(|| "unborn".to_string()),
                            entry.branch.clone(),
                            if entry.git_dirty { "dirty" } else { "clean" }.to_string(),
                            entry.crab_change.0.clone(),
                        ]
                    })
                    .collect(),
            )),
        ),
        options,
    )
}
