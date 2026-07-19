use super::*;

impl Trail {
    pub fn list_lanes(&self) -> Result<Vec<LaneDetails>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.lane_id, a.name, a.kind, a.provider, a.model, a.created_at, a.metadata_json, \
                    b.ref_name, b.base_change, b.head_change, b.base_root, b.head_root, b.session_id, b.workdir, b.status, b.created_at, b.updated_at \
             FROM lanes a JOIN lane_branches b ON a.lane_id = b.lane_id \
             ORDER BY a.created_at ASC, a.name ASC",
        )?;
        let rows = stmt.query_map([], lane_details_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn rewrite_restored_lane_workdir_paths(&mut self) -> Result<u64> {
        let rows = {
            let mut stmt = self.conn.prepare(
                "SELECT b.lane_id, a.name \
                 FROM lane_branches b JOIN lanes a ON a.lane_id = b.lane_id \
                 WHERE b.workdir IS NOT NULL",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };

        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let rewrite = (|| -> Result<u64> {
            let mut rewritten = 0;
            for (lane_id, name) in rows {
                let workdir = self.default_lane_workdir_path(&name)?;
                self.conn.execute(
                    "UPDATE lane_branches SET workdir = ?1, updated_at = ?2 WHERE lane_id = ?3",
                    params![workdir.to_string_lossy(), now_ts(), lane_id],
                )?;
                rewritten += 1;
            }

            // Backups do not contain `.trail/views`. Invalidate every view and
            // its derived environment state before normal open recovery can
            // inspect source-workspace absolute paths from the copied SQLite
            // store. A lane can create a fresh view in the restored workspace.
            self.conn.execute_batch(
                "DELETE FROM environment_secret_access_audit
                   WHERE generation_id IN (SELECT generation_id FROM environment_generations);
                 DELETE FROM environment_generation_runtime_secrets
                   WHERE generation_id IN (SELECT generation_id FROM environment_generations);
                 DELETE FROM environment_generation_runtime_resources
                   WHERE generation_id IN (SELECT generation_id FROM environment_generations);
                 DELETE FROM environment_generation_external_artifacts
                   WHERE generation_id IN (SELECT generation_id FROM environment_generations);
                 DELETE FROM environment_generation_caches
                   WHERE generation_id IN (SELECT generation_id FROM environment_generations);
                 DELETE FROM environment_generation_edges
                   WHERE generation_id IN (SELECT generation_id FROM environment_generations);
                 DELETE FROM environment_generation_outputs
                   WHERE generation_id IN (SELECT generation_id FROM environment_generations);
                 DELETE FROM environment_generation_components
                   WHERE generation_id IN (SELECT generation_id FROM environment_generations);
                 DELETE FROM environment_view_generations;
                 DELETE FROM environment_generations;
                 DELETE FROM environment_sync_attempts;
                 DELETE FROM environment_component_runtime_secrets;
                 DELETE FROM environment_component_runtime_resources;
                 DELETE FROM environment_component_external_artifacts;
                 DELETE FROM environment_component_caches;
                 DELETE FROM environment_component_dependencies;
                 DELETE FROM environment_component_output_bindings;
                 DELETE FROM environment_component_bindings;
                 DELETE FROM environment_component_states;
                 DELETE FROM workspace_environment_states;
                 DELETE FROM workspace_view_layers;
                 DELETE FROM workspace_git_shadows;
                 DELETE FROM workspace_views;",
            )?;
            Ok(rewritten)
        })();
        match rewrite {
            Ok(rewritten) => {
                if let Err(err) = self.conn.execute_batch("COMMIT;") {
                    let _ = self.conn.execute_batch("ROLLBACK;");
                    return Err(Error::from(err));
                }
                Ok(rewritten)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                Err(err)
            }
        }
    }

    pub fn lane_details(&self, lane: &str) -> Result<LaneDetails> {
        match self.lane_branch(lane) {
            Ok(branch) => {
                let record = self.lane_record(&branch.lane_id)?;
                Ok(LaneDetails { record, branch })
            }
            Err(Error::RefNotFound(_)) => self.unique_retired_lane_details(lane),
            Err(error) => Err(error),
        }
    }

    fn unique_retired_lane_details(&self, former_name: &str) -> Result<LaneDetails> {
        if validate_ref_segment(former_name).is_err() {
            return Err(Error::RefNotFound(former_name.to_string()));
        }
        let retired_name_prefix = format!("retired/{former_name}/");
        let mut stmt = self.conn.prepare(
            "SELECT a.lane_id,a.name,a.kind,a.provider,a.model,a.created_at,a.metadata_json, \
                    b.ref_name,b.base_change,b.head_change,b.base_root,b.head_root,b.session_id, \
                    b.workdir,b.status,b.created_at,b.updated_at \
             FROM lanes a JOIN lane_branches b ON b.lane_id=a.lane_id \
             WHERE substr(a.name,1,length(?1))=?1 \
             ORDER BY a.lane_id LIMIT 2",
        )?;
        let retired = stmt
            .query_map([retired_name_prefix], lane_details_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        match retired.as_slice() {
            [] => Err(Error::RefNotFound(lane_ref(former_name))),
            [details] => Ok(details.clone()),
            _ => Err(Error::InvalidInput(format!(
                "retired lane name `{former_name}` is ambiguous; select a specific lane ID"
            ))),
        }
    }

    pub fn resolve_lane_handle(&self, handle: &str) -> Result<String> {
        if validate_ref_segment(handle).is_ok() && self.try_get_ref(&lane_ref(handle))?.is_some() {
            return Ok(handle.to_string());
        }
        if handle.starts_with("lane_") {
            let name = self
                .conn
                .query_row(
                    "SELECT name FROM lanes WHERE lane_id = ?1",
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

    pub fn lane_status(&self, lane: &str) -> Result<LaneStatusReport> {
        let details = self.lane_details(lane)?;
        let source = self.get_ref(&details.branch.ref_name)?;
        let base = self.ref_from_change(&details.branch.base_change)?;
        let changed_paths = self.diff_root_file_summaries(&base.root_id, &source.root_id)?;
        let workdir_changed_paths = self
            .lane_workdir_changed_paths(&details.branch, &source)?
            .unwrap_or_default();
        let workdir_state = details
            .branch
            .workdir
            .as_ref()
            .map(|_| worktree_state_from_changes(&workdir_changed_paths));
        let queued_merges: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM lane_merge_queue WHERE lane_id = ?1 AND status IN ('queued', 'running')",
            params![details.branch.lane_id],
            |row| row.get(0),
        )?;
        Ok(LaneStatusReport {
            latest_test: self.latest_lane_test(&details.branch.lane_id)?,
            latest_eval: self.latest_lane_gate(&details.branch.lane_id, "eval")?,
            base_status: self.lane_base_status(&details.branch)?,
            lane: details,
            changed_paths,
            queued_merges: queued_merges as u64,
            workdir_state,
            workdir_changed_paths,
        })
    }

    pub(crate) fn lane_base_status(&self, branch: &LaneBranch) -> Result<Option<LaneBaseStatus>> {
        let target_branch = self.config.workspace.default_branch.as_str();
        let target_ref = match self.resolve_branch_ref(target_branch) {
            Ok(target_ref) => target_ref,
            Err(_) => return Ok(None),
        };
        let operations_behind =
            self.first_parent_distance(&target_ref.change_id, &branch.base_change)?;
        Ok(Some(LaneBaseStatus {
            target_branch: target_branch.to_string(),
            target_ref: target_ref.name,
            target_change: target_ref.change_id,
            lane_base_change: branch.base_change.clone(),
            stale: operations_behind.is_some_and(|behind| behind > 0),
            operations_behind,
        }))
    }

    pub fn lane_contribution(&self, lane: &str, limit: usize) -> Result<LaneContributionReport> {
        let limit = normalize_query_limit(limit, 1000)?;
        let status = self.lane_status(lane)?;
        let operations = self.lane_timeline(lane, limit)?;
        let sessions = self.list_lane_sessions(Some(lane))?;
        let recent_events = self.list_lane_events(Some(lane), None, None, None, limit)?;
        let approvals = self.list_lane_approvals(Some(lane), None)?;
        Ok(LaneContributionReport {
            status,
            operations,
            sessions,
            recent_events,
            approvals,
        })
    }

    pub fn lane_gate_history(
        &self,
        lane: &str,
        kind: Option<&str>,
        limit: usize,
    ) -> Result<LaneGateHistoryReport> {
        let limit = normalize_query_limit(limit, 1000)?;
        let details = self.lane_details(lane)?;
        let kind_filter = normalize_lane_gate_filter(kind)?;
        let gates = self.lane_gate_history_for_id(&details.branch.lane_id, kind_filter, limit)?;
        Ok(LaneGateHistoryReport {
            lane: details,
            kind: kind_filter.unwrap_or("all").to_string(),
            limit,
            gates,
        })
    }
}
