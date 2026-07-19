use super::*;

#[cfg(debug_assertions)]
std::thread_local! {
    static FAIL_PATCH_POST_COMMIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn set_patch_post_commit_failure_for_current_thread(enabled: bool) {
    FAIL_PATCH_POST_COMMIT.with(|fail| fail.set(enabled));
}

#[cfg(debug_assertions)]
fn fail_patch_post_commit_if_requested() -> Result<()> {
    if FAIL_PATCH_POST_COMMIT.with(std::cell::Cell::get) {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "injected lane patch post-commit failure",
        )));
    }
    Ok(())
}

#[cfg(not(debug_assertions))]
fn fail_patch_post_commit_if_requested() -> Result<()> {
    Ok(())
}

fn lane_patch_committed_repair(operation: &ObjectId, error: Error) -> Error {
    match error {
        Error::CommittedRepairRequired { .. } | Error::OperationCommittedRepairRequired { .. } => {
            error
        }
        error => Error::OperationCommittedRepairRequired {
            operation: operation.0.clone(),
            repair: "lane patch post-commit metadata and marker".into(),
            reason: error.to_string(),
        },
    }
}

fn retry_patch_after_full_reconciliation(error: &Error) -> bool {
    matches!(
        error,
        Error::ChangeLedgerReconcileRequired { reason, .. }
            if reason == "controlled interval sidecar chain requires reconciliation"
    )
}

impl Trail {
    pub fn apply_lane_patch(
        &mut self,
        lane: &str,
        patch: PatchDocument,
    ) -> Result<LanePatchReport> {
        let metrics = self.operation_metrics.clone();
        let result_metrics = metrics.clone();
        profile_operation_metrics(
            metrics.as_ref(),
            OperationMetricsKind::StructuredPatch,
            || {
                self.reset_case_fold_index_metrics();
                if crate::db::change_ledger::command_authority_enabled() {
                    let branch = self.lane_branch(lane)?;
                    if self.lane_uses_native_materialized_ledger(&branch)? {
                        crate::db::change_ledger::materialized_lane_daemon_expected_scope(
                            self, lane,
                        )?;
                    }
                }
                let _lock = if crate::db::change_ledger::command_authority_enabled() {
                    None
                } else {
                    Some(self.acquire_write_lock()?)
                };
                // A native observer may have durably advanced its sidecar while
                // SQLite publication was momentarily unavailable. The
                // controlled projection is target-based (not an incremental
                // edit of workdir bytes), so after a full reconciliation it is
                // safe to retry the exact patch once. Keeping this here makes
                // CLI, HTTP, and MCP callers share the same recovery behavior.
                let result = self.apply_lane_patch_with_reconciliation_retry(lane, patch, None);
                if let (Some(metrics), Ok(report)) = (&result_metrics, &result) {
                    metrics.add(OperationMetricsDelta {
                        final_path_count: saturating_u64_from_usize(report.changed_paths.len()),
                        ..OperationMetricsDelta::default()
                    });
                }
                result
            },
        )
    }

    pub(crate) fn apply_lane_patch_with_reconciliation_retry(
        &mut self,
        lane: &str,
        patch: PatchDocument,
        api_turn: Option<&LaneTurn>,
    ) -> Result<LanePatchReport> {
        let retry_patch = patch.clone();
        match self.apply_lane_patch_locked(lane, patch, api_turn) {
            Err(error) if retry_patch_after_full_reconciliation(&error) => {
                crate::db::change_ledger::materialized_lane_daemon_full_reconcile(self, lane)?;
                self.apply_lane_patch_locked(lane, retry_patch, api_turn)
            }
            result => result,
        }
    }

