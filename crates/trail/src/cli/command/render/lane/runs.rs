use super::render_json;

use trail::model::*;
use trail::Result;

pub(crate) fn render_lane_run_pause(
    report: &LaneRunPauseReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Paused run {} for {}",
            report.run_state.run_id, report.run_state.lane_id
        );
        println!("Reason: {}", report.run_state.reason);
        println!("Summary: {}", report.run_state.summary);
        if let Some(approval_id) = &report.run_state.approval_id {
            println!("Approval: {approval_id}");
        }
    }
    Ok(())
}

pub(crate) fn render_lane_run_resume(
    report: &LaneRunResumeReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Resumed run {} for {}",
            report.run_state.run_id, report.run_state.lane_id
        );
        if let Some(resumed_at) = report.run_state.resumed_at {
            println!("Resumed at: {resumed_at}");
        }
    }
    Ok(())
}

pub(crate) fn render_lane_run_list(
    run_states: &[LaneRunState],
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(run_states);
    }
    if !quiet {
        if run_states.is_empty() {
            println!("No lane run states");
        }
        for run_state in run_states {
            let approval = run_state.approval_id.as_deref().unwrap_or("-");
            println!(
                "{} {} lane={} reason={} approval={}",
                run_state.run_id, run_state.status, run_state.lane_id, run_state.reason, approval
            );
            println!("  {}", run_state.summary);
        }
    }
    Ok(())
}

pub(crate) fn render_lane_run_state(
    run_state: &LaneRunState,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(run_state);
    }
    if !quiet {
        println!("Lane run: {}", run_state.run_id);
        println!("Lane: {}", run_state.lane_id);
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
