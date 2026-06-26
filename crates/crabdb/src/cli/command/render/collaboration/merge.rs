use super::super::render_json;

use crabdb::model::*;
use crabdb::Result;

pub(crate) fn render_merge(report: &MergeReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.dry_run {
            println!(
                "Would merge {} into {} as {}",
                report.source_ref, report.target_ref, report.operation.0
            );
        } else {
            println!(
                "Merged {} into {} as {}",
                report.source_ref, report.target_ref, report.operation.0
            );
        }
        for conflict in &report.conflicts {
            println!("  conflict {conflict}");
        }
        for path in &report.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
    }
    Ok(())
}

pub(crate) fn render_merge_queue_add(
    report: &MergeQueueAddReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Queued {} into {} as {}",
            report.entry.source_ref, report.entry.target_ref, report.entry.queue_id
        );
    }
    Ok(())
}

pub(crate) fn render_merge_queue_list(
    entries: &[MergeQueueEntry],
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if !quiet {
        for entry in entries {
            println!(
                "{} {} priority={} {} -> {}",
                entry.queue_id, entry.status, entry.priority, entry.source_ref, entry.target_ref
            );
        }
    }
    Ok(())
}

pub(crate) fn render_merge_queue_run(
    report: &MergeQueueRunReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.processed.is_empty() {
            println!("Merge queue is empty");
        }
        for item in &report.processed {
            match (&item.operation, &item.error) {
                (Some(operation), _) => println!(
                    "{} {} as {} {} -> {}",
                    item.queue_id, item.status, operation.0, item.source_ref, item.target_ref
                ),
                (None, Some(error)) => println!(
                    "{} {} {} -> {}: {}",
                    item.queue_id, item.status, item.source_ref, item.target_ref, error
                ),
                (None, None) => println!(
                    "{} {} {} -> {}",
                    item.queue_id, item.status, item.source_ref, item.target_ref
                ),
            }
        }
        if report.stopped_on_conflict {
            println!("Paused on conflict");
        } else if report.stopped_on_failure {
            println!("Paused on failure");
        }
    }
    Ok(())
}

pub(crate) fn render_merge_queue_remove(
    report: &MergeQueueRemoveReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Cancelled {}", report.entry.queue_id);
    }
    Ok(())
}

pub(crate) fn render_conflicts(
    entries: &[ConflictSetSummary],
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if !quiet {
        if entries.is_empty() {
            println!("No conflicts");
        }
        for entry in entries {
            println!(
                "{} {} {} -> {}",
                entry.conflict_set_id,
                entry.status,
                entry.source_ref.as_deref().unwrap_or("-"),
                entry.target_ref.as_deref().unwrap_or("-")
            );
            for detail in &entry.details {
                println!("  {detail}");
            }
        }
    }
    Ok(())
}

pub(crate) fn render_conflict(entry: &ConflictSetSummary, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(entry);
    }
    if !quiet {
        println!("Conflict: {}", entry.conflict_set_id);
        println!("Status: {}", entry.status);
        if let Some(merge_id) = &entry.merge_id {
            println!("Merge: {merge_id}");
        }
        if let Some(source) = &entry.source_ref {
            println!("Source: {source}");
        }
        if let Some(target) = &entry.target_ref {
            println!("Target: {target}");
        }
        for detail in &entry.details {
            println!("  {detail}");
        }
        if let Some(explanation) = &entry.explanation {
            println!();
            println!("Explanation:");
            println!(
                "  Merge: {} target={} source={} base={}",
                explanation.merge.merge_id,
                explanation.merge.target_change.0,
                explanation.merge.source_change.0,
                explanation.merge.base_change.0
            );
            for path in &explanation.paths {
                println!("  Path: {}", path.path);
                println!("    {}", path.summary);
                println!("    Reason: {}", path.reason);
                if let Some(target) = &path.target {
                    println!("    Target: {}", side_summary(target));
                }
                if let Some(source) = &path.source {
                    println!("    Source: {}", side_summary(source));
                }
                for line in &path.lines {
                    println!("    Line {}: {}", line.line_id, line.reason);
                    if let Some(base) = &line.base {
                        println!("      base: {}", printable_line(base));
                    }
                    if let Some(target) = &line.target {
                        println!("      target: {}", printable_line(target));
                    }
                    if let Some(source) = &line.source {
                        println!("      source: {}", printable_line(source));
                    }
                }
                println!(
                    "    Recommend: {} ({}) - {}",
                    path.recommendation.resolution,
                    path.recommendation.confidence,
                    path.recommendation.reason
                );
            }
            if !explanation.next_steps.is_empty() {
                println!();
                println!("Next steps:");
                for step in &explanation.next_steps {
                    println!("  - {step}");
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn render_conflict_resolve(
    report: &ConflictResolveReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.resolution == "manual" {
            println!(
                "Resolved {} manually as {}",
                report.conflict_set_id, report.operation.0
            );
        } else {
            println!(
                "Resolved {} by taking {} as {}",
                report.conflict_set_id, report.resolution, report.operation.0
            );
        }
        for path in &report.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
    }
    Ok(())
}

fn side_summary(side: &ConflictSideProvenance) -> String {
    let mut value = format!(
        "{} {} by {} on {}",
        side.kind, side.change_id.0, side.actor_id, side.branch
    );
    if let Some(session) = &side.session_id {
        value.push_str(&format!(" session={session}"));
    }
    if let Some(message) = &side.message {
        value.push_str(&format!(" - {message}"));
    }
    value
}

fn printable_line(value: &str) -> String {
    value.replace('\r', "\\r").replace('\n', "\\n")
}
