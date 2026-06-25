use std::time::Duration;

use super::DiffArgs;

use crabdb;
use crabdb::model::*;
use crabdb::{CrabDb, Error, Result, WorktreeState};

pub(crate) fn render_json<T: serde::Serialize + ?Sized>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

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

pub(crate) fn render_guardrail_check(
    report: &GuardrailCheckReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Guardrail decision: {}", report.decision);
        println!("Action: {}", report.action);
        if let Some(agent) = &report.agent {
            println!("Agent: {}", agent.record.name);
        }
        if !report.reasons.is_empty() {
            println!("Reasons:");
            for reason in &report.reasons {
                println!(
                    "  {} [{}]: {}",
                    reason.code, reason.severity, reason.message
                );
            }
        }
        if !report.path_checks.is_empty() {
            println!("Paths:");
            for check in &report.path_checks {
                let status = if check.ignored { "ignored" } else { "allowed" };
                match &check.source {
                    Some(source) => println!("  {}: {} ({})", check.path, status, source),
                    None => println!("  {}: {}", check.path, status),
                }
            }
        }
        if let Some(approval) = &report.approval_request {
            println!("Approval suggested: {}", approval.summary);
        }
        if !report.satisfied_approvals.is_empty() {
            println!("Satisfied approvals:");
            for approval in &report.satisfied_approvals {
                println!("  {} {}", approval.approval_id, approval.action);
            }
        }
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

pub(crate) fn render_show(result: &ShowResult, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(result);
    }
    if quiet {
        return Ok(());
    }
    match result {
        ShowResult::Operation { value } => {
            let op = &value.operation;
            println!("Operation: {}", op.change_id.0);
            println!("Kind: {:?}", op.kind);
            println!("Branch: {}", op.branch);
            println!("Actor: {}", op.actor.id);
            if let Some(message) = &op.message {
                println!("Message: {message}");
            }
            if !op.parents.is_empty() {
                println!("Parents:");
                for parent in &op.parents {
                    println!("  {}", parent.0);
                }
            }
            if let Some(before) = &op.before_root {
                println!("Before root: {}", before.0);
            }
            println!("After root: {}", op.after_root.0);
            for path in &value.changed_paths {
                println!(
                    "  {:?} {} (+{} -{})",
                    path.kind, path.path, path.additions, path.deletions
                );
            }
            for message in &value.messages {
                println!("Message object: {} {}", message.id.0, message.body);
            }
        }
        ShowResult::Message { value } => {
            println!("Message: {}", value.id.0);
            println!("Role: {}", value.role);
            if let Some(agent_id) = &value.agent_id {
                println!("Agent: {agent_id}");
            }
            if let Some(session_id) = &value.session_id {
                println!("Session: {session_id}");
            }
            if let Some(change_id) = &value.change_id {
                println!("Change: {}", change_id.0);
            }
            println!("{}", value.body);
        }
        ShowResult::Ref { value } => {
            println!("Ref: {}", value.name);
            println!("Change: {}", value.change_id.0);
            println!("Root: {}", value.root_id.0);
            println!("Generation: {}", value.generation);
        }
        ShowResult::Agent { value } => {
            println!("Agent: {}", value.agent_id);
            println!("Ref: {}", value.ref_name);
            println!("Status: {}", value.status);
            println!("Base: {}", value.base_change.0);
            println!("Head: {}", value.head_change.0);
            if let Some(workdir) = &value.workdir {
                println!("Workdir: {workdir}");
            }
        }
        ShowResult::Object { value } => {
            println!("Object: {}", value.object_id.0);
            println!("Kind: {}", value.kind);
            println!("Version: {}", value.version);
            println!("Size: {}", value.size_bytes);
        }
    }
    Ok(())
}

pub(crate) fn render_object_inspect(
    report: &ObjectInspectReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if quiet {
        return Ok(());
    }
    println!("Object: {}", report.info.object_id.0);
    println!("Kind: {}", report.info.kind);
    println!("Version: {}", report.info.version);
    println!("Size: {}", report.info.size_bytes);
    println!("Created at: {}", report.info.created_at);
    if report
        .summary
        .as_object()
        .map(|summary| !summary.is_empty())
        .unwrap_or(true)
    {
        println!("Summary:");
        let rendered = serde_json::to_string_pretty(&report.summary)?;
        for line in rendered.lines() {
            println!("  {line}");
        }
    }
    Ok(())
}