    pub(crate) fn apply_lane_patch_locked(
        &mut self,
        lane: &str,
        patch: PatchDocument,
        api_turn: Option<&LaneTurn>,
    ) -> Result<LanePatchReport> {
        // TRAIL_FS_PRODUCER: structured_patch_projection StructuredPatchProjection controlled
        self.reset_case_fold_index_metrics();
        validate_ref_segment(lane)?;
        let lane_row = self.lane_branch(lane)?;
        let ref_name = lane_row.ref_name.clone();
        let head = self.get_ref(&ref_name)?;
        let controlled_projection = crate::db::change_ledger::command_authority_enabled()
            && self.lane_uses_native_materialized_ledger(&lane_row)?;
        if let Some(turn) = api_turn {
            if turn.lane_id != lane_row.lane_id {
                return Err(Error::InvalidInput(format!(
                    "turn `{}` belongs to another lane",
                    turn.turn_id
                )));
            }
            if turn.before_change != head.change_id {
                return Err(Error::StaleBranch(ref_name));
            }
        }
        if !patch.allow_stale {
            match &patch.base_change {
                Some(base_change) if base_change == &head.change_id.0 => {}
                Some(base_change) => {
                    return Err(Error::PatchRejected(format!(
                        "patch base {base_change} does not match lane head {}; set allow_stale=true to apply without a fresh base",
                        head.change_id.0
                    )));
                }
                None if api_turn.is_some() => {}
                None => {
                    return Err(Error::PatchRejected(format!(
                        "patch requires base_change matching lane head {}; set allow_stale=true to apply without a fresh base",
                        head.change_id.0
                    )));
                }
            }
        }
        let empty_patch = patch.edits.is_empty();
        for edit in &patch.edits {
            self.ensure_patch_edit_allowed(edit, patch.allow_ignored)?;
        }

        let patch_session_id = if let Some(turn) = api_turn {
            if patch.session_id.is_some() && patch.session_id != turn.session_id {
                return Err(Error::InvalidInput(format!(
                    "patch session does not match turn `{}`",
                    turn.turn_id
                )));
            }
            turn.session_id.clone()
        } else {
            patch.session_id.clone().or(lane_row.session_id.clone())
        };
        if let Some(session_id) = &patch_session_id {
            self.preflight_lane_session_owner(&lane_row.lane_id, session_id)?;
        }

        let touched_paths = self.patch_touched_paths(&patch.edits)?;
        self.ensure_lane_patch_policy(&lane_row, &patch, &touched_paths)?;
        let previous_touched = self.load_root_files_for_paths(&head.root_id, &touched_paths)?;
        self.preflight_replace_line_batch(&patch.edits, &previous_touched)?;
        let case_fold_tree = self.ensure_patch_final_root_paths_safe(
            &head.root_id,
            &previous_touched,
            &patch.edits,
        )?;
        let actor = Actor::lane(lane);
        let change_id = self.allocate_change_id(&actor.id, "lane_patch")?;
        let mut target_touched = previous_touched.clone();
        let mut manual_line_changes = Vec::new();
        let mut file_seq = 1;
        let mut line_seq = 1;

        for edit in patch.edits {
            self.apply_patch_edit_to_files(
                edit,
                &mut target_touched,
                &change_id,
                &mut file_seq,
                &mut line_seq,
                &mut manual_line_changes,
            )?;
        }

        let built = self.build_root_from_touched_file_entries_incremental_with_case_fold_tree(
            &head.root_id,
            &previous_touched,
            &target_touched,
            &change_id,
            case_fold_tree,
        )?;
        let mut diff = self.diff_file_maps(&previous_touched, &target_touched)?;
        for (path, file_id, line) in manual_line_changes {
            if let Some(change) = diff
                .changes
                .iter_mut()
                .find(|change| change.path == path && change.file_id.as_ref() == Some(&file_id))
                && !change
                    .line_changes
                    .iter()
                    .any(|existing| existing.line_id == line.line_id)
            {
                change.line_changes.push(line);
            }
        }
        if diff.changes.is_empty() && !empty_patch {
            return Err(Error::PatchRejected(
                "patch produced no changes".to_string(),
            ));
        }
        let changed_summaries = diff.summaries.clone();

        let patch_message = patch.message.as_deref().map(redact_sensitive_text);
        if let Some(session_id) = &patch_session_id {
            self.ensure_lane_session(&lane_row.lane_id, session_id, None)?;
        }
        let turn_id = if let Some(turn) = api_turn {
            turn.turn_id.clone()
        } else {
            self.open_lane_turn(
                &lane_row.lane_id,
                patch_session_id.as_deref(),
                &lane_row.base_change,
                &head.change_id,
                Some(&serde_json::json!({
                    "kind": "patch",
                    "path_count": diff.summaries.len()
                })),
            )?
        };
        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind: OperationKind::LanePatch,
            parents: vec![head.change_id.clone()],
            before_root: Some(head.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: ref_name.clone(),
            actor,
            session_id: patch_session_id.clone(),
            message: patch_message.clone(),
            changes: diff.changes,
            created_at: now_ts(),
        };
        // All reads that may recover/reconcile public authority, plus patch
        // policy and cleanliness preflight, must finish before Prepared. The
        // controlled apply closure below contains only the bounded durable
        // filesystem projection.
        let controlled_expected = if controlled_projection {
            Some(
                crate::db::change_ledger::prepare_materialized_lane_controlled_projection(
                    self,
                    &lane_row.lane_id,
                )?,
            )
        } else {
            None
        };
        let operation_id = if let Some(expected) = controlled_expected {
            let evidence = crate::db::change_ledger::IntentEvidence {
                exact_paths: touched_paths
                    .iter()
                    .map(|path| crate::db::change_ledger::LedgerPath::parse(path))
                    .collect::<Result<Vec<_>>>()?,
                complete_prefixes: Vec::new(),
            };
            crate::db::change_ledger::run_ref_advancing_projection(
                self,
                &expected,
                &head,
                &lane_row.lane_id,
                crate::db::change_ledger::IntentProducer::StructuredPatchProjection,
                &operation,
                &evidence,
                crate::db::change_ledger::RefAdvancingProjectionMode::ControlledIntent,
                |db, intent| {
                    crate::db::change_ledger::with_materialized_lane_controlled_interval(
                        db,
                        &lane_row.lane_id,
                        intent,
                        &evidence,
                        |db| {
                            db.invalidate_lane_marker_if_materialized(&lane_row)?;
                            db.apply_lane_patch_workdir_projection(
                                &lane_row,
                                &head,
                                &built.root_id,
                                &changed_summaries,
                                &previous_touched,
                                &target_touched,
                                false,
                            )
                        },
                        |db, policy, candidates| {
                            let comparison = db.compare_controlled_projection_target(
                                policy,
                                candidates,
                                &built.root_id,
                                crate::db::change_ledger::CandidateMaterialization::ManifestOnly,
                            )?;
                            if comparison.summaries.is_empty() {
                                Ok(())
                            } else {
                                Err(Error::ChangeLedgerReconcileRequired {
                                    scope: expected.scope_id.to_text(),
                                    state: "stale_baseline".into(),
                                    reason: "structured patch pinned verification did not match its target root".into(),
                                    command: format!("trail lane status {}", lane_row.lane_id),
                                })
                            }
                        },
                    )
                },
                |db, publication| {
                    crate::db::change_ledger::accept_materialized_lane_daemon_baseline(
                        db,
                        &lane_row.lane_id,
                        &expected,
                        &publication.baseline,
                    )
                },
            )?
            .operation_id
        } else {
            self.invalidate_lane_marker_if_materialized(&lane_row)?;
            self.apply_lane_patch_workdir_projection(
                &lane_row,
                &head,
                &built.root_id,
                &changed_summaries,
                &previous_touched,
                &target_touched,
                true,
            )?;
            crate::db::change_ledger::commit_lane_operation_atomic(
                self,
                &head,
                &lane_row.lane_id,
                &operation,
                None,
            )?
        };
        let post_commit = (|| -> Result<()> {
            fail_patch_post_commit_if_requested()?;
            let message_id = if let Some(message) = patch_message {
                Some(self.store_message(
                    "lane",
                    &message,
                    Some(&lane_row.lane_id),
                    patch_session_id.as_deref(),
                    Some(&change_id),
                    operation.created_at,
                )?)
            } else {
                None
            };
            self.insert_lane_event_with_context(
                &lane_row.lane_id,
                patch_session_id.as_deref(),
                Some(&turn_id),
                "patch_applied",
                Some(&change_id),
                message_id.as_ref(),
                &serde_json::json!({
                    "ref_name": ref_name.clone(),
                    "root_id": built.root_id.0.clone(),
                    "session_id": patch_session_id.clone(),
                    "allow_ignored": patch.allow_ignored,
                    "allow_stale": patch.allow_stale,
                    "changed_paths": changed_summaries.iter().map(|item| item.path.clone()).collect::<Vec<_>>()
                }),
            )?;
            if patch_session_id.is_some() {
                self.conn.execute(
                    "UPDATE lane_branches SET session_id=COALESCE(?1,session_id),updated_at=?2
                     WHERE lane_id=?3 AND head_change=?4 AND head_root=?5",
                    params![
                        patch_session_id,
                        now_ts(),
                        lane_row.lane_id,
                        change_id.0,
                        built.root_id.0,
                    ],
                )?;
            }
            if api_turn.is_some() {
                self.update_lane_turn_progress(&turn_id, "patch_applied", Some(&change_id))?;
            } else {
                self.finish_lane_turn(&turn_id, "patch_applied", Some(&change_id))?;
            }
            self.publish_lane_marker_if_materialized(lane)?;
            Ok(())
        })();
        post_commit.map_err(|error| lane_patch_committed_repair(&operation_id, error))?;
        Ok(LanePatchReport {
            lane_id: lane_row.lane_id,
            operation: change_id,
            root_id: built.root_id,
            changed_paths: changed_summaries,
            path_index: self.case_fold_index_metrics_report(),
        })
    }

