use super::*;

use trail::model::*;
use trail::Result;

pub(crate) fn render_config_list(
    entries: &[ConfigEntry],
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if entries.is_empty() {
        return render_document(
            &TerminalDocument::new("No configuration values", UiTone::Neutral),
            options,
        );
    }
    let rows = entries
        .iter()
        .map(|entry| {
            vec![
                entry.key.clone(),
                entry.value.clone(),
                entry.value_type.clone(),
                if entry.read_only {
                    "read-only"
                } else {
                    "editable"
                }
                .to_string(),
            ]
        })
        .collect();
    render_document(
        &TerminalDocument::new(
            format!("{} configuration value(s)", entries.len()),
            UiTone::Neutral,
        )
        .block(UiBlock::Table(UiTable::new(
            vec![
                UiColumn::left("KEY", 0, 12),
                UiColumn::left("VALUE", 0, 16),
                UiColumn::left("TYPE", 2, 8),
                UiColumn::left("ACCESS", 2, 8),
            ],
            rows,
        ))),
        options,
    )
}

pub(crate) fn render_config_entry(
    entry: &ConfigEntry,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(entry);
    }
    render_document(
        &TerminalDocument::new(format!("Configuration {}", entry.key), UiTone::Neutral).block(
            UiBlock::Metadata(vec![
                ("Value".to_string(), entry.value.clone()),
                ("Type".to_string(), entry.value_type.clone()),
                (
                    "Access".to_string(),
                    if entry.read_only {
                        "read-only"
                    } else {
                        "editable"
                    }
                    .to_string(),
                ),
            ]),
        ),
        options,
    )
}

pub(crate) fn render_config_set(
    report: &ConfigSetReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new(
            format!("Updated configuration {}", report.key),
            UiTone::Success,
        )
        .block(UiBlock::Metadata(vec![
            ("Previous".to_string(), report.old_value.clone()),
            ("Current".to_string(), report.new_value.clone()),
        ])),
        options,
    )
}
