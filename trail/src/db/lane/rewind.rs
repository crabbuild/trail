use super::*;

#[cfg(debug_assertions)]
std::thread_local! {
    static FAIL_REWIND_POST_COMMIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn set_rewind_post_commit_failure_for_current_thread(enabled: bool) {
    FAIL_REWIND_POST_COMMIT.with(|fail| fail.set(enabled));
}

#[cfg(debug_assertions)]
fn fail_rewind_post_commit_if_requested() -> Result<()> {
    if FAIL_REWIND_POST_COMMIT.with(std::cell::Cell::get) {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "injected lane rewind post-commit failure",
        )));
    }
    Ok(())
}

#[cfg(not(debug_assertions))]
fn fail_rewind_post_commit_if_requested() -> Result<()> {
    Ok(())
}

fn lane_rewind_committed_repair(operation: &str, error: Error) -> Error {
    match error {
        Error::CommittedRepairRequired { .. } | Error::OperationCommittedRepairRequired { .. } => {
            error
        }
        error => Error::OperationCommittedRepairRequired {
            operation: operation.to_string(),
            repair: "lane rewind post-commit metadata and workdir".into(),
            reason: error.to_string(),
        },
    }
}

impl Trail {
    pub fn rewind_lane(
        &mut self,
        lane: &str,
        target: &str,
        record_current: bool,
        sync_workdir: bool,
    ) -> Result<LaneRewindReport> {
        // TRAIL_FS_PRODUCER: lane_rewind_projection RestoreProjection controlled
        validate_ref_segment(lane)?;
        if target.trim().is_empty() {
            return Err(Error::InvalidInput(
                "rewind target cannot be empty".to_string(),
            ));
        }

        let mut recorded_current = None;
        if record_current {
            let branch = self.lane_branch(lane)?;
            if branch
                .workdir
                .as_deref()
                .is_some_and(|workdir| Path::new(workdir).is_dir())
            {
                let report = self.record_lane_workdir(
                    lane,
                    Some(format!("Record current lane `{lane}` before rewind")),
                )?;
                recorded_current = report.operation;
            }
        }

        let controlled_rewind =
            if crate::db::change_ledger::command_authority_enabled() && sync_workdir {
                let branch = self.lane_branch(lane)?;
                let materialized = branch
                    .workdir
                    .as_deref()
                    .is_some_and(|workdir| Path::new(workdir).is_dir());
                let native = materialized
                    && !self
                        .lane_workdir_mode_for(&self.lane_record(&branch.lane_id)?, &branch)?
                        .is_transparent_cow();
                if native {
                    crate::db::change_ledger::materialized_lane_daemon_expected_scope(self, lane)?;
                }
                native
            } else {
                false
            };
        let mut report = {
            let _lock = if crate::db::change_ledger::command_authority_enabled() {
                None
            } else {
                Some(self.acquire_write_lock()?)
            };
            let branch = self.lane_branch(lane)?;
            let head = self.get_ref(&branch.ref_name)?;
            if sync_workdir {
                self.ensure_lane_workdir_clean(&branch, &head)?;
            }

            let target_ref = self.resolve_refish(target)?;
            if head.change_id == target_ref.change_id && head.root_id == target_ref.root_id {
                return Err(Error::InvalidInput(format!(
                    "lane `{lane}` is already at `{target}`"
                )));
            }

            let (preserved_branch, preserved_ref) = if record_current {
                let preserved_branch = self.preserve_rewind_head_branch(lane, &head)?;
                let preserved_ref = branch_ref(&preserved_branch);
                (Some(preserved_branch), Some(preserved_ref))
            } else {
                (None, None)
            };

            let current_files = self.load_root_files(&head.root_id)?;
            let target_files = self.load_root_files(&target_ref.root_id)?;
            let diff = self.diff_file_maps(&current_files, &target_files)?;
            let actor = Actor::system();
            let change_id = self.allocate_change_id(&actor.id, "lane_rewind")?;
            let message = match &preserved_branch {
                Some(branch) => format!(
                    "Rewind lane `{lane}` to `{target}`; preserved previous head as `{branch}`"
                ),
                None => format!("Rewind lane `{lane}` to `{target}`"),
            };
            let operation = Operation {
                version: OP_OBJECT_VERSION,
                change_id: change_id.clone(),
                kind: OperationKind::LaneRewind,
                parents: vec![head.change_id.clone()],
                before_root: Some(head.root_id.clone()),
                after_root: target_ref.root_id.clone(),
                branch: branch.ref_name.clone(),
                actor,
                session_id: branch.session_id.clone(),
                message: Some(message),
                changes: diff.changes,
                created_at: now_ts(),
            };
            let operation_id = if controlled_rewind {
                let expected =
                    crate::db::change_ledger::prepare_materialized_lane_controlled_projection(
                        self,
                        &branch.lane_id,
                    )?;
                let mut evidence_paths = diff
                    .summaries
                    .iter()
                    .flat_map(|summary| {
                        std::iter::once(summary.path.as_str()).chain(summary.old_path.as_deref())
                    })
                    .map(crate::db::change_ledger::LedgerPath::parse)
                    .collect::<Result<Vec<_>>>()?;
                evidence_paths.sort();
                evidence_paths.dedup();
                let evidence = crate::db::change_ledger::IntentEvidence {
                    exact_paths: evidence_paths,
                    complete_prefixes: Vec::new(),
                };
                crate::db::change_ledger::run_ref_advancing_projection(
                    self,
                    &expected,
                    &head,
                    &branch.lane_id,
                    crate::db::change_ledger::IntentProducer::RestoreProjection,
                    &operation,
                    &evidence,
                    crate::db::change_ledger::RefAdvancingProjectionMode::ControlledIntent,
                    |db, intent| {
                        crate::db::change_ledger::with_materialized_lane_controlled_interval(
                            db,
                            &branch.lane_id,
                            intent,
                            &evidence,
                            |db| {
                                db.invalidate_lane_marker_if_materialized(&branch)?;
                                db.apply_rewind_workdir_projection(
                                    &branch,
                                    &head.root_id,
                                    &target_ref.root_id,
                                )
                            },
                            |db, policy, candidates| {
                                let comparison = db.compare_controlled_projection_target(
                                    policy,
                                    candidates,
                                    &target_ref.root_id,
                                    crate::db::change_ledger::CandidateMaterialization::ManifestOnly,
                                )?;
                                if comparison.summaries.is_empty() {
                                    Ok(())
                                } else {
                                    Err(Error::ChangeLedgerReconcileRequired {
                                        scope: expected.scope_id.to_text(),
                                        state: "stale_baseline".into(),
                                        reason: "rewind pinned verification did not match its target root".into(),
                                        command: format!("trail lane status {}", branch.lane_id),
                                    })
                                }
                            },
                        )
                    },
                    |db, publication| {
                        crate::db::change_ledger::accept_materialized_lane_daemon_baseline(
                            db,
                            &branch.lane_id,
                            &expected,
                            &publication.baseline,
                        )
                    },
                )?
                .operation_id
            } else {
                self.invalidate_lane_marker_if_materialized(&branch)?;
                crate::db::change_ledger::commit_lane_operation_atomic(
                    self,
                    &head,
                    &branch.lane_id,
                    &operation,
                    None,
                )?
            };
            let post_commit = (|| -> Result<()> {
                fail_rewind_post_commit_if_requested()?;
                if crate::db::change_ledger::command_authority_enabled() && !sync_workdir {
                    self.conn.execute(
                        "UPDATE changed_path_scopes
                         SET trust_state='stale_baseline',trust_reason='rewind_without_workdir_alignment',updated_at=?1
                         WHERE scope_id=?2 AND retired_at IS NULL",
                        params![
                            now_ts(),
                            crate::db::change_ledger::materialized_lane_scope_id(
                                &self.config.workspace.id.0,
                                &branch.lane_id,
                            )
                            .to_text()
                        ],
                    )?;
                }
                self.conn.execute(
                    "UPDATE lane_branches SET status='active',updated_at=?1
                     WHERE lane_id=?2 AND head_change=?3 AND head_root=?4",
                    params![now_ts(), branch.lane_id, change_id.0, target_ref.root_id.0],
                )?;
                self.insert_lane_event_with_context(
                    &branch.lane_id,
                    branch.session_id.as_deref(),
                    None,
                    "lane_rewound",
                    Some(&change_id),
                    None,
                    &serde_json::json!({
                        "target": target,
                        "target_change": target_ref.change_id.0,
                        "target_root": target_ref.root_id.0,
                        "previous_change": head.change_id.0,
                        "previous_root": head.root_id.0,
                        "record_current": record_current,
                        "recorded_current": recorded_current.as_ref().map(|id| id.0.clone()),
                        "preserved_branch": preserved_branch.clone(),
                        "preserved_ref": preserved_ref.clone(),
                        "sync_workdir": sync_workdir,
                        "changed_paths": diff.summaries.iter().map(|item| item.path.clone()).collect::<Vec<_>>()
                    }),
                )?;
                Ok(())
            })();
            post_commit.map_err(|error| lane_rewind_committed_repair(&operation_id.0, error))?;

            LaneRewindReport {
                lane_id: branch.lane_id,
                ref_name: branch.ref_name,
                target: target.to_string(),
                previous_change: head.change_id,
                previous_root: head.root_id,
                target_change: target_ref.change_id,
                target_root: target_ref.root_id.clone(),
                operation: change_id,
                root_id: target_ref.root_id,
                changed_paths: diff.summaries,
                recorded_current,
                preserved_branch,
                preserved_ref,
                workdir: branch.workdir.clone(),
                workdir_synced: controlled_rewind && branch.workdir.is_some(),
            }
        };

        if sync_workdir && report.workdir.is_some() && !controlled_rewind {
            let sync = self
                .sync_lane_workdir(lane, true)
                .map_err(|error| lane_rewind_committed_repair(&report.operation.0, error))?;
            report.workdir = Some(sync.workdir);
            report.workdir_synced = true;
        }

        Ok(report)
    }