    fn refresh_clean_materialized_workdir_for_lane_patch(
        &self,
        workdir_path: &Path,
        previous_root_id: &ObjectId,
        next_root_id: &ObjectId,
        previous_touched: &BTreeMap<String, FileEntry>,
        target_touched: &BTreeMap<String, FileEntry>,
    ) -> Result<bool> {
        if !self.clean_workdir_manifest_allows_touched_path_update(
            workdir_path,
            previous_root_id,
            previous_touched,
            target_touched,
        )? {
            return Ok(false);
        }

        self.materialize_files_best_effort_at(workdir_path, previous_touched, target_touched)?;
        self.update_clean_workdir_manifest_from_file_subset(
            workdir_path,
            previous_root_id,
            next_root_id,
            previous_touched,
            target_touched,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "carries the fixed lane projection publication state"
    )]
    fn apply_lane_patch_workdir_projection(
        &self,
        lane_row: &LaneBranch,
        head: &RefRecord,
        next_root_id: &ObjectId,
        changed_summaries: &[FileDiffSummary],
        previous_touched: &BTreeMap<String, FileEntry>,
        target_touched: &BTreeMap<String, FileEntry>,
        allow_legacy_manifest_shortcut: bool,
    ) -> Result<()> {
        let Some(workdir) = &lane_row.workdir else {
            return Ok(());
        };
        let workdir_path = Path::new(workdir);
        let sparse_paths = self.lane_sparse_workdir_paths(lane_row, workdir_path)?;
        let (previous_files, target_files) = if let Some(sparse_paths) = sparse_paths {
            let mut selected_paths = sparse_paths;
            for summary in changed_summaries {
                selected_paths.push(summary.path.clone());
                if let Some(old_path) = &summary.old_path {
                    selected_paths.push(old_path.clone());
                }
            }
            selected_paths.sort();
            selected_paths.dedup();
            let previous_files = self.load_root_files_for_paths(&head.root_id, &selected_paths)?;
            let target_files = self.load_root_files_for_paths(next_root_id, &selected_paths)?;
            self.write_sparse_workdir_manifest(workdir_path, target_files.keys())?;
            (previous_files, target_files)
        } else if allow_legacy_manifest_shortcut
            && self.refresh_clean_materialized_workdir_for_lane_patch(
                workdir_path,
                &head.root_id,
                next_root_id,
                previous_touched,
                target_touched,
            )?
        {
            (BTreeMap::new(), BTreeMap::new())
        } else if !allow_legacy_manifest_shortcut {
            // Controlled projection already holds exact changed-path evidence
            // and verifies the projected bytes against the immutable target
            // root before publication. Project only the touched maps: loading
            // and materializing both complete roots here would turn a k-path
            // patch (including genuine k=0) back into O(repository paths).
            self.materialize_files_at(workdir_path, previous_touched, target_touched)?;
            return Ok(());
        } else {
            (
                self.load_root_files(&head.root_id)?,
                self.load_root_files(next_root_id)?,
            )
        };
        if !previous_files.is_empty() || !target_files.is_empty() {
            if crate::db::change_ledger::command_authority_enabled() {
                self.materialize_files_at(workdir_path, &previous_files, &target_files)?;
            } else {
                self.materialize_files_best_effort_at(
                    workdir_path,
                    &previous_files,
                    &target_files,
                )?;
            }
            self.write_clean_workdir_manifest(
                workdir_path,
                next_root_id,
                &target_files,
                target_files.keys(),
            )?;
        }
        Ok(())
    }

