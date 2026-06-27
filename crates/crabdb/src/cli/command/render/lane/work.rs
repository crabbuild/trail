use super::render_json;

use crabdb::model::*;
use crabdb::Result;
use std::io::Write;

pub(crate) fn render_lane_record(report: &LaneRecordReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        match &report.operation {
            Some(operation) => {
                println!("Recorded lane workdir {}", operation.0);
                for path in &report.changed_paths {
                    println!("  {:?} {}", path.kind, path.path);
                }
            }
            None => println!("No lane workdir changes to record"),
        }
    }
    Ok(())
}

pub(crate) fn render_lane_record_preview(
    report: &LaneRecordPreviewReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Lane workdir record preview: {}", report.workdir);
        println!("Head: {}", report.head_change.0);
        println!("Policy allowed: {}", report.policy.allowed);
        if let Some(error) = &report.policy.error {
            println!("Policy error: {error}");
        }
        for warning in &report.policy.warnings {
            println!("Policy warning: {warning}");
        }
        if report.clean {
            println!("No lane workdir changes to record");
        } else {
            println!("Changed paths:");
            for path in &report.changed_paths {
                println!("  {:?} {}", path.kind, path.path);
            }
        }
        if !report.oversized_files.is_empty() {
            println!("Oversized files:");
            for file in &report.oversized_files {
                println!(
                    "  {} ({} bytes > {} bytes)",
                    file.path, file.size_bytes, file.limit_bytes
                );
            }
        }
        if !report.ignored_paths.is_empty() {
            println!("Ignored paths:");
            for path in &report.ignored_paths {
                println!("  {} ({})", path.path, path.source);
            }
        }
        if !report.risky_paths.is_empty() {
            println!("Risky paths:");
            for path in &report.risky_paths {
                println!("  {} [{}] {}", path.path, path.kind, path.message);
            }
        }
    }
    Ok(())
}

pub(crate) fn render_lane_rewind(report: &LaneRewindReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Rewound lane {} from {} to {}",
            report.lane_id, report.previous_change.0, report.operation.0
        );
        println!("Target: {} ({})", report.target, report.target_change.0);
        if let Some(operation) = &report.recorded_current {
            println!("Recorded current workdir: {}", operation.0);
        }
        if let Some(branch) = &report.preserved_branch {
            println!("Preserved previous head: {branch}");
        }
        if report.workdir_synced {
            if let Some(workdir) = &report.workdir {
                println!("Synced workdir: {workdir}");
            }
        } else if report.workdir.is_some() {
            println!("Workdir not synced");
        }
        for path in &report.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
    }
    Ok(())
}

pub(crate) fn render_lane_watch(report: &LaneWatchReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Watched {} for {} iteration(s); recorded {} operation(s)",
            report.lane_id,
            report.iterations,
            report.recorded_operations.len()
        );
        for operation in &report.recorded_operations {
            println!("  {operation}");
        }
        for path in &report.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
    }
    Ok(())
}

pub(crate) fn render_lane_test(report: &LaneTestReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Lane {} {} for {}",
            report.kind, report.status, report.lane_id
        );
        println!("Turn: {}", report.turn_id);
        println!("Command: {}", report.command.join(" "));
        if let Some(suite) = &report.suite {
            println!("Suite: {suite}");
        }
        if report.score.is_some() || report.threshold.is_some() {
            let score = report
                .score
                .map(|value| value.to_string())
                .unwrap_or_else(|| "n/a".to_string());
            let threshold = report
                .threshold
                .map(|value| value.to_string())
                .unwrap_or_else(|| "n/a".to_string());
            println!("Score: {score} / threshold {threshold}");
        }
        match report.exit_code {
            Some(code) => println!("Exit: {code}"),
            None if report.timed_out => println!("Exit: timed out"),
            None => println!("Exit: unavailable"),
        }
        println!("Duration: {} ms", report.duration_ms);
        println!("Stdout object: {}", report.stdout_object.0);
        println!("Stderr object: {}", report.stderr_object.0);
        if !report.stdout_preview.is_empty() {
            println!("Stdout:");
            print!("{}", report.stdout_preview);
            if !report.stdout_preview.ends_with('\n') {
                println!();
            }
        }
        if !report.stderr_preview.is_empty() {
            println!("Stderr:");
            eprint!("{}", report.stderr_preview);
            if !report.stderr_preview.ends_with('\n') {
                eprintln!();
            }
        }
    }
    Ok(())
}

pub(crate) fn render_lane_workdir(
    report: &LaneWorkdirReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if let Some(workdir) = &report.workdir {
            println!("{workdir}");
        } else {
            println!("Lane {} has no materialized workdir", report.lane_id);
        }
    }
    Ok(())
}

pub(crate) fn render_lane_file_read(
    report: &LaneFileReadReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        std::io::stdout().write_all(report.content.as_bytes())?;
    }
    Ok(())
}

pub(crate) fn render_lane_workdir_sync(
    report: &LaneWorkdirSyncReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Synced lane workdir: {}", report.workdir);
        println!("Head: {}", report.head_change.0);
        if report.forced {
            println!("Forced: true");
        }
        if let Some(rescue_workdir) = &report.rescue_workdir {
            println!("Rescue workdir: {rescue_workdir}");
        }
        for path in &report.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
    }
    Ok(())
}

pub(crate) fn render_lane_patch(report: &LanePatchReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Applied lane patch {}", report.operation.0);
        for path in &report.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
    }
    Ok(())
}

pub(crate) fn render_lane_remove(report: &LaneRemoveReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Removed lane {} ({})", report.lane_id, report.ref_name);
        if let Some(workdir) = &report.removed_workdir {
            println!("Removed workdir: {workdir}");
        }
    }
    Ok(())
}
