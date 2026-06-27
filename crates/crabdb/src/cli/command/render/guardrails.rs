use super::render_json;

use crabdb::model::*;
use crabdb::Result;

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
        if let Some(lane) = &report.lane {
            println!("Lane: {}", lane.record.name);
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