    pub(crate) fn lane_uses_native_materialized_ledger(&self, branch: &LaneBranch) -> Result<bool> {
        let Some(workdir) = branch.workdir.as_deref() else {
            return Ok(false);
        };
        if !Path::new(workdir).is_dir() {
            return Ok(false);
        }
        let mode = self.lane_workdir_mode_for(&self.lane_record(&branch.lane_id)?, branch)?;
        Ok(!mode.is_transparent_cow())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controlled_materialized_projection_never_loads_complete_roots() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "base\n").unwrap();
        for index in 0..128 {
            fs::write(
                workspace.path().join(format!("untouched-{index:03}.txt")),
                format!("untouched {index}\n"),
            )
            .unwrap();
        }
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.spawn_lane("bounded-patch", Some("main"), true, None, None)
            .unwrap();
        let branch = db.lane_branch("bounded-patch").unwrap();
        let head = db.get_ref(&branch.ref_name).unwrap();
        let workdir = PathBuf::from(branch.workdir.as_deref().unwrap());
        let previous = db
            .load_root_files_for_paths(&head.root_id, &["README.md".to_string()])
            .unwrap();

        let metrics = Arc::new(OperationMetricsState::default());
        db.operation_metrics = Some(Arc::clone(&metrics));
        db.apply_lane_patch_workdir_projection(
            &branch,
            &head,
            &head.root_id,
            &[],
            &BTreeMap::new(),
            &BTreeMap::new(),
            false,
        )
        .unwrap();
        assert!(workdir.join("README.md").is_file());
        assert!(workdir.join("untouched-127.txt").is_file());

