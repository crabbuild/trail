use super::render_json;

use crabdb::model::*;
use crabdb::Result;

pub(crate) fn render_git_import_update(
    report: &GitImportReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        match &report.operation {
            Some(change) => {
                println!("Imported Git update {}", change.0);
                println!(
                    "Imported: {} files ({} text, {} opaque, {} binary)",
                    report.imported.files,
                    report.imported.text,
                    report.imported.opaque,
                    report.imported.binary
                );
                for path in &report.changed_paths {
                    println!("  {:?} {}", path.kind, path.path);
                }
            }
            None => println!("No Git-tracked changes to import"),
        }
    }
    Ok(())
}

pub(crate) fn render_git_export(report: &GitExportReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Created Git commit: {}", report.commit);
        println!("Range: {}", report.range);
        println!("CrabDB operation: {}", report.operation.0);
        println!("Root: {}", report.root_id.0);
        if let Some(parent) = &report.parent {
            println!("Parent: {parent}");
        }
        if let Some(mapping) = &report.mapping {
            println!("Mapping: {}", mapping.mapping_id);
        }
    }
    Ok(())
}

pub(crate) fn render_git_mappings(entries: &[GitMapping], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if !quiet {
        if entries.is_empty() {
            println!("No Git mappings");
        }
        for entry in entries {
            let git_head = entry
                .git_head
                .as_deref()
                .map(|head| head.get(..12).unwrap_or(head))
                .unwrap_or("unborn");
            let dirty = if entry.git_dirty { " dirty" } else { "" };
            println!(
                "{} {}{} {} {} {}",
                entry.direction,
                git_head,
                dirty,
                entry.branch,
                entry.crab_change.0,
                entry.crab_root.0
            );
        }
    }
    Ok(())
}
