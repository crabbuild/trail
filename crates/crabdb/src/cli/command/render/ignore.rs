use super::render_json;

use crabdb::model::*;
use crabdb::Result;

pub(crate) fn render_ignore_list(report: &IgnoreListReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Ignore file: {}", report.path);
        if report.patterns.is_empty() {
            println!("No ignore patterns");
        } else {
            for pattern in &report.patterns {
                println!("{}: {}", pattern.line, pattern.pattern);
            }
        }
    }
    Ok(())
}

pub(crate) fn render_ignore_add(report: &IgnoreAddReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.added {
            println!("Added ignore pattern: {}", report.pattern);
        } else {
            println!("Ignore pattern already present: {}", report.pattern);
        }
    }
    Ok(())
}

pub(crate) fn render_ignore_remove(
    report: &IgnoreRemoveReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.removed {
            println!("Removed ignore pattern: {}", report.pattern);
        } else {
            println!("Ignore pattern not present: {}", report.pattern);
        }
    }
    Ok(())
}

pub(crate) fn render_ignore_check(
    report: &IgnoreCheckReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        match (&report.ignored, &report.source) {
            (true, Some(source)) => println!("{}: ignored ({})", report.path, source),
            (true, None) => println!("{}: ignored", report.path),
            (false, _) => println!("{}: not ignored", report.path),
        }
    }
    Ok(())
}
