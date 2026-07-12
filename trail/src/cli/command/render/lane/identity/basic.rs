use super::render_json;
use crate::cli::command::render::*;

use trail::model::*;
use trail::Result;

pub(crate) fn render_lane_spawn(
    report: &LaneSpawnReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let mut metadata = vec![
        ("Base".to_string(), report.base_change.0.clone()),
        (
            "Requested mode".to_string(),
            report.requested_workdir_mode.as_str().to_string(),
        ),
        (
            "Resolved mode".to_string(),
            report.workdir_mode.as_str().to_string(),
        ),
        (
            "Backend".to_string(),
            report
                .workdir_backend
                .map(WorkdirBackend::as_str)
                .unwrap_or("unverified")
                .to_string(),
        ),
    ];
    if let Some(materialization) = &report.materialization {
        metadata.push((
            "Materialized".to_string(),
            format!(
                "{} cloned ({} bytes), {} copied ({} bytes)",
                materialization.cloned_files,
                materialization.cloned_bytes,
                materialization.copied_files,
                materialization.copied_bytes
            ),
        ));
        if let Some(reason) = materialization.fallback_reason {
            metadata.push(("Fallback".to_string(), reason.as_str().to_string()));
        }
    }
    let mut document =
        TerminalDocument::new(format!("Created lane {}", report.lane_id), UiTone::Success)
            .block(UiBlock::Metadata(metadata));
    if let Some(workdir) = &report.workdir {
        document = document.block(UiBlock::Notice(format!("Workdir: {workdir}")));
    }
    if !report.sparse_paths.is_empty() {
        document = document.block(UiBlock::Metadata(vec![(
            "Sparse paths".to_string(),
            report.sparse_paths.join(", "),
        )]));
    }
    document = document.next(
        format!("trail lane status {}", report.lane_id),
        "inspect the lane before beginning work",
    );
    render_document(&document, options)
}

pub(crate) fn render_lane_list(
    entries: &[LaneDetails],
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if entries.is_empty() {
        return render_document(
            &TerminalDocument::new("No lanes", UiTone::Neutral).next(
                "trail lane spawn <name>",
                "create an isolated lane for new work",
            ),
            options,
        );
    }
    render_document(
        &TerminalDocument::new(format!("{} lane(s)", entries.len()), UiTone::Neutral).block(
            UiBlock::Table(UiTable::new(
                vec![
                    UiColumn::left("NAME", 0, 10),
                    UiColumn::left("STATUS", 0, 8),
                    UiColumn::left("REF", 1, 12),
                    UiColumn::left("HEAD", 2, 10),
                ],
                entries
                    .iter()
                    .map(|entry| {
                        vec![
                            entry.record.name.clone(),
                            entry.branch.status.clone(),
                            entry.branch.ref_name.clone(),
                            entry.branch.head_change.0.clone(),
                        ]
                    })
                    .collect(),
            )),
        ),
        options,
    )
}

pub(crate) fn render_lane_details(
    details: &LaneDetails,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(details);
    }
    let mut metadata = vec![
        ("ID".to_string(), details.record.lane_id.clone()),
        ("Ref".to_string(), details.branch.ref_name.clone()),
        ("Status".to_string(), details.branch.status.clone()),
        ("Base".to_string(), details.branch.base_change.0.clone()),
        ("Head".to_string(), details.branch.head_change.0.clone()),
    ];
    if let Some(provider) = &details.record.provider {
        metadata.push(("Provider".to_string(), provider.clone()));
    }
    if let Some(model) = &details.record.model {
        metadata.push(("Model".to_string(), model.clone()));
    }
    if let Some(session_id) = &details.branch.session_id {
        metadata.push(("Session".to_string(), session_id.clone()));
    }
    if let Some(workdir) = &details.branch.workdir {
        metadata.push(("Workdir".to_string(), workdir.clone()));
    }
    render_document(
        &TerminalDocument::new(format!("Lane {}", details.record.name), UiTone::Neutral)
            .block(UiBlock::Metadata(metadata))
            .next(
                format!("trail lane status {}", details.record.name),
                "inspect lane state and merge readiness",
            ),
        options,
    )
}