pub(crate) fn render_root_inspect(
    report: &RootInspectReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if quiet {
        return Ok(());
    }
    println!("Root: {}", report.root_id.0);
    println!("Created by: {}", report.root.created_by.0);
    println!("Files: {}", report.root.file_count);
    println!("Total text bytes: {}", report.root.total_text_bytes);
    if let Some(path_root) = &report.root.path_map_root {
        println!("Path map: {path_root}");
    }
    if let Some(file_root) = &report.root.file_index_map_root {
        println!("File index: {file_root}");
    }
    for file in &report.files {
        println!(
            "  {:?} {} {} -> {} ({} bytes)",
            file.kind, file.path, file.file_id, file.content_object.0, file.size_bytes
        );
    }
    Ok(())
}

pub(crate) fn render_text_inspect(
    report: &TextInspectReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if quiet {
        return Ok(());
    }
    println!("Text: {}", report.text_id.0);
    println!("Content hash: {}", report.content.content_hash);
    println!(
        "Lines: {} (showing {})",
        report.content.line_count,
        report.lines.len()
    );
    println!("Bytes: {}", report.content.byte_count);
    for line in &report.lines {
        let text = serde_json::to_string(&line.text)?;
        println!(
            "  {} {} {:?} {}",
            line.line_number, line.line_id, line.newline, text
        );
    }
    if report.truncated {
        println!("  ... truncated; pass --limit 0 to show all lines");
    }
    Ok(())
}

pub(crate) fn render_map_range(report: &MapRangeReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if quiet {
        return Ok(());
    }
    println!("Map: {}", report.map_id);
    println!("Type: {}", report.map_type);
    println!("Entries: {}", report.entries.len());
    for entry in &report.entries {
        let key = render_map_key(&entry.key);
        let value = render_map_value_summary(&entry.value)?;
        println!("  {key} -> {value}");
    }
    if report.truncated {
        println!("  ... truncated; pass --limit 0 to show all entries");
    }
    Ok(())
}

pub(crate) fn render_map_diff(report: &MapDiffReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if quiet {
        return Ok(());
    }
    println!("Map diff {}..{}", report.left_map_id, report.right_map_id);
    println!("Type: {}", report.map_type);
    println!("Changes: {}", report.changes.len());
    for change in &report.changes {
        let key = render_map_key(&change.key);
        println!("  {} {key}", change.kind);
        if let Some(old_value) = &change.old_value {
            println!("    old: {}", render_map_value_summary(old_value)?);
        }
        if let Some(new_value) = &change.new_value {
            println!("    new: {}", render_map_value_summary(new_value)?);
        }
    }
    if report.truncated {
        println!("  ... truncated; pass --limit 0 to show all changes");
    }
    Ok(())
}

pub(crate) fn render_map_key(key: &crabdb::model::MapKeyInspect) -> String {
    key.text
        .clone()
        .unwrap_or_else(|| format!("hex:{}", key.hex))
}

pub(crate) fn render_map_value_summary(value: &crabdb::model::MapValueInspect) -> Result<String> {
    if let Some(text) = &value.text {
        if value.summary == serde_json::json!({ "bytes": value.bytes }) {
            return Ok(format!("{text:?}"));
        }
    }
    let summary = serde_json::to_string(&value.summary)?;
    if value.truncated {
        Ok(format!(
            "{summary} ({} bytes, hex preview truncated)",
            value.bytes
        ))
    } else {
        Ok(format!("{summary} ({} bytes)", value.bytes))
    }
}

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

pub(crate) fn render_merge(report: &MergeReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.dry_run {
            println!(
                "Would merge {} into {} as {}",
                report.source_ref, report.target_ref, report.operation.0
            );
        } else {
            println!(
                "Merged {} into {} as {}",
                report.source_ref, report.target_ref, report.operation.0
            );
        }
        for conflict in &report.conflicts {
            println!("  conflict {conflict}");
        }
        for path in &report.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
    }
    Ok(())
}

pub(crate) fn render_merge_queue_add(
    report: &MergeQueueAddReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Queued {} into {} as {}",
            report.entry.source_ref, report.entry.target_ref, report.entry.queue_id
        );
    }
    Ok(())
}

pub(crate) fn render_merge_queue_list(
    entries: &[MergeQueueEntry],
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if !quiet {
        for entry in entries {
            println!(
                "{} {} priority={} {} -> {}",
                entry.queue_id, entry.status, entry.priority, entry.source_ref, entry.target_ref
            );
        }
    }
    Ok(())
}

