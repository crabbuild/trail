use super::render_json;

use crabdb;
use crabdb::model::*;
use crabdb::Result;

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
