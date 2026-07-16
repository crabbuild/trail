use super::*;

impl Trail {
    pub fn lane_readiness(&self, lane: &str) -> Result<LaneReadinessReport> {
        let environment_refresh_error = if self.lane_workspace_view(lane)?.is_some() {
            self.refresh_workspace_environment_staleness(lane)
                .err()
                .map(|err| err.to_string())
        } else {
            None
        };
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
        if let Some(view) = self.lane_workspace_view(lane)? {
            let missing_paths = [
                &view.source_upper,
                &view.generated_upper,
                &view.scratch_upper,
                &view.meta_dir,
            ]
            .into_iter()
            .filter(|path| !Path::new(path).is_dir())
            .cloned()
            .collect::<Vec<_>>();
            if !missing_paths.is_empty()
                || matches!(view.status.as_str(), "failed" | "unhealthy" | "corrupt")
            {
                blockers.push(readiness_issue(
                    "workspace_view_unhealthy",
                    "layered workspace view is not healthy",
                    Some(serde_json::json!({
                        "view_id": view.view_id,
                        "status": view.status,
                        "missing_paths": missing_paths,
                    })),
                ));
            }
            if let (Some(pid), Some(token)) = (view.owner_pid, view.owner_start_token.as_deref()) {
                if process_matches_start_token(pid, token) {
                    blockers.push(readiness_issue(
                        "workspace_view_active_writers",
                        format!("workspace view has an active writer in process {pid}"),
                        Some(serde_json::json!({
                            "view_id": view.view_id,
                            "owner_pid": pid,
                            "heartbeat_at": view.heartbeat_at,
                        })),
                    ));
                } else {
                    blockers.push(readiness_issue(
                        "workspace_view_unhealthy",
                        "workspace view has a stale mount lease that must be recovered",
                        Some(serde_json::json!({
                            "view_id": view.view_id,
                            "stale_owner_pid": pid,
                        })),
                    ));
                }
            }
            let journal_sequence = self.workspace_view_last_journal_sequence(&view)?;
            if journal_sequence > view.checkpoint_seq && !status.workdir_changed_paths.is_empty() {
                blockers.push(readiness_issue(
                    "uncheckpointed_source_changes",
                    "source upper contains changes newer than the last workspace checkpoint",
                    Some(serde_json::json!({
                        "view_id": view.view_id,
                        "checkpoint_sequence": view.checkpoint_seq,
                        "journal_sequence": journal_sequence,
                        "paths": status.workdir_changed_paths.iter().map(|path| path.path.clone()).collect::<Vec<_>>(),
                    })),
                ));
            }
            if let Some(error) = &environment_refresh_error {
                blockers.push(readiness_issue(
                    "dependency_environment_stale",
                    format!("dependency environment could not be refreshed: {error}"),
                    Some(serde_json::json!({"view_id": view.view_id})),
                ));
            }
            for environment in self.workspace_environment_status(lane)? {
                match environment.status.as_str() {
                    "ready"
                        if environment.attached_key.as_deref()
                            == Some(environment.expected_key.as_str()) => {}
                    "failed" => blockers.push(readiness_issue(
                        "dependency_layer_build_failed",
                        format!(
                            "dependency environment `{}` failed to build",
                            environment.adapter
                        ),
                        Some(serde_json::json!({
                            "adapter": environment.adapter,
                            "expected_key": environment.expected_key,
                            "reason": environment.reason,
                        })),
                    )),
                    _ => blockers.push(readiness_issue(
                        "dependency_environment_stale",
                        format!(
                            "dependency environment `{}` is not synchronized",
                            environment.adapter
                        ),
                        Some(serde_json::json!({
                            "adapter": environment.adapter,
                            "status": environment.status,
                            "expected_key": environment.expected_key,
                            "attached_key": environment.attached_key,
                            "reason": environment.reason,
                        })),
                    )),
                }
            }
            if let Some(generation) = self.active_environment_generation(lane)? {
                for component in generation.components {
                    for resource in component.runtime_resources {
                        for secret in &resource.secret_statuses {
                            if secret.reference.required && secret.status != "available" {
                                blockers.push(readiness_issue(
                                    "environment_secret_unavailable",
                                    format!(
                                        "required secret reference `{}` for runtime resource `{}` is unavailable",
                                        secret.reference.name, resource.declaration.name
                                    ),
                                    Some(serde_json::json!({
                                        "generation_id": generation.generation_id,
                                        "component_id": component.component_id,
                                        "resource": resource.declaration.name,
                                        "secret": secret.reference.name,
                                        "provider": secret.reference.provider,
                                        "status": secret.status,
                                        "reason": secret.reason,
                                    })),
                                ));
                            }
                        }
                        if resource.status != "running" || resource.health_status != "healthy" {
                            blockers.push(readiness_issue(
                                "environment_runtime_unhealthy",
                                format!(
                                    "runtime resource `{}` for component `{}` is not healthy",
                                    resource.declaration.name, component.component_id
                                ),
                                Some(serde_json::json!({
                                    "generation_id": generation.generation_id,
                                    "component_id": component.component_id,
                                    "resource": resource.declaration.name,
                                    "allocation_id": resource.allocation_id,
                                    "status": resource.status,
                                    "health_status": resource.health_status,
                                    "reason": resource.reason,
                                })),
                            ));
                        }
                    }
                }
            }
            for layer in self.workspace_view_layer_reports(&view.view_id)? {
                if layer.state != "ready" || self.verify_workspace_layer(&layer.layer_id).is_err() {
                    blockers.push(readiness_issue(
                        "workspace_layer_corrupt",
                        format!(
                            "workspace layer `{}` failed integrity verification",
                            layer.layer_id
                        ),
                        Some(serde_json::json!({
                            "layer_id": layer.layer_id,
                            "state": layer.state,
                        })),
                    ));
                }
            }
            let quota = self.workspace_quota_status(lane)?;
            if !quota.exceeded.is_empty() {
                blockers.push(readiness_issue(
                    "workspace_quota_exceeded",
                    "workspace view or cache exceeds its configured resource quota",
                    Some(serde_json::to_value(&quota)?),
                ));
            }
            if let Some(shadow) = self.workspace_git_shadow(&view)? {
                let shadow = self.refresh_workspace_git_shadow(&shadow)?;
                if shadow.status != "ready" || shadow.current_head != shadow.pinned_head {
                    blockers.push(readiness_issue(
                        "shadow_git_head_diverged",
                        "shadow Git HEAD diverged from the pinned workspace commit",
                        Some(serde_json::json!({
                            "view_id": view.view_id,
                            "pinned_head": shadow.pinned_head,
                            "current_head": shadow.current_head,
                            "policy": shadow.policy,
                        })),
                    ));
                }
            }
        }
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

        let head = self.get_ref(&status.lane.branch.ref_name)?;
        let expected_environment_keys = self
            .workspace_environment_status(lane)
            .unwrap_or_default()
            .into_iter()
            .map(|environment| environment.expected_key)
            .collect::<BTreeSet<_>>();
        for (kind, gate) in [
            ("test", status.latest_test.as_ref()),
            ("eval", status.latest_eval.as_ref()),
        ] {
            let Some(gate) = gate.filter(|gate| gate.success) else {
                continue;
            };
            if gate.source_root.as_ref() != Some(&head.root_id) {
                blockers.push(readiness_issue(
                    format!("{kind}_gate_stale_source_root"),
                    format!("latest {kind} gate did not run against the current lane root"),
                    Some(serde_json::json!({
                        "event_id": gate.event_id,
                        "gate_root": gate.source_root,
                        "current_root": head.root_id,
                    })),
                ));
            }
            let gate_environment_keys = gate
                .environment_keys
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>();
            if gate_environment_keys != expected_environment_keys {
                blockers.push(readiness_issue(
                    format!("{kind}_gate_stale_environment"),
                    format!("latest {kind} gate used different workspace environment layers"),
                    Some(serde_json::json!({
                        "event_id": gate.event_id,
                        "gate_environment_keys": gate_environment_keys,
                        "current_environment_keys": expected_environment_keys,
                    })),
                ));
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