pub(crate) fn render_merge_queue_run(
    report: &MergeQueueRunReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.processed.is_empty() {
            println!("Merge queue is empty");
        }
        for item in &report.processed {
            match (&item.operation, &item.error) {
                (Some(operation), _) => println!(
                    "{} {} as {} {} -> {}",
                    item.queue_id, item.status, operation.0, item.source_ref, item.target_ref
                ),
                (None, Some(error)) => println!(
                    "{} {} {} -> {}: {}",
                    item.queue_id, item.status, item.source_ref, item.target_ref, error
                ),
                (None, None) => println!(
                    "{} {} {} -> {}",
                    item.queue_id, item.status, item.source_ref, item.target_ref
                ),
            }
        }
        if report.stopped_on_conflict {
            println!("Paused on conflict");
        } else if report.stopped_on_failure {
            println!("Paused on failure");
        }
    }
    Ok(())
}

pub(crate) fn render_merge_queue_remove(
    report: &MergeQueueRemoveReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Cancelled {}", report.entry.queue_id);
    }
    Ok(())
}

pub(crate) fn render_conflicts(
    entries: &[ConflictSetSummary],
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if !quiet {
        if entries.is_empty() {
            println!("No conflicts");
        }
        for entry in entries {
            println!(
                "{} {} {} -> {}",
                entry.conflict_set_id,
                entry.status,
                entry.source_ref.as_deref().unwrap_or("-"),
                entry.target_ref.as_deref().unwrap_or("-")
            );
            for detail in &entry.details {
                println!("  {detail}");
            }
        }
    }
    Ok(())
}

pub(crate) fn render_conflict(entry: &ConflictSetSummary, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(entry);
    }
    if !quiet {
        println!("Conflict: {}", entry.conflict_set_id);
        println!("Status: {}", entry.status);
        if let Some(merge_id) = &entry.merge_id {
            println!("Merge: {merge_id}");
        }
        if let Some(source) = &entry.source_ref {
            println!("Source: {source}");
        }
        if let Some(target) = &entry.target_ref {
            println!("Target: {target}");
        }
        for detail in &entry.details {
            println!("  {detail}");
        }
    }
    Ok(())
}

pub(crate) fn render_conflict_resolve(
    report: &ConflictResolveReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.resolution == "manual" {
            println!(
                "Resolved {} manually as {}",
                report.conflict_set_id, report.operation.0
            );
        } else {
            println!(
                "Resolved {} by taking {} as {}",
                report.conflict_set_id, report.resolution, report.operation.0
            );
        }
        for path in &report.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
    }
    Ok(())
}

pub(crate) fn render_anchor_create(
    report: &AnchorCreateReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Created anchor {} at {}:{}",
            report.anchor.id.0, report.anchor.created_path, report.anchor.created_line
        );
    }
    Ok(())
}

pub(crate) fn render_anchor_resolve(
    report: &AnchorResolveReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Anchor: {}", report.anchor.id.0);
        println!("Label: {}", report.anchor.label);
        println!("Status: {}", report.status);
        if let (Some(path), Some(line_number)) = (&report.path, report.line_number) {
            println!("Location: {path}:{line_number}");
        } else if let Some(path) = &report.path {
            println!("Path: {path}");
        }
        if let Some(text) = &report.text {
            println!("{text}");
        }
    }
    Ok(())
}

pub(crate) fn render_anchor_list(anchors: &[Anchor], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(&anchors);
    }
    if !quiet {
        for anchor in anchors {
            println!(
                "{} {} {}:{}",
                anchor.id.0, anchor.label, anchor.created_path, anchor.created_line
            );
        }
    }
    Ok(())
}

pub(crate) fn render_anchor_delete(
    report: &AnchorDeleteReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Deleted anchor {}", report.anchor_id.0);
    }
    Ok(())
}

pub(crate) fn render_lease_acquire(
    report: &LeaseAcquireReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Acquired lease {} {} {} {}",
            report.lease.lease_id,
            report.lease.mode,
            report.lease.agent_id,
            report.lease.path.as_deref().unwrap_or("<workspace>")
        );
    }
    Ok(())
}

