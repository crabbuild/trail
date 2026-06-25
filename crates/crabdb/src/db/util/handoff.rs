use super::*;

pub(crate) fn handoff_next_steps(
    readiness: &AgentReadinessReport,
    current_session: Option<&AgentSessionDetails>,
) -> Vec<String> {
    let mut steps = Vec::new();
    for blocker in &readiness.blockers {
        match blocker.code.as_str() {
            "agent_removed" => steps
                .push("Restore or respawn the agent branch before continuing the handoff.".into()),
            "dirty_workdir" => steps.push(
                "Record or force-sync the materialized workdir before reviewing or merging.".into(),
            ),
            "pending_approvals" => {
                steps.push("Resolve pending human approvals before merge.".into())
            }
            "open_conflicts" => steps.push("Resolve open conflict sets before merge.".into()),
            "latest_test_failed" => steps.push("Fix and rerun the latest test gate.".into()),
            "latest_eval_failed" => steps.push("Fix and rerun the latest eval gate.".into()),
            "missing_required_test_suite" => {
                steps.push("Run the required named test suite before merge.".into())
            }
            "missing_required_eval_suite" => {
                steps.push("Run the required named eval suite before merge.".into())
            }
            "required_test_suite_failed" => {
                steps.push("Fix and rerun the failed required test suite.".into())
            }
            "required_eval_suite_failed" => {
                steps.push("Fix and rerun the failed required eval suite.".into())
            }
            _ => steps.push(blocker.message.clone()),
        }
    }

    if steps.is_empty() {
        steps.push("Review changed paths, recent operations, and provenance before merge.".into());
    }

    for warning in &readiness.warnings {
        match warning.code.as_str() {
            "missing_latest_test" => {
                steps.push("Run a test gate if this branch should be merged.".into())
            }
            "missing_latest_eval" => {
                steps.push("Run an eval gate when model or policy quality matters.".into())
            }
            "no_changed_paths" => steps
                .push("Confirm this is an audit-only handoff or record the intended work.".into()),
            "queued_merge" => steps.push(
                "Inspect the existing queued or running merge before queuing another.".into(),
            ),
            _ => steps.push(warning.message.clone()),
        }
    }

    match current_session {
        Some(details) if details.session.status == "active" => steps.push(format!(
            "Continue or close active session `{}` after the receiving agent catches up.",
            details.session.session_id
        )),
        Some(details) => steps.push(format!(
            "Use session `{}` as historical context for this handoff.",
            details.session.session_id
        )),
        None => steps
            .push("Start a new session or turn if the receiving agent will continue work.".into()),
    }

    steps.dedup();
    steps
}