    fn preserve_rewind_head_branch(&self, lane: &str, head: &RefRecord) -> Result<String> {
        let short_change = short_rewind_change_id(&head.change_id);
        for suffix in 0..1000u16 {
            let branch = if suffix == 0 {
                format!("rewind/{lane}/{short_change}")
            } else {
                format!("rewind/{lane}/{short_change}-{suffix}")
            };
            validate_ref_segment(&branch)?;
            let ref_name = branch_ref(&branch);
            if self.try_get_ref(&ref_name)?.is_none() {
                self.set_ref(
                    &ref_name,
                    &head.change_id,
                    &head.root_id,
                    &head.operation_id,
                )?;
                return Ok(branch);
            }
        }
        Err(Error::InvalidInput(format!(
            "could not allocate rewind preservation branch for `{lane}`"
        )))
    }

    fn apply_rewind_workdir_projection(
        &self,
        branch: &LaneBranch,
        previous_root: &ObjectId,
        target_root: &ObjectId,
    ) -> Result<()> {
        let Some(workdir) = branch.workdir.as_deref() else {
            return Ok(());
        };
        let workdir = Path::new(workdir);
        let sparse = self.lane_sparse_workdir_paths(branch, workdir)?;
        let (previous_files, target_files) = if let Some(paths) = sparse {
            (
                self.load_root_files_for_selections(previous_root, &paths)?,
                self.load_root_files_for_selections(target_root, &paths)?,
            )
        } else {
            (
                self.load_root_files(previous_root)?,
                self.load_root_files(target_root)?,
            )
        };
        if crate::db::change_ledger::command_authority_enabled() {
            self.materialize_files_at(workdir, &previous_files, &target_files)?;
        } else {
            self.materialize_files_best_effort_at(workdir, &previous_files, &target_files)?;
        }
        if self.lane_sparse_workdir_paths(branch, workdir)?.is_some() {
            self.write_sparse_workdir_manifest(workdir, target_files.keys())?;
        }
        self.write_clean_workdir_manifest(workdir, target_root, &target_files, target_files.keys())
    }
}

