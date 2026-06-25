use super::*;

impl CrabDb {
    pub fn agent_readiness(&self, agent: &str) -> Result<AgentReadinessReport> {
        let status = self.agent_status(agent)?;
        let agent_ref = status.agent.branch.ref_name.clone();
        let pending_approvals = self.list_agent_approvals(Some(agent), Some("pending"))?;
        let conflicts = self
            .list_conflicts()?
            .into_iter()
            .filter(|conflict| {
                conflict.status != "resolved"
                    && (conflict.source_ref.as_deref() == Some(agent_ref.as_str())
                        || conflict.target_ref.as_deref() == Some(agent_ref.as_str()))
            })
            .collect::<Vec<_>>();

        let mut blockers = Vec::new();
        let mut warnings = Vec::new();
        if status.agent.branch.status == "removed" {
            blockers.push(readiness_issue(
                "agent_removed",
                "agent branch has already been removed",
                Some(serde_json::json!({ "status": status.agent.branch.status })),
            ));
        }

        let workdir_state = status.workdir_state.clone();
        if workdir_state
            .as_ref()
            .is_some_and(|state| state != &WorktreeState::Clean)
        {
            let paths = status
                .workdir_changed_paths
                .iter()
                .map(|path| path.path.clone())
                .collect::<Vec<_>>();
            blockers.push(readiness_issue(
                "dirty_workdir",
                "materialized agent workdir has unrecorded changes",
                Some(serde_json::json!({
                    "state": workdir_state.clone(),
                    "paths": paths
                })),
            ));
        }

        if !pending_approvals.is_empty() {
            let approval_ids = pending_approvals
                .iter()
                .map(|approval| approval.approval_id.clone())
                .collect::<Vec<_>>();
            blockers.push(readiness_issue(
                "pending_approvals",
                format!(
                    "{} human approval request(s) are still pending",
                    pending_approvals.len()
                ),
                Some(serde_json::json!({ "approval_ids": approval_ids })),
            ));
        }

        if !conflicts.is_empty() {
            let conflict_ids = conflicts
                .iter()
                .map(|conflict| conflict.conflict_set_id.clone())
                .collect::<Vec<_>>();
            blockers.push(readiness_issue(
                "open_conflicts",
                format!("{} merge conflict set(s) are still open", conflicts.len()),
                Some(serde_json::json!({ "conflict_set_ids": conflict_ids })),
            ));
        }

        match &status.latest_test {
            Some(test) if !test.success => blockers.push(readiness_issue(
                "latest_test_failed",
                "latest recorded test gate did not pass",
                Some(serde_json::json!({
                    "event_id": test.event_id,
                    "status": test.status,
                    "exit_code": test.exit_code,
                    "command": test.command,
                    "suite": test.suite,
                    "score": test.score,
                    "threshold": test.threshold
                })),
            )),
            Some(_) => {}
            None => {
                let issue = readiness_issue(
                    "missing_latest_test",
                    "no test gate has been recorded for this agent",
                    None,
                );
                if self.config.agent.require_test_gate {
                    blockers.push(issue);
                } else {
                    warnings.push(issue);
                }
            }
        }

        match &status.latest_eval {
            Some(eval) if !eval.success => blockers.push(readiness_issue(
                "latest_eval_failed",
                "latest recorded eval gate did not pass",
                Some(serde_json::json!({
                    "event_id": eval.event_id,
                    "status": eval.status,
                    "exit_code": eval.exit_code,
                    "command": eval.command,
                    "suite": eval.suite,
                    "score": eval.score,
                    "threshold": eval.threshold
                })),
            )),
            Some(_) => {}
            None => {
                let issue = readiness_issue(
                    "missing_latest_eval",
                    "no eval gate has been recorded for this agent",
                    None,
                );
                if self.config.agent.require_eval_gate {
                    blockers.push(issue);
                } else {
                    warnings.push(issue);
                }
            }
        }

        blockers.extend(self.required_gate_suite_issues(
            &status.agent.branch.agent_id,
            "test",
            &self.config.agent.required_test_suites,
        )?);
        blockers.extend(self.required_gate_suite_issues(
            &status.agent.branch.agent_id,
            "eval",
            &self.config.agent.required_eval_suites,
        )?);

        if status.changed_paths.is_empty() {
            warnings.push(readiness_issue(
                "no_changed_paths",
                "agent branch does not currently differ from its base",
                None,
            ));
        }
        if status.queued_merges > 0 {
            warnings.push(readiness_issue(
                "queued_merge",
                "agent already has a queued or running merge",
                Some(serde_json::json!({ "queued_merges": status.queued_merges })),
            ));
        }

        let ready = blockers.is_empty();
        Ok(AgentReadinessReport {
            agent: status.agent,
            ready,
            status: if ready { "ready" } else { "blocked" }.to_string(),
            blockers,
            warnings,
            changed_paths: status.changed_paths,
            workdir_state,
            workdir_changed_paths: status.workdir_changed_paths,
            queued_merges: status.queued_merges,
            pending_approvals,
            conflicts,
            latest_test: status.latest_test,
            latest_eval: status.latest_eval,
        })
    }

    pub fn agent_handoff(&self, agent: &str, limit: usize) -> Result<AgentHandoffReport> {
        let limit = normalize_query_limit(limit, 1000)?;
        let readiness = self.agent_readiness(agent)?;
        let agent_details = readiness.agent.clone();
        let current_session = agent_details
            .branch
            .session_id
            .as_deref()
            .map(|session_id| self.show_agent_session(session_id))
            .transpose()?;
        let recent_sessions = self
            .list_agent_sessions(Some(agent))?
            .into_iter()
            .take(limit)
            .collect::<Vec<_>>();
        let recent_events = self.list_agent_events(Some(agent), None, None, None, limit)?;
        let recent_spans = self.list_agent_trace_spans(Some(agent), None, None, None, limit)?;
        let recent_operations = self.agent_timeline(agent, limit)?;
        let next_steps = handoff_next_steps(&readiness, current_session.as_ref());
        Ok(AgentHandoffReport {
            agent: agent_details,
            readiness,
            current_session,
            recent_sessions,
            recent_events,
            recent_spans,
            recent_operations,
            next_steps,
        })
    }
}
