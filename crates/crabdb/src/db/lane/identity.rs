use super::*;

impl CrabDb {
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

        let mut rewritten = 0;
        for (lane_id, name) in rows {
            let workdir = self.default_lane_workdir_path(&name)?;
            self.conn.execute(
                "UPDATE lane_branches SET workdir = ?1, updated_at = ?2 WHERE lane_id = ?3",
                params![workdir.to_string_lossy(), now_ts(), lane_id],
            )?;
            rewritten += 1;
        }
        Ok(rewritten)
    }

    pub fn lane_details(&self, lane: &str) -> Result<LaneDetails> {
        let branch = self.lane_branch(lane)?;
        let record = self.lane_record(&branch.lane_id)?;
        Ok(LaneDetails { record, branch })
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
            "SELECT COUNT(*) FROM merge_queue WHERE source_ref = ?1 AND status IN ('queued', 'running')",
            params![details.branch.ref_name],
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
