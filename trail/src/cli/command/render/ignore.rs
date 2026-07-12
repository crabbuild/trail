use super::*;

use trail::model::*;
use trail::Result;

pub(crate) fn render_ignore_list(
    report: &IgnoreListReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut document =
        TerminalDocument::new("Trail ignore rules", UiTone::Neutral).context(report.path.clone());
    if report.patterns.is_empty() {
        document = document.block(UiBlock::paragraph("No ignore patterns are configured."));
    } else {
        document = document.block(UiBlock::Table(UiTable::new(
            vec![
                UiColumn::right("LINE", 0, 4),
                UiColumn::left("PATTERN", 0, 16),
            ],
            report
                .patterns
                .iter()
                .map(|pattern| vec![pattern.line.to_string(), pattern.pattern.clone()])
                .collect(),
        )));
    }
    render_document(&document, options)
}

pub(crate) fn render_ignore_add(
    report: &IgnoreAddReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let (lead, tone) = if report.added {
        ("Added ignore pattern", UiTone::Success)
    } else {
        ("Ignore pattern already present", UiTone::Neutral)
    };
    render_document(
        &TerminalDocument::new(lead, tone).block(UiBlock::paragraph(&report.pattern)),
        options,
    )
}

pub(crate) fn render_ignore_remove(
    report: &IgnoreRemoveReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let (lead, tone) = if report.removed {
        ("Removed ignore pattern", UiTone::Success)
    } else {
        ("Ignore pattern not present", UiTone::Neutral)
    };
    render_document(
        &TerminalDocument::new(lead, tone).block(UiBlock::paragraph(&report.pattern)),
        options,
    )
}

pub(crate) fn render_ignore_check(
    report: &IgnoreCheckReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut document = TerminalDocument::new(
        if report.ignored {
            format!("{} is ignored", report.path)
        } else {
            format!("{} is not ignored", report.path)
        },
        if report.ignored {
            UiTone::Attention
        } else {
            UiTone::Success
        },
    );
    if let Some(source) = &report.source {
        document = document.block(UiBlock::Metadata(vec![(
            "Rule".to_string(),
            source.clone(),
        )]));
    }
    render_document(&document, options)
}
