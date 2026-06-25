use super::render_json;

use crabdb::model::*;
use crabdb::Result;

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
