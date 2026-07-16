use crate::cli::command::render::*;

use trail::model::*;
use trail::Result;

pub(crate) fn render_anchor_create(
    report: &AnchorCreateReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new(
            format!("Created anchor {}", report.anchor.label),
            UiTone::Success,
        )
        .block(anchor_metadata(&report.anchor))
        .next(
            format!("trail anchor resolve {}", report.anchor.id.0),
            "confirm the anchor still points to the intended code",
        ),
        options,
    )
}

pub(crate) fn render_anchor_resolve(
    report: &AnchorResolveReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let resolved = matches!(report.status.as_str(), "resolved" | "ok" | "active");
    let mut metadata = vec![
        ("Status".to_string(), report.status.clone()),
        ("Branch".to_string(), report.branch.clone()),
    ];
    if let Some(path) = &report.path {
        let location = report
            .line_number
            .map(|line| format!("{path}:{line}"))
            .unwrap_or_else(|| path.clone());
        metadata.push(("Location".to_string(), location));
    }
    let mut document = TerminalDocument::new(
        format!("Anchor {}", report.anchor.label),
        if resolved {
            UiTone::Success
        } else {
            UiTone::Attention
        },
    )
    .block(UiBlock::Metadata(metadata));
    if let Some(text) = &report.text {
        document = document.block(UiBlock::Patch {
            title: "Anchored text".to_string(),
            text: text.clone(),
        });
    }
    render_document(&document, options)
}

pub(crate) fn render_anchor_list(
    anchors: &[Anchor],
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(&anchors);
    }
    if anchors.is_empty() {
        return render_document(
            &TerminalDocument::new("No anchors", UiTone::Neutral),
            options,
        );
    }
    render_document(
        &TerminalDocument::new(format!("{} anchor(s)", anchors.len()), UiTone::Neutral).block(
            UiBlock::Table(UiTable::new(
                vec![
                    UiColumn::left("LABEL", 0, 10),
                    UiColumn::left("LOCATION", 0, 16),
                    UiColumn::left("ID", 2, 12),
                ],
                anchors
                    .iter()
                    .map(|anchor| {
                        vec![
                            anchor.label.clone(),
                            format!("{}:{}", anchor.created_path, anchor.created_line),
                            anchor.id.0.clone(),
                        ]
                    })
                    .collect(),
            )),
        ),
        options,
    )
}

pub(crate) fn render_anchor_delete(
    report: &AnchorDeleteReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new(
            format!("Deleted anchor {}", report.anchor_id.0),
            UiTone::Success,
        ),
        options,
    )
}

pub(crate) fn render_lease_acquire(
    report: &LeaseAcquireReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new(
            format!("Acquired {} lease", report.lease.mode),
            UiTone::Success,
        )
        .block(lease_metadata(&report.lease, options))
        .next(
            format!("trail lease release {}", report.lease.lease_id),
            "release the lease when this coordinated work is complete",
        ),
        options,
    )
}

pub(crate) fn render_lane_claim(
    report: &LaneClaimReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    let tone = if report.claimed {
        UiTone::Success
    } else {
        UiTone::Blocked
    };
    let mut document = TerminalDocument::new(
        if report.claimed {
            format!("Claimed {} for {}", report.path, report.lane_id)
        } else {
            format!("Could not claim {}", report.path)
        },
        tone,
    )
    .block(UiBlock::Metadata(vec![
        ("Lane".to_string(), report.lane_id.clone()),
        ("Ref".to_string(), report.ref_name.clone()),
        ("Mode".to_string(), report.mode.clone()),
        ("TTL".to_string(), format!("{} seconds", report.ttl_secs)),
    ]));
    if !report.conflicts.is_empty() {
        document = document.block(UiBlock::section(
            "Conflicting leases:",
            vec![UiBlock::Table(lease_table(&report.conflicts, options))],
        ));
    }
    if let Some(warning) = &report.warning {
        document = document.block(UiBlock::Checklist(vec![UiCheck::new(
            UiCheckState::Warn,
            "Claim warning",
            warning,
        )]));
    }
    if let Some(warning) = &report.hydration_warning {
        document = document.block(UiBlock::Checklist(vec![UiCheck::new(
            UiCheckState::Warn,
            "Hydration warning",
            warning,
        )]));
    }
    if !report.hydrated_paths.is_empty() {
        document = document.block(UiBlock::Notice(format!(
            "Hydrated {} sparse path(s)",
            report.hydrated_paths.len()
        )));
    }
    if !report.claimed {
        document = document.next(
            "trail lease list",
            "inspect ownership before retrying the claim",
        );
    }
    render_document(&document, options)
}

pub(crate) fn render_lease_list(
    leases: &[LeaseRecord],
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(&leases);
    }
    if leases.is_empty() {
        return render_document(
            &TerminalDocument::new("No active leases", UiTone::Success),
            options,
        );
    }
    render_document(
        &TerminalDocument::new(format!("{} active lease(s)", leases.len()), UiTone::Neutral)
            .block(UiBlock::Table(lease_table(leases, options))),
        options,
    )
}

pub(crate) fn render_lease_release(
    report: &LeaseReleaseReport,
    json: bool,
    options: &RenderOptions,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    render_document(
        &TerminalDocument::new(
            if report.released {
                format!("Released lease {}", report.lease_id)
            } else {
                format!("Lease {} was already released", report.lease_id)
            },
            UiTone::Success,
        ),
        options,
    )
}

fn anchor_metadata(anchor: &Anchor) -> UiBlock {
    UiBlock::Metadata(vec![
        ("ID".to_string(), anchor.id.0.clone()),
        (
            "Location".to_string(),
            format!("{}:{}", anchor.created_path, anchor.created_line),
        ),
        ("Change".to_string(), anchor.created_change.0.clone()),
    ])
}

fn lease_metadata(lease: &LeaseRecord, options: &RenderOptions) -> UiBlock {
    UiBlock::Metadata(vec![
        ("Lease".to_string(), lease.lease_id.clone()),
        ("Lane".to_string(), lease.lane_id.clone()),
        ("Ref".to_string(), lease.ref_name.clone()),
        (
            "Path".to_string(),
            lease
                .path
                .clone()
                .unwrap_or_else(|| "workspace".to_string()),
        ),
        (
            "Expires".to_string(),
            format_timestamp(lease.expires_at, options),
        ),
    ])
}

fn lease_table(leases: &[LeaseRecord], options: &RenderOptions) -> UiTable {
    UiTable::new(
        vec![
            UiColumn::left("LANE", 0, 10),
            UiColumn::left("PATH", 0, 12),
            UiColumn::left("MODE", 1, 7),
            UiColumn::left("EXPIRES", 0, 10),
            UiColumn::left("LEASE ID", 2, 12),
        ],
        leases
            .iter()
            .map(|lease| {
                vec![
                    lease.lane_id.clone(),
                    lease
                        .path
                        .clone()
                        .unwrap_or_else(|| "workspace".to_string()),
                    lease.mode.clone(),
                    format_timestamp(lease.expires_at, options),
                    lease.lease_id.clone(),
                ]
            })
            .collect(),
    )
}
