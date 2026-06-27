use super::*;

impl CrabDb {
    pub(crate) fn lane_branch_for_turn(
        &mut self,
        lane: &str,
        from: Option<&str>,
        base_change: Option<&str>,
    ) -> Result<LaneBranch> {
        match self.lane_branch(lane) {
            Ok(branch) => Ok(branch),
            Err(Error::RefNotFound(_)) => self.spawn_lane_branch_for_turn(lane, from, base_change),
            Err(err) => Err(err),
        }
    }

    fn spawn_lane_branch_for_turn(
        &mut self,
        lane: &str,
        from: Option<&str>,
        base_change: Option<&str>,
    ) -> Result<LaneBranch> {
        let source_selector = match base_change.or(from) {
            Some(selector) => selector.to_string(),
            None => self.current_branch()?,
        };
        let source = self.resolve_refish(&source_selector)?;
        let lane_id = format!("lane_{}", crate::ids::short_hash(lane.as_bytes(), 8));
        let ref_name = lane_ref(lane);
        if self.try_get_ref(&ref_name)?.is_some() {
            return Err(Error::InvalidInput(format!("lane `{lane}` already exists")));
        }
        let workdir = if self.default_lane_materialize_for_ref(Some(&source_selector))? {
            let dir = self.materialize_lane_workdir(lane, &source.root_id, None)?;
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
            "INSERT INTO lanes (lane_id, name, kind, provider, model, created_at, metadata_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                lane_id,
                lane,
                "coding-lane",
                Option::<String>::None,
                Option::<String>::None,
                now,
                Option::<String>::None
            ],
        )?;
        self.conn.execute(
            "INSERT INTO lane_branches \
             (lane_id, ref_name, base_change, head_change, base_root, head_root, session_id, workdir, status, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, 'active', ?8, ?8)",
            params![
                lane_id,
                ref_name,
                source.change_id.0,
                source.change_id.0,
                source.root_id.0,
                source.root_id.0,
                workdir.clone(),
                now
            ],
        )?;
        self.insert_lane_event(
            &format!("lane_{}", crate::ids::short_hash(lane.as_bytes(), 8)),
            "lane_spawned",
            Some(&source.change_id),
            None,
            &serde_json::json!({
                "ref_name": lane_ref(lane),
                "base_root": source.root_id.0.clone(),
                "workdir": workdir.clone(),
                "source": "api"
            }),
        )?;
        self.lane_branch(lane)
    }
}
