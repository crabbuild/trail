use super::render_json;

use crabdb::model::*;
use crabdb::Result;

mod coordination;
mod merge;

pub(crate) use coordination::*;
pub(crate) use merge::*;

pub(crate) fn render_branch(report: &BranchReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Created branch {} from {}", report.name, report.from.0);
    }
    Ok(())
}

pub(crate) fn render_branch_list(
    entries: &[BranchListEntry],
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if !quiet {
        for entry in entries {
            let marker = if entry.is_current { "*" } else { " " };
            println!("{marker} {} {}", entry.name, entry.change_id.0);
        }
    }
    Ok(())
}

pub(crate) fn render_branch_delete(
    report: &BranchDeleteReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Deleted branch {}", report.name);
    }
    Ok(())
}

pub(crate) fn render_branch_rename(
    report: &BranchRenameReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Renamed branch {} to {}", report.old_name, report.new_name);
    }
    Ok(())
}