        db.apply_lane_patch_workdir_projection(
            &branch,
            &head,
            &head.root_id,
            &[],
            &previous,
            &BTreeMap::new(),
            false,
        )
        .unwrap();
        assert!(!workdir.join("README.md").exists());
        assert!(workdir.join("untouched-127.txt").is_file());
        let counters = metrics.snapshot();
        assert_eq!(counters.full_root_range_count, 0);
        assert_eq!(counters.full_filesystem_walk_count, 0);
    }

    #[test]
    fn empty_patch_records_a_successful_noop_operation() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "base\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let metrics = Arc::new(OperationMetricsState::default());
        db.operation_metrics = Some(Arc::clone(&metrics));
        db.spawn_lane("empty-patch", Some("main"), false, None, None)
            .unwrap();
        let before = db.get_ref("refs/lanes/empty-patch").unwrap();
        let before_files = db.load_root_files(&before.root_id).unwrap();
        let report = db
            .apply_lane_patch(
                "empty-patch",
                PatchDocument {
                    base_change: Some(before.change_id.0.clone()),
                    message: Some("empty patch checkpoint".into()),
                    session_id: None,
                    allow_ignored: false,
                    allow_stale: false,
                    edits: Vec::new(),
                },
            )
            .unwrap();
        let after = db.get_ref("refs/lanes/empty-patch").unwrap();
        let after_files = db.load_root_files(&after.root_id).unwrap();

        assert!(report.changed_paths.is_empty());
        assert_eq!(before_files, after_files);
        assert_eq!(report.root_id, after.root_id);
        assert_ne!(after.change_id, before.change_id);
        assert_eq!(report.operation, after.change_id);
        let operation_metrics = metrics.last_report();
        assert_eq!(operation_metrics.operation, "structured_patch");
        assert_eq!(operation_metrics.outcome, OperationMetricsOutcome::Success);
        assert_eq!(operation_metrics.final_path_count, 0);
    }

    #[test]
    fn post_commit_patch_failure_reports_repair_required_and_keeps_committed_head() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "base\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.spawn_lane("patch-repair", Some("main"), false, None, None)
            .unwrap();
        let before = db.get_ref("refs/lanes/patch-repair").unwrap();
        let patch = PatchDocument {
            base_change: Some(before.change_id.0.clone()),
            message: Some("committed patch".into()),
            session_id: None,
            allow_ignored: false,
            allow_stale: false,
            edits: vec![PatchEdit::Write {
                path: "README.md".into(),
                content: "patched\n".into(),
                executable: false,
            }],
        };

        set_patch_post_commit_failure_for_current_thread(true);
        let result = db.apply_lane_patch("patch-repair", patch);
        set_patch_post_commit_failure_for_current_thread(false);

        let error = result.unwrap_err();
        let after = db.get_ref("refs/lanes/patch-repair").unwrap();
        assert_ne!(after.change_id, before.change_id);
        assert_ne!(after.root_id, before.root_id);
        match error {
            Error::OperationCommittedRepairRequired {
                operation,
                repair,
                reason,
            } => {
                assert_eq!(operation, after.operation_id.0);
                assert!(repair.contains("lane patch"));
                assert!(reason.contains("injected lane patch post-commit failure"));
            }
            other => panic!("expected committed repair error, got {other:?}"),
        }
    }
}
