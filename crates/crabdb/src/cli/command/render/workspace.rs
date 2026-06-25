use super::render_json;

use crabdb::model::*;
use crabdb::{Result, WorktreeState};

pub(crate) fn render_init(report: &InitReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Initialized CrabDB workspace");
        println!("Workspace: {}", report.workspace_id.0);
        println!("Branch: {}", report.branch);
        println!("Initial operation: {}", report.operation.0);
        println!(
            "Imported: {} files ({} text, {} opaque, {} binary)",
            report.imported.files,
            report.imported.text,
            report.imported.opaque,
            report.imported.binary
        );
    }
    Ok(())
}

pub(crate) fn render_status(report: &StatusReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Branch: {}", report.branch);
        println!("Head: {}", report.head.change_id.0);
        println!("Root: {}", report.head.root_id.0);
        println!(
            "Worktree: {}",
            match report.worktree_state {
                WorktreeState::Clean => "clean",
                WorktreeState::DirtyTracked => "dirty",
                WorktreeState::DirtyUntracked => "dirty with untracked paths",
            }
        );
        for path in &report.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
    }
    Ok(())
}

pub(crate) fn render_record(report: &RecordReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        match &report.operation {
            Some(change) => {
                println!("Recorded {}", change.0);
                for path in &report.changed_paths {
                    println!("  {:?} {}", path.kind, path.path);
                }
            }
            None => println!("No changes to record"),
        }
    }
    Ok(())
}

pub(crate) fn render_timeline(entries: &[TimelineEntry], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if !quiet {
        for entry in entries {
            let message = entry.message.as_deref().unwrap_or("");
            println!(
                "{} {:?} {} {}",
                entry.change_id.0, entry.kind, entry.branch, message
            );
        }
    }
    Ok(())
}

pub(crate) fn render_checkout(report: &CheckoutReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.dry_run {
            println!(
                "Would check out {} ({} changed paths)",
                report.change_id.0,
                report.changed_paths.len()
            );
        } else {
            println!(
                "Checked out {} ({} files)",
                report.change_id.0, report.written_files
            );
        }
        if let Some(output_root) = &report.output_root {
            println!("Output: {output_root}");
        }
        if let Some(recorded) = &report.recorded_dirty {
            println!("Recorded dirty worktree: {}", recorded.0);
        }
        for path in &report.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
    }
    Ok(())
}