pub(crate) fn render_agent_claim(report: &AgentClaimReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.claimed {
            if let Some(lease) = &report.lease {
                println!(
                    "Claimed {} for {} until {} ({})",
                    report.path, report.agent_id, lease.expires_at, lease.lease_id
                );
            } else {
                println!("Claimed {} for {}", report.path, report.agent_id);
            }
        } else if let Some(warning) = &report.warning {
            println!("Warning: {warning}");
        } else {
            println!("Path {} is already claimed", report.path);
        }
    }
    Ok(())
}

pub(crate) fn render_lease_list(leases: &[LeaseRecord], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(&leases);
    }
    if !quiet {
        if leases.is_empty() {
            println!("No active leases");
        }
        for lease in leases {
            println!(
                "{} {} {} {} expires_at={}",
                lease.lease_id,
                lease.mode,
                lease.agent_id,
                lease.path.as_deref().unwrap_or("<workspace>"),
                lease.expires_at
            );
        }
    }
    Ok(())
}

pub(crate) fn render_lease_release(
    report: &LeaseReleaseReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Released lease {}", report.lease_id);
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

pub(crate) fn render_agent_spawn(report: &AgentSpawnReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Spawned {} at {}", report.agent_id, report.base_change.0);
        if let Some(workdir) = &report.workdir {
            println!("Workdir: {workdir}");
        }
    }
    Ok(())
}

pub(crate) fn render_agent_list(entries: &[AgentDetails], json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(&entries);
    }
    if !quiet {
        for entry in entries {
            println!(
                "{} {} {} {}",
                entry.record.name,
                entry.branch.status,
                entry.branch.head_change.0,
                entry.branch.ref_name
            );
        }
    }
    Ok(())
}

pub(crate) fn render_agent_details(details: &AgentDetails, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(details);
    }
    if !quiet {
        println!("Agent: {}", details.record.name);
        println!("ID: {}", details.record.agent_id);
        println!("Ref: {}", details.branch.ref_name);
        println!("Status: {}", details.branch.status);
        println!("Base: {}", details.branch.base_change.0);
        println!("Head: {}", details.branch.head_change.0);
        if let Some(provider) = &details.record.provider {
            println!("Provider: {provider}");
        }
        if let Some(model) = &details.record.model {
            println!("Model: {model}");
        }
        if let Some(session_id) = &details.branch.session_id {
            println!("Session: {session_id}");
        }
        if let Some(workdir) = &details.branch.workdir {
            println!("Workdir: {workdir}");
        }
    }
    Ok(())
}

pub(crate) fn render_agent_status(
    report: &AgentStatusReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "{} {} ({} changed paths, {} queued merges)",
            report.agent.record.name,
            report.agent.branch.status,
            report.changed_paths.len(),
            report.queued_merges
        );
        for path in &report.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
        if let Some(state) = &report.workdir_state {
            println!("Workdir: {:?}", state);
            for path in &report.workdir_changed_paths {
                println!("  workdir {:?} {}", path.kind, path.path);
            }
        }
        if let Some(test) = &report.latest_test {
            let command = if test.command.is_empty() {
                String::new()
            } else {
                format!(" {}", test.command.join(" "))
            };
            println!(
                "Latest test: {}{} ({} ms)",
                test.status, command, test.duration_ms
            );
        }
        if let Some(eval) = &report.latest_eval {
            let command = if eval.command.is_empty() {
                String::new()
            } else {
                format!(" {}", eval.command.join(" "))
            };
            println!(
                "Latest eval: {}{} ({} ms)",
                eval.status, command, eval.duration_ms
            );
        }
    }
    Ok(())
}

pub(crate) fn render_agent_contribution(
    report: &AgentContributionReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        let status = &report.status;
        println!(
            "Agent contribution: {} ({})",
            status.agent.record.name, status.agent.branch.status
        );
        println!("Ref: {}", status.agent.branch.ref_name);
        println!(
            "Base: {}  Head: {}",
            status.agent.branch.base_change.0, status.agent.branch.head_change.0
        );
        println!(
            "Changed paths: {}  Operations: {}  Sessions: {}  Events: {}  Approvals: {}",
            status.changed_paths.len(),
            report.operations.len(),
            report.sessions.len(),
            report.recent_events.len(),
            report.approvals.len()
        );
        for path in &status.changed_paths {
            println!("  {:?} {}", path.kind, path.path);
        }
        if let Some(test) = &status.latest_test {
            println!("Latest test: {} ({})", test.status, test.command.join(" "));
        }
        if let Some(eval) = &status.latest_eval {
            println!("Latest eval: {} ({})", eval.status, eval.command.join(" "));
        }
        if !report.operations.is_empty() {
            println!("Recent operations:");
            for operation in &report.operations {
                println!(
                    "  {} {:?} {} path(s) {}",
                    operation.change_id.0,
                    operation.kind,
                    operation.path_count,
                    operation.message.as_deref().unwrap_or("")
                );
            }
        }
        let pending_approvals = report
            .approvals
            .iter()
            .filter(|approval| approval.status == "pending")
            .count();
        if pending_approvals > 0 {
            println!("Pending approvals: {pending_approvals}");
        }
    }
    Ok(())
}

