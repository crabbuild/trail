use super::*;

impl Trail {
    pub fn lane_timeline(&self, lane: &str, limit: usize) -> Result<Vec<TimelineEntry>> {
        let branch = self.lane_branch(lane)?;
        let mut stmt = self.conn.prepare(
            "SELECT change_id, kind, branch, actor_id, message, created_at, path_count \
             FROM operations WHERE branch = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![branch.ref_name, limit as i64], timeline_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub fn checkout_lane(&mut self, lane: &str, force: bool) -> Result<CheckoutReport> {
        self.checkout_lane_with_options(lane, force, false, None)
    }

    pub fn checkout_lane_with_options(
        &mut self,
        lane: &str,
        force: bool,
        dry_run: bool,
        workdir: Option<&Path>,
    ) -> Result<CheckoutReport> {
        let ref_name = self.lane_branch(lane)?.ref_name;
        self.checkout_with_options(&ref_name, force, dry_run, workdir, false)
    }

    pub fn remove_lane(&mut self, lane: &str, force: bool) -> Result<LaneRemoveReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(lane)?;
        let branch = self.lane_branch(lane)?;
        let preserved_view = self.lane_workspace_view(lane)?;
        if let Some(view) = &preserved_view {
            if let (Some(pid), Some(token)) = (view.owner_pid, view.owner_start_token.as_deref()) {
                if process_matches_start_token(pid, token) {
                    return Err(Error::InvalidInput(format!(
                        "lane `{lane}` has an active workspace writer in process {pid}; unmount or stop it before removal"
                    )));
                }
            }
        }
        let preserved_space = preserved_view
            .as_ref()
            .map(|_| self.lane_workspace_space(lane))
            .transpose()?;
        if branch.status != "merged" && branch.head_change != branch.base_change && !force {
            return Err(Error::InvalidInput(format!(
                "lane `{lane}` has unmerged changes; pass --force to remove"
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
        for backend in ["overlay-cow", "nfs-cow"] {
            let state = self.db_dir.join(backend).join(lane);
            if state.exists() {
                fs::remove_dir_all(state)?;
            }
        }
        self.conn.execute(
            "UPDATE lane_branches SET status = 'removed', updated_at = ?1 WHERE lane_id = ?2",
            params![now_ts(), branch.lane_id],
        )?;
        self.insert_lane_event(
            &branch.lane_id,
            "lane_removed",
            Some(&branch.head_change),
            None,
            &serde_json::json!({
                "ref_name": branch.ref_name.clone(),
                "forced": force,
                "preserved_view_id": preserved_view.as_ref().map(|view| view.view_id.as_str()),
                "preserved_source_bytes": preserved_space.as_ref().map(|space| space.uncheckpointed_source_bytes),
                "preserved_generated_bytes": preserved_space.as_ref().map(|space| space.generated_upper_bytes),
            }),
        )?;
        Ok(LaneRemoveReport {
            lane_id: branch.lane_id,
            ref_name: branch.ref_name,
            removed_workdir: branch.workdir,
            forced: force,
        })
    }
}
