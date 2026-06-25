use super::render_json;

use crabdb::model::*;
use crabdb::Result;

pub(crate) fn render_config_list(entries: &[ConfigEntry], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if !quiet {
        for entry in entries {
            let read_only = if entry.read_only { " (read-only)" } else { "" };
            println!(
                "{} = {} [{}]{}",
                entry.key, entry.value, entry.value_type, read_only
            );
        }
    }
    Ok(())
}

pub(crate) fn render_config_entry(entry: &ConfigEntry, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(entry);
    }
    if !quiet {
        println!("{}", entry.value);
    }
    Ok(())
}

pub(crate) fn render_config_set(report: &ConfigSetReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "{}: {} -> {}",
            report.key, report.old_value, report.new_value
        );
    }
    Ok(())
}