pub(crate) fn render_agent_gate_history(
    report: &AgentGateHistoryReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Agent gates for {} ({}, limit {})",
            report.agent.record.name, report.kind, report.limit
        );
        for gate in &report.gates {
            let suite = gate.suite.as_deref().unwrap_or("-");
            let score = gate
                .score
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string());
            let threshold = gate
                .threshold
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string());
            println!(
                "  {} {} {} suite={} score={} threshold={} {}",
                gate.created_at,
                gate.kind,
                gate.status,
                suite,
                score,
                threshold,
                gate.command.join(" ")
            );
        }
    }
    Ok(())
}

pub(crate) fn render_agent_readiness(
    report: &AgentReadinessReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Agent readiness: {} ({})",
            report.agent.record.name, report.status
        );
        println!("Ref: {}", report.agent.branch.ref_name);
        println!(
            "Ready: {}  Changed paths: {}  Blockers: {}  Warnings: {}",
            report.ready,
            report.changed_paths.len(),
            report.blockers.len(),
            report.warnings.len()
        );
        if !report.blockers.is_empty() {
            println!("Blockers:");
            for blocker in &report.blockers {
                println!("  {}: {}", blocker.code, blocker.message);
            }
        }
        if !report.warnings.is_empty() {
            println!("Warnings:");
            for warning in &report.warnings {
                println!("  {}: {}", warning.code, warning.message);
            }
        }
        if let Some(test) = &report.latest_test {
            println!("Latest test: {} ({})", test.status, test.command.join(" "));
        }
        if let Some(eval) = &report.latest_eval {
            println!("Latest eval: {} ({})", eval.status, eval.command.join(" "));
        }
    }
    Ok(())
}

pub(crate) fn render_agent_handoff(
    report: &AgentHandoffReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Agent handoff: {} ({})",
            report.agent.record.name, report.readiness.status
        );
        println!("Ref: {}", report.agent.branch.ref_name);
        println!(
            "Ready: {}  Sessions: {}  Events: {}  Spans: {}  Operations: {}",
            report.readiness.ready,
            report.recent_sessions.len(),
            report.recent_events.len(),
            report.recent_spans.len(),
            report.recent_operations.len()
        );
        if let Some(session) = &report.current_session {
            println!(
                "Current session: {} ({})",
                session.session.session_id, session.session.status
            );
            println!(
                "Session context: {} turn(s), {} message(s), {} event(s), {} operation(s)",
                session.turns.len(),
                session.messages.len(),
                session.events.len(),
                session.operations.len()
            );
        }
        if !report.readiness.blockers.is_empty() {
            println!("Blockers:");
            for blocker in &report.readiness.blockers {
                println!("  {}: {}", blocker.code, blocker.message);
            }
        }
        if !report.next_steps.is_empty() {
            println!("Next steps:");
            for step in &report.next_steps {
                println!("  {step}");
            }
        }
    }
    Ok(())
}

pub(crate) fn render_agent_message(
    report: &AgentMessageReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Added message {} ({})", report.message_id.0, report.role);
    }
    Ok(())
}

pub(crate) fn render_agent_turn_start(
    report: &crabdb::AgentTurnStartReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Started turn {} for {}",
            report.turn.turn_id, report.turn.agent_id
        );
        println!("Session: {}", report.session.session_id);
        println!("Base: {}", report.turn.before_change.0);
        println!("Root: {}", report.base_root.0);
    }
    Ok(())
}

