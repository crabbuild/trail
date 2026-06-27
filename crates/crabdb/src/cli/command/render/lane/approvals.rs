use super::render_json;

use crabdb::model::*;
use crabdb::Result;

pub(crate) fn render_approval_request(
    report: &LaneApprovalRequestReport,
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
    approvals: &[LaneApproval],
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
                approval.approval_id, approval.status, approval.lane_id, approval.action
            );
            println!("  {}", approval.summary);
        }
    }
    Ok(())
}

pub(crate) fn render_approval(approval: &LaneApproval, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(approval);
    }
    if !quiet {
        println!("Approval: {}", approval.approval_id);
        println!("Lane: {}", approval.lane_id);
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
    report: &LaneApprovalDecisionReport,
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
