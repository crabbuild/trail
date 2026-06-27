use super::super::render_json;

use crabdb::model::*;
use crabdb::Result;

pub(crate) fn render_anchor_create(
    report: &AnchorCreateReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Created anchor {} at {}:{}",
            report.anchor.id.0, report.anchor.created_path, report.anchor.created_line
        );
    }
    Ok(())
}

pub(crate) fn render_anchor_resolve(
    report: &AnchorResolveReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Anchor: {}", report.anchor.id.0);
        println!("Label: {}", report.anchor.label);
        println!("Status: {}", report.status);
        if let (Some(path), Some(line_number)) = (&report.path, report.line_number) {
            println!("Location: {path}:{line_number}");
        } else if let Some(path) = &report.path {
            println!("Path: {path}");
        }
        if let Some(text) = &report.text {
            println!("{text}");
        }
    }
    Ok(())
}

pub(crate) fn render_anchor_list(anchors: &[Anchor], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(&anchors);
    }
    if !quiet {
        for anchor in anchors {
            println!(
                "{} {} {}:{}",
                anchor.id.0, anchor.label, anchor.created_path, anchor.created_line
            );
        }
    }
    Ok(())
}

pub(crate) fn render_anchor_delete(
    report: &AnchorDeleteReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Deleted anchor {}", report.anchor_id.0);
    }
    Ok(())
}

pub(crate) fn render_lease_acquire(
    report: &LeaseAcquireReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Acquired lease {} {} {} {}",
            report.lease.lease_id,
            report.lease.mode,
            report.lease.lane_id,
            report.lease.path.as_deref().unwrap_or("<workspace>")
        );
    }
    Ok(())
}

pub(crate) fn render_lane_claim(report: &LaneClaimReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.claimed {
            if let Some(lease) = &report.lease {
                println!(
                    "Claimed {} for {} until {} ({})",
                    report.path, report.lane_id, lease.expires_at, lease.lease_id
                );
            } else {
                println!("Claimed {} for {}", report.path, report.lane_id);
            }
            if !report.hydrated_paths.is_empty() {
                println!(
                    "Hydrated {} sparse workdir paths",
                    report.hydrated_paths.len()
                );
            }
            if let Some(warning) = &report.hydration_warning {
                println!("Warning: {warning}");
            }
        } else if let Some(warning) = &report.warning {
            println!("Warning: {warning}");
        } else {
            println!("Path {} is already claimed", report.path);
        }
    }
    Ok(())
}

pub(crate) fn render_lease_list(leases: &[LeaseRecord], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(&leases);
    }
    if !quiet {
        if leases.is_empty() {
            println!("No active leases");
        }
        for lease in leases {
            println!(
                "{} {} {} {} expires_at={}",
                lease.lease_id,
                lease.mode,
                lease.lane_id,
                lease.path.as_deref().unwrap_or("<workspace>"),
                lease.expires_at
            );
        }
    }
    Ok(())
}

pub(crate) fn render_lease_release(
    report: &LeaseReleaseReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Released lease {}", report.lease_id);
    }
    Ok(())
}
