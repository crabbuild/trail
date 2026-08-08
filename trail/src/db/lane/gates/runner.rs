use super::*;

impl Trail {
    pub(crate) fn run_lane_gate(
        &mut self,
        kind: &str,
        lane: &str,
        command: Vec<String>,
        turn_id: Option<&str>,
        timeout_secs: u64,
        options: LaneGateOptions,
    ) -> Result<LaneTestReport> {
        let (started_event_type, finished_event_type, run_kind, passed_status, failed_status) =
            match kind {
                "test" => (
                    "test_started",
                    "test_finished",
                    "test_run",
                    "test_passed",
                    "test_failed",
                ),
                "eval" => (
                    "eval_started",
                    "eval_finished",
                    "eval_run",
                    "eval_passed",
                    "eval_failed",
                ),
                other => {
                    return Err(Error::InvalidInput(format!(
                        "lane gate kind must be test or eval, got `{other}`"
                    )));
                }
            };
        validate_ref_segment(lane)?;
        if command.is_empty() {
            return Err(Error::InvalidInput(format!(
                "lane {kind} requires a command after `--`"
            )));
        }
        if timeout_secs == 0 {
            return Err(Error::InvalidInput(format!(
                "lane {kind} timeout must be greater than zero"
            )));
        }
        let options = normalize_lane_gate_options(kind, options)?;
        let suite = options.suite.clone();
        let score = options.score;
        let threshold = options.threshold;
        let surface = if kind == "test" {
            "lane_test"
        } else {
            "lane_eval"
        };
        let mut managed = self.prepare_managed_lane_execution(lane, surface, &command)?;

        let setup = (|| -> Result<_> {
            let (
                lane_id,
                session_id,
                workdir,
                turn_id,
                head_change,
                source_root,
                _workdir_mode,
                view,
                environment_keys,
                layer_ids,
                started_event_id,
            ) = {
                let _lock = self.acquire_write_lock()?;
                let branch = self.lane_branch(lane)?;
                let lane_record = self.lane_record(&branch.lane_id)?;
                let workdir_mode = self.lane_workdir_mode_for(&lane_record, &branch)?;
                let Some(workdir) = branch.workdir.clone() else {
                    return Err(Error::InvalidInput(format!(
                        "lane `{lane}` does not have a materialized workdir"
                    )));
                };
                let workdir_path = PathBuf::from(&workdir);
                if !workdir_path.is_dir() {
                    return Err(Error::WorkspaceNotFound(workdir_path));
                }
                let head = self.get_ref(&branch.ref_name)?;
                let view = self.lane_workspace_view(lane)?;
                let environments = if view.is_some() {
                    self.workspace_environment_status(lane)?
                } else {
                    Vec::new()
                };
                if let Some(environment) = environments
                    .iter()
                    .find(|environment| environment.status != "ready")
                {
                    return Err(Error::InvalidInput(format!(
                    "lane `{lane}` dependency environment `{}` is {}; run `trail deps sync {lane}` before validation",
                    environment.adapter, environment.status
                )));
                }
                let environment_keys = environments
                    .iter()
                    .map(|environment| environment.expected_key.clone())
                    .collect::<Vec<_>>();
                let layer_ids = if let Some(view) = &view {
                    self.workspace_layer_bindings_for_source_upper(Path::new(&view.source_upper))?
                        .into_iter()
                        .filter_map(|binding| binding.layer_id)
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let (turn_id, session_id) = if let Some(turn_id) = turn_id {
                    let turn = self.lane_turn(turn_id)?;
                    if turn.lane_id != branch.lane_id {
                        return Err(Error::InvalidInput(format!(
                            "turn `{turn_id}` does not belong to lane `{lane}`"
                        )));
                    }
                    if turn.ended_at.is_some() {
                        return Err(Error::InvalidInput(format!(
                            "turn `{turn_id}` is already ended"
                        )));
                    }
                    (turn.turn_id, turn.session_id)
                } else {
                    let turn_id = self.open_lane_turn(
                        &branch.lane_id,
                        branch.session_id.as_deref(),
                        &branch.base_change,
                        &head.change_id,
                        Some(&serde_json::json!({
                            "kind": run_kind,
                            "command": command.clone(),
                            "suite": suite.clone(),
                            "score": score,
                            "threshold": threshold
                        })),
                    )?;
                    (turn_id, branch.session_id.clone())
                };
                let started_event_id = self.insert_lane_event_with_context(
                    &branch.lane_id,
                    session_id.as_deref(),
                    Some(&turn_id),
                    started_event_type,
                    Some(&head.change_id),
                    None,
                    &serde_json::json!({
                        "kind": kind,
                        "command": command.clone(),
                        "suite": suite.clone(),
                        "score": score,
                        "threshold": threshold,
                        "workdir": workdir.clone(),
                        "timeout_secs": timeout_secs,
                        "head_change": head.change_id.0.clone(),
                        "source_root": head.root_id.0.clone(),
                        "view_id": view.as_ref().map(|view| view.view_id.as_str()),
                        "view_generation": view.as_ref().map(|view| view.generation),
                        "environment_keys": environment_keys.clone(),
                        "layer_ids": layer_ids.clone()
                    }),
                )?;
                (
                    branch.lane_id,
                    session_id,
                    workdir,
                    turn_id,
                    head.change_id,
                    head.root_id,
                    workdir_mode,
                    view,
                    environment_keys,
                    layer_ids,
                    started_event_id,
                )
            };
            Ok((
                lane_id,
                session_id,
                workdir,
                turn_id,
                head_change,
                source_root,
                _workdir_mode,
                view,
                environment_keys,
                layer_ids,
                started_event_id,
            ))
        })();
        let (
            lane_id,
            session_id,
            workdir,
            turn_id,
            head_change,
            source_root,
            _workdir_mode,
            view,
            environment_keys,
            layer_ids,
            started_event_id,
        ) = match setup {
            Ok(setup) => setup,
            Err(error) => {
                self.mark_managed_lane_execution_command(
                    &mut managed,
                    "failed",
                    Some(&format!("managed lane {kind} setup failed: {error}")),
                    None,
                )?;
                let lifecycle = self.finalize_managed_lane_execution(
                    managed,
                    Some(format!("Managed lane {kind} failed-setup checkpoint")),
                );
                let cleanup = [
                    lifecycle.checkpoint_error.as_deref(),
                    lifecycle.disposal_error.as_deref(),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
                if cleanup.is_empty() {
                    return Err(error);
                }
                return Err(Error::Corrupt(format!(
                    "{error}; managed finalization also failed: {}",
                    cleanup.join("; ")
                )));
            }
        };

        let environment = managed.environment.clone();
        let run = match run_command_with_timeout_env(
            &command,
            &managed.workdir,
            Duration::from_secs(timeout_secs),
            &environment,
        ) {
            Ok(run) => run,
            Err(error) => {
                self.mark_managed_lane_execution_command(
                    &mut managed,
                    "failed",
                    Some(&error.to_string()),
                    None,
                )?;
                let lifecycle = self.finalize_managed_lane_execution(
                    managed,
                    Some(format!("Managed lane {kind} failed-launch checkpoint")),
                );
                let cleanup = [
                    lifecycle.checkpoint_error.as_deref(),
                    lifecycle.disposal_error.as_deref(),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
                if cleanup.is_empty() {
                    return Err(error);
                }
                return Err(Error::Corrupt(format!(
                    "{error}; managed finalization also failed: {}",
                    cleanup.join("; ")
                )));
            }
        };
        let execution_error = (!run.success && run.exit_code.is_none())
            .then(|| String::from_utf8_lossy(&run.stderr).trim().to_string());
        self.mark_managed_lane_execution_command(
            &mut managed,
            if run.success { "succeeded" } else { "failed" },
            execution_error.as_deref(),
            run.exit_code,
        )?;
        let lifecycle = self.finalize_managed_lane_execution(
            managed,
            Some(format!("Managed lane {kind} checkpoint")),
        );
        let threshold_met = score
            .zip(threshold)
            .map(|(score, threshold)| score >= threshold);
        let gate_success = run.success && threshold_met.unwrap_or(true);
        let status = if gate_success {
            passed_status
        } else {
            failed_status
        }
        .to_string();
        let stdout_bytes = run.stdout.len() as u64;
        let stderr_bytes = run.stderr.len() as u64;
        let stdout_hash = sha256_hex(&run.stdout);
        let stderr_hash = sha256_hex(&run.stderr);
        let (stdout_preview, stdout_truncated) = output_preview(&run.stdout);
        let (stderr_preview, stderr_truncated) = output_preview(&run.stderr);

        let (stdout_object, stderr_object, finished_event_id) = {
            let _lock = self.acquire_write_lock()?;
            let stdout_object = self.put_blob(run.stdout.clone())?;
            let stderr_object = self.put_blob(run.stderr.clone())?;
            let finished_event_id = self.insert_lane_event_with_context(
                &lane_id,
                session_id.as_deref(),
                Some(&turn_id),
                finished_event_type,
                Some(&head_change),
                None,
                &serde_json::json!({
                    "kind": kind,
                    "command": command.clone(),
                    "suite": suite.clone(),
                    "score": score,
                    "threshold": threshold,
                    "threshold_met": threshold_met,
                    "status": status.clone(),
                    "success": gate_success,
                    "process_success": run.success,
                    "exit_code": run.exit_code,
                    "timed_out": run.timed_out,
                    "duration_ms": run.duration_ms,
                    "stdout_object": stdout_object.0.clone(),
                    "stderr_object": stderr_object.0.clone(),
                    "stdout_bytes": stdout_bytes,
                    "stderr_bytes": stderr_bytes,
                    "stdout_hash": stdout_hash,
                    "stderr_hash": stderr_hash,
                    "stdout_preview": stdout_preview.clone(),
                    "stderr_preview": stderr_preview.clone(),
                    "stdout_truncated": stdout_truncated,
                    "stderr_truncated": stderr_truncated,
                    "source_root": source_root.0.clone(),
                    "view_id": view.as_ref().map(|view| view.view_id.as_str()),
                    "view_generation": view.as_ref().map(|view| view.generation),
                    "environment_keys": environment_keys.clone(),
                    "layer_ids": layer_ids.clone()
                }),
            )?;
            self.finish_lane_turn(&turn_id, &status, Some(&head_change))?;
            (stdout_object, stderr_object, finished_event_id)
        };

        if gate_success {
            let gate_name = suite.as_deref().unwrap_or(kind);
            self.promote_successful_gate_environment_outputs(
                lane,
                gate_name,
                &serde_json::json!({
                    "event_id": finished_event_id,
                    "turn_id": turn_id,
                    "source_root": source_root.0,
                    "environment_keys": environment_keys,
                    "command": command,
                    "success": true
                }),
            )?;
        }

        Ok(LaneTestReport {
            lane_id,
            turn_id,
            session_id,
            workdir,
            source_root,
            view_id: view.as_ref().map(|view| view.view_id.clone()),
            view_generation: view.as_ref().map(|view| view.generation),
            environment_keys,
            layer_ids,
            command,
            kind: kind.to_string(),
            suite,
            score,
            threshold,
            status,
            success: gate_success,
            exit_code: run.exit_code,
            timed_out: run.timed_out,
            duration_ms: run.duration_ms,
            stdout_object,
            stderr_object,
            stdout_bytes,
            stderr_bytes,
            stdout_preview,
            stderr_preview,
            stdout_truncated,
            stderr_truncated,
            started_event_id,
            finished_event_id,
            lifecycle,
        })
    }
}
