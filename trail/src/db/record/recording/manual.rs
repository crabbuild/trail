use super::*;

#[cfg(debug_assertions)]
type ObservedRecordAfterCompareHook = Box<dyn FnOnce() -> Result<()> + Send>;

#[cfg(debug_assertions)]
static OBSERVED_RECORD_AFTER_COMPARE_HOOK: std::sync::OnceLock<
    std::sync::Mutex<Option<ObservedRecordAfterCompareHook>>,
> = std::sync::OnceLock::new();

#[cfg(debug_assertions)]
static OBSERVED_RECORD_WITH_LOCK_HOOK: std::sync::OnceLock<
    std::sync::Mutex<Option<ObservedRecordAfterCompareHook>>,
> = std::sync::OnceLock::new();

#[cfg(debug_assertions)]
pub(crate) fn install_observed_record_after_compare_hook(
    hook: impl FnOnce() -> Result<()> + Send + 'static,
) {
    *OBSERVED_RECORD_AFTER_COMPARE_HOOK
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = Some(Box::new(hook));
}

#[cfg(debug_assertions)]
pub(crate) fn install_observed_record_with_lock_hook(
    hook: impl FnOnce() -> Result<()> + Send + 'static,
) {
    *OBSERVED_RECORD_WITH_LOCK_HOOK
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = Some(Box::new(hook));
}

#[cfg(debug_assertions)]
fn run_observed_record_after_compare_hook() -> Result<()> {
    let hook = OBSERVED_RECORD_AFTER_COMPARE_HOOK
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take();
    if let Some(hook) = hook {
        hook()?;
    }
    Ok(())
}

#[cfg(debug_assertions)]
fn run_observed_record_with_lock_hook() -> Result<()> {
    let hook = OBSERVED_RECORD_WITH_LOCK_HOOK
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take();
    if let Some(hook) = hook {
        hook()?;
    }
    Ok(())
}

impl Trail {
    pub fn record(
        &mut self,
        branch: Option<&str>,
        message: Option<String>,
        actor: Actor,
        watch: bool,
    ) -> Result<RecordReport> {
        let metrics = self.operation_metrics.clone();
        profile_operation_metrics(metrics.as_ref(), OperationMetricsKind::Record, || {
            self.record_with_options(
                branch,
                message,
                actor,
                RecordOptions {
                    kind: Some(if watch {
                        OperationKind::WatchRecord
                    } else {
                        OperationKind::ManualRecord
                    }),
                    ..RecordOptions::default()
                },
            )
        })
    }

    pub fn record_with_options(
        &mut self,
        branch: Option<&str>,
        message: Option<String>,
        actor: Actor,
        options: RecordOptions,
    ) -> Result<RecordReport> {
        let metrics = self.operation_metrics.clone();
        profile_operation_metrics(metrics.as_ref(), OperationMetricsKind::Record, || {
            if crate::db::change_ledger::command_authority_enabled() {
                // Native fence durability uses the same workspace writer
                // exclusion.  The observed path obtains c1/c2 first, then
                // takes the lock for its single CAS transaction below.
                self.record_with_options_unlocked(branch, message, actor, options)
            } else {
                let _lock = self.acquire_write_lock()?;
                self.record_with_options_unlocked(branch, message, actor, options)
            }
        })
    }

    pub(crate) fn record_with_options_unlocked(
        &mut self,
        branch: Option<&str>,
        message: Option<String>,
        actor: Actor,
        options: RecordOptions,
    ) -> Result<RecordReport> {
        let metrics = self.operation_metrics.clone();
        let result_metrics = metrics.clone();
        profile_operation_metrics(metrics.as_ref(), OperationMetricsKind::Record, || {
            let result =
                self.record_with_options_unlocked_profiled(branch, message, actor, options);
            if let (Some(metrics), Ok(report)) = (&result_metrics, &result) {
                metrics.add(OperationMetricsDelta {
                    final_path_count: saturating_u64_from_usize(report.changed_paths.len()),
                    ..OperationMetricsDelta::default()
                });
            }
            result
        })
    }

