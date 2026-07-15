use super::*;

impl Trail {
    pub fn acquire_lease(
        &mut self,
        lane: &str,
        path: Option<&str>,
        mode: &str,
        ttl_secs: u64,
    ) -> Result<LeaseAcquireReport> {
        validate_ref_segment(lane)?;
        let mode = parse_lease_mode(mode)?;
        if ttl_secs == 0 {
            return Err(Error::InvalidInput(
                "lease ttl must be greater than zero".to_string(),
            ));
        }
        let branch = self.lane_branch(lane)?;
        if crate::db::change_ledger::command_authority_enabled()
            && self.lane_uses_native_materialized_ledger(&branch)?
        {
            crate::db::change_ledger::materialized_lane_daemon_expected_scope(
                self,
                &branch.lane_id,
            )?;
        }
        let _lock = self.acquire_write_lock()?;
        let path = path.map(normalize_relative_path).transpose()?;
        let file_id = if let Some(path) = &path {
            let ref_record = self.get_ref(&branch.ref_name)?;
            let files =
                self.load_root_files_for_paths(&ref_record.root_id, std::slice::from_ref(path))?;
            files.get(path).map(|entry| file_id_key(&entry.file_id))
        } else {
            None
        };
        let now = now_ts();
        if let Some(existing) =
            self.existing_active_lease(&branch.lane_id, path.as_deref(), mode)?
        {
            return Ok(LeaseAcquireReport { lease: existing });
        }
        let conflicts = self.conflicting_active_leases(&branch.lane_id, path.as_deref(), mode)?;
        if !conflicts.is_empty() {
            let holders = conflicts
                .iter()
                .map(|lease| format!("{} {}", lease.lane_id, lease.lease_id))
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
            branch.lane_id,
            branch.ref_name,
            path.as_deref().unwrap_or("workspace"),
            mode,
            expires_at,
            now_nanos()
        );
        let lease_id = format!("lease_{}", crate::ids::short_hash(seed.as_bytes(), 16));
        self.conn.execute(
            "INSERT INTO leases \
             (lease_id, lane_id, ref_name, path, file_id, mode, expires_at, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                lease_id,
                branch.lane_id,
                branch.ref_name,
                path,
                file_id,
                mode,
                expires_at,
                now
            ],
        )?;
        let lease = self.lease(&lease_id)?;
        self.insert_lane_event(
            &branch.lane_id,
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

    pub fn claim_lane_path(
        &mut self,
        lane: &str,
        path: &str,
        ttl_secs: u64,
    ) -> Result<LaneClaimReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(lane)?;
        if ttl_secs == 0 {
            return Err(Error::InvalidInput(
                "lane claim ttl must be greater than zero".to_string(),
            ));
        }
        let branch = self.lane_branch(lane)?;
        let path = normalize_relative_path(path)?;
        let mode = "write";
        if let Some(existing) = self.existing_active_lease(&branch.lane_id, Some(&path), mode)? {
            let (hydrated_paths, hydration_warning) =
                self.claim_sparse_hydration(lane, &branch, &path);
            return Ok(LaneClaimReport {
                lane_id: branch.lane_id,
                ref_name: branch.ref_name,
                path,
                mode: mode.to_string(),
                ttl_secs,
                claimed: true,
                lease: Some(existing),
                conflicts: Vec::new(),
                hydrated_paths,
                warning: None,
                hydration_warning,
            });
        }

        let conflicts = self.conflicting_active_leases(&branch.lane_id, Some(&path), mode)?;
        if !conflicts.is_empty() {
            let holders = conflicts
                .iter()
                .map(|lease| format!("{} {}", lease.lane_id, lease.lease_id))
                .collect::<Vec<_>>()
                .join(", ");
            let warning = format!("`{path}` is already claimed by {holders}");
            self.insert_lane_event(
                &branch.lane_id,
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
            return Ok(LaneClaimReport {
                lane_id: branch.lane_id,
                ref_name: branch.ref_name,
                path,
                mode: mode.to_string(),
                ttl_secs,
                claimed: false,
                lease: None,
                conflicts,
                hydrated_paths: Vec::new(),
                warning: Some(warning),
                hydration_warning: None,
            });
        }

        let file_id = {
            let ref_record = self.get_ref(&branch.ref_name)?;
            let files =
                self.load_root_files_for_paths(&ref_record.root_id, std::slice::from_ref(&path))?;
            files.get(&path).map(|entry| file_id_key(&entry.file_id))
        };
        let now = now_ts();
        let expires_at = now + ttl_secs as i64;
        let seed = format!(
            "{}:{}:{}:{}:{}:{}",
            branch.lane_id,
            branch.ref_name,
            path,
            mode,
            expires_at,
            now_nanos()
        );
        let lease_id = format!("lease_{}", crate::ids::short_hash(seed.as_bytes(), 16));
        self.conn.execute(
            "INSERT INTO leases \
             (lease_id, lane_id, ref_name, path, file_id, mode, expires_at, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                lease_id,
                branch.lane_id,
                branch.ref_name,
                path,
                file_id,
                mode,
                expires_at,
                now
            ],
        )?;
        let lease = self.lease(&lease_id)?;
        let (hydrated_paths, hydration_warning) = self.claim_sparse_hydration(lane, &branch, &path);
        self.insert_lane_event(
            &lease.lane_id,
            "lane_claimed_path",
            Some(&branch.head_change),
            None,
            &serde_json::json!({
                "lease_id": &lease.lease_id,
                "path": &lease.path,
                "mode": &lease.mode,
                "expires_at": lease.expires_at
            }),
        )?;
        Ok(LaneClaimReport {
            lane_id: lease.lane_id.clone(),
            ref_name: lease.ref_name.clone(),
            path: lease.path.clone().unwrap_or_else(|| path.to_string()),
            mode: lease.mode.clone(),
            ttl_secs,
            claimed: true,
            lease: Some(lease),
            conflicts: Vec::new(),
            hydrated_paths,
            warning: None,
            hydration_warning,
        })
    }

    fn claim_sparse_hydration(
        &mut self,
        lane: &str,
        branch: &LaneBranch,
        path: &str,
    ) -> (Vec<String>, Option<String>) {
        match self.hydrate_sparse_lane_workdir_paths_unlocked(
            lane,
            branch,
            &[path.to_string()],
            false,
            false,
        ) {
            Ok(paths) => (paths, None),
            Err(err) => (
                Vec::new(),
                Some(format!("claimed path was not hydrated: {err}")),
            ),
        }
    }

    pub fn list_leases(&self, include_expired: bool) -> Result<Vec<LeaseRecord>> {
        if include_expired {
            let mut stmt = self.conn.prepare(
                "SELECT lease_id, lane_id, ref_name, path, file_id, mode, expires_at, created_at \
                 FROM leases ORDER BY expires_at ASC, created_at ASC",
            )?;
            let rows = stmt.query_map([], lease_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Error::from)
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT lease_id, lane_id, ref_name, path, file_id, mode, expires_at, created_at \
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
            self.insert_lane_event(
                &lease.lane_id,
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