fn short_rewind_change_id(change_id: &ChangeId) -> String {
    crate::ids::change_id_hash(&change_id.0)
        .unwrap_or(&change_id.0)
        .chars()
        .take(12)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_commit_rewind_failure_reports_repair_required_and_keeps_committed_head() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "base\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let spawned = db
            .spawn_lane("rewind-repair", Some("main"), false, None, None)
            .unwrap();
        let base_change = spawned.base_change;
        let base_root = db.get_ref("refs/lanes/rewind-repair").unwrap().root_id;
        let patch = PatchDocument {
            base_change: Some(base_change.0.clone()),
            message: Some("advance before rewind".into()),
            session_id: None,
            allow_ignored: false,
            allow_stale: false,
            edits: vec![PatchEdit::Write {
                path: "README.md".into(),
                content: "advanced\n".into(),
                executable: false,
            }],
        };
        db.apply_lane_patch("rewind-repair", patch).unwrap();
        let before = db.get_ref("refs/lanes/rewind-repair").unwrap();

        set_rewind_post_commit_failure_for_current_thread(true);
        let result = db.rewind_lane("rewind-repair", &base_change.0, false, false);
        set_rewind_post_commit_failure_for_current_thread(false);

        let error = result.unwrap_err();
        let after = db.get_ref("refs/lanes/rewind-repair").unwrap();
        assert_ne!(after.change_id, before.change_id);
        assert_eq!(after.root_id, base_root);
        match error {
            Error::OperationCommittedRepairRequired {
                operation,
                repair,
                reason,
            } => {
                assert_eq!(operation, after.operation_id.0);
                assert!(repair.contains("lane rewind"));
                assert!(reason.contains("injected lane rewind post-commit failure"));
            }
            other => panic!("expected committed repair error, got {other:?}"),
        }
    }
}
