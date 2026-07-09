use std::io::IsTerminal;
use std::time::Duration;

use super::super::DiffArgs;
use super::render_json;

use trail::model::*;
use trail::{Error, Result, Trail};

pub(crate) fn diff_from_args(db: &mut Trail, args: &DiffArgs) -> Result<DiffSummary> {
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
    color: bool,
) -> Result<()> {
    render_diff_with_title(summary, json, quiet, stat, color, None)
}

pub(crate) fn render_diff_with_title(
    summary: &DiffSummary,
    json: bool,
    quiet: bool,
    stat: bool,
    color: bool,
    title: Option<&str>,
) -> Result<()> {
    if json {
        return render_json(summary);
    }
    if !quiet {
        let total_additions: u64 = summary.files.iter().map(|file| file.additions).sum();
        let total_deletions: u64 = summary.files.iter().map(|file| file.deletions).sum();
        println!("{}", title.unwrap_or("Diff"));
        println!("  from: {}", summary.from);
        println!("  to:   {}", summary.to);
        if summary.files.is_empty() {
            println!("  No file changes");
            return Ok(());
        }
        println!(
            "  {} file(s) changed, +{} -{}",
            summary.files.len(),
            total_additions,
            total_deletions
        );
        println!();

        for file in &summary.files {
            println!("  {}", format_file_diff_line(file));
            print_line_change_summary(file);
            if let Some(patch) = &file.patch {
                if !patch.starts_with('\n') {
                    println!();
                }
                print_patch(patch, color && std::io::stdout().is_terminal());
                if !patch.ends_with('\n') {
                    println!();
                }
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

fn print_patch(patch: &str, color: bool) {
    if !color {
        print!("{patch}");
        return;
    }
    for line in patch.split_inclusive('\n') {
        print!("{}", color_patch_line(line));
    }
}

fn color_patch_line(line: &str) -> String {
    let color = if line.starts_with("@@") {
        Some("\x1b[36m")
    } else if line.starts_with("diff --git")
        || line.starts_with("diff --trail")
        || line.starts_with("index ")
    {
        Some("\x1b[1m")
    } else if line.starts_with("--- ") || line.starts_with("+++ ") {
        Some("\x1b[1m")
    } else if line.starts_with('+') {
        Some("\x1b[32m")
    } else if line.starts_with('-') {
        Some("\x1b[31m")
    } else {
        None
    };
    match color {
        Some(color) => format!("{color}{line}\x1b[0m"),
        None => line.to_string(),
    }
}

fn format_file_diff_line(file: &FileDiffSummary) -> String {
    let path = file
        .old_path
        .as_ref()
        .map(|old_path| format!("{old_path} -> {}", file.path))
        .unwrap_or_else(|| file.path.clone());
    format!(
        "{} {:<11} {} {}",
        file_change_marker(&file.kind),
        file_change_label(&file.kind),
        path,
        format_change_stat(file.additions, file.deletions)
    )
}

fn file_change_marker(kind: &FileChangeKind) -> &'static str {
    match kind {
        FileChangeKind::Added => "A",
        FileChangeKind::Modified => "M",
        FileChangeKind::Deleted => "D",
        FileChangeKind::Renamed => "R",
        FileChangeKind::TypeChanged => "T",
    }
}

fn file_change_label(kind: &FileChangeKind) -> &'static str {
    match kind {
        FileChangeKind::Added => "added",
        FileChangeKind::Modified => "modified",
        FileChangeKind::Deleted => "deleted",
        FileChangeKind::Renamed => "renamed",
        FileChangeKind::TypeChanged => "type-changed",
    }
}

fn format_change_stat(additions: u64, deletions: u64) -> String {
    let bar = format_change_bar(additions, deletions);
    if bar.is_empty() {
        format!("(+{additions} -{deletions})")
    } else {
        format!("(+{additions} -{deletions}) {bar}")
    }
}

fn format_change_bar(additions: u64, deletions: u64) -> String {
    let total = additions + deletions;
    if total == 0 {
        return String::new();
    }
    let width = total.min(24) as usize;
    let mut plus = ((additions * width as u64) + total - 1) / total;
    if additions == 0 {
        plus = 0;
    }
    let minus = width.saturating_sub(plus as usize);
    format!("{}{}", "+".repeat(plus as usize), "-".repeat(minus))
}

fn print_line_change_summary(file: &FileDiffSummary) {
    if file.line_changes.is_empty() {
        return;
    }
    let mut added = 0_u64;
    let mut modified = 0_u64;
    let mut deleted = 0_u64;
    let mut moved = 0_u64;
    for line in &file.line_changes {
        match &line.kind {
            LineChangeKind::Added => added += 1,
            LineChangeKind::Modified => modified += 1,
            LineChangeKind::Deleted => deleted += 1,
            LineChangeKind::Moved => moved += 1,
        }
    }
    println!("    lines: +{added} ~{modified} -{deleted} moved {moved}");
    for line in file.line_changes.iter().take(8) {
        println!(
            "      {} {} old={} new={}",
            line_change_marker(&line.kind),
            format_line_id(&line.line_id),
            format_optional_line_number(line.old_line_number),
            format_optional_line_number(line.new_line_number)
        );
    }
    if file.line_changes.len() > 8 {
        println!(
            "      ... {} more line changes",
            file.line_changes.len() - 8
        );
    }
}

fn line_change_marker(kind: &LineChangeKind) -> &'static str {
    match kind {
        LineChangeKind::Added => "+",
        LineChangeKind::Modified => "~",
        LineChangeKind::Deleted => "-",
        LineChangeKind::Moved => ">",
    }
}

fn format_line_id(line_id: &trail::LineId) -> String {
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
