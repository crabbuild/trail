use super::render_json;

use crabdb::model::*;
use crabdb::Result;

pub(crate) fn render_doctor(report: &DoctorReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Doctor: {}", report.status);
        for check in &report.checks {
            println!("[{}] {}: {}", check.status, check.name, check.message);
        }
    }
    Ok(())
}

pub(crate) fn render_backup_create(
    report: &BackupCreateReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Created backup: {}", report.path);
        println!("Branch: {}", report.branch);
        println!("Refs: {}", report.ref_count);
        println!("Operations: {}", report.operation_count);
        println!("SQLite bytes: {}", report.sqlite_bytes);
        println!("SQLite SHA-256: {}", report.sqlite_sha256);
        if !report.fsck_errors.is_empty() {
            println!("FSCK warnings:");
            for error in &report.fsck_errors {
                println!("  {error}");
            }
        }
    }
    Ok(())
}

pub(crate) fn render_backup_verify(
    report: &BackupVerifyReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        let status = if report.valid { "valid" } else { "invalid" };
        println!("Backup {status}: {}", report.path);
        if let Some(branch) = &report.branch {
            println!("Branch: {branch}");
        }
        println!(
            "Checked {} refs, {} roots, {} text objects",
            report.checked_refs, report.checked_roots, report.checked_texts
        );
        for error in &report.errors {
            println!("  {error}");
        }
    }
    Ok(())
}

pub(crate) fn render_backup_restore(
    report: &BackupRestoreReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Restored backup: {}", report.backup_path);
        println!("Workspace: {}", report.workspace);
        println!("Branch: {}", report.branch);
        println!("Replaced existing DB: {}", report.replaced_existing);
        println!("Rewritten lane workdirs: {}", report.rewritten_workdirs);
        println!(
            "Checked {} refs, {} roots, {} text objects",
            report.checked_refs, report.checked_roots, report.checked_texts
        );
    }
    Ok(())
}

pub(crate) fn render_fsck(report: &FsckReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Checked {} refs, {} roots, {} text objects",
            report.checked_refs, report.checked_roots, report.checked_texts
        );
        if report.errors.is_empty() {
            println!("No errors");
        } else {
            for error in &report.errors {
                println!("  {error}");
            }
        }
    }
    Ok(())
}

pub(crate) fn render_index_rebuild(
    report: &IndexRebuildReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Rebuilt indexes: {} operations, {} parents, {} file rows, {} line rows, {} messages",
            report.operations,
            report.operation_parents,
            report.file_history_rows,
            report.line_history_rows,
            report.messages
        );
        if report.rich_text_hydrated > 0 {
            println!("Hydrated {} lazy text object(s)", report.rich_text_hydrated);
        }
        for error in &report.errors {
            println!("  warning: {error}");
        }
    }
    Ok(())
}

pub(crate) fn render_worktree_index(
    report: &WorktreeIndexReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Worktree index refreshed: {} files, {} cached entries in {}ms",
            report.files, report.indexed_entries, report.duration_ms
        );
    }
    Ok(())
}

pub(crate) fn render_gc(report: &GcReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.dry_run {
            println!(
                "GC dry run: {} prunable of {} known objects ({} reachable, {} unknown preserved)",
                report.prunable_objects,
                report.total_known_objects,
                report.reachable_objects,
                report.preserved_unknown_objects
            );
        } else {
            println!(
                "GC pruned {} objects ({} reachable, {} unknown preserved)",
                report.pruned_objects, report.reachable_objects, report.preserved_unknown_objects
            );
        }
        for error in &report.errors {
            println!("  warning: {error}");
        }
    }
    Ok(())
}
