use super::*;

type RawLaneRetirementRow = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    bool,
    Vec<u8>,
    Vec<u8>,
    Option<String>,
    Option<String>,
    Option<String>,
    i64,
    i64,
    Option<i64>,
);

impl Trail {
    pub(crate) fn recover_lane_retirement_before_spawn(&mut self, lane: &str) -> Result<()> {
        let branch = match self.lane_branch(lane) {
            Ok(branch) => branch,
            Err(Error::RefNotFound(_)) => return Ok(()),
            Err(error) => return Err(error),
        };
        let Some(retirement) = self.lane_retirement(&branch.lane_id)? else {
            return Ok(());
        };
        if retirement.phase == LaneRetirementPhase::Completed {
            return Ok(());
        }

        let recovery = if retirement.phase == LaneRetirementPhase::RepairRequired {
            self.resume_lane_retirement(&branch.lane_id).map(|_| ())
        } else {
            self.remove_lane(lane, retirement.forced).map(|_| ())
        };
        if let Err(error) = recovery {
            let retirement = self.lane_retirement(&branch.lane_id)?.unwrap_or(retirement);
            return Err(Error::OperationCommittedRepairRequired {
                operation: retirement.retirement_id,
                repair: retirement
                    .repair_command
                    .unwrap_or_else(|| format!("trail lane rm {lane} --force")),
                reason: format!(
                    "lane `{lane}` removal must complete before its name can be reused: {error}"
                ),
            });
        }
        Ok(())
    }

    pub fn resume_lane_retirement(&mut self, lane_id: &str) -> Result<LaneRetirementReport> {
        if !lane_id.starts_with("lane_") {
            return Err(Error::InvalidInput(
                "lane retirement repair requires an exact lane ID".into(),
            ));
        }
        let retirement = self
            .lane_retirement(lane_id)?
            .ok_or_else(|| Error::RefNotFound(lane_id.to_string()))?;
        if retirement.phase == LaneRetirementPhase::Completed {
            return Ok(retirement);
        }
        if retirement.phase != LaneRetirementPhase::RepairRequired {
            return Err(Error::InvalidInput(format!(
                "lane retirement `{}` is not repair-required",
                retirement.retirement_id
            )));
        }
        let resume_phase = retirement.resume_phase.ok_or_else(|| {
            Error::Corrupt(format!(
                "repair-required retirement `{}` has no resume phase",
                retirement.retirement_id
            ))
        })?;
        let resume_phase = lane_retirement_phase_name(resume_phase);
        self.conn.execute(
            "UPDATE lane_retirements
             SET phase=?1,resume_phase=NULL,last_error_code=NULL,last_error_message=NULL,
                 updated_at=?2
             WHERE retirement_id=?3 AND phase='repair_required'",
            params![resume_phase, now_ts(), &retirement.retirement_id],
        )?;
        self.remove_lane(&retirement.former_name, retirement.forced)?;
        self.lane_retirement(lane_id)?
            .ok_or_else(|| Error::Corrupt("completed lane retirement disappeared".into()))
    }

    pub(crate) fn mark_lane_retirement_repair_required(
        &self,
        lane_id: &str,
        error: &Error,
    ) -> Result<()> {
        let Some(retirement) = self.lane_retirement(lane_id)? else {
            return Ok(());
        };
        if matches!(
            retirement.phase,
            LaneRetirementPhase::Completed | LaneRetirementPhase::RepairRequired
        ) {
            return Ok(());
        }
        let resume_phase = lane_retirement_phase_name(retirement.phase);
        self.conn.execute(
            "UPDATE lane_retirements
             SET phase='repair_required',resume_phase=?1,last_error_code=?2,
                 last_error_message=?3,updated_at=?4
             WHERE retirement_id=?5 AND phase=?1",
            params![
                resume_phase,
                error.code(),
                error.to_string(),
                now_ts(),
                &retirement.retirement_id
            ],
        )?;
        Ok(())
    }