    fn record_with_options_unlocked_profiled(
        &mut self,
        branch: Option<&str>,
        message: Option<String>,
        actor: Actor,
        options: RecordOptions,
    ) -> Result<RecordReport> {
        let branch = branch.map(str::to_string).unwrap_or(self.current_branch()?);
        let ref_name = branch_ref(&branch);
        let head = self.get_ref(&ref_name)?;
        self.note_operation_metrics(OperationMetricsDelta {
            input_path_count: saturating_u64_from_usize(options.paths.len()),
            ..OperationMetricsDelta::default()
        });
        let selected_paths = normalize_record_paths(&options.paths)?;
        let explicit_selections = (!selected_paths.is_empty())
            .then(|| SelectionSet::from_paths(&selected_paths))
            .transpose()?;
        self.note_operation_metrics(OperationMetricsDelta {
            canonical_path_count: explicit_selections
                .as_ref()
                .map(|selections| saturating_u64_from_usize(selections.as_slice().len()))
                .unwrap_or(0),
            ..OperationMetricsDelta::default()
        });
        let session_id = options
            .session_id
            .clone()
            .map(|session_id| {
                validate_session_id(&session_id)?;
                self.lane_session(&session_id)?;
                Ok::<String, Error>(session_id)
            })
            .transpose()?;
        if crate::db::change_ledger::command_authority_enabled()
            && selected_paths.is_empty()
            && branch == self.current_branch()?
        {
            self.note_operation_metrics(OperationMetricsDelta {
                selected_worktree_index_sqlite_not_applicable_count: 1,
                ..OperationMetricsDelta::default()
            });
            return self.record_from_changed_path_ledger(
                branch, head, message, actor, options, session_id,
            );
        }
        let daemon_snapshot = if selected_paths.is_empty() {
            self.daemon_worktree_snapshot()
        } else {
            None
        };
        let fallback_generation = match &daemon_snapshot {
            Some(
                DaemonWorktreeSnapshot::Clean { generation, .. }
                | DaemonWorktreeSnapshot::Dirty { generation, .. }
                | DaemonWorktreeSnapshot::Overflow { generation },
            ) => Some(*generation),
            None => None,
        };
        let daemon_dirty_paths = match daemon_snapshot {
            Some(DaemonWorktreeSnapshot::Clean {
                generation: _,
                root_id: Some(clean_root),
            }) => {
                if self.clean_baseline_matches_visible_root(Some(&clean_root), &head.root_id) {
                    return Ok(RecordReport {
                        branch,
                        operation: None,
                        root_id: head.root_id,
                        changed_paths: Vec::new(),
                    });
                }
                None
            }
            Some(DaemonWorktreeSnapshot::Dirty { generation, paths })
                if paths.len() <= self.daemon_dirty_path_limit() =>
            {
                Some((generation, paths))
            }
            _ => None,
        };
        let git_policy = if selected_paths.is_empty() && daemon_dirty_paths.is_none() {
            Some(self.workspace_ignore_policy_snapshot())
        } else {
            None
        };
        let fast_dirty_paths = match git_policy.as_ref() {
            Some(policy) => self.scan_git_dirty_tracked_paths_with_policy(policy)?,
            None => None,
        };
        let disk_files;
        let build_selected_paths;
        let build_selection_set;
        let previous_files;
        let record_generation;
        if let Some((generation, paths)) = daemon_dirty_paths {
            previous_files = self.load_root_files_for_selections(&head.root_id, &paths)?;
            let policy = self.workspace_ignore_policy_snapshot();
            let snapshot =
                self.selected_worktree_snapshot_with_policy(&previous_files, &paths, &policy)?;
            self.reconcile_daemon_status_paths(
                &head.root_id,
                &paths,
                &snapshot.summaries,
                generation,
            );
            if snapshot.paths.is_empty() {
                return Ok(RecordReport {
                    branch,
                    operation: None,
                    root_id: head.root_id,
                    changed_paths: Vec::new(),
                });
            }
            disk_files = snapshot.files;
            build_selected_paths = Some(snapshot.paths);
            build_selection_set = None;
            record_generation = Some(generation);
        } else if let Some(paths) = fast_dirty_paths {
            if paths.is_empty() {
                return Ok(RecordReport {
                    branch,
                    operation: None,
                    root_id: head.root_id,
                    changed_paths: Vec::new(),
                });
            }
            previous_files = self.load_root_files_for_paths(&head.root_id, &paths)?;
            let snapshot = self.selected_worktree_snapshot_with_policy(
                &previous_files,
                &paths,
                git_policy
                    .as_ref()
                    .expect("Git candidates have an operation policy snapshot"),
            )?;
            if snapshot.paths.is_empty() {
                return Ok(RecordReport {
                    branch,
                    operation: None,
                    root_id: head.root_id,
                    changed_paths: Vec::new(),
                });
            }
            disk_files = snapshot.files;
            build_selected_paths = Some(snapshot.paths);
            build_selection_set = None;
            record_generation = fallback_generation;
        } else if !selected_paths.is_empty() {
            let selections = explicit_selections
                .as_ref()
                .expect("nonempty explicit record paths have a selection set");
            previous_files = self.load_root_files_for_selections(&head.root_id, &selected_paths)?;
            let policy = self.workspace_ignore_policy_snapshot();
            disk_files = self.scan_record_selection_files_with_policy(
                &selected_paths,
                selections,
                options.allow_ignored,
                &policy,
            )?;
            build_selected_paths = Some(selected_paths.clone());
            build_selection_set = Some(selections.clone());
            record_generation = None;
        } else {
            let refresh = self.refresh_worktree_index_streaming_report()?;
            let baseline = self.worktree_index_baseline_root()?;
            if !refresh.changed
                && self.clean_baseline_matches_visible_root(baseline.as_ref(), &head.root_id)
            {
                return Ok(RecordReport {
                    branch,
                    operation: None,
                    root_id: head.root_id,
                    changed_paths: Vec::new(),
                });
            }
            let summaries = self.diff_root_to_worktree_index(&head.root_id)?;
            if summaries.is_empty() {
                self.set_worktree_index_baseline(&head.root_id)?;
                return Ok(RecordReport {
                    branch,
                    operation: None,
                    root_id: head.root_id,
                    changed_paths: Vec::new(),
                });
            }
            let paths = summaries
                .iter()
                .map(|summary| summary.path.clone())
                .collect::<Vec<_>>();
            disk_files = self.scan_visible_files_for_paths(&paths)?;
            previous_files = self.load_root_files_for_paths(&head.root_id, &paths)?;
            build_selected_paths = Some(paths);
            build_selection_set = None;
            record_generation = fallback_generation;
        }
        let change_id = self.allocate_change_id(&actor.id, "record")?;
        let built = if let Some(paths) = build_selected_paths.as_deref() {
            if let Some(selections) = build_selection_set.as_ref() {
                self.build_root_for_selected_record_incremental_with_selection_set(
                    &head.root_id,
                    &previous_files,
                    &disk_files,
                    paths,
                    selections,
                    options.allow_ignored,
                    &change_id,
                )?
            } else {
                self.build_root_for_selected_record_incremental(
                    &head.root_id,
                    &previous_files,
                    &disk_files,
                    paths,
                    options.allow_ignored,
                    &change_id,
                )?
            }
        } else {
            self.build_root_from_disk_files(&disk_files, &change_id, Some(&previous_files))?
        };
        let diff = self.diff_file_maps(&previous_files, &built.files)?;

        if diff.changes.is_empty() {
            return Ok(RecordReport {
                branch,
                operation: None,
                root_id: head.root_id,
                changed_paths: Vec::new(),
            });
        }

        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind: options.kind.unwrap_or(OperationKind::ManualRecord),
            parents: vec![head.change_id.clone()],
            before_root: Some(head.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: branch.clone(),
            actor,
            session_id,
            message: message.map(|message| redact_sensitive_text(&message)),
            changes: diff.changes,
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&head, &change_id, &built.root_id, &operation_id)?;
        if selected_paths.is_empty() {
            self.set_worktree_index_baseline(&built.root_id)?;
            self.reconcile_daemon_full_status(&built.root_id, &[], record_generation);
        }
        Ok(RecordReport {
            branch,
            operation: Some(change_id),
            root_id: built.root_id,
            changed_paths: diff.summaries,
        })
    }