pub(crate) fn render_agent_turn_details(
    details: &crabdb::AgentTurnDetails,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(details);
    }
    if !quiet {
        println!("Turn: {}", details.turn.turn_id);
        println!("Agent: {}", details.turn.agent_id);
        println!("Status: {}", details.turn.status);
        if let Some(session) = &details.session {
            println!("Session: {}", session.session_id);
        }
        println!("Base: {}", details.turn.before_change.0);
        if let Some(after) = &details.turn.after_change {
            println!("After: {}", after.0);
        }
        println!("Messages: {}", details.messages.len());
        println!("Events: {}", details.events.len());
        println!("Operations: {}", details.operations.len());
        for event in &details.events {
            println!("  event {} {}", event.event_id, event.event_type);
        }
        for operation in &details.operations {
            let message = operation.message.as_deref().unwrap_or("");
            println!(
                "  op {} {:?} {}",
                operation.change_id.0, operation.kind, message
            );
        }
    }
    Ok(())
}

pub(crate) fn render_agent_turn_event(
    report: &crabdb::AgentTurnEventReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Added event {} {}",
            report.event.event_id, report.event.event_type
        );
    }
    Ok(())
}

pub(crate) fn render_agent_events(
    events: &[AgentEventRecord],
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(events);
    }
    if !quiet {
        for event in events {
            let session = event.session_id.as_deref().unwrap_or("-");
            let turn = event.turn_id.as_deref().unwrap_or("-");
            println!(
                "{} {} agent={} session={} turn={}",
                event.event_id, event.event_type, event.agent_id, session, turn
            );
        }
    }
    Ok(())
}

pub(crate) fn render_agent_trace_span_start(
    report: &AgentTraceSpanStartReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Started span {} {} {}",
            report.span.span_id, report.span.span_type, report.span.name
        );
        println!("Trace: {}", report.span.trace_id);
    }
    Ok(())
}

pub(crate) fn render_agent_trace_span_end(
    report: &AgentTraceSpanEndReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Ended span {} {}", report.span.span_id, report.span.status);
        if let Some(duration_ms) = report.span.duration_ms {
            println!("Duration: {duration_ms} ms");
        }
    }
    Ok(())
}

pub(crate) fn render_agent_trace_spans(
    spans: &[AgentTraceSpan],
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(spans);
    }
    if !quiet {
        for span in spans {
            let parent = span.parent_span_id.as_deref().unwrap_or("-");
            let turn = span.turn_id.as_deref().unwrap_or("-");
            let duration = span
                .duration_ms
                .map(|duration_ms| format!("{duration_ms}ms"))
                .unwrap_or_else(|| "-".to_string());
            println!(
                "{} {} {} status={} trace={} parent={} turn={} duration={}",
                span.span_id,
                span.span_type,
                span.name,
                span.status,
                span.trace_id,
                parent,
                turn,
                duration
            );
        }
    }
    Ok(())
}

pub(crate) fn render_agent_trace_summary(
    report: &AgentTraceSummaryReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Trace summary: {} spans ({} open, {} ended, {} failed)",
            report.span_count,
            report.open_span_count,
            report.ended_span_count,
            report.failed_span_count
        );
        if let Some(trace_id) = &report.trace_id {
            println!("Trace: {trace_id}");
        }
        if let Some(agent_id) = &report.agent_id {
            println!("Agent: {agent_id}");
        }
        if let Some(turn_id) = &report.turn_id {
            println!("Turn: {turn_id}");
        }
        if report.total_duration_ms > 0 {
            let average = report
                .average_duration_ms
                .map(|duration| format!("{duration:.1}"))
                .unwrap_or_else(|| "-".to_string());
            println!(
                "Duration: total={}ms max={}ms avg={}ms",
                report.total_duration_ms, report.max_duration_ms, average
            );
        }
        println!("Statuses: {}", render_named_counts(&report.status_counts));
        println!("Types: {}", render_named_counts(&report.span_type_counts));
        println!("Traces: {}", render_named_counts(&report.trace_counts));
        if !report.slowest_spans.is_empty() {
            println!("Slowest spans:");
            for span in &report.slowest_spans {
                println!(
                    "  {} {} {} {}ms",
                    span.span_id,
                    span.span_type,
                    span.status,
                    span.duration_ms.unwrap_or(0)
                );
            }
        }
        if !report.open_spans.is_empty() {
            println!("Open spans:");
            for span in &report.open_spans {
                println!("  {} {} {}", span.span_id, span.span_type, span.name);
            }
        }
    }
    Ok(())
}

