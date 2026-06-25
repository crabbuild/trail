use super::render_json;

use crabdb::model::*;
use crabdb::Result;

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

pub(crate) fn render_map_key(key: &MapKeyInspect) -> String {
    key.text
        .clone()
        .unwrap_or_else(|| format!("hex:{}", key.hex))
}

pub(crate) fn render_map_value_summary(value: &MapValueInspect) -> Result<String> {
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