    pub fn purge_lane(&mut self, lane_id: &str, force: bool) -> Result<LaneRetirementReport> {
        if !force {
            return Err(Error::InvalidInput(
                "lane purge is irreversible; pass --force".into(),
            ));
        }
        if !lane_id.starts_with("lane_") {
            return Err(Error::InvalidInput(
                "lane purge requires an exact lane ID".into(),
            ));
        }
        let _lock = self.acquire_write_lock()?;
        let mut retirement = self
            .lane_retirement(lane_id)?
            .ok_or_else(|| Error::RefNotFound(lane_id.to_string()))?;
        if retirement.phase != LaneRetirementPhase::Completed {
            return Err(Error::InvalidInput(format!(
                "lane `{lane_id}` has not completed removal; resume `{}` first",
                retirement
                    .repair_command
                    .as_deref()
                    .unwrap_or("trail lane rm --force")
            )));
        }
        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let purged = (|| -> Result<()> {
            self.conn.execute_batch("PRAGMA defer_foreign_keys=ON;")?;
            for statement in [
                "DELETE FROM agent_hook_receipts
                 WHERE mapping_id IN (
                     SELECT mapping_id FROM lane_agent_sessions WHERE lane_id=?1)
                    OR raw_artifact_id IN (
                     SELECT artifact_id FROM lane_artifacts WHERE lane_id=?1)
                    OR installation_id IN (
                     SELECT installation_id FROM agent_hook_installations WHERE lane_id=?1)",
                "DELETE FROM lane_agent_session_aliases
                 WHERE mapping_id IN (
                     SELECT mapping_id FROM lane_agent_sessions WHERE lane_id=?1)",
                "DELETE FROM lane_session_attestation_turns
                 WHERE attestation_id IN (
                     SELECT attestation_id FROM lane_session_attestations WHERE lane_id=?1)
                    OR turn_id IN (SELECT turn_id FROM lane_turns WHERE lane_id=?1)",
                "DELETE FROM lane_provenance_edges WHERE lane_id=?1",
                "DELETE FROM lane_provenance_nodes WHERE lane_id=?1",
                "DELETE FROM git_agent_links WHERE lane_id=?1",
                "DELETE FROM lane_turn_evidence_manifests WHERE lane_id=?1",
                "DELETE FROM lane_session_attestations WHERE lane_id=?1",
                "DELETE FROM lane_learnings WHERE lane_id=?1",
                "DELETE FROM lane_artifacts WHERE lane_id=?1",
                "DELETE FROM lane_agent_sessions WHERE lane_id=?1",
                "DELETE FROM lane_acp_sessions WHERE lane_id=?1",
                "DELETE FROM lane_approvals WHERE lane_id=?1",
                "DELETE FROM lane_run_states WHERE lane_id=?1",
                "DELETE FROM lane_trace_span_events WHERE lane_id=?1",
                "DELETE FROM lane_events WHERE lane_id=?1",
                "DELETE FROM messages WHERE lane_id=?1",
                "DELETE FROM lane_turns WHERE lane_id=?1",
                "DELETE FROM lane_sessions WHERE lane_id=?1",
                "DELETE FROM agent_capture_runs WHERE lane_id=?1",
                "DELETE FROM agent_hook_installations WHERE lane_id=?1",
                "DELETE FROM external_mutation_audit WHERE lane_id=?1",
                "DELETE FROM lane_merge_queue WHERE lane_id=?1",
                "DELETE FROM leases WHERE lane_id=?1",
                "DELETE FROM lane_initialization_owners
                 WHERE initialization_id IN (
                     SELECT initialization_id FROM lane_initializations WHERE lane_id=?1)",
                "DELETE FROM lane_initializations WHERE lane_id=?1",
            ] {
                self.conn.execute(statement, [lane_id])?;
            }
            self.conn
                .execute("DELETE FROM lane_branches WHERE lane_id=?1", [lane_id])?;
            self.conn
                .execute("DELETE FROM lanes WHERE lane_id=?1", [lane_id])?;
            self.conn.execute(
                "DELETE FROM lane_retirements WHERE retirement_id=?1 AND lane_id=?2",
                params![&retirement.retirement_id, lane_id],
            )?;
            Ok(())
        })();
        match purged {
            Ok(()) => self.conn.execute_batch("COMMIT;")?,
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                return Err(error);
            }
        }
        retirement.kind = LaneRetirementKind::Purge;
        Ok(retirement)
    }

    pub(crate) fn recover_lane_retirements(&mut self) -> Result<()> {
        let pending = {
            let mut statement = self.conn.prepare(
                "SELECT former_name,forced
                 FROM lane_retirements
                 WHERE kind='remove' AND phase NOT IN ('completed','repair_required')
                 ORDER BY created_at,retirement_id",
            )?;
            statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        for (former_name, forced) in pending {
            self.remove_lane(&former_name, forced)?;
        }
        Ok(())
    }

    pub(crate) fn recover_lane_retirements_before_derived_paths(&mut self) -> Result<()> {
        let pending = {
            let mut statement = self.conn.prepare(
                "SELECT former_name,forced
                 FROM lane_retirements
                 WHERE kind='remove' AND phase IN ('bindings_retired','private_deleted')
                 ORDER BY created_at,retirement_id",
            )?;
            statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        for (former_name, forced) in pending {
            self.remove_lane(&former_name, forced)?;
        }
        Ok(())
    }

    pub(crate) fn prepare_lane_removal(
        &self,
        lane: &str,
        branch: &LaneBranch,
        view: Option<&LaneWorkspaceViewReport>,
        space: Option<&WorkspaceSpaceReport>,
        forced: bool,
    ) -> Result<String> {
        if let Some(existing) = self.lane_retirement(&branch.lane_id)? {
            if existing.former_name != lane || existing.kind != LaneRetirementKind::Remove {
                return Err(Error::Conflict(format!(
                    "lane `{lane}` already has incompatible retirement `{}`",
                    existing.retirement_id
                )));
            }
            return Ok(existing.retirement_id);
        }
        let generation_ids = if let Some(view) = view {
            let mut statement = self.conn.prepare(
                "SELECT generation_id FROM environment_generations
                 WHERE view_id=?1 ORDER BY generation_sequence,generation_id",
            )?;
            statement
                .query_map([&view.view_id], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        let provenance = LaneRetirementProvenance {
            ref_name: branch.ref_name.clone(),
            base_change: branch.base_change.0.clone(),
            head_change: branch.head_change.0.clone(),
            base_root: branch.base_root.0.clone(),
            head_root: branch.head_root.0.clone(),
            view_id: view.map(|value| value.view_id.clone()),
            environment_generation_ids: generation_ids,
            source_bytes: space.map_or(0, |value| value.uncheckpointed_source_bytes),
            generated_bytes: space.map_or(0, |value| value.generated_upper_bytes),
            scratch_bytes: space.map_or(0, |value| value.scratch_upper_bytes),
        };
        let mut private_paths = Vec::new();
        if let Some(view) = view {
            private_paths.extend([
                view.source_upper.clone(),
                view.generated_upper.clone(),
                view.scratch_upper.clone(),
            ]);
            if let Some(view_root) = Path::new(&view.meta_dir).parent() {
                private_paths.push(view_root.to_string_lossy().into_owned());
            }
        }
        if let Some(workdir) = branch.workdir.as_ref() {
            private_paths.push(workdir.clone());
        }
        for backend in ["fuse-cow", "nfs-cow", "dokan-cow"] {
            private_paths.push(
                self.db_dir
                    .join(backend)
                    .join(lane)
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        private_paths.sort();
        private_paths.dedup();
        let retirement_id = format!(
            "ret_{}",
            crate::ids::short_hash(format!("{}:{lane}:remove", branch.lane_id).as_bytes(), 24)
        );
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO lane_retirements(
                 retirement_id,lane_id,former_name,kind,phase,forced,
                 provenance_json,private_paths_json,last_error_code,last_error_message,
                 repair_command,created_at,updated_at,completed_at)
             VALUES(?1,?2,?3,'remove','prepared',?4,?5,?6,NULL,NULL,?7,?8,?8,NULL)",
            params![
                &retirement_id,
                &branch.lane_id,
                lane,
                forced,
                serde_json::to_vec(&provenance)?,
                serde_json::to_vec(&private_paths)?,
                format!("trail lane rm {lane} --force"),
                now
            ],
        )?;
        Ok(retirement_id)
    }

    pub(crate) fn lane_retirement_has_runtime_resources(&self, lane_id: &str) -> Result<bool> {
        self.conn
            .query_row(
                "SELECT EXISTS(
                     SELECT 1
                     FROM environment_generation_runtime_resources r
                     JOIN environment_generations g ON g.generation_id=r.generation_id
                     JOIN workspace_views v ON v.view_id=g.view_id
                     WHERE v.lane_id=?1 AND r.status!='stopped'
                 )",
                [lane_id],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub(crate) fn mark_lane_retirement_runtime_stopped(&self, retirement_id: &str) -> Result<()> {
        self.advance_lane_retirement_phase(
            retirement_id,
            &["prepared", "runtime_stopped"],
            "runtime_stopped",
        )
    }

    pub(crate) fn retire_lane_environment_bindings(
        &self,
        retirement_id: &str,
        view: Option<&LaneWorkspaceViewReport>,
    ) -> Result<()> {
        let Some(view) = view else {
            return self.advance_lane_retirement_phase(
                retirement_id,
                &["runtime_stopped", "bindings_retired"],
                "bindings_retired",
            );
        };
        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let retired = (|| -> Result<()> {
            let now = now_ts();
            self.conn.execute(
                "DELETE FROM environment_view_generations WHERE view_id=?1",
                [&view.view_id],
            )?;
            self.conn.execute(
                "UPDATE environment_generations
                 SET state='retired',retired_at=COALESCE(retired_at,?1)
                 WHERE view_id=?2 AND state='active'",
                params![now, &view.view_id],
            )?;
            self.conn.execute(
                "DELETE FROM workspace_view_layers WHERE view_id=?1",
                [&view.view_id],
            )?;
            self.conn.execute(
                "UPDATE workspace_views
                 SET status='retiring',owner_pid=NULL,owner_start_token=NULL,
                     heartbeat_at=NULL,updated_at=?1
                 WHERE view_id=?2",
                params![now, &view.view_id],
            )?;
            self.advance_lane_retirement_phase_in_transaction(
                retirement_id,
                &["runtime_stopped", "bindings_retired"],
                "bindings_retired",
                now,
            )
        })();
        match retired {
            Ok(()) => self.conn.execute_batch("COMMIT;")?,
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                return Err(error);
            }
        }
        Ok(())
    }

    pub(crate) fn delete_lane_retirement_private_paths(
        &self,
        retirement_id: &str,
        branch: &LaneBranch,
    ) -> Result<()> {
        let retirement = self
            .lane_retirement(&branch.lane_id)?
            .ok_or_else(|| Error::Corrupt("lane retirement disappeared".into()))?;
        if retirement.retirement_id != retirement_id {
            return Err(Error::Conflict(
                "lane retirement identity changed before private cleanup".into(),
            ));
        }
        let db_root = canonicalize_lossless(&self.db_dir)?;
        let workspace_root = canonicalize_lossless(&self.workspace_root)?;
        let explicit_workdir = branch
            .workdir
            .as_deref()
            .map(Path::new)
            .map(canonicalize_lossless)
            .transpose()?;
        let mut paths = retirement
            .private_paths
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        paths.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
        for path in paths {
            if !path.exists() {
                continue;
            }
            let confined = canonicalize_lossless(&path)?;
            let authorized = confined.starts_with(&db_root)
                || confined.starts_with(&workspace_root)
                || explicit_workdir.as_ref() == Some(&confined);
            if !authorized || confined == db_root || confined == workspace_root {
                return Err(Error::InvalidInput(format!(
                    "lane retirement path is not confined: `{}`",
                    path.display()
                )));
            }
            if confined.is_dir() {
                fs::remove_dir_all(&confined)?;
            } else {
                fs::remove_file(&confined)?;
            }
        }
        self.advance_lane_retirement_phase(
            retirement_id,
            &["bindings_retired", "private_deleted"],
            "private_deleted",
        )
    }

    pub(crate) fn compact_lane_retirement_in_transaction(
        &self,
        retirement_id: &str,
        view: Option<&LaneWorkspaceViewReport>,
        completed_at: i64,
    ) -> Result<()> {
        if let Some(view) = view {
            for statement in [
                "DELETE FROM artifact_generation_bindings
                 WHERE generation_id IN (
                     SELECT generation_id FROM environment_generations WHERE view_id=?1)",
                "DELETE FROM environment_secret_access_audit
                 WHERE generation_id IN (
                     SELECT generation_id FROM environment_generations WHERE view_id=?1)",
                "DELETE FROM environment_generation_runtime_secrets
                 WHERE generation_id IN (
                     SELECT generation_id FROM environment_generations WHERE view_id=?1)",
                "DELETE FROM environment_generation_runtime_resources
                 WHERE generation_id IN (
                     SELECT generation_id FROM environment_generations WHERE view_id=?1)",
                "DELETE FROM environment_generation_external_artifacts
                 WHERE generation_id IN (
                     SELECT generation_id FROM environment_generations WHERE view_id=?1)",
                "DELETE FROM environment_generation_caches
                 WHERE generation_id IN (
                     SELECT generation_id FROM environment_generations WHERE view_id=?1)",
                "DELETE FROM environment_generation_edges
                 WHERE generation_id IN (
                     SELECT generation_id FROM environment_generations WHERE view_id=?1)",
                "DELETE FROM environment_generation_outputs
                 WHERE generation_id IN (
                     SELECT generation_id FROM environment_generations WHERE view_id=?1)",
                "DELETE FROM environment_generation_components
                 WHERE generation_id IN (
                     SELECT generation_id FROM environment_generations WHERE view_id=?1)",
            ] {
                self.conn.execute(statement, [&view.view_id])?;
            }
            self.conn.execute(
                "DELETE FROM environment_generations WHERE view_id=?1",
                [&view.view_id],
            )?;
            for table in [
                "environment_sync_attempts",
                "environment_component_runtime_secrets",
                "environment_component_runtime_resources",
                "environment_component_external_artifacts",
                "environment_component_caches",
                "environment_component_dependencies",
                "environment_component_output_bindings",
                "environment_component_bindings",
                "environment_component_states",
                "workspace_environment_states",
                "workspace_view_layers",
                "workspace_git_shadows",
            ] {
                self.conn.execute(
                    &format!("DELETE FROM {table} WHERE view_id=?1"),
                    [&view.view_id],
                )?;
            }
            self.conn.execute(
                "DELETE FROM workspace_views WHERE view_id=?1",
                [&view.view_id],
            )?;
        }
        let changed = self.conn.execute(
            "UPDATE lane_retirements
             SET phase='completed',updated_at=?1,completed_at=?1,
                 last_error_code=NULL,last_error_message=NULL
             WHERE retirement_id=?2 AND phase IN ('private_deleted','completed')",
            params![completed_at, retirement_id],
        )?;
        if changed != 1 {
            return Err(Error::Conflict(
                "lane retirement changed before completion".into(),
            ));
        }
        Ok(())
    }

    fn advance_lane_retirement_phase(
        &self,
        retirement_id: &str,
        expected: &[&str],
        next: &str,
    ) -> Result<()> {
        self.advance_lane_retirement_phase_in_transaction(retirement_id, expected, next, now_ts())
    }

    fn advance_lane_retirement_phase_in_transaction(
        &self,
        retirement_id: &str,
        expected: &[&str],
        next: &str,
        updated_at: i64,
    ) -> Result<()> {
        let current = self.conn.query_row(
            "SELECT phase FROM lane_retirements WHERE retirement_id=?1",
            [retirement_id],
            |row| row.get::<_, String>(0),
        )?;
        if current == next {
            return Ok(());
        }
        if lane_retirement_phase_rank(&current)
            .zip(lane_retirement_phase_rank(next))
            .is_some_and(|(current, next)| current > next)
        {
            return Ok(());
        }
        if !expected.contains(&current.as_str()) {
            return Err(Error::Conflict(format!(
                "lane retirement `{retirement_id}` is `{current}`, expected one of {}",
                expected.join(", ")
            )));
        }
        let changed = self.conn.execute(
            "UPDATE lane_retirements SET phase=?1,updated_at=?2
             WHERE retirement_id=?3 AND phase=?4",
            params![next, updated_at, retirement_id, current],
        )?;
        if changed != 1 {
            return Err(Error::Conflict(format!(
                "lane retirement `{retirement_id}` changed concurrently"
            )));
        }
        Ok(())
    }

    pub fn archive_lane(&mut self, lane: &str) -> Result<LaneDetails> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(lane)?;
        let branch = self.lane_branch(lane)?;
        match branch.status.as_str() {
            "active" | "merged" => {}
            "archived" => return self.lane_details(&branch.lane_id),
            status => {
                return Err(Error::InvalidInput(format!(
                    "lane `{lane}` cannot be archived from status `{status}`"
                )));
            }
        }
        self.get_ref(&branch.ref_name)?;
        let updated_at = now_ts();
        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let archived = (|| -> Result<()> {
            let changed = self.conn.execute(
                "UPDATE lane_branches
                 SET status='archived',updated_at=?1
                 WHERE lane_id=?2 AND status=?3",
                params![updated_at, &branch.lane_id, &branch.status],
            )?;
            if changed != 1 {
                return Err(Error::WorkspaceLocked(format!(
                    "lane `{lane}` changed while it was being archived"
                )));
            }
            self.insert_lane_event(
                &branch.lane_id,
                "lane_archived",
                Some(&branch.head_change),
                None,
                &serde_json::json!({"previous_status": branch.status}),
            )?;
            Ok(())
        })();
        match archived {
            Ok(()) => self.conn.execute_batch("COMMIT;")?,
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                return Err(error);
            }
        }
        self.lane_details(&branch.lane_id)
    }

    pub fn unarchive_lane(&mut self, lane: &str) -> Result<LaneDetails> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(lane)?;
        let branch = self.lane_branch(lane)?;
        if branch.status == "active" {
            return self.lane_details(&branch.lane_id);
        }
        if branch.status != "archived" {
            return Err(Error::InvalidInput(format!(
                "lane `{lane}` cannot be unarchived from status `{}`",
                branch.status
            )));
        }
        let current = self.get_ref(&branch.ref_name)?;
        if current.change_id != branch.head_change || current.root_id != branch.head_root {
            return Err(Error::Corrupt(format!(
                "archived lane `{lane}` ref no longer matches its retained branch"
            )));
        }
        let updated_at = now_ts();
        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let restored = (|| -> Result<()> {
            let changed = self.conn.execute(
                "UPDATE lane_branches
                 SET status='active',updated_at=?1
                 WHERE lane_id=?2 AND status='archived'",
                params![updated_at, &branch.lane_id],
            )?;
            if changed != 1 {
                return Err(Error::WorkspaceLocked(format!(
                    "lane `{lane}` changed while it was being unarchived"
                )));
            }
            self.insert_lane_event(
                &branch.lane_id,
                "lane_unarchived",
                Some(&branch.head_change),
                None,
                &serde_json::json!({}),
            )?;
            Ok(())
        })();
        match restored {
            Ok(()) => self.conn.execute_batch("COMMIT;")?,
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                return Err(error);
            }
        }
        self.lane_details(&branch.lane_id)
    }

    pub fn lane_retirement(&self, handle: &str) -> Result<Option<LaneRetirementReport>> {
        let mut statement = self.conn.prepare(
            "SELECT retirement_id,lane_id,former_name,kind,phase,resume_phase,forced,
                    provenance_json,private_paths_json,last_error_code,last_error_message,
                    repair_command,created_at,updated_at,completed_at
             FROM lane_retirements
             WHERE lane_id=?1 OR former_name=?1
             ORDER BY created_at DESC,retirement_id DESC
             LIMIT 2",
        )?;
        let rows = statement
            .query_map([handle], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                    row.get(11)?,
                    row.get(12)?,
                    row.get(13)?,
                    row.get(14)?,
                ))
            })?
            .collect::<std::result::Result<Vec<RawLaneRetirementRow>, _>>()?;
        match rows.as_slice() {
            [] => Ok(None),
            [row] => decode_lane_retirement(row).map(Some),
            _ => Err(Error::InvalidInput(format!(
                "retired lane name `{handle}` is ambiguous; select a specific lane ID"
            ))),
        }
    }
}

