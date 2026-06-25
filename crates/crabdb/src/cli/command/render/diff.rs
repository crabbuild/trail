use std::time::Duration;

use super::super::DiffArgs;
use super::render_json;

use crabdb::model::*;
use crabdb::{CrabDb, Error, Result};

pub(crate) fn diff_from_args(db: &mut CrabDb, args: &DiffArgs) -> Result<DiffSummary> {
    let forms = usize::from(args.range.is_some())
        + usize::from(args.root.is_some())
        + usize::from(args.dirty);
    if forms != 1 {
        return Err(Error::InvalidInput(
            "diff requires exactly one of RANGE, --root ROOT..ROOT, or --dirty".to_string(),
        ));
    }
    if let Some(range) = &args.range {
        db.diff_range_with_options(range, args.patch, args.show_line_ids)
    } else if let Some(root_range) = &args.root {
        db.diff_roots(root_range, args.patch, args.show_line_ids)
    } else {
        db.diff_dirty(args.patch, args.show_line_ids)
    }
}

pub(crate) fn watch_interval(interval_secs: u64, debounce_ms: Option<u64>) -> Result<Duration> {
    if let Some(ms) = debounce_ms {
        if ms == 0 {
            return Err(Error::InvalidInput(
                "watch debounce must be greater than 0ms".to_string(),
            ));
        }
        return Ok(Duration::from_millis(ms));
    }
    if interval_secs == 0 {
        return Err(Error::InvalidInput(
            "watch interval must be greater than 0 seconds".to_string(),
        ));
    }
    Ok(Duration::from_secs(interval_secs))
}

pub(crate) fn render_diff(
    summary: &DiffSummary,
    json: bool,
    quiet: bool,
    stat: bool,
) -> Result<()> {
    if json {
        return render_json(summary);
    }
    if !quiet {
        println!("Diff {}..{}", summary.from, summary.to);
        let mut total_additions = 0;
        let mut total_deletions = 0;
        for file in &summary.files {
            total_additions += file.additions;
            total_deletions += file.deletions;
            println!(
                "  {:?} {} (+{} -{})",
                file.kind, file.path, file.additions, file.deletions
            );
            for line in &file.line_changes {
                println!(
                    "    {:?} {} old={} new={}",
                    line.kind,
                    format_line_id(&line.line_id),
                    format_optional_line_number(line.old_line_number),
                    format_optional_line_number(line.new_line_number)
                );
            }
            if let Some(patch) = &file.patch {
                print!("{patch}");
            }
        }
        if stat {
            println!(
                "{} files changed, {} additions, {} deletions",
                summary.files.len(),
                total_additions,
                total_deletions
            );
        }
    }
    Ok(())
}

fn format_line_id(line_id: &crabdb::LineId) -> String {
    format!("{}:{}", line_id.origin_change.0, line_id.local_seq)
}

fn format_optional_line_number(line: Option<u64>) -> String {
    line.map(|line| line.to_string())
        .unwrap_or_else(|| "-".to_string())
}

pub(crate) fn render_history(result: &HistoryResult, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(result);
    }
    if !quiet {
        println!("{}", result.selector);
        for entry in &result.file_history {
            let old_path = entry
                .old_path
                .as_ref()
                .map(|path| format!(" from {path}"))
                .unwrap_or_default();
            println!(
                "{} {:?} {}{}",
                entry.change_id.0, entry.kind, entry.path, old_path
            );
        }
        for entry in &result.line_history {
            let line = entry
                .line_number
                .map(|line| format!(":{line}"))
                .unwrap_or_default();
            println!(
                "{} {:?} {}{}",
                entry.change_id.0, entry.kind, entry.path, line
            );
        }
    }
    Ok(())
}

pub(crate) fn render_code_from(result: &CodeFromResult, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(result);
    }
    if !quiet {
        println!("{}", result.selector);
        if result.operations.is_empty() {
            println!("No operations found");
        }
        for operation in &result.operations {
            let message = operation.message.as_deref().unwrap_or("");
            println!(
                "{} {:?} {} {}",
                operation.change_id.0, operation.kind, operation.branch, message
            );
            for path in &operation.changed_paths {
                println!("  {:?} {}", path.kind, path.path);
            }
        }
    }
    Ok(())
}

pub(crate) fn render_why(result: &WhyResult, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(result);
    }
    if !quiet {
        println!(
            "{}:{} {}",
            result.path, result.line_number, result.current_text
        );
        println!(
            "Line ID: {}:{}",
            result.line_id.origin_change.0, result.line_id.local_seq
        );
        println!("Introduced by: {}", result.introduced_by.0);
        println!("Last content change: {}", result.last_content_change.0);
        for item in &result.history {
            println!("  {:?} {} {}", item.kind, item.change_id.0, item.path);
        }
    }
    Ok(())
}