pub(crate) fn render_named_counts(counts: &[crabdb::model::NamedCount]) -> String {
    if counts.is_empty() {
        return "-".to_string();
    }
    counts
        .iter()
        .map(|count| format!("{}={}", count.name, count.count))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn render_agent_trace_span(
    span: &AgentTraceSpan,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(span);
    }
    if !quiet {
        println!("Span: {}", span.span_id);
        println!("Trace: {}", span.trace_id);
        println!("Type: {}", span.span_type);
        println!("Name: {}", span.name);
        println!("Status: {}", span.status);
        if let Some(parent) = &span.parent_span_id {
            println!("Parent: {parent}");
        }
        if let Some(turn) = &span.turn_id {
            println!("Turn: {turn}");
        }
        if let Some(duration_ms) = span.duration_ms {
            println!("Duration: {duration_ms} ms");
        }
    }
    Ok(())
}

pub(crate) fn render_agent_turn_end(
    report: &crabdb::AgentTurnEndReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Ended turn {} as {}",
            report.turn.turn_id, report.turn.status
        );
    }
    Ok(())
}

pub(crate) fn render_agent_run_pause(
    report: &AgentRunPauseReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Paused run {} for {}",
            report.run_state.run_id, report.run_state.agent_id
        );
        println!("Reason: {}", report.run_state.reason);
        println!("Summary: {}", report.run_state.summary);
        if let Some(approval_id) = &report.run_state.approval_id {
            println!("Approval: {approval_id}");
        }
    }
    Ok(())
}

pub(crate) fn render_agent_run_resume(
    report: &AgentRunResumeReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Resumed run {} for {}",
            report.run_state.run_id, report.run_state.agent_id
        );
        if let Some(resumed_at) = report.run_state.resumed_at {
            println!("Resumed at: {resumed_at}");
        }
    }
    Ok(())
}

pub(crate) fn render_agent_run_list(
    run_states: &[AgentRunState],
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(run_states);
    }
    if !quiet {
        if run_states.is_empty() {
            println!("No agent run states");
        }
        for run_state in run_states {
            let approval = run_state.approval_id.as_deref().unwrap_or("-");
            println!(
                "{} {} agent={} reason={} approval={}",
                run_state.run_id, run_state.status, run_state.agent_id, run_state.reason, approval
            );
            println!("  {}", run_state.summary);
        }
    }
    Ok(())
}

pub(crate) fn render_agent_run_state(
    run_state: &AgentRunState,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(run_state);
    }
    if !quiet {
        println!("Agent run: {}", run_state.run_id);
        println!("Agent: {}", run_state.agent_id);
        println!("Status: {}", run_state.status);
        println!("Reason: {}", run_state.reason);
        println!("Summary: {}", run_state.summary);
        if let Some(session_id) = &run_state.session_id {
            println!("Session: {session_id}");
        }
        if let Some(turn_id) = &run_state.turn_id {
            println!("Turn: {turn_id}");
        }
        if let Some(approval_id) = &run_state.approval_id {
            println!("Approval: {approval_id}");
        }
        if let Some(reviewer) = &run_state.reviewer {
            println!("Reviewer: {reviewer}");
        }
        if let Some(note) = &run_state.note {
            println!("Note: {note}");
        }
    }
    Ok(())
}

pub(crate) fn render_session_start(
    report: &AgentSessionStartReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Started session {} for {}",
            report.session.session_id, report.session.agent_id
        );
        if let Some(title) = &report.session.title {
            println!("Title: {title}");
        }
    }
    Ok(())
}

pub(crate) fn render_session_current(
    reports: &[AgentSessionCurrentReport],
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(&reports);
    }
    if !quiet {
        if reports.is_empty() {
            println!("No active sessions");
        }
        for report in reports {
            match &report.session {
                Some(session) => {
                    let title = session.title.as_deref().unwrap_or("");
                    println!(
                        "{} {} {} {}",
                        report.agent_name, session.session_id, session.status, title
                    );
                }
                None => println!("{} has no active session", report.agent_name),
            }
        }
    }
    Ok(())
}

pub(crate) fn render_session_list(
    sessions: &[AgentSession],
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(&sessions);
    }
    if !quiet {
        if sessions.is_empty() {
            println!("No sessions");
        }
        for session in sessions {
            let title = session.title.as_deref().unwrap_or("");
            println!(
                "{} {} {} {}",
                session.session_id, session.status, session.agent_id, title
            );
        }
    }
    Ok(())
}

