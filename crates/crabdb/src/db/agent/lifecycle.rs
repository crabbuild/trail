use super::*;

impl CrabDb {
    pub fn spawn_agent(
        &mut self,
        name: &str,
        from: Option<&str>,
        materialize: bool,
        provider: Option<String>,
        model: Option<String>,
    ) -> Result<AgentSpawnReport> {
        self.spawn_agent_with_workdir(name, from, materialize, provider, model, None)
    }

    pub fn spawn_agent_with_workdir(
        &mut self,
        name: &str,
        from: Option<&str>,
        materialize: bool,
        provider: Option<String>,
        model: Option<String>,
        workdir: Option<PathBuf>,
    ) -> Result<AgentSpawnReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(name)?;
        if workdir.is_some() && !materialize {
            return Err(Error::InvalidInput(
                "custom agent workdir requires materialization to be enabled".to_string(),
            ));
        }
        let source = match from {
            Some(refish) => self.resolve_refish(refish)?,
            None => self.resolve_branch_ref(&self.current_branch()?)?,
        };
        let agent_id = format!("agent_{}", crate::ids::short_hash(name.as_bytes(), 8));
        let ref_name = agent_ref(name);
        if self.try_get_ref(&ref_name)?.is_some() {
            return Err(Error::InvalidInput(format!(
                "agent `{name}` already exists"
            )));
        }
        let workdir_path = if materialize {
            Some(self.resolve_agent_workdir_path(name, workdir.as_deref())?)
        } else {
            None
        };
        let materialized_workdir = if let Some(dir) = &workdir_path {
            self.materialize_agent_workdir_at(&source.root_id, dir, workdir.is_some())?;
            Some(dir.to_string_lossy().to_string())
        } else {
            None
        };
        self.set_ref(
            &ref_name,
            &source.change_id,
            &source.root_id,
            &source.operation_id,
        )?;
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO agents (agent_id, name, kind, provider, model, created_at, metadata_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                agent_id,
                name,
                "coding-agent",
                provider,
                model,
                now,
                Option::<String>::None
            ],
        )?;
        self.conn.execute(
            "INSERT INTO agent_branches \
             (agent_id, ref_name, base_change, head_change, base_root, head_root, session_id, workdir, status, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'active', ?9, ?9)",
            params![
                agent_id,
                ref_name,
                source.change_id.0,
                source.change_id.0,
                source.root_id.0,
                source.root_id.0,
                Option::<String>::None,
                materialized_workdir,
                now
            ],
        )?;
        self.insert_agent_event(
            &agent_id,
            "agent_spawned",
            Some(&source.change_id),
            None,
            &serde_json::json!({
                "ref_name": ref_name.clone(),
                "base_root": source.root_id.0.clone(),
                "workdir": materialized_workdir.clone()
            }),
        )?;
        Ok(AgentSpawnReport {
            agent_id,
            ref_name,
            base_change: source.change_id,
            workdir: materialized_workdir,
        })
    }

    pub(crate) fn materialize_agent_workdir(
        &self,
        name: &str,
        root_id: &ObjectId,
        custom_workdir: Option<&Path>,
    ) -> Result<PathBuf> {
        let dir = self.resolve_agent_workdir_path(name, custom_workdir)?;
        self.materialize_agent_workdir_at(root_id, &dir, custom_workdir.is_some())?;
        Ok(dir)
    }

    pub(crate) fn materialize_agent_workdir_at(
        &self,
        root_id: &ObjectId,
        dir: &Path,
        custom_workdir: bool,
    ) -> Result<()> {
        prepare_agent_workdir(dir, custom_workdir)?;
        let empty = BTreeMap::new();
        let files = self.load_root_files(root_id)?;
        materialize_into(&self.workspace_root, dir, &empty, &files, |entry| {
            self.materialize_entry_bytes(entry)
        })
    }

    pub(crate) fn resolve_agent_workdir_path(
        &self,
        name: &str,
        custom_workdir: Option<&Path>,
    ) -> Result<PathBuf> {
        let raw = match custom_workdir {
            Some(path) if path.is_absolute() => path.to_path_buf(),
            Some(path) => self.workspace_root.join(path),
            None => self.default_agent_workdir_path(name)?,
        };
        let normalized = normalize_workdir_path(&raw)?;
        let normalized = canonicalize_existing_workdir_prefix(&normalized)?;
        self.validate_agent_workdir_path(&normalized)?;
        Ok(normalized)
    }

    pub(crate) fn default_agent_workdir_path(&self, name: &str) -> Result<PathBuf> {
        Ok(self.default_agent_worktrees_base()?.join(name))
    }

    pub(crate) fn default_agent_worktrees_base(&self) -> Result<PathBuf> {
        let rel = normalize_relative_path(&self.config.agent.worktrees_dir)?;
        normalize_workdir_path(&self.workspace_root.join(path_from_rel(&rel)))
    }

    pub(crate) fn validate_agent_workdir_path(&self, path: &Path) -> Result<()> {
        if path == self.workspace_root {
            return Err(Error::InvalidPath {
                path: path.to_string_lossy().to_string(),
                reason: "agent workdir cannot be the workspace root".to_string(),
            });
        }
        let worktrees_base = self.default_agent_worktrees_base()?;
        if path == worktrees_base {
            return Err(Error::InvalidPath {
                path: path.to_string_lossy().to_string(),
                reason: "agent workdir must include an agent-specific directory".to_string(),
            });
        }
        if path.starts_with(&self.workspace_root) && !path.starts_with(&worktrees_base) {
            return Err(Error::InvalidPath {
                path: path.to_string_lossy().to_string(),
                reason: format!(
                    "agent workdirs inside the workspace must live under `{}`",
                    worktrees_base.display()
                ),
            });
        }
        if let Ok(metadata) = fs::symlink_metadata(path) {
            if metadata.file_type().is_symlink() {
                return Err(Error::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                    reason: "agent workdir cannot be a symlink".to_string(),
                });
            }
        }
        Ok(())
    }

    pub(crate) fn resolve_checkout_workdir_path(&self, workdir: &Path) -> Result<PathBuf> {
        let raw = if workdir.is_absolute() {
            workdir.to_path_buf()
        } else {
            self.workspace_root.join(workdir)
        };
        let normalized = normalize_workdir_path(&raw)?;
        let normalized = canonicalize_existing_workdir_prefix(&normalized)?;
        let workspace = self.workspace_root.canonicalize()?;
        if normalized == workspace {
            return Err(Error::InvalidPath {
                path: normalized.to_string_lossy().to_string(),
                reason: "checkout workdir cannot be the workspace root".to_string(),
            });
        }
        if normalized.starts_with(&workspace) {
            let db_dir = self.db_dir.canonicalize()?;
            if !normalized.starts_with(&db_dir) {
                return Err(Error::InvalidPath {
                    path: normalized.to_string_lossy().to_string(),
                    reason: format!(
                        "checkout workdir inside the workspace must live under `{}`",
                        db_dir.display()
                    ),
                });
            }
        }
        Ok(normalized)
    }

    pub fn list_agents(&self) -> Result<Vec<AgentDetails>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.agent_id, a.name, a.kind, a.provider, a.model, a.created_at, a.metadata_json, \
                    b.ref_name, b.base_change, b.head_change, b.base_root, b.head_root, b.session_id, b.workdir, b.status, b.created_at, b.updated_at \
             FROM agents a JOIN agent_branches b ON a.agent_id = b.agent_id \
             ORDER BY a.created_at ASC, a.name ASC",
        )?;
        let rows = stmt.query_map([], agent_details_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn rewrite_restored_agent_workdir_paths(&mut self) -> Result<u64> {
        let rows = {
            let mut stmt = self.conn.prepare(
                "SELECT b.agent_id, a.name \
                 FROM agent_branches b JOIN agents a ON a.agent_id = b.agent_id \
                 WHERE b.workdir IS NOT NULL",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };

        let mut rewritten = 0;
        for (agent_id, name) in rows {
            let workdir = self.default_agent_workdir_path(&name)?;
            self.conn.execute(
                "UPDATE agent_branches SET workdir = ?1, updated_at = ?2 WHERE agent_id = ?3",
                params![workdir.to_string_lossy(), now_ts(), agent_id],
            )?;
            rewritten += 1;
        }
        Ok(rewritten)
    }

    pub fn agent_details(&self, agent: &str) -> Result<AgentDetails> {
        let branch = self.agent_branch(agent)?;
        let record = self.agent_record(&branch.agent_id)?;
        Ok(AgentDetails { record, branch })
    }

    pub fn resolve_agent_handle(&self, handle: &str) -> Result<String> {
        if validate_ref_segment(handle).is_ok() && self.try_get_ref(&agent_ref(handle))?.is_some() {
            return Ok(handle.to_string());
        }
        if handle.starts_with("agent_") {
            let name = self
                .conn
                .query_row(
                    "SELECT name FROM agents WHERE agent_id = ?1",
                    params![handle],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(name) = name {
                return Ok(name);
            }
        }
        Err(Error::RefNotFound(handle.to_string()))
    }

    pub fn agent_status(&self, agent: &str) -> Result<AgentStatusReport> {
        let details = self.agent_details(agent)?;
        let source = self.get_ref(&details.branch.ref_name)?;
        let base = self.ref_from_change(&details.branch.base_change)?;
        let base_files = self.load_root_files(&base.root_id)?;
        let source_files = self.load_root_files(&source.root_id)?;
        let diff = self.diff_file_maps(&base_files, &source_files)?;
        let workdir_changed_paths = self
            .agent_workdir_changed_paths(&details.branch, &source)?
            .unwrap_or_default();
        let workdir_state = details
            .branch
            .workdir
            .as_ref()
            .map(|_| worktree_state_from_changes(&workdir_changed_paths));
        let queued_merges: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM merge_queue WHERE source_ref = ?1 AND status IN ('queued', 'running')",
            params![details.branch.ref_name],
            |row| row.get(0),
        )?;
        Ok(AgentStatusReport {
            latest_test: self.latest_agent_test(&details.branch.agent_id)?,
            latest_eval: self.latest_agent_gate(&details.branch.agent_id, "eval")?,
            agent: details,
            changed_paths: diff.summaries,
            queued_merges: queued_merges as u64,
            workdir_state,
            workdir_changed_paths,
        })
    }

    pub fn agent_contribution(&self, agent: &str, limit: usize) -> Result<AgentContributionReport> {
        let limit = normalize_query_limit(limit, 1000)?;
        let status = self.agent_status(agent)?;
        let operations = self.agent_timeline(agent, limit)?;
        let sessions = self.list_agent_sessions(Some(agent))?;
        let recent_events = self.list_agent_events(Some(agent), None, None, None, limit)?;
        let approvals = self.list_agent_approvals(Some(agent), None)?;
        Ok(AgentContributionReport {
            status,
            operations,
            sessions,
            recent_events,
            approvals,
        })
    }

    pub fn agent_gate_history(
        &self,
        agent: &str,
        kind: Option<&str>,
        limit: usize,
    ) -> Result<AgentGateHistoryReport> {
        let limit = normalize_query_limit(limit, 1000)?;
        let details = self.agent_details(agent)?;
        let kind_filter = normalize_agent_gate_filter(kind)?;
        let gates = self.agent_gate_history_for_id(&details.branch.agent_id, kind_filter, limit)?;
        Ok(AgentGateHistoryReport {
            agent: details,
            kind: kind_filter.unwrap_or("all").to_string(),
            limit,
            gates,
        })
    }

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
