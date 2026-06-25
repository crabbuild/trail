use super::*;

impl CrabDb {
    pub fn record_agent_workdir(
        &mut self,
        agent: &str,
        message: Option<String>,
    ) -> Result<AgentRecordReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;
        let branch = self.agent_branch(agent)?;
        let Some(workdir) = branch.workdir.clone() else {
            return Err(Error::InvalidInput(format!(
                "agent `{agent}` does not have a materialized workdir"
            )));
        };
        let workdir_path = PathBuf::from(&workdir);
        if !workdir_path.is_dir() {
            return Err(Error::WorkspaceNotFound(workdir_path));
        }
        let head = self.get_ref(&branch.ref_name)?;
        let previous_files = self.load_root_files(&head.root_id)?;
        let disk_files = self.scan_files_under(&workdir_path)?;
        let actor = Actor::agent(agent);
        let change_id = self.allocate_change_id(&actor.id, "agent_record")?;
        let built =
            self.build_root_from_disk_files(&disk_files, &change_id, Some(&previous_files))?;
        let diff = self.diff_file_maps(&previous_files, &built.files)?;
        if diff.changes.is_empty() {
            return Ok(AgentRecordReport {
                agent_id: branch.agent_id,
                operation: None,
                root_id: head.root_id,
                changed_paths: Vec::new(),
            });
        }
        if let Some(session_id) = &branch.session_id {
            self.ensure_agent_session(&branch.agent_id, session_id, None)?;
        }
        let turn_id = self.open_agent_turn(
            &branch.agent_id,
            branch.session_id.as_deref(),
            &branch.base_change,
            &head.change_id,
            Some(&serde_json::json!({
                "kind": "workdir_record",
                "path_count": diff.summaries.len()
            })),
        )?;

        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind: OperationKind::AgentRecord,
            parents: vec![head.change_id.clone()],
            before_root: Some(head.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: branch.ref_name.clone(),
            actor,
            session_id: branch.session_id.clone(),
            message: message.as_deref().map(redact_sensitive_text),
            changes: diff.changes,
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&head, &change_id, &built.root_id, &operation_id)?;
        let message_id = if let Some(message) = message {
            Some(self.store_message(
                "agent",
                &message,
                Some(&branch.agent_id),
                branch.session_id.as_deref(),
                Some(&change_id),
                operation.created_at,
            )?)
        } else {
            None
        };
        self.conn.execute(
            "UPDATE agent_branches SET head_change = ?1, head_root = ?2, updated_at = ?3 WHERE agent_id = ?4",
            params![change_id.0, built.root_id.0, now_ts(), branch.agent_id],
        )?;
        self.insert_agent_event_with_context(
            &branch.agent_id,
            branch.session_id.as_deref(),
            Some(&turn_id),
            "workdir_recorded",
            Some(&change_id),
            message_id.as_ref(),
            &serde_json::json!({
                "workdir": workdir,
                "root_id": built.root_id.0.clone(),
                "session_id": branch.session_id.clone(),
                "changed_paths": diff.summaries.iter().map(|item| item.path.clone()).collect::<Vec<_>>()
            }),
        )?;
        self.finish_agent_turn(&turn_id, "completed", Some(&change_id))?;
        Ok(AgentRecordReport {
            agent_id: branch.agent_id,
            operation: Some(change_id),
            root_id: built.root_id,
            changed_paths: diff.summaries,
        })
    }

    pub fn agent_workdir(&self, agent: &str) -> Result<AgentWorkdirReport> {
        validate_ref_segment(agent)?;
        let branch = self.agent_branch(agent)?;
        Ok(AgentWorkdirReport {
            agent_id: branch.agent_id,
            workdir: branch.workdir,
        })
    }

    pub fn sync_agent_workdir(
        &mut self,
        agent: &str,
        force: bool,
    ) -> Result<AgentWorkdirSyncReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;
        let branch = self.agent_branch(agent)?;
        let Some(workdir) = branch.workdir.clone() else {
            return Err(Error::InvalidInput(format!(
                "agent `{agent}` does not have a materialized workdir"
            )));
        };
        let workdir_path = PathBuf::from(&workdir);
        if workdir_path.exists() && !workdir_path.is_dir() {
            if force {
                fs::remove_file(&workdir_path)?;
            } else {
                return Err(Error::InvalidInput(format!(
                    "agent `{agent}` workdir path exists but is not a directory"
                )));
            }
        }
        let head = self.get_ref(&branch.ref_name)?;
        let target_files = self.load_root_files(&head.root_id)?;
        let workdir_exists = workdir_path.is_dir();
        let changed_paths = if workdir_exists {
            self.agent_workdir_changed_paths(&branch, &head)?
                .unwrap_or_default()
        } else {
            self.diff_file_maps(&BTreeMap::new(), &target_files)?
                .summaries
        };
        if workdir_exists && !changed_paths.is_empty() && !force {
            let preview = changed_paths
                .iter()
                .take(5)
                .map(|path| format!("{:?} {}", path.kind, path.path))
                .collect::<Vec<_>>()
                .join(", ");
            let suffix = if changed_paths.len() > 5 {
                format!(", ... {} more", changed_paths.len() - 5)
            } else {
                String::new()
            };
            return Err(Error::DirtyWorktreeWithMessage(format!(
                "agent `{agent}` workdir has unrecorded changes; run `crabdb agent record {agent}` or pass `--force` to sync: {preview}{suffix}"
            )));
        }
        if force && workdir_path.exists() {
            fs::remove_dir_all(&workdir_path)?;
        }
        fs::create_dir_all(&workdir_path)?;
        let previous = if force || !workdir_exists {
            BTreeMap::new()
        } else {
            target_files.clone()
        };
        materialize_into(
            &self.workspace_root,
            &workdir_path,
            &previous,
            &target_files,
            |entry| self.materialize_entry_bytes(entry),
        )?;
        self.insert_agent_event(
            &branch.agent_id,
            "workdir_synced",
            Some(&head.change_id),
            None,
            &serde_json::json!({
                "workdir": workdir.clone(),
                "forced": force,
                "changed_paths": changed_paths.iter().map(|item| item.path.clone()).collect::<Vec<_>>()
            }),
        )?;
        Ok(AgentWorkdirSyncReport {
            agent_id: branch.agent_id,
            workdir,
            head_change: head.change_id,
            root_id: head.root_id,
            forced: force,
            changed_paths,
        })
    }

    pub fn watch_agent_workdir(
        &mut self,
        agent: &str,
        message: Option<String>,
        interval: Duration,
        iterations: Option<u64>,
    ) -> Result<AgentWatchReport> {
        validate_ref_segment(agent)?;
        let branch = self.agent_branch(agent)?;
        if branch.workdir.is_none() {
            return Err(Error::InvalidInput(format!(
                "agent `{agent}` does not have a materialized workdir"
            )));
        }
        let mut report = AgentWatchReport {
            agent_id: branch.agent_id,
            iterations: 0,
            recorded_operations: Vec::new(),
            changed_paths: Vec::new(),
        };
        loop {
            let record = self.record_agent_workdir(agent, message.clone())?;
            report.iterations += 1;
            if let Some(operation) = record.operation {
                report.recorded_operations.push(operation);
                report.changed_paths.extend(record.changed_paths);
            }
            if iterations.is_some_and(|limit| report.iterations >= limit) {
                break;
            }
            std::thread::sleep(interval);
        }
        Ok(report)
    }

    pub fn run_agent_test(
        &mut self,
        agent: &str,
        command: Vec<String>,
        turn_id: Option<&str>,
        timeout_secs: u64,
    ) -> Result<AgentTestReport> {
        self.run_agent_test_with_options(
            agent,
            command,
            turn_id,
            timeout_secs,
            AgentGateOptions::default(),
        )
    }

    pub fn run_agent_test_with_options(
        &mut self,
        agent: &str,
        command: Vec<String>,
        turn_id: Option<&str>,
        timeout_secs: u64,
        options: AgentGateOptions,
    ) -> Result<AgentTestReport> {
        self.run_agent_gate("test", agent, command, turn_id, timeout_secs, options)
    }

    pub fn run_agent_eval(
        &mut self,
        agent: &str,
        command: Vec<String>,
        turn_id: Option<&str>,
        timeout_secs: u64,
    ) -> Result<AgentTestReport> {
        self.run_agent_eval_with_options(
            agent,
            command,
            turn_id,
            timeout_secs,
            AgentGateOptions::default(),
        )
    }

    pub fn run_agent_eval_with_options(
        &mut self,
        agent: &str,
        command: Vec<String>,
        turn_id: Option<&str>,
        timeout_secs: u64,
        options: AgentGateOptions,
    ) -> Result<AgentTestReport> {
        self.run_agent_gate("eval", agent, command, turn_id, timeout_secs, options)
    }

    pub(crate) fn run_agent_gate(
        &mut self,
        kind: &str,
        agent: &str,
        command: Vec<String>,
        turn_id: Option<&str>,
        timeout_secs: u64,
        options: AgentGateOptions,
    ) -> Result<AgentTestReport> {
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
                        "agent gate kind must be test or eval, got `{other}`"
                    )));
                }
            };
        validate_ref_segment(agent)?;
        if command.is_empty() {
            return Err(Error::InvalidInput(format!(
                "agent {kind} requires a command after `--`"
            )));
        }
        if timeout_secs == 0 {
            return Err(Error::InvalidInput(format!(
                "agent {kind} timeout must be greater than zero"
            )));
        }
        let options = normalize_agent_gate_options(kind, options)?;
        let suite = options.suite.clone();
        let score = options.score;
        let threshold = options.threshold;

        let (agent_id, session_id, workdir, turn_id, head_change, started_event_id) = {
            let _lock = self.acquire_write_lock()?;
            let branch = self.agent_branch(agent)?;
            let Some(workdir) = branch.workdir.clone() else {
                return Err(Error::InvalidInput(format!(
                    "agent `{agent}` does not have a materialized workdir"
                )));
            };
            let workdir_path = PathBuf::from(&workdir);
            if !workdir_path.is_dir() {
                return Err(Error::WorkspaceNotFound(workdir_path));
            }
            let head = self.get_ref(&branch.ref_name)?;
            let (turn_id, session_id) = if let Some(turn_id) = turn_id {
                let turn = self.agent_turn(turn_id)?;
                if turn.agent_id != branch.agent_id {
                    return Err(Error::InvalidInput(format!(
                        "turn `{turn_id}` does not belong to agent `{agent}`"
                    )));
                }
                if turn.ended_at.is_some() {
                    return Err(Error::InvalidInput(format!(
                        "turn `{turn_id}` is already ended"
                    )));
                }
                (turn.turn_id, turn.session_id)
            } else {
                let turn_id = self.open_agent_turn(
                    &branch.agent_id,
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
            let started_event_id = self.insert_agent_event_with_context(
                &branch.agent_id,
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
                branch.agent_id,
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
            let finished_event_id = self.insert_agent_event_with_context(
                &agent_id,
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
            self.finish_agent_turn(&turn_id, &status, Some(&head_change))?;
            (stdout_object, stderr_object, finished_event_id)
        };

        Ok(AgentTestReport {
            agent_id,
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

    pub fn agent_timeline(&self, agent: &str, limit: usize) -> Result<Vec<TimelineEntry>> {
        let branch = self.agent_branch(agent)?;
        let mut stmt = self.conn.prepare(
            "SELECT change_id, kind, branch, actor_id, message, created_at, path_count \
             FROM operations WHERE branch = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![branch.ref_name, limit as i64], timeline_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub fn checkout_agent(&mut self, agent: &str, force: bool) -> Result<CheckoutReport> {
        self.checkout_agent_with_options(agent, force, false, None)
    }

    pub fn checkout_agent_with_options(
        &mut self,
        agent: &str,
        force: bool,
        dry_run: bool,
        workdir: Option<&Path>,
    ) -> Result<CheckoutReport> {
        let ref_name = self.agent_branch(agent)?.ref_name;
        self.checkout_with_options(&ref_name, force, dry_run, workdir, false)
    }

    pub fn remove_agent(&mut self, agent: &str, force: bool) -> Result<AgentRemoveReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;
        let branch = self.agent_branch(agent)?;
        if branch.status != "merged" && branch.head_change != branch.base_change && !force {
            return Err(Error::InvalidInput(format!(
                "agent `{agent}` has unmerged changes; pass --force to remove"
            )));
        }
        remove_ref_file(&self.db_dir, &branch.ref_name)?;
        self.conn
            .execute("DELETE FROM refs WHERE name = ?1", params![branch.ref_name])?;
        if let Some(workdir) = &branch.workdir {
            let path = PathBuf::from(workdir);
            if path.exists() {
                fs::remove_dir_all(&path)?;
            }
        }
        self.conn.execute(
            "UPDATE agent_branches SET status = 'removed', updated_at = ?1 WHERE agent_id = ?2",
            params![now_ts(), branch.agent_id],
        )?;
        self.insert_agent_event(
            &branch.agent_id,
            "agent_removed",
            Some(&branch.head_change),
            None,
            &serde_json::json!({
                "ref_name": branch.ref_name.clone(),
                "forced": force
            }),
        )?;
        Ok(AgentRemoveReport {
            agent_id: branch.agent_id,
            ref_name: branch.ref_name,
            removed_workdir: branch.workdir,
            forced: force,
        })
    }

    pub fn acquire_lease(
        &mut self,
        agent: &str,
        path: Option<&str>,
        mode: &str,
        ttl_secs: u64,
    ) -> Result<LeaseAcquireReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;
        let mode = parse_lease_mode(mode)?;
        if ttl_secs == 0 {
            return Err(Error::InvalidInput(
                "lease ttl must be greater than zero".to_string(),
            ));
        }
        let branch = self.agent_branch(agent)?;
        let path = path.map(normalize_relative_path).transpose()?;
        let file_id = if let Some(path) = &path {
            let ref_record = self.get_ref(&branch.ref_name)?;
            let files = self.load_root_files(&ref_record.root_id)?;
            files.get(path).map(|entry| file_id_key(&entry.file_id))
        } else {
            None
        };
        let now = now_ts();
        if let Some(existing) =
            self.existing_active_lease(&branch.agent_id, path.as_deref(), mode)?
        {
            return Ok(LeaseAcquireReport { lease: existing });
        }
        let conflicts = self.conflicting_active_leases(&branch.agent_id, path.as_deref(), mode)?;
        if !conflicts.is_empty() {
            let holders = conflicts
                .iter()
                .map(|lease| format!("{} {}", lease.agent_id, lease.lease_id))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(Error::Conflict(format!(
                "active lease conflict on {} held by {holders}",
                path.as_deref().unwrap_or("<workspace>")
            )));
        }

        let expires_at = now + ttl_secs as i64;
        let seed = format!(
            "{}:{}:{}:{}:{}:{}",
            branch.agent_id,
            branch.ref_name,
            path.as_deref().unwrap_or("workspace"),
            mode,
            expires_at,
            now_nanos()
        );
        let lease_id = format!("lease_{}", crate::ids::short_hash(seed.as_bytes(), 16));
        self.conn.execute(
            "INSERT INTO leases \
             (lease_id, agent_id, ref_name, path, file_id, mode, expires_at, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                lease_id,
                branch.agent_id,
                branch.ref_name,
                path,
                file_id,
                mode,
                expires_at,
                now
            ],
        )?;
        let lease = self.lease(&lease_id)?;
        self.insert_agent_event(
            &branch.agent_id,
            "lease_acquired",
            Some(&branch.head_change),
            None,
            &serde_json::json!({
                "lease_id": lease.lease_id,
                "path": lease.path,
                "mode": lease.mode,
                "expires_at": lease.expires_at
            }),
        )?;
        Ok(LeaseAcquireReport { lease })
    }

    pub fn claim_agent_path(
        &mut self,
        agent: &str,
        path: &str,
        ttl_secs: u64,
    ) -> Result<AgentClaimReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;
        if ttl_secs == 0 {
            return Err(Error::InvalidInput(
                "agent claim ttl must be greater than zero".to_string(),
            ));
        }
        let branch = self.agent_branch(agent)?;
        let path = normalize_relative_path(path)?;
        let mode = "write";
        if let Some(existing) = self.existing_active_lease(&branch.agent_id, Some(&path), mode)? {
            return Ok(AgentClaimReport {
                agent_id: branch.agent_id,
                ref_name: branch.ref_name,
                path,
                mode: mode.to_string(),
                ttl_secs,
                claimed: true,
                lease: Some(existing),
                conflicts: Vec::new(),
                warning: None,
            });
        }

        let conflicts = self.conflicting_active_leases(&branch.agent_id, Some(&path), mode)?;
        if !conflicts.is_empty() {
            let holders = conflicts
                .iter()
                .map(|lease| format!("{} {}", lease.agent_id, lease.lease_id))
                .collect::<Vec<_>>()
                .join(", ");
            let warning = format!("`{path}` is already claimed by {holders}");
            self.insert_agent_event(
                &branch.agent_id,
                "claim_conflicted",
                Some(&branch.head_change),
                None,
                &serde_json::json!({
                    "path": &path,
                    "mode": mode,
                    "conflicts": &conflicts,
                    "warning": &warning
                }),
            )?;
            return Ok(AgentClaimReport {
                agent_id: branch.agent_id,
                ref_name: branch.ref_name,
                path,
                mode: mode.to_string(),
                ttl_secs,
                claimed: false,
                lease: None,
                conflicts,
                warning: Some(warning),
            });
        }

        let file_id = {
            let ref_record = self.get_ref(&branch.ref_name)?;
            let files = self.load_root_files(&ref_record.root_id)?;
            files.get(&path).map(|entry| file_id_key(&entry.file_id))
        };
        let now = now_ts();
        let expires_at = now + ttl_secs as i64;
        let seed = format!(
            "{}:{}:{}:{}:{}:{}",
            branch.agent_id,
            branch.ref_name,
            path,
            mode,
            expires_at,
            now_nanos()
        );
        let lease_id = format!("lease_{}", crate::ids::short_hash(seed.as_bytes(), 16));
        self.conn.execute(
            "INSERT INTO leases \
             (lease_id, agent_id, ref_name, path, file_id, mode, expires_at, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                lease_id,
                branch.agent_id,
                branch.ref_name,
                path,
                file_id,
                mode,
                expires_at,
                now
            ],
        )?;
        let lease = self.lease(&lease_id)?;
        self.insert_agent_event(
            &lease.agent_id,
            "agent_claimed_path",
            Some(&branch.head_change),
            None,
            &serde_json::json!({
                "lease_id": &lease.lease_id,
                "path": &lease.path,
                "mode": &lease.mode,
                "expires_at": lease.expires_at
            }),
        )?;
        Ok(AgentClaimReport {
            agent_id: lease.agent_id.clone(),
            ref_name: lease.ref_name.clone(),
            path: lease.path.clone().unwrap_or_else(|| path.to_string()),
            mode: lease.mode.clone(),
            ttl_secs,
            claimed: true,
            lease: Some(lease),
            conflicts: Vec::new(),
            warning: None,
        })
    }

    pub fn list_leases(&self, include_expired: bool) -> Result<Vec<LeaseRecord>> {
        if include_expired {
            let mut stmt = self.conn.prepare(
                "SELECT lease_id, agent_id, ref_name, path, file_id, mode, expires_at, created_at \
                 FROM leases ORDER BY expires_at ASC, created_at ASC",
            )?;
            let rows = stmt.query_map([], lease_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Error::from)
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT lease_id, agent_id, ref_name, path, file_id, mode, expires_at, created_at \
                 FROM leases WHERE expires_at > ?1 ORDER BY expires_at ASC, created_at ASC",
            )?;
            let rows = stmt.query_map(params![now_ts()], lease_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Error::from)
        }
    }

    pub fn release_lease(&mut self, lease_id: &str) -> Result<LeaseReleaseReport> {
        let _lock = self.acquire_write_lock()?;
        let lease = self.lease(lease_id)?;
        let deleted = self
            .conn
            .execute("DELETE FROM leases WHERE lease_id = ?1", params![lease_id])?;
        if deleted > 0 {
            self.insert_agent_event(
                &lease.agent_id,
                "lease_released",
                None,
                None,
                &serde_json::json!({
                    "lease_id": lease.lease_id,
                    "path": lease.path,
                    "mode": lease.mode
                }),
            )?;
        }
        Ok(LeaseReleaseReport {
            lease_id: lease_id.to_string(),
            released: deleted > 0,
        })
    }
}
