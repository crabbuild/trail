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

        let (lane_id, session_id, workdir, turn_id, head_change, started_event_id) = {
            let _lock = self.acquire_write_lock()?;
            let branch = self.lane_branch(lane)?;
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
                    "head_change": head.change_id.0.clone()
                }),
            )?;
            (
                branch.lane_id,
                session_id,
                workdir,
                turn_id,
                head.change_id,
                started_event_id,
            )
        };

        let run = run_command_with_timeout(
            &command,
            Path::new(&workdir),
            Duration::from_secs(timeout_secs),
        )?;
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
                    "stderr_truncated": stderr_truncated
                }),
            )?;
            self.finish_lane_turn(&turn_id, &status, Some(&head_change))?;
            (stdout_object, stderr_object, finished_event_id)
        };

        Ok(LaneTestReport {
            lane_id,
            turn_id,
            session_id,
            workdir,
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
        })
    }
}