fn lane_retirement_phase_rank(phase: &str) -> Option<u8> {
    match phase {
        "prepared" => Some(0),
        "runtime_stopped" => Some(1),
        "bindings_retired" => Some(2),
        "private_deleted" => Some(3),
        "completed" => Some(4),
        "repair_required" => None,
        _ => None,
    }
}

pub(crate) fn lane_retirement_phase_name(phase: LaneRetirementPhase) -> &'static str {
    match phase {
        LaneRetirementPhase::Prepared => "prepared",
        LaneRetirementPhase::RuntimeStopped => "runtime_stopped",
        LaneRetirementPhase::BindingsRetired => "bindings_retired",
        LaneRetirementPhase::PrivateDeleted => "private_deleted",
        LaneRetirementPhase::Completed => "completed",
        LaneRetirementPhase::RepairRequired => "repair_required",
    }
}

fn decode_lane_retirement(row: &RawLaneRetirementRow) -> Result<LaneRetirementReport> {
    let kind = match row.3.as_str() {
        "remove" => LaneRetirementKind::Remove,
        "purge" => LaneRetirementKind::Purge,
        value => {
            return Err(Error::Corrupt(format!(
                "lane retirement has invalid kind `{value}`"
            )));
        }
    };
    let phase = decode_lane_retirement_phase(&row.4)?;
    let resume_phase = row
        .5
        .as_deref()
        .map(decode_lane_retirement_phase)
        .transpose()?;
    Ok(LaneRetirementReport {
        retirement_id: row.0.clone(),
        lane_id: row.1.clone(),
        former_name: row.2.clone(),
        kind,
        phase,
        resume_phase,
        forced: row.6,
        provenance: serde_json::from_slice(&row.7)?,
        private_paths: serde_json::from_slice(&row.8)?,
        last_error_code: row.9.clone(),
        last_error_message: row.10.clone(),
        repair_command: row.11.clone(),
        created_at: row.12,
        updated_at: row.13,
        completed_at: row.14,
    })
}

fn decode_lane_retirement_phase(value: &str) -> Result<LaneRetirementPhase> {
    let phase = match value {
        "prepared" => LaneRetirementPhase::Prepared,
        "runtime_stopped" => LaneRetirementPhase::RuntimeStopped,
        "bindings_retired" => LaneRetirementPhase::BindingsRetired,
        "private_deleted" => LaneRetirementPhase::PrivateDeleted,
        "completed" => LaneRetirementPhase::Completed,
        "repair_required" => LaneRetirementPhase::RepairRequired,
        value => {
            return Err(Error::Corrupt(format!(
                "lane retirement has invalid phase `{value}`"
            )));
        }
    };
    Ok(phase)
}
