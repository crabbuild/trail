use super::render_json;

use crabdb::model::*;
use crabdb::Result;

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

pub(crate) fn render_agent_review_packet(
    report: &AgentReviewPacketReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!(
            "Agent review: {} ({})",
            report.agent.record.name, report.readiness.status
        );
        println!("Ref: {}", report.agent.branch.ref_name);
        println!(
            "Ready: {}  Changed paths: {}  Blockers: {}  Warnings: {}",
            report.readiness.ready,
            report.changed_paths.len(),
            report.readiness.blockers.len(),
            report.readiness.warnings.len()
        );
        println!(
            "Evidence: {} operation(s), {} session(s), {} event(s), {} span(s), {} approval(s), {} gate(s), {} conflict(s), {} queued merge(s)",
            report.evidence_summary.operations,
            report.evidence_summary.sessions,
            report.evidence_summary.events,
            report.evidence_summary.spans,
            report.evidence_summary.approvals,
            report.evidence_summary.gates,
            report.evidence_summary.conflicts,
            report.evidence_summary.queued_merges
        );
        if !report.readiness.blockers.is_empty() {
            println!("Blockers:");
            for blocker in &report.readiness.blockers {
                println!("  {}: {}", blocker.code, blocker.message);
            }
        }
        if !report.readiness.warnings.is_empty() {
            println!("Warnings:");
            for warning in &report.readiness.warnings {
                println!("  {}: {}", warning.code, warning.message);
            }
        }
        if let Some(test) = &report.latest_test {
            println!("Latest test: {} ({})", test.status, test.command.join(" "));
        }
        if let Some(eval) = &report.latest_eval {
            println!("Latest eval: {} ({})", eval.status, eval.command.join(" "));
        }
        if report.evidence_summary.pending_approvals > 0 {
            println!(
                "Pending approvals: {}",
                report.evidence_summary.pending_approvals
            );
        }
        if !report.conflicts.is_empty() {
            println!("Conflicts:");
            for conflict in &report.conflicts {
                println!("  {} {}", conflict.conflict_set_id, conflict.status);
                for detail in &conflict.details {
                    println!("    {detail}");
                }
            }
        }
        if !report.changed_paths.is_empty() {
            println!("Changed paths:");
            for path in &report.changed_paths {
                println!("  {:?} {}", path.kind, path.path);
            }
        }
        if !report.recent_operations.is_empty() {
            println!("Recent operations:");
            for operation in &report.recent_operations {
                println!(
                    "  {} {:?} {} path(s) {}",
                    operation.change_id.0,
                    operation.kind,
                    operation.path_count,
                    operation.message.as_deref().unwrap_or("")
                );
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
