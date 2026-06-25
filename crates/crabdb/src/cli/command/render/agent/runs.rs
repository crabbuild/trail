use super::render_json;

use crabdb::model::*;
use crabdb::Result;

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
