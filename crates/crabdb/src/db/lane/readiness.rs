use super::*;

impl CrabDb {
    pub fn lane_readiness(&self, lane: &str) -> Result<LaneReadinessReport> {
        let status = self.lane_status(lane)?;
        let lane_ref = status.lane.branch.ref_name.clone();
        let pending_approvals = self.list_lane_approvals(Some(lane), Some("pending"))?;
        let conflicts = self
            .list_conflicts()?
            .into_iter()
            .filter(|conflict| {
                conflict.status != "resolved"
                    && (conflict.source_ref.as_deref() == Some(lane_ref.as_str())
                        || conflict.target_ref.as_deref() == Some(lane_ref.as_str()))
            })
            .collect::<Vec<_>>();

        let mut blockers = Vec::new();
        let mut warnings = Vec::new();
        if status.lane.branch.status == "removed" {
            blockers.push(readiness_issue(
                "lane_removed",
                "lane branch has already been removed",
                Some(serde_json::json!({ "status": status.lane.branch.status })),
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
                "materialized lane workdir has unrecorded changes",
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
                    "no test gate has been recorded for this lane",
                    None,
                );
                if self.config.lane.require_test_gate {
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
                    "no eval gate has been recorded for this lane",
                    None,
                );
                if self.config.lane.require_eval_gate {
                    blockers.push(issue);
                } else {
                    warnings.push(issue);
                }
            }
        }

        blockers.extend(self.required_gate_suite_issues(
            &status.lane.branch.lane_id,
            "test",
            &self.config.lane.required_test_suites,
        )?);
        blockers.extend(self.required_gate_suite_issues(
            &status.lane.branch.lane_id,
            "eval",
            &self.config.lane.required_eval_suites,
        )?);

        if status.changed_paths.is_empty() {
            warnings.push(readiness_issue(
                "no_changed_paths",
                "lane branch does not currently differ from its base",
                None,
            ));
        }
        if let Some(issue) = self.stale_lane_base_warning(status.base_status.as_ref()) {
            warnings.push(issue);
        }
        if status.queued_merges > 0 {
            warnings.push(readiness_issue(
                "queued_merge",
                "lane already has a queued or running merge",
                Some(serde_json::json!({ "queued_merges": status.queued_merges })),
            ));
        }

        let ready = blockers.is_empty();
        Ok(LaneReadinessReport {
            lane: status.lane,
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

    fn stale_lane_base_warning(
        &self,
        base_status: Option<&LaneBaseStatus>,
    ) -> Option<LaneReadinessIssue> {
        let base_status = base_status?;
        let operations_behind = base_status.operations_behind?;
        if operations_behind == 0 {
            return None;
        }
        let plural = if operations_behind == 1 {
            "operation"
        } else {
            "operations"
        };
        Some(readiness_issue(
            "stale_lane_base",
            format!(
                "lane started {operations_behind} {plural} behind `{}`",
                base_status.target_branch
            ),
            Some(serde_json::json!({
                "target_branch": base_status.target_branch,
                "target_ref": base_status.target_ref,
                "target_change": base_status.target_change,
                "lane_base_change": base_status.lane_base_change,
                "operations_behind": operations_behind
            })),
        ))
    }

    pub fn lane_handoff(&self, lane: &str, limit: usize) -> Result<LaneHandoffReport> {
        let limit = normalize_query_limit(limit, 1000)?;
        let readiness = self.lane_readiness(lane)?;
        let lane_details = readiness.lane.clone();
        let current_session = lane_details
            .branch
            .session_id
            .as_deref()
            .map(|session_id| self.show_lane_session(session_id))
            .transpose()?;
        let recent_sessions = self
            .list_lane_sessions(Some(lane))?
            .into_iter()
            .take(limit)
            .collect::<Vec<_>>();
        let recent_events = self.list_lane_events(Some(lane), None, None, None, limit)?;
        let recent_spans = self.list_lane_trace_spans(Some(lane), None, None, None, limit)?;
        let recent_operations = self.lane_timeline(lane, limit)?;
        let next_steps = handoff_next_steps(&readiness, current_session.as_ref());
        Ok(LaneHandoffReport {
            lane: lane_details,
            readiness,
            current_session,
            recent_sessions,
            recent_events,
            recent_spans,
            recent_operations,
            next_steps,
        })
    }

    pub fn lane_review_packet(&self, lane: &str, limit: usize) -> Result<LaneReviewPacketReport> {
        let limit = normalize_query_limit(limit, 1000)?;
        let readiness = self.lane_readiness(lane)?;
        let lane_details = readiness.lane.clone();
        let current_session = lane_details
            .branch
            .session_id
            .as_deref()
            .map(|session_id| self.show_lane_session(session_id))
            .transpose()?;
        let recent_operations = self.lane_timeline(lane, limit)?;
        let recent_sessions = self
            .list_lane_sessions(Some(lane))?
            .into_iter()
            .take(limit)
            .collect::<Vec<_>>();
        let recent_events = self.list_lane_events(Some(lane), None, None, None, limit)?;
        let recent_spans = self.list_lane_trace_spans(Some(lane), None, None, None, limit)?;
        let approvals = self.list_lane_approvals(Some(lane), None)?;
        let recent_approvals = approvals.iter().take(limit).cloned().collect::<Vec<_>>();
        let recent_gates =
            self.lane_gate_history_for_id(&lane_details.branch.lane_id, None, limit)?;
        let conflicts = readiness
            .conflicts
            .iter()
            .take(limit)
            .cloned()
            .collect::<Vec<_>>();
        let next_steps = handoff_next_steps(&readiness, current_session.as_ref());
        let evidence_summary = LaneReviewEvidenceSummary {
            operations: recent_operations.len(),
            sessions: recent_sessions.len(),
            events: recent_events.len(),
            spans: recent_spans.len(),
            approvals: recent_approvals.len(),
            pending_approvals: readiness.pending_approvals.len(),
            conflicts: readiness.conflicts.len(),
            queued_merges: readiness.queued_merges,
            gates: recent_gates.len(),
        };

        Ok(LaneReviewPacketReport {
            lane: lane_details,
            changed_paths: readiness.changed_paths.clone(),
            workdir_state: readiness.workdir_state.clone(),
            latest_test: readiness.latest_test.clone(),
            latest_eval: readiness.latest_eval.clone(),
            readiness,
            evidence_summary,
            recent_gates,
            recent_operations,
            recent_sessions,
            recent_events,
            recent_spans,
            recent_approvals,
            conflicts,
            next_steps,
        })
    }
}
