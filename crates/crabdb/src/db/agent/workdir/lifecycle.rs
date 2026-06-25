use super::*;

impl CrabDb {
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
}