    fn record_from_changed_path_ledger(
        &mut self,
        branch: String,
        head: RefRecord,
        message: Option<String>,
        actor: Actor,
        options: RecordOptions,
        session_id: Option<String>,
    ) -> Result<RecordReport> {
        let branch_for_build = branch.clone();
        let actor_for_build = actor.clone();
        let message_for_build = message.clone();
        let session_for_build = session_id.clone();
        let kind = options.kind.unwrap_or(OperationKind::ManualRecord);
        let (built_record, fenced) =
            self.with_workspace_authoritative_command_snapshot(|db, policy, candidates, _git| {
                let comparison = db.compare_authoritative_candidates(
                    policy,
                    candidates,
                    &head.root_id,
                    crate::db::change_ledger::CandidateMaterialization::RecordBytes,
                )?;
                if comparison.summaries.is_empty() {
                    return Ok(None);
                }
                #[cfg(debug_assertions)]
                run_observed_record_after_compare_hook()?;
                let change_id = db.allocate_change_id(&actor_for_build.id, "record")?;
                let disk_files = comparison.disk_files.as_ref().ok_or_else(|| {
                    crate::Error::Corrupt(
                        "record candidate comparison omitted pinned contents".into(),
                    )
                })?;
                let built = db.build_root_for_selected_disk_files_incremental(
                    &head.root_id,
                    &comparison.baseline_files,
                    disk_files,
                    &comparison.selections,
                    &change_id,
                )?;
                let built_manifest = built
                    .files
                    .iter()
                    .map(|(path, entry)| {
                        (
                            path.clone(),
                            DiskManifest {
                                kind: entry.kind.clone(),
                                executable: entry.executable,
                                content_hash: entry.content_hash.clone(),
                            },
                        )
                    })
                    .collect::<BTreeMap<_, _>>();
                if built_manifest != comparison.disk_manifest {
                    return Err(crate::Error::ChangeLedgerReconcileRequired {
                        scope: candidates.expected.scope_id.to_text(),
                        state: "untrusted_gap".into(),
                        reason: "authorized candidate contents changed during root construction"
                            .into(),
                        command: "trail record".into(),
                    });
                }
                let diff = db.diff_file_maps(&comparison.baseline_files, &built.files)?;
                if diff.changes.is_empty() {
                    return Ok(None);
                }
                Ok(Some((
                    Operation {
                        version: OP_OBJECT_VERSION,
                        change_id: change_id.clone(),
                        kind: kind.clone(),
                        parents: vec![head.change_id.clone()],
                        before_root: Some(head.root_id.clone()),
                        after_root: built.root_id.clone(),
                        branch: branch_for_build.clone(),
                        actor: actor_for_build.clone(),
                        session_id: session_for_build.clone(),
                        message: message_for_build
                            .clone()
                            .map(|message| redact_sensitive_text(&message)),
                        changes: diff.changes,
                        created_at: now_ts(),
                    },
                    built.root_id,
                    diff.summaries,
                )))
            })?;
        let Some((operation, root_id, changed_paths)) = built_record else {
            return Ok(RecordReport {
                branch,
                operation: None,
                root_id: head.root_id,
                changed_paths: Vec::new(),
            });
        };
        let observed = crate::db::change_ledger::ObservedRecordCut {
            expected: fenced.candidates.expected.clone(),
            c1: fenced.candidates.cut.clone(),
            c2: fenced.c2,
            acknowledgement_tokens: fenced.candidates.acknowledgement_tokens,
        };
        let _observer_exclusion =
            crate::db::begin_authorized_observer_write_exclusion(&self.db_dir);
        let _lock = Self::with_write_lock_wait(std::time::Duration::from_secs(5), || {
            self.acquire_write_lock()
        })?;
        // Keep exclusion through mirror repair, daemon baseline rebind, and
        // worktree-index baseline publication. Narrowing it to the SQLite
        // COMMIT would expose those shared persistent transitions to another
        // writer even though the authoritative SQL transaction had finished.
        #[cfg(debug_assertions)]
        run_observed_record_with_lock_hook()?;
        self.commit_observed_record(&operation, &head, &observed, true, None)?;
        self.set_worktree_index_baseline(&root_id)?;
        Ok(RecordReport {
            branch,
            operation: Some(operation.change_id),
            root_id,
            changed_paths,
        })
    }
}
