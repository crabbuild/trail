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
        let views = {
            let mut stmt = self.conn.prepare(
                "SELECT v.view_id,v.lane_id,a.name,v.checkpoint_seq,v.generation
                 FROM workspace_views v JOIN lanes a ON a.lane_id=v.lane_id
                 ORDER BY v.view_id",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?.max(0) as u64,
                    row.get::<_, i64>(4)?.max(0) as u64,
                ))
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

            // Cache and runtime projections are host-local. Preserve durable
            // artifact objects, snapshots, envelopes, attestations, historical
            // generations, and exact generation bindings, but retire active
            // pointers and remove every omitted projection before recovery.
            self.conn.execute_batch(
                "UPDATE workspace_layer_publications
                   SET phase='recovered', successor_generation_id=NULL,
                       layer_id=NULL,
                       error_code='backup_restore_cache_invalidated',
                       error_message='backup restore excluded workspace cache bytes',
                       updated_at=unixepoch(), finished_at=unixepoch()
                   WHERE phase IN ('prepared','snapshotted','validated','published','activated');
                 DELETE FROM environment_hot_access_sessions;
                 DELETE FROM environment_hot_sets;
                 DELETE FROM workspace_layer_pins;
                 DELETE FROM environment_secret_access_audit
                   WHERE generation_id IN (SELECT generation_id FROM environment_generations);
                 DELETE FROM environment_generation_runtime_secrets
                   WHERE generation_id IN (SELECT generation_id FROM environment_generations);
                 DELETE FROM environment_generation_runtime_resources
                   WHERE generation_id IN (SELECT generation_id FROM environment_generations);
                 DELETE FROM environment_generation_external_artifacts
                   WHERE generation_id IN (SELECT generation_id FROM environment_generations);
                 DELETE FROM environment_generation_caches
                   WHERE generation_id IN (SELECT generation_id FROM environment_generations);
                 UPDATE environment_generation_outputs SET layer_id=NULL;
                 UPDATE environment_generation_components SET layer_id=NULL;
                 DELETE FROM environment_view_generations;
                 UPDATE environment_generations
                    SET state='retired', retired_at=COALESCE(retired_at,unixepoch())
                    WHERE state='active';
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
                 DELETE FROM workspace_layer_artifact_shadows;
                 DELETE FROM artifact_materializations;
                 DELETE FROM environment_cache_namespaces;
                 DELETE FROM workspace_layers;
                 DELETE FROM workspace_git_shadows;",
            )?;

            for (view_id, lane_id, lane_name, checkpoint_seq, generation) in views {
                let mut components = Path::new(&view_id).components();
                let confined = matches!(components.next(), Some(Component::Normal(_)))
                    && components.next().is_none();
                let staged_view = self.db_dir.join("views").join(&view_id);
                let staged_source = staged_view.join("source-upper");
                let staged_meta = staged_view.join("meta");
                if !confined || !staged_source.is_dir() || !staged_meta.is_dir() {
                    self.conn.execute(
                        "DELETE FROM workspace_views WHERE view_id=?1",
                        params![view_id],
                    )?;
                    continue;
                }

                let staged_generated = staged_view.join("generated-upper");
                let staged_scratch = staged_view.join("scratch-upper");
                fs::create_dir_all(&staged_generated)?;
                fs::create_dir_all(&staged_scratch)?;
                scrub_restored_view_metadata(&staged_meta)?;
                let mut barrier = ViewMutationBarrier::exclusive(&staged_meta)?;
                barrier.record_checkpoint_cut(checkpoint_seq, generation)?;

                let staged_database_view = self.db_dir.join("views").join(&view_id);
                let mountpoint = self.default_lane_workdir_path(&lane_name)?;
                self.conn.execute(
                    "UPDATE workspace_views
                     SET mountpoint=?1,source_upper=?2,generated_upper=?3,scratch_upper=?4,
                         meta_dir=?5,journal_path=?6,status='recovered',owner_pid=NULL,
                         owner_start_token=NULL,heartbeat_at=NULL,updated_at=?7
                     WHERE view_id=?8 AND lane_id=?9",
                    params![
                        mountpoint.to_string_lossy(),
                        staged_database_view.join("source-upper").to_string_lossy(),
                        staged_database_view
                            .join("generated-upper")
                            .to_string_lossy(),
                        staged_database_view.join("scratch-upper").to_string_lossy(),
                        staged_database_view.join("meta").to_string_lossy(),
                        staged_database_view
                            .join("meta")
                            .join("mutation-journal.jsonl")
                            .to_string_lossy(),
                        now_ts(),
                        view_id,
                        lane_id,
                    ],
                )?;
            }
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

    pub(crate) fn finalize_restored_workspace_view_paths(&mut self) -> Result<()> {
        let view_ids = {
            let mut statement = self
                .conn
                .prepare("SELECT view_id FROM workspace_views ORDER BY view_id")?;
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let final_db_dir = self.workspace_root.join(".trail");
        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let update = (|| -> Result<()> {
            for view_id in view_ids {
                let final_view = final_db_dir.join("views").join(&view_id);
                self.conn.execute(
                    "UPDATE workspace_views
                     SET source_upper=?1,generated_upper=?2,scratch_upper=?3,
                         meta_dir=?4,journal_path=?5,updated_at=?6
                     WHERE view_id=?7",
                    params![
                        final_view.join("source-upper").to_string_lossy(),
                        final_view.join("generated-upper").to_string_lossy(),
                        final_view.join("scratch-upper").to_string_lossy(),
                        final_view.join("meta").to_string_lossy(),
                        final_view
                            .join("meta")
                            .join("mutation-journal.jsonl")
                            .to_string_lossy(),
                        now_ts(),
                        view_id,
                    ],
                )?;
            }
            Ok(())
        })();
        match update {
            Ok(()) => self.conn.execute_batch("COMMIT;").map_err(Error::from),
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                Err(error)
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

fn scrub_restored_view_metadata(meta_dir: &Path) -> Result<()> {
    for entry in fs::read_dir(meta_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("checkpoint-barrier.")
            || matches!(
                name.as_ref(),
                "mount.json" | "unmount-request.json" | "view.json"
            )
        {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
        }
    }
    Ok(())
}
