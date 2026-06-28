use super::render_json;

use crabdb::model::*;
use crabdb::Result;

pub(crate) fn render_lane_spawn(report: &LaneSpawnReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Spawned {} at {}", report.lane_id, report.base_change.0);
        println!("Workdir mode: {}", report.workdir_mode.as_str());
        if let Some(cow_backend) = &report.cow_backend {
            println!("COW backend: {cow_backend}");
        }
        if !report.sparse_paths.is_empty() {
            println!("Sparse paths: {}", report.sparse_paths.join(", "));
        }
        if let Some(workdir) = &report.workdir {
            println!("Workdir: {workdir}");
        }
    }
    Ok(())
}

pub(crate) fn render_lane_list(entries: &[LaneDetails], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if !quiet {
        for entry in entries {
            println!(
                "{} {} {} {}",
                entry.record.name,
                entry.branch.status,
                entry.branch.head_change.0,
                entry.branch.ref_name
            );
        }
    }
    Ok(())
}

pub(crate) fn render_lane_details(details: &LaneDetails, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(details);
    }
    if !quiet {
        println!("Lane: {}", details.record.name);
        println!("ID: {}", details.record.lane_id);
        println!("Ref: {}", details.branch.ref_name);
        println!("Status: {}", details.branch.status);
        println!("Base: {}", details.branch.base_change.0);
        println!("Head: {}", details.branch.head_change.0);
        if let Some(provider) = &details.record.provider {
            println!("Provider: {provider}");
        }
        if let Some(model) = &details.record.model {
            println!("Model: {model}");
        }
        if let Some(session_id) = &details.branch.session_id {
            println!("Session: {session_id}");
        }
        if let Some(workdir) = &details.branch.workdir {
            println!("Workdir: {workdir}");
        }
        if let Some(metadata_json) = &details.record.metadata_json {
            if let Ok(metadata) = serde_json::from_str::<serde_json::Value>(metadata_json) {
                if let Some(mode) = metadata
                    .get("workdir_mode")
                    .and_then(serde_json::Value::as_str)
                {
                    println!("Workdir mode: {mode}");
                }
            }
        }
    }
    Ok(())
}
