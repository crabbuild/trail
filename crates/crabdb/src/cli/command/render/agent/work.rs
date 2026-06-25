use super::render_json;

use crabdb::model::*;
use crabdb::Result;

pub(crate) fn render_agent_record(
    report: &AgentRecordReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        match &report.operation {
            Some(operation) => {
                println!("Recorded agent workdir {}", operation.0);
                for path in &report.changed_paths {
                    println!("  {:?} {}", path.kind, path.path);
                }
            }
            None => println!("No agent workdir changes to record"),
        }
    }
    Ok(())
}

pub(crate) fn render_agent_watch(report: &AgentWatchReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Watched {} for {} iteration(s); recorded {} operation(s)",
            report.agent_id,
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

pub(crate) fn render_agent_test(report: &AgentTestReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Agent {} {} for {}",
            report.kind, report.status, report.agent_id
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

pub(crate) fn render_agent_workdir(
    report: &AgentWorkdirReport,
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
            println!("Agent {} has no materialized workdir", report.agent_id);
        }
    }
    Ok(())
}

pub(crate) fn render_agent_workdir_sync(
    report: &AgentWorkdirSyncReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Synced agent workdir: {}", report.workdir);
        println!("Head: {}", report.head_change.0);
        if report.forced {
            println!("Forced: true");
        }
        for path in &report.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
    }
    Ok(())
}

pub(crate) fn render_agent_patch(report: &AgentPatchReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Applied agent patch {}", report.operation.0);
        for path in &report.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
    }
    Ok(())
}

pub(crate) fn render_agent_remove(
    report: &AgentRemoveReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Removed agent {} ({})", report.agent_id, report.ref_name);
        if let Some(workdir) = &report.removed_workdir {
            println!("Removed workdir: {workdir}");
        }
    }
    Ok(())
}
