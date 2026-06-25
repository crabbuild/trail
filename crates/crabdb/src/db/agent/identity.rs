use super::*;

impl CrabDb {
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
        let changed_paths = self.diff_root_file_summaries(&base.root_id, &source.root_id)?;
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
            changed_paths,
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
}