pub(crate) fn render_session_details(
    details: &AgentSessionDetails,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(details);
    }
    if !quiet {
        println!("Session: {}", details.session.session_id);
        println!("Agent: {}", details.session.agent_id);
        println!("Status: {}", details.session.status);
        if let Some(title) = &details.session.title {
            println!("Title: {title}");
        }
        println!("Turns: {}", details.turns.len());
        println!("Messages: {}", details.messages.len());
        println!("Operations: {}", details.operations.len());
        for turn in &details.turns {
            let after = turn
                .after_change
                .as_ref()
                .map(|change| change.0.as_str())
                .unwrap_or("-");
            println!("  {} {} {}", turn.turn_id, turn.status, after);
        }
        for operation in &details.operations {
            let message = operation.message.as_deref().unwrap_or("");
            println!(
                "  op {} {:?} {}",
                operation.change_id.0, operation.kind, message
            );
        }
    }
    Ok(())
}

pub(crate) fn render_session_context(
    report: &AgentSessionContextReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Session context: {}", report.session.session_id);
        println!("Agent: {}", report.session.agent_id);
        println!("Status: {}", report.session.status);
        if let Some(title) = &report.session.title {
            println!("Title: {title}");
        }
        println!(
            "Totals: {} messages, {} events, {} turns, {} operations",
            report.message_count, report.event_count, report.turn_count, report.operation_count
        );
        if !report.recent_messages.is_empty() {
            println!("Recent messages:");
            for message in &report.recent_messages {
                let preview = single_line_preview(&message.body, 80);
                println!("  {} {} {}", message.id.0, message.role, preview);
            }
        }
        if !report.recent_turns.is_empty() {
            println!("Recent turns:");
            for turn in &report.recent_turns {
                println!("  {} {}", turn.turn_id, turn.status);
            }
        }
        if !report.recent_operations.is_empty() {
            println!("Recent operations:");
            for operation in &report.recent_operations {
                let message = operation.message.as_deref().unwrap_or("");
                println!(
                    "  {} {:?} {}",
                    operation.change_id.0, operation.kind, message
                );
            }
        }
    }
    Ok(())
}

pub(crate) fn render_session_end(
    report: &AgentSessionEndReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Ended session {} as {}",
            report.session.session_id, report.session.status
        );
    }
    Ok(())
}

fn single_line_preview(value: &str, limit: usize) -> String {
    let mut preview = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if preview.len() > limit {
        preview.truncate(limit.saturating_sub(3));
        preview.push_str("...");
    }
    preview
}

pub(crate) fn render_approval_request(
    report: &AgentApprovalRequestReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Requested approval {} {}",
            report.approval.approval_id, report.approval.action
        );
        println!("{}", report.approval.summary);
        if let Some(run_state) = &report.run_state {
            println!("Paused run: {}", run_state.run_id);
        }
    }
    Ok(())
}

pub(crate) fn render_approval_list(
    approvals: &[AgentApproval],
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(&approvals);
    }
    if !quiet {
        if approvals.is_empty() {
            println!("No approvals");
        }
        for approval in approvals {
            println!(
                "{} {} {} {}",
                approval.approval_id, approval.status, approval.agent_id, approval.action
            );
            println!("  {}", approval.summary);
        }
    }
    Ok(())
}

pub(crate) fn render_approval(approval: &AgentApproval, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(approval);
    }
    if !quiet {
        println!("Approval: {}", approval.approval_id);
        println!("Agent: {}", approval.agent_id);
        println!("Status: {}", approval.status);
        println!("Action: {}", approval.action);
        println!("Summary: {}", approval.summary);
        if let Some(session_id) = &approval.session_id {
            println!("Session: {session_id}");
        }
        if let Some(turn_id) = &approval.turn_id {
            println!("Turn: {turn_id}");
        }
        if let Some(reviewer) = &approval.reviewer {
            println!("Reviewer: {reviewer}");
        }
        if let Some(note) = &approval.note {
            println!("Note: {note}");
        }
    }
    Ok(())
}

pub(crate) fn render_approval_decision(
    report: &AgentApprovalDecisionReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Decision {} for {}",
            report.decision, report.approval.approval_id
        );
        if !report.run_states.is_empty() {
            println!("Linked run states: {}", report.run_states.len());
        }
    }
    Ok(())
}

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
        println!("Rewritten agent workdirs: {}", report.rewritten_workdirs);
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
        for error in &report.errors {
            println!("  warning: {error}");
        }
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
