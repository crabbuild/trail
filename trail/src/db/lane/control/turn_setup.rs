use super::super::workdir::MaterializationPolicy;
use super::*;

impl Trail {
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
        // TRAIL_FS_PRODUCER: turn_lane_spawn Materialize controlled
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
        let mut materialization = None;
        let workdir = if self.default_lane_materialize_for_ref(Some(&source_selector))? {
            let dir = self.resolve_lane_workdir_path(lane, None)?;
            let outcome = self.materialize_lane_root_staged(
                &source.root_id,
                &dir,
                false,
                MaterializationPolicy::Auto,
            )?;
            materialization = Some(outcome);
            Some(dir.to_string_lossy().to_string())
        } else {
            None
        };
        let metadata_json = materialization
            .as_ref()
            .map(|outcome| {
                serde_json::to_string(&serde_json::json!({
                    "requested_workdir_mode": "auto",
                    "workdir_mode": outcome.resolved_mode.as_str(),
                    "workdir_backend": outcome.backend.as_str(),
                    "materialization": outcome.report,
                    "sparse_paths": [],
                    "include_neighbors": false,
                    "transparent_cow_available": false
                }))
            })
            .transpose()?;
        let now = now_ts();
        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let association = (|| -> Result<()> {
            self.insert_new_ref_database_only(
                &ref_name,
                &source.change_id,
                &source.root_id,
                &source.operation_id,
            )?;
            super::super::lifecycle::fail_lane_association_if_requested("turn_after_ref")?;
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
                    metadata_json
                ],
            )?;
            super::super::lifecycle::fail_lane_association_if_requested("turn_after_lane")?;
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
            super::super::lifecycle::fail_lane_association_if_requested("turn_after_branch")?;
            Ok(())
        })();
        match association {
            Ok(()) => self.conn.execute_batch("COMMIT;")?,
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                if let Some(outcome) = materialization.as_ref() {
                    self.abort_materialization_operation(&outcome.materialization_operation_id)?;
                }
                return Err(error);
            }
        }
        let committed_operation = materialization
            .as_ref()
            .map(|outcome| outcome.materialization_operation_id.clone())
            .unwrap_or_else(|| source.operation_id.0.clone());
        super::super::lifecycle::committed_lane_step(
            &committed_operation,
            "turn lane ref mirror",
            (|| {
                super::super::lifecycle::fail_lane_association_if_requested("turn_ref_repair")?;
                self.repair_new_ref_mirror(
                    &ref_name,
                    &source.change_id,
                    &source.root_id,
                    &source.operation_id,
                )
            })(),
        )?;
        if let Some(outcome) = materialization.as_ref() {
            super::super::lifecycle::committed_lane_step(
                &committed_operation,
                "turn lane materialization journal completion",
                (|| {
                    super::super::lifecycle::fail_lane_association_if_requested(
                        "turn_journal_completion",
                    )?;
                    self.complete_materialization_operation(&outcome.materialization_operation_id)
                })(),
            )?;
        }
        super::super::lifecycle::committed_lane_step(
            &committed_operation,
            "turn lane post-association reconciliation",
            super::super::lifecycle::fail_lane_association_if_requested("turn_after_commit"),
        )?;
        if workdir.is_some() && crate::db::change_ledger::command_authority_enabled() {
            let expected =
                crate::db::change_ledger::prepare_materialized_lane_controlled_projection(
                    self, &lane_id,
                )
                .map_err(|error| Error::OperationCommittedRepairRequired {
                    operation: materialization
                        .as_ref()
                        .map(|outcome| outcome.materialization_operation_id.clone())
                        .unwrap_or_else(|| source.operation_id.0.clone()),
                    repair: "turn lane ledger reconciliation".into(),
                    reason: error.to_string(),
                })?;
            let evidence = crate::db::change_ledger::IntentEvidence {
                exact_paths: Vec::new(),
                complete_prefixes: Vec::new(),
            };
            crate::db::change_ledger::run_projection_alignment(
                self,
                &expected,
                crate::db::change_ledger::IntentProducer::Materialize,
                &evidence,
                crate::db::change_ledger::ProjectionAlignmentMode::Aligned,
                |db, intent| {
                    crate::db::change_ledger::with_materialized_lane_controlled_interval(
                        db,
                        &lane_id,
                        intent,
                        &evidence,
                        |_| Ok(()),
                        |db, policy, candidates| {
                            let comparison = db.compare_controlled_projection_target(
                                policy,
                                candidates,
                                &source.root_id,
                                crate::db::change_ledger::CandidateMaterialization::ManifestOnly,
                            )?;
                            if comparison.summaries.is_empty() {
                                Ok(())
                            } else {
                                Err(Error::ChangeLedgerReconcileRequired {
                                    scope: expected.scope_id.to_text(),
                                    state: "stale_baseline".into(),
                                    reason:
                                        "turn lane materialization did not match its target root"
                                            .into(),
                                    command: format!("trail lane status {lane_id}"),
                                })
                            }
                        },
                    )
                },
                |db| db.publish_lane_marker_if_materialized(&lane_id),
            )
            .map_err(|error| Error::OperationCommittedRepairRequired {
                operation: materialization
                    .as_ref()
                    .map(|outcome| outcome.materialization_operation_id.clone())
                    .unwrap_or_else(|| source.operation_id.0.clone()),
                repair: "turn lane ledger alignment".into(),
                reason: error.to_string(),
            })?;
        } else if workdir.is_some() {
            super::super::lifecycle::committed_lane_step(
                &committed_operation,
                "turn lane marker publication",
                (|| {
                    super::super::lifecycle::fail_lane_association_if_requested("turn_marker")?;
                    self.publish_lane_marker_if_materialized(lane)
                })(),
            )?;
        }
        super::super::lifecycle::committed_lane_step(
            &committed_operation,
            "turn lane event publication",
            (|| {
                super::super::lifecycle::fail_lane_association_if_requested("turn_event")?;
                self.insert_lane_event(
                    &format!("lane_{}", crate::ids::short_hash(lane.as_bytes(), 8)),
                    "lane_spawned",
                    Some(&source.change_id),
                    None,
                    &serde_json::json!({
                        "ref_name": lane_ref(lane),
                        "base_root": source.root_id.0.clone(),
                        "workdir": workdir.clone(),
                        "requested_workdir_mode": materialization.as_ref().map(|_| "auto"),
                        "workdir_mode": materialization.as_ref().map(|outcome| outcome.resolved_mode.as_str()),
                        "workdir_backend": materialization.as_ref().map(|outcome| outcome.backend.as_str()),
                        "materialization": materialization.as_ref().map(|outcome| &outcome.report),
                        "source": "api"
                    }),
                )
            })(),
        )?;
        let branch = self.lane_branch(lane);
        super::super::lifecycle::committed_lane_step(
            &committed_operation,
            "turn lane branch readback",
            branch,
        )
    }
}
