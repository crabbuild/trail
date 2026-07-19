use super::initialization::{
    lane_initialization_record, LaneInitializationRecord, LaneInitializationReservation,
    ResolvedLaneSpawnRequest,
};
use super::workdir::{MaterializationOutcome, MaterializationPolicy};
use super::*;

#[cfg(debug_assertions)]
std::thread_local! {
    static FAIL_SPARSE_SELECTION_WRITE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FAIL_LANE_ASSOCIATION_BOUNDARY: std::cell::RefCell<Option<&'static str>> = const { std::cell::RefCell::new(None) };
    static FAIL_LANE_INITIALIZATION_IO: std::cell::RefCell<Option<(&'static str, bool)>> = const { std::cell::RefCell::new(None) };
}

#[cfg(debug_assertions)]
pub(crate) fn set_lane_initialization_io_failure_for_current_thread(
    boundary: Option<&'static str>,
    disk_full: bool,
) {
    FAIL_LANE_INITIALIZATION_IO.with(|selected| {
        *selected.borrow_mut() = boundary.map(|boundary| (boundary, disk_full));
    });
}

#[cfg(debug_assertions)]
fn fail_lane_initialization_io_if_requested(boundary: &'static str) -> Result<()> {
    let selected = FAIL_LANE_INITIALIZATION_IO.with(|selected| *selected.borrow());
    match selected {
        Some((selected, true)) if selected == boundary => {
            Err(Error::Io(std::io::Error::from_raw_os_error(28)))
        }
        Some((selected, false)) if selected == boundary => Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("injected permission failure at {boundary}"),
        ))),
        _ => Ok(()),
    }
}

#[cfg(not(debug_assertions))]
fn fail_lane_initialization_io_if_requested(_boundary: &'static str) -> Result<()> {
    Ok(())
}

#[cfg(debug_assertions)]
pub(crate) fn set_sparse_selection_write_failure_for_current_thread(enabled: bool) {
    FAIL_SPARSE_SELECTION_WRITE.with(|fail| fail.set(enabled));
}

#[cfg(debug_assertions)]
fn fail_sparse_selection_write_if_requested() -> Result<()> {
    if FAIL_SPARSE_SELECTION_WRITE.with(std::cell::Cell::get) {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "injected sparse-selection publication failure",
        )));
    }
    Ok(())
}

#[cfg(debug_assertions)]
pub(crate) fn set_lane_association_failure_for_current_thread(boundary: Option<&'static str>) {
    FAIL_LANE_ASSOCIATION_BOUNDARY.with(|selected| *selected.borrow_mut() = boundary);
}

#[cfg(debug_assertions)]
pub(crate) fn fail_lane_association_if_requested(boundary: &'static str) -> Result<()> {
    if FAIL_LANE_ASSOCIATION_BOUNDARY.with(|selected| *selected.borrow() == Some(boundary)) {
        return Err(Error::InvalidInput(format!(
            "injected lane association failure at {boundary}"
        )));
    }
    Ok(())
}

#[cfg(not(debug_assertions))]
pub(crate) fn fail_lane_association_if_requested(_boundary: &'static str) -> Result<()> {
    Ok(())
}

pub(crate) fn committed_lane_step<T>(
    operation: &str,
    repair: &str,
    result: Result<T>,
) -> Result<T> {
    result.map_err(|error| Error::OperationCommittedRepairRequired {
        operation: operation.to_string(),
        repair: repair.to_string(),
        reason: error.to_string(),
    })
}

const LARGE_LANE_MATERIALIZE_FILE_THRESHOLD: u64 = 10_000;

#[cfg(debug_assertions)]
fn lane_initialization_crash_cut(boundary: &str) -> Result<()> {
    if std::env::var("TRAIL_TEST_LANE_INITIALIZATION_CRASH_AFTER").as_deref() != Ok(boundary) {
        return Ok(());
    }
    let path = std::env::var_os("TRAIL_TEST_LANE_INITIALIZATION_HANDSHAKE")
        .map(PathBuf::from)
        .ok_or_else(|| Error::InvalidInput("missing crash handshake path".into()))?;
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)?;
    use std::io::Write;
    file.write_all(boundary.as_bytes())?;
    file.sync_all()?;
    fs::OpenOptions::new()
        .read(true)
        .open(path.parent().ok_or_else(|| Error::InvalidPath {
            path: path.display().to_string(),
            reason: "handshake has no parent".into(),
        })?)?
        .sync_all()?;
    loop {
        std::thread::park();
    }
}

#[cfg(not(debug_assertions))]
fn lane_initialization_crash_cut(_boundary: &str) -> Result<()> {
    Ok(())
}

impl Trail {
    fn committed_lane_initialization_step<T>(
        &mut self,
        initialization: &LaneInitializationRecord,
        result: Result<T>,
    ) -> Result<T> {
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                let repaired = self.mark_lane_initialization_repair_required(
                    &initialization.initialization_id,
                    &error,
                )?;
                Err(Error::CommittedRepairRequired {
                    lane: repaired.lane_name,
                    initialization_id: repaired.initialization_id,
                    request_fingerprint: repaired.request_fingerprint,
                    operation_id: repaired.operation_id,
                    phase: LaneInitializationPhase::RepairRequired,
                    committed: true,
                    repair: repaired.repair_command.unwrap_or_else(|| {
                        format!(
                            "trail lane repair-initialization {}",
                            initialization.lane_name
                        )
                    }),
                    reason: error.to_string(),
                })
            }
        }
    }

    fn lane_spawn_report_for_initialization(
        &self,
        initialization: &LaneInitializationRecord,
        resumed: bool,
    ) -> Result<LaneSpawnReport> {
        let details = self.lane_details(&initialization.lane_id)?;
        let requested_workdir_mode =
            self.lane_requested_workdir_mode_for(&details.record, &details.branch)?;
        let workdir_mode = self.lane_workdir_mode_for(&details.record, &details.branch)?;
        let workdir_backend = self.lane_workdir_backend_for(&details.record)?;
        let materialization = self.lane_materialization_report_for(&details.record)?;
        let sparse_paths = self.lane_report_sparse_paths(&details.branch)?;
        Ok(LaneSpawnReport {
            initialization_id: initialization.initialization_id.clone(),
            request_fingerprint: initialization.request_fingerprint.clone(),
            phase: initialization.phase,
            committed: matches!(
                initialization.phase,
                LaneInitializationPhase::Associated
                    | LaneInitializationPhase::ObserverReady
                    | LaneInitializationPhase::RepairRequired
            ),
            resumed,
            lane_id: details.branch.lane_id,
            ref_name: details.branch.ref_name,
            base_change: details.branch.base_change,
            workdir: details.branch.workdir,
            requested_workdir_mode,
            workdir_mode: workdir_mode.clone(),
            workdir_backend,
            materialization,
            sparse_paths,
            transparent_cow_available: workdir_mode.is_transparent_cow(),
        })
    }

    pub fn default_lane_materialize(&self) -> bool {
        self.config.lane.default_materialize
    }

    pub fn default_lane_materialize_for_ref(&self, from: Option<&str>) -> Result<bool> {
        if !self.config.lane.default_materialize {
            return Ok(false);
        }
        let source = match from {
            Some(refish) => self.resolve_refish(refish)?,
            None => self.resolve_branch_ref(&self.current_branch()?)?,
        };
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, &source.root_id)?;
        Ok(root.file_count <= LARGE_LANE_MATERIALIZE_FILE_THRESHOLD)
    }

    pub fn resolve_lane_spawn_workdir_mode(
        &self,
        from: Option<&str>,
        requested_mode: Option<&str>,
        materialize: Option<bool>,
        no_materialize: bool,
        custom_workdir: bool,
        sparse_paths: &[String],
    ) -> Result<LaneWorkdirMode> {
        let mode = if let Some("auto") = requested_mode {
            LaneWorkdirMode::Auto
        } else if let Some(requested_mode) = requested_mode {
            parse_lane_workdir_mode(requested_mode)?
        } else if no_materialize || materialize == Some(false) {
            LaneWorkdirMode::Virtual
        } else if !sparse_paths.is_empty() {
            LaneWorkdirMode::Sparse
        } else if custom_workdir || materialize == Some(true) {
            LaneWorkdirMode::Auto
        } else if self.default_lane_materialize_for_ref(from)? {
            LaneWorkdirMode::Auto
        } else {
            LaneWorkdirMode::Virtual
        };

        if no_materialize && mode != LaneWorkdirMode::Virtual {
            return Err(Error::InvalidInput(
                "--no-materialize requires workdir mode `virtual`".to_string(),
            ));
        }
        if materialize == Some(false) && mode != LaneWorkdirMode::Virtual {
            return Err(Error::InvalidInput(
                "--materialize=false requires workdir mode `virtual`".to_string(),
            ));
        }
        if materialize == Some(true) && mode == LaneWorkdirMode::Virtual {
            return Err(Error::InvalidInput(
                "--materialize=true cannot be combined with workdir mode `virtual`".to_string(),
            ));
        }
        validate_lane_workdir_mode_request(&mode, custom_workdir, sparse_paths)?;
        Ok(mode)
    }

    pub fn spawn_lane(
        &mut self,
        name: &str,
        from: Option<&str>,
        materialize: bool,
        provider: Option<String>,
        model: Option<String>,
    ) -> Result<LaneSpawnReport> {
        self.spawn_lane_with_workdir(name, from, materialize, provider, model, None)
    }

    pub fn spawn_lane_with_workdir(
        &mut self,
        name: &str,
        from: Option<&str>,
        materialize: bool,
        provider: Option<String>,
        model: Option<String>,
        workdir: Option<PathBuf>,
    ) -> Result<LaneSpawnReport> {
        self.spawn_lane_with_workdir_paths(name, from, materialize, provider, model, workdir, &[])
    }

    pub fn spawn_lane_with_workdir_paths(
        &mut self,
        name: &str,
        from: Option<&str>,
        materialize: bool,
        provider: Option<String>,
        model: Option<String>,
        workdir: Option<PathBuf>,
        sparse_paths: &[String],
    ) -> Result<LaneSpawnReport> {
        self.spawn_lane_with_workdir_paths_and_neighbors(
            name,
            from,
            materialize,
            provider,
            model,
            workdir,
            sparse_paths,
            false,
        )
    }

    pub fn spawn_lane_with_workdir_paths_and_neighbors(
        &mut self,
        name: &str,
        from: Option<&str>,
        materialize: bool,
        provider: Option<String>,
        model: Option<String>,
        workdir: Option<PathBuf>,
        sparse_paths: &[String],
        include_neighbors: bool,
    ) -> Result<LaneSpawnReport> {
        let workdir_mode = if materialize {
            if sparse_paths.is_empty() {
                LaneWorkdirMode::Auto
            } else {
                LaneWorkdirMode::Sparse
            }
        } else {
            LaneWorkdirMode::Virtual
        };
        self.spawn_lane_with_workdir_mode_paths_and_neighbors(
            name,
            from,
            workdir_mode,
            provider,
            model,
            workdir,
            sparse_paths,
            include_neighbors,
        )
    }

    pub fn spawn_lane_with_workdir_mode_paths_and_neighbors(
        &mut self,
        name: &str,
        from: Option<&str>,
        workdir_mode: LaneWorkdirMode,
        provider: Option<String>,
        model: Option<String>,
        workdir: Option<PathBuf>,
        sparse_paths: &[String],
        include_neighbors: bool,
    ) -> Result<LaneSpawnReport> {
        self.spawn_lane_with_workdir_mode_paths_and_neighbors_inner(
            name,
            from,
            workdir_mode,
            provider,
            model,
            workdir,
            sparse_paths,
            include_neighbors,
            false,
        )
    }

    #[doc(hidden)]
    pub fn spawn_lane_with_deferred_initial_ledger(
        &mut self,
        name: &str,
        from: Option<&str>,
        workdir_mode: LaneWorkdirMode,
        provider: Option<String>,
        model: Option<String>,
        workdir: Option<PathBuf>,
        sparse_paths: &[String],
        include_neighbors: bool,
    ) -> Result<LaneSpawnReport> {
        self.spawn_lane_with_workdir_mode_paths_and_neighbors_inner(
            name,
            from,
            workdir_mode,
            provider,
            model,
            workdir,
            sparse_paths,
            include_neighbors,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_lane_with_workdir_mode_paths_and_neighbors_inner(
        &mut self,
        name: &str,
        from: Option<&str>,
        workdir_mode: LaneWorkdirMode,
        provider: Option<String>,
        model: Option<String>,
        workdir: Option<PathBuf>,
        sparse_paths: &[String],
        include_neighbors: bool,
        defer_initial_ledger: bool,
    ) -> Result<LaneSpawnReport> {
        // TRAIL_FS_PRODUCER: lane_spawn_materialize Materialize controlled
        let ledger_authority = crate::db::change_ledger::command_authority_enabled();
        let _lock = if ledger_authority {
            None
        } else {
            Some(self.acquire_write_lock()?)
        };
        validate_ref_segment(name)?;
        validate_lane_workdir_mode_request(&workdir_mode, workdir.is_some(), sparse_paths)?;
        let sparse_paths = normalize_record_paths(sparse_paths)?;
        let source = match from {
            Some(refish) if refish.starts_with("refs/heads/") => self.resolve_branch_ref(
                refish
                    .strip_prefix("refs/heads/")
                    .expect("prefix was checked"),
            )?,
            Some(refish) => self.resolve_refish(refish)?,
            None => self.resolve_branch_ref(&self.current_branch()?)?,
        };
        let mut lane_id = format!("lane_{}", crate::ids::short_hash(name.as_bytes(), 8));
        let ref_name = lane_ref(name);
        let workdir_path = if workdir_mode.materializes() {
            Some(self.resolve_lane_workdir_path(name, workdir.as_deref())?)
        } else {
            None
        };
        let source_ref = if from.is_some_and(crate::ids::is_change_id) {
            format!("detached:{}", source.change_id.0)
        } else {
            source.name.clone()
        };
        let mut request = ResolvedLaneSpawnRequest::new(
            &self.config.workspace.id.0,
            name,
            lane_id.clone(),
            source_ref,
            source.change_id.clone(),
            source.root_id.clone(),
            source.operation_id.clone(),
            workdir_mode.clone(),
            workdir_path.clone(),
            sparse_paths.clone(),
            include_neighbors,
            provider.clone(),
            model.clone(),
        )?;
        lane_id = format!(
            "lane_{}",
            crate::ids::short_hash(
                format!("{name}\0{}", request.request_fingerprint).as_bytes(),
                8
            )
        );
        request.lane_id.clone_from(&lane_id);
        let repair_command = format!("trail lane repair-initialization {name}");
        let reservation_lock = if ledger_authority {
            Some(crate::db::acquire_workspace_lock_for_lane_association(
                &self.db_dir,
                &self.db_dir.join(crate::db::DB_RELATIVE_PATH),
                &request.initialization_id,
                &repair_command,
            )?)
        } else {
            None
        };
        let reservation = self.reserve_lane_initialization(&request)?;
        drop(reservation_lock);
        let (initialization, resumed) = match reservation {
            LaneInitializationReservation::Start(record) => (record, false),
            LaneInitializationReservation::Resume(record) => (record, true),
            LaneInitializationReservation::Ready(record) => {
                return self.lane_spawn_report_for_initialization(&record, true);
            }
        };
        lane_initialization_crash_cut("after_reservation")?;
        if matches!(
            initialization.phase,
            LaneInitializationPhase::Associated | LaneInitializationPhase::RepairRequired
        ) {
            drop(_lock);
            return self.repair_lane_initialization(&request.lane_name);
        }
        let transparent_cow_available = request.requested_workdir_mode.is_transparent_cow();
        let mut sparse_policy_paths = None;
        let mut resolved_workdir_mode = request.requested_workdir_mode.clone();
        let mut workdir_backend = request
            .requested_workdir_mode
            .default_backend()
            .unwrap_or(WorkdirBackend::Clone);
        let mut materialization_report: Option<MaterializationReport> = initialization
            .materialization_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?;
        let mut materialization_operation_id = if initialization.phase
            == LaneInitializationPhase::Materialized
            && request.workdir.is_some()
        {
            Some(initialization.operation_id.clone())
        } else {
            None
        };
        if let Some(report) = &materialization_report {
            workdir_backend = report.backend();
        }
        fail_lane_initialization_io_if_requested("workdir_write")?;
        let materialized_workdir = if initialization.phase == LaneInitializationPhase::Materialized
        {
            initialization
                .workdir
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned())
        } else if let Some(dir) = &request.workdir {
            match &request.requested_workdir_mode {
                LaneWorkdirMode::FuseCow => {
                    self.prepare_fuse_cow_lane_workdir(name, dir, workdir.is_some())?;
                }
                LaneWorkdirMode::DokanCow => {
                    #[cfg(target_os = "windows")]
                    self.prepare_dokan_cow_lane_workdir(name, dir, workdir.is_some())?;
                    #[cfg(not(target_os = "windows"))]
                    return Err(Error::InvalidInput(
                        "dokan-cow workdirs are currently supported only on Windows".to_string(),
                    ));
                }
                LaneWorkdirMode::NfsCow => {
                    self.prepare_nfs_cow_lane_workdir(name, dir, workdir.is_some())?;
                }
                LaneWorkdirMode::Sparse => {
                    let (report, operation_id) = self
                        .materialize_lane_workdir_at_paths_with_neighbors(
                            &request.source_root,
                            dir,
                            workdir.is_some(),
                            &request.sparse_paths,
                            request.include_neighbors,
                        )?;
                    materialization_operation_id = operation_id;
                    if let Some(report) = report {
                        workdir_backend = report.backend();
                        materialization_report = Some(report);
                    }
                    if !request.sparse_paths.is_empty() {
                        sparse_policy_paths = self.sparse_workdir_paths(dir)?;
                    }
                }
                LaneWorkdirMode::NativeCow
                | LaneWorkdirMode::PortableCopy
                | LaneWorkdirMode::Auto => {
                    let policy = match request.requested_workdir_mode {
                        LaneWorkdirMode::NativeCow => MaterializationPolicy::StrictNative,
                        LaneWorkdirMode::PortableCopy => MaterializationPolicy::Portable,
                        LaneWorkdirMode::Auto => MaterializationPolicy::Auto,
                        _ => unreachable!(),
                    };
                    let outcome = self.materialize_lane_root_staged(
                        &request.source_root,
                        dir,
                        workdir.is_some(),
                        policy,
                    )?;
                    resolved_workdir_mode = outcome.resolved_mode;
                    workdir_backend = outcome.backend;
                    materialization_operation_id =
                        Some(outcome.materialization_operation_id.clone());
                    materialization_report = Some(outcome.report);
                }
                LaneWorkdirMode::Virtual => {}
            }
            Some(dir.to_string_lossy().to_string())
        } else {
            None
        };
        let initialization_operation = materialization_operation_id
            .as_ref()
            .map(|operation_id| ObjectId(operation_id.clone()))
            .unwrap_or_else(|| request.source_operation.clone());
        for boundary in ["file_sync", "directory_sync"] {
            if let Err(error) = fail_lane_initialization_io_if_requested(boundary) {
                if let Some(operation_id) = materialization_operation_id.as_deref() {
                    self.abort_materialization_operation(operation_id)?;
                }
                return Err(error);
            }
        }
        if initialization.phase == LaneInitializationPhase::Reserved {
            let materialization_lock = if ledger_authority {
                Some(crate::db::acquire_workspace_lock_for_lane_association(
                    &self.db_dir,
                    &self.db_dir.join(crate::db::DB_RELATIVE_PATH),
                    &request.initialization_id,
                    &repair_command,
                )?)
            } else {
                None
            };
            self.mark_lane_initialization_materialized(
                &request,
                &initialization_operation,
                materialization_report.as_ref(),
            )?;
            drop(materialization_lock);
        }
        lane_initialization_crash_cut("after_materialization")?;
        let sparse_paths_for_report = sparse_policy_paths.clone().unwrap_or_default();
        let requested_workdir_mode = request.requested_workdir_mode.clone();
        let metadata_json = serde_json::to_string(&serde_json::json!({
            "source_ref": request.source_ref,
            "requested_workdir_mode": requested_workdir_mode.as_str(),
            "workdir_mode": resolved_workdir_mode.as_str(),
            "workdir_backend": workdir_backend.as_str(),
            "materialization": materialization_report,
            "sparse_paths": sparse_paths_for_report,
            "include_neighbors": request.include_neighbors,
            "transparent_cow_available": transparent_cow_available
        }))?;
        let now = now_ts();
        let association_lock = if ledger_authority {
            Some(crate::db::acquire_workspace_lock_for_lane_association(
                &self.db_dir,
                &self.db_dir.join(crate::db::DB_RELATIVE_PATH),
                &request.initialization_id,
                &repair_command,
            )?)
        } else {
            None
        };
        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let association = (|| -> Result<()> {
            self.insert_new_ref_database_only(
                &ref_name,
                &request.source_change,
                &request.source_root,
                &request.source_operation,
            )?;
            fail_lane_association_if_requested("spawn_after_ref")?;
            self.conn.execute(
                "INSERT INTO lanes (lane_id, name, kind, provider, model, created_at, metadata_json) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    lane_id,
                    name,
                    "coding-lane",
                    request.provider,
                    request.model,
                    now,
                    metadata_json
                ],
            )?;
            fail_lane_association_if_requested("spawn_after_lane")?;
            self.conn.execute(
                "INSERT INTO lane_branches \
                 (lane_id, ref_name, base_change, head_change, base_root, head_root, session_id, workdir, status, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'active', ?9, ?9)",
                params![
                    lane_id,
                    ref_name,
                    request.source_change.0,
                    request.source_change.0,
                    request.source_root.0,
                    request.source_root.0,
                    Option::<String>::None,
                    materialized_workdir,
                    now
                ],
            )?;
            fail_lane_association_if_requested("spawn_after_branch")?;
            fail_lane_initialization_io_if_requested("association_sqlite_commit")?;
            let changed = self.conn.execute(
                "UPDATE lane_initializations SET phase='associated',updated_at=?1
                 WHERE initialization_id=?2 AND request_fingerprint=?3
                   AND phase='materialized'",
                params![now, request.initialization_id, request.request_fingerprint,],
            )?;
            if changed != 1 {
                return Err(Error::Corrupt(format!(
                    "lane initialization `{}` could not transition materialized -> associated",
                    request.initialization_id
                )));
            }
            Ok(())
        })();
        match association {
            Ok(()) => self.conn.execute_batch("COMMIT;")?,
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                self.conn.execute(
                    "DELETE FROM lane_initializations
                     WHERE initialization_id=?1 AND phase='materialized'
                       AND NOT EXISTS(SELECT 1 FROM refs WHERE name=?2)",
                    params![request.initialization_id, ref_name],
                )?;
                if let Some(operation_id) = materialization_operation_id.as_deref() {
                    self.abort_materialization_operation(operation_id)?;
                }
                return Err(error);
            }
        }
        drop(association_lock);
        lane_initialization_crash_cut("after_association")?;
        let committed_operation = materialization_operation_id
            .clone()
            .unwrap_or_else(|| request.source_operation.0.clone());
        let mirror = (|| {
            fail_lane_association_if_requested("spawn_ref_repair")?;
            self.repair_new_ref_mirror(
                &ref_name,
                &request.source_change,
                &request.source_root,
                &request.source_operation,
            )
        })();
        self.committed_lane_initialization_step(&initialization, mirror)?;
        lane_initialization_crash_cut("after_reconciliation")?;
        if let Some(operation_id) = materialization_operation_id.as_deref() {
            let journal = (|| {
                fail_lane_association_if_requested("spawn_journal_completion")?;
                self.complete_materialization_operation(operation_id)
            })();
            self.committed_lane_initialization_step(&initialization, journal)?;
        }
        let reconciliation = fail_lane_association_if_requested("spawn_after_commit");
        self.committed_lane_initialization_step(&initialization, reconciliation)?;
        if defer_initial_ledger
            && ledger_authority
            && materialized_workdir.is_some()
            && !request.requested_workdir_mode.is_transparent_cow()
        {
            return Ok(LaneSpawnReport {
                initialization_id: initialization.initialization_id,
                request_fingerprint: initialization.request_fingerprint,
                phase: LaneInitializationPhase::Associated,
                committed: true,
                resumed,
                lane_id,
                ref_name,
                base_change: request.source_change,
                workdir: materialized_workdir,
                requested_workdir_mode,
                workdir_mode: resolved_workdir_mode,
                workdir_backend: Some(workdir_backend),
                materialization: materialization_report,
                sparse_paths: sparse_policy_paths.unwrap_or_default(),
                transparent_cow_available,
            });
        }
        if ledger_authority
            && materialized_workdir.is_some()
            && !request.requested_workdir_mode.is_transparent_cow()
        {
            let expected_result =
                crate::db::change_ledger::prepare_materialized_lane_controlled_projection(
                    self, &lane_id,
                );
            let expected =
                self.committed_lane_initialization_step(&initialization, expected_result)?;
            let evidence = crate::db::change_ledger::IntentEvidence {
                exact_paths: Vec::new(),
                complete_prefixes: Vec::new(),
            };
            let alignment = crate::db::change_ledger::run_projection_alignment(
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
                                &request.source_root,
                                crate::db::change_ledger::CandidateMaterialization::ManifestOnly,
                            )?;
                            if comparison.summaries.is_empty() {
                                Ok(())
                            } else {
                                Err(Error::ChangeLedgerReconcileRequired {
                                    scope: expected.scope_id.to_text(),
                                    state: "stale_baseline".into(),
                                    reason: format!(
                                        "initial lane materialization did not match its target root: {:?}",
                                        comparison.summaries
                                    ),
                                    command: format!("trail lane status {lane_id}"),
                                })
                            }
                        },
                    )
                },
                |db| db.publish_lane_marker_if_materialized(&lane_id),
            );
            self.committed_lane_initialization_step(&initialization, alignment)?;
        } else if materialized_workdir.is_some() {
            let marker = (|| {
                fail_lane_association_if_requested("spawn_marker")?;
                self.publish_lane_marker_if_materialized(name)
            })();
            self.committed_lane_initialization_step(&initialization, marker)?;
        }
        lane_initialization_crash_cut("after_marker")?;
        if request.requested_workdir_mode.is_transparent_cow() {
            committed_lane_step(
                &committed_operation,
                "initial lane workspace view publication",
                (|| {
                    fail_lane_association_if_requested("spawn_workspace_view")?;
                    let mountpoint = materialized_workdir.as_deref().ok_or_else(|| {
                        Error::Corrupt("transparent COW lane has no mountpoint".to_string())
                    })?;
                    self.create_workspace_view(
                        &lane_id,
                        &request.source_change,
                        &request.source_root,
                        platform_workspace_backend(&request.requested_workdir_mode),
                        Path::new(mountpoint),
                    )
                })(),
            )?;
        }
        let spawn_event = (|| {
            fail_lane_association_if_requested("spawn_event")?;
            self.insert_lane_event(
                &lane_id,
                "lane_spawned",
                Some(&request.source_change),
                None,
                &serde_json::json!({
                    "ref_name": ref_name.clone(),
                    "base_root": request.source_root.0.clone(),
                    "workdir": materialized_workdir.clone(),
                    "requested_workdir_mode": requested_workdir_mode.as_str(),
                    "workdir_mode": resolved_workdir_mode.as_str(),
                    "workdir_backend": workdir_backend.as_str(),
                    "materialization": materialization_report,
                    "sparse_paths": sparse_policy_paths.clone().unwrap_or_default(),
                    "include_neighbors": request.include_neighbors,
                    "transparent_cow_available": transparent_cow_available
                }),
            )
        })();
        self.committed_lane_initialization_step(&initialization, spawn_event)?;
        lane_initialization_crash_cut("after_spawn_event")?;
        self.mark_lane_initialization_observer_ready(&request)?;
        Ok(LaneSpawnReport {
            initialization_id: request.initialization_id,
            request_fingerprint: request.request_fingerprint,
            phase: LaneInitializationPhase::ObserverReady,
            committed: true,
            resumed,
            lane_id,
            ref_name,
            base_change: request.source_change,
            workdir: materialized_workdir,
            requested_workdir_mode,
            workdir_mode: resolved_workdir_mode,
            workdir_backend: Some(workdir_backend),
            materialization: materialization_report,
            sparse_paths: sparse_policy_paths.unwrap_or_default(),
            transparent_cow_available,
        })
    }

    #[doc(hidden)]
    pub fn resume_deferred_initial_lane_ledger(&mut self, lane: &str) -> Result<LaneSpawnReport> {
        let details = self.lane_details(lane)?;
        let metadata = details.record.metadata_json.as_deref().ok_or_else(|| {
            Error::Corrupt(format!(
                "lane `{}` is missing its spawn metadata",
                details.record.name
            ))
        })?;
        let metadata: serde_json::Value = serde_json::from_str(metadata)?;
        let metadata_field = |name: &str| {
            metadata
                .get(name)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| Error::Corrupt(format!("lane spawn metadata is missing `{name}`")))
        };
        let requested_workdir_mode =
            LaneWorkdirMode::parse(metadata_field("requested_workdir_mode")?)
                .ok_or_else(|| Error::Corrupt("invalid requested lane workdir mode".into()))?;
        let workdir_mode = LaneWorkdirMode::parse(metadata_field("workdir_mode")?)
            .ok_or_else(|| Error::Corrupt("invalid resolved lane workdir mode".into()))?;
        let workdir_backend = serde_json::from_value::<WorkdirBackend>(
            metadata
                .get("workdir_backend")
                .cloned()
                .ok_or_else(|| Error::Corrupt("lane spawn metadata is missing backend".into()))?,
        )?;
        let materialization = metadata
            .get("materialization")
            .filter(|value| !value.is_null())
            .cloned()
            .map(serde_json::from_value::<MaterializationReport>)
            .transpose()?;
        let sparse_paths = metadata
            .get("sparse_paths")
            .cloned()
            .map(serde_json::from_value::<Vec<String>>)
            .transpose()?
            .unwrap_or_default();
        let include_neighbors = metadata
            .get("include_neighbors")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let transparent_cow_available = metadata
            .get("transparent_cow_available")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        if details.branch.workdir.is_some() && !workdir_mode.is_transparent_cow() {
            let expected =
                crate::db::change_ledger::prepare_materialized_lane_controlled_projection(
                    self,
                    &details.branch.lane_id,
                )?;
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
                        &details.branch.lane_id,
                        intent,
                        &evidence,
                        |_| Ok(()),
                        |db, policy, candidates| {
                            let comparison = db.compare_controlled_projection_target(
                                policy,
                                candidates,
                                &details.branch.head_root,
                                crate::db::change_ledger::CandidateMaterialization::ManifestOnly,
                            )?;
                            if comparison.summaries.is_empty() {
                                Ok(())
                            } else {
                                Err(Error::ChangeLedgerReconcileRequired {
                                    scope: expected.scope_id.to_text(),
                                    state: "stale_baseline".into(),
                                    reason: format!(
                                        "initial lane materialization did not match its target root: {:?}",
                                        comparison.summaries
                                    ),
                                    command: format!(
                                        "trail lane status {}",
                                        details.branch.lane_id
                                    ),
                                })
                            }
                        },
                    )
                },
                |db| db.publish_lane_marker_if_materialized(&details.branch.lane_id),
            )?;
        }

        let event_exists = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM lane_events WHERE lane_id=?1 AND event_type='lane_spawned')",
            [&details.branch.lane_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !event_exists {
            self.insert_lane_event(
                &details.branch.lane_id,
                "lane_spawned",
                Some(&details.branch.base_change),
                None,
                &serde_json::json!({
                    "ref_name": details.branch.ref_name,
                    "base_root": details.branch.base_root.0,
                    "workdir": details.branch.workdir,
                    "requested_workdir_mode": requested_workdir_mode.as_str(),
                    "workdir_mode": workdir_mode.as_str(),
                    "workdir_backend": workdir_backend.as_str(),
                    "materialization": materialization,
                    "sparse_paths": sparse_paths,
                    "include_neighbors": include_neighbors,
                    "transparent_cow_available": transparent_cow_available,
                }),
            )?;
        }

        let initialization = self.complete_deferred_lane_initialization(&details.branch.lane_id)?;
        Ok(LaneSpawnReport {
            initialization_id: initialization.initialization_id,
            request_fingerprint: initialization.request_fingerprint,
            phase: LaneInitializationPhase::ObserverReady,
            committed: true,
            resumed: true,
            lane_id: details.branch.lane_id,
            ref_name: details.branch.ref_name,
            base_change: details.branch.base_change,
            workdir: details.branch.workdir,
            requested_workdir_mode,
            workdir_mode,
            workdir_backend: Some(workdir_backend),
            materialization,
            sparse_paths,
            transparent_cow_available,
        })
    }

    /// Validate and idempotently finish a durably associated lane initialization.
    pub fn repair_lane_initialization(&mut self, lane: &str) -> Result<LaneSpawnReport> {
        validate_ref_segment(lane)?;
        let initialization = lane_initialization_record(&self.conn, lane)?
            .ok_or_else(|| Error::InvalidInput(format!("lane `{lane}` has no initialization")))?;
        if initialization.phase == LaneInitializationPhase::ObserverReady {
            return self.lane_spawn_report_for_initialization(&initialization, true);
        }
        if !matches!(
            initialization.phase,
            LaneInitializationPhase::Associated | LaneInitializationPhase::RepairRequired
        ) {
            return Err(Error::InvalidInput(format!(
                "lane `{lane}` initialization is {:?}; repeat the identical spawn command",
                initialization.phase
            )));
        }
        let result = self.repair_lane_initialization_once(&initialization);
        match result {
            Ok(report) => Ok(report),
            Err(error) => {
                let repaired = self.mark_lane_initialization_repair_required(
                    &initialization.initialization_id,
                    &error,
                )?;
                Err(Error::CommittedRepairRequired {
                    lane: repaired.lane_name,
                    initialization_id: repaired.initialization_id,
                    request_fingerprint: repaired.request_fingerprint,
                    operation_id: repaired.operation_id,
                    phase: LaneInitializationPhase::RepairRequired,
                    committed: true,
                    repair: repaired.repair_command.unwrap_or_else(|| {
                        format!(
                            "trail lane repair-initialization {}",
                            initialization.lane_name
                        )
                    }),
                    reason: error.to_string(),
                })
            }
        }
    }

    fn repair_lane_initialization_once(
        &mut self,
        initialization: &LaneInitializationRecord,
    ) -> Result<LaneSpawnReport> {
        let expected_id = {
            let mut digest = sha2::Sha256::new();
            use sha2::Digest;
            digest.update(b"trail-lane-initialization-v1\0");
            digest.update(self.config.workspace.id.0.as_bytes());
            digest.update([0]);
            digest.update(initialization.lane_name.as_bytes());
            digest.update([0]);
            digest.update(initialization.request_fingerprint.as_bytes());
            format!("init_{}", hex::encode(digest.finalize()))
        };
        if initialization.request_fingerprint.starts_with("sha256:")
            && initialization.initialization_id != expected_id
        {
            return Err(Error::Corrupt(format!(
                "lane initialization `{}` does not match its fingerprint",
                initialization.initialization_id
            )));
        }
        let details = self.lane_details(&initialization.lane_id)?;
        if details.record.name != initialization.lane_name
            || details.record.lane_id != initialization.lane_id
            || details.branch.status != "active"
        {
            return Err(Error::Corrupt(
                "lane initialization association identity changed".into(),
            ));
        }
        let head = self.get_ref(&details.branch.ref_name)?;
        if head.change_id != details.branch.head_change || head.root_id != details.branch.head_root
        {
            return Err(Error::Corrupt(
                "lane ref does not match the active branch head".into(),
            ));
        }
        if let Some(workdir) = &initialization.workdir {
            let metadata = fs::symlink_metadata(workdir)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(Error::Corrupt(format!(
                    "lane workdir `{}` is not the original directory identity",
                    workdir.display()
                )));
            }
            super::workdir::materialized_lane_root_identity(workdir)?;
            if details.branch.workdir.as_deref() != Some(workdir.to_string_lossy().as_ref()) {
                return Err(Error::Corrupt(
                    "lane initialization workdir does not match the associated branch".into(),
                ));
            }
        }
        self.repair_new_ref_mirror(
            &details.branch.ref_name,
            &head.change_id,
            &head.root_id,
            &head.operation_id,
        )?;
        if initialization.materialization_json.is_some() {
            let journal = self
                .db_dir
                .join("materialization-operations")
                .join(format!("{}.json", initialization.operation_id));
            if journal.exists() {
                self.complete_materialization_operation(&initialization.operation_id)?;
            }
        }
        self.resume_deferred_initial_lane_ledger(&initialization.lane_name)
    }

    pub fn ensure_lane_workdir_materialized(
        &mut self,
        lane: &str,
        workdir: Option<PathBuf>,
    ) -> Result<LaneWorkdirReport> {
        // TRAIL_FS_PRODUCER: lane_ensure_materialized Materialize controlled
        let ledger_authority = crate::db::change_ledger::command_authority_enabled();
        let _lock = if ledger_authority {
            None
        } else {
            Some(self.acquire_write_lock()?)
        };
        validate_ref_segment(lane)?;
        let branch = self.lane_branch(lane)?;
        if let Some(existing) = branch.workdir.clone() {
            if let Some(requested) = workdir.as_deref() {
                let requested = self.resolve_lane_workdir_path(lane, Some(requested))?;
                let existing_path = normalize_workdir_path(&PathBuf::from(&existing))?;
                if requested != existing_path {
                    return Err(Error::InvalidInput(format!(
                        "lane `{lane}` already has materialized workdir `{}`",
                        existing_path.display()
                    )));
                }
            }
            let record = self.lane_record(&branch.lane_id)?;
            let workdir_mode = self.lane_workdir_mode_for(&record, &branch)?;
            let requested_workdir_mode = self.lane_requested_workdir_mode_for(&record, &branch)?;
            let workdir_backend = self.lane_workdir_backend_for(&record)?;
            let materialization = self.lane_materialization_report_for(&record)?;
            let sparse_paths = self.lane_report_sparse_paths(&branch)?;
            let transparent_cow_available = workdir_mode.is_transparent_cow();
            return Ok(LaneWorkdirReport {
                lane_id: branch.lane_id,
                workdir: Some(existing),
                requested_workdir_mode,
                workdir_backend,
                materialization,
                sparse_paths,
                transparent_cow_available,
                workdir_mode,
            });
        }

        let head = self.get_ref(&branch.ref_name)?;
        let dir = self.resolve_lane_workdir_path(lane, workdir.as_deref())?;
        let outcome = self.materialize_lane_root_staged(
            &head.root_id,
            &dir,
            workdir.is_some(),
            MaterializationPolicy::Auto,
        )?;
        let workdir = dir.to_string_lossy().to_string();
        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let association = (|| -> Result<()> {
            self.update_lane_materialization_metadata(
                &branch.lane_id,
                &LaneWorkdirMode::Auto,
                &outcome,
            )?;
            fail_lane_association_if_requested("ensure_after_lane_metadata")?;
            let changed = self.conn.execute(
                "UPDATE lane_branches SET workdir = ?1, updated_at = ?2
                 WHERE lane_id = ?3 AND workdir IS NULL AND head_root=?4",
                params![workdir, now_ts(), branch.lane_id, head.root_id.0],
            )?;
            if changed != 1 {
                return Err(Error::StaleBranch(branch.ref_name.clone()));
            }
            fail_lane_association_if_requested("ensure_after_branch")?;
            Ok(())
        })();
        match association {
            Ok(()) => self.conn.execute_batch("COMMIT;")?,
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                self.abort_materialization_operation(&outcome.materialization_operation_id)?;
                return Err(error);
            }
        }
        let committed_operation = outcome.materialization_operation_id.clone();
        committed_lane_step(
            &committed_operation,
            "ensured lane materialization journal completion",
            (|| {
                fail_lane_association_if_requested("ensure_journal_completion")?;
                self.complete_materialization_operation(&committed_operation)
            })(),
        )?;
        committed_lane_step(
            &committed_operation,
            "ensured lane post-association reconciliation",
            fail_lane_association_if_requested("ensure_after_commit"),
        )?;
        if ledger_authority {
            let expected =
                crate::db::change_ledger::prepare_materialized_lane_controlled_projection(
                    self,
                    &branch.lane_id,
                )
                .map_err(|error| Error::OperationCommittedRepairRequired {
                    operation: outcome.materialization_operation_id.clone(),
                    repair: "ensured materialized lane ledger reconciliation".into(),
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
                        &branch.lane_id,
                        intent,
                        &evidence,
                        |_| Ok(()),
                        |db, policy, candidates| {
                            let comparison = db.compare_controlled_projection_target(
                                policy,
                                candidates,
                                &head.root_id,
                                crate::db::change_ledger::CandidateMaterialization::ManifestOnly,
                            )?;
                            if comparison.summaries.is_empty() {
                                Ok(())
                            } else {
                                Err(Error::ChangeLedgerReconcileRequired {
                                    scope: expected.scope_id.to_text(),
                                    state: "stale_baseline".into(),
                                    reason:
                                        "ensured lane materialization did not match its target root"
                                            .into(),
                                    command: format!("trail lane status {}", branch.lane_id),
                                })
                            }
                        },
                    )
                },
                |db| db.publish_lane_marker_if_materialized(&branch.lane_id),
            )
            .map_err(|error| Error::OperationCommittedRepairRequired {
                operation: outcome.materialization_operation_id.clone(),
                repair: "ensured materialized lane ledger alignment".into(),
                reason: error.to_string(),
            })?;
        }
        committed_lane_step(
            &committed_operation,
            "ensured lane event publication",
            (|| {
                fail_lane_association_if_requested("ensure_event")?;
                self.insert_lane_event(
                    &branch.lane_id,
                    "workdir_materialized",
                    Some(&head.change_id),
                    None,
                    &serde_json::json!({
                        "workdir": workdir,
                        "root_id": head.root_id.0
                    }),
                )
            })(),
        )?;
        committed_lane_step(
            &committed_operation,
            "ensured lane marker publication",
            (|| {
                fail_lane_association_if_requested("ensure_marker")?;
                self.publish_lane_marker_if_materialized(lane)
            })(),
        )?;
        Ok(LaneWorkdirReport {
            lane_id: branch.lane_id,
            workdir: Some(dir.to_string_lossy().to_string()),
            requested_workdir_mode: LaneWorkdirMode::Auto,
            workdir_mode: outcome.resolved_mode,
            workdir_backend: Some(outcome.backend),
            materialization: Some(outcome.report),
            sparse_paths: Vec::new(),
            transparent_cow_available: false,
        })
    }

    pub(crate) fn materialize_lane_workdir_at_paths_with_neighbors(
        &self,
        root_id: &ObjectId,
        dir: &Path,
        custom_workdir: bool,
        sparse_paths: &[String],
        include_neighbors: bool,
    ) -> Result<(Option<MaterializationReport>, Option<String>)> {
        if sparse_paths.is_empty() {
            let outcome = self.materialize_lane_root_staged(
                root_id,
                dir,
                custom_workdir,
                MaterializationPolicy::Auto,
            )?;
            return Ok((None, Some(outcome.materialization_operation_id)));
        }
        let files = if include_neighbors {
            self.load_root_files_for_selections_with_neighbors(root_id, sparse_paths)?
        } else {
            self.load_root_files_for_selections(root_id, sparse_paths)?
        };
        let outcome =
            self.materialize_sparse_lane_root_staged(root_id, dir, custom_workdir, &files)?;
        Ok((
            Some(outcome.report),
            Some(outcome.materialization_operation_id),
        ))
    }

    pub(crate) fn sparse_workdir_paths(&self, dir: &Path) -> Result<Option<Vec<String>>> {
        let manifest = sparse_workdir_manifest_path(dir);
        if !manifest.exists() {
            return Ok(None);
        }
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&manifest)?)?;
        let Some(paths) = value
            .get("materialized_paths")
            .and_then(serde_json::Value::as_array)
        else {
            return Err(Error::Corrupt(format!(
                "invalid sparse workdir manifest `{}`",
                manifest.display()
            )));
        };
        let mut normalized = BTreeSet::new();
        for path in paths {
            let Some(path) = path.as_str() else {
                return Err(Error::Corrupt(format!(
                    "invalid sparse workdir manifest path in `{}`",
                    manifest.display()
                )));
            };
            normalized.insert(normalize_relative_path(path)?);
        }
        Ok(Some(normalized.into_iter().collect()))
    }

    pub(crate) fn lane_sparse_workdir_paths(
        &self,
        branch: &LaneBranch,
        dir: &Path,
    ) -> Result<Option<Vec<String>>> {
        if let Some(paths) = self.sparse_workdir_paths(dir)? {
            return Ok(Some(paths));
        }
        self.lane_sparse_paths_from_metadata(&branch.lane_id)
    }

    pub(crate) fn lane_workdir_mode_for(
        &self,
        record: &LaneRecord,
        branch: &LaneBranch,
    ) -> Result<LaneWorkdirMode> {
        if let Some(metadata_json) = &record.metadata_json {
            let value: serde_json::Value = serde_json::from_str(metadata_json)?;
            if let Some(mode) = value
                .get("workdir_mode")
                .and_then(serde_json::Value::as_str)
            {
                return parse_lane_workdir_mode(mode);
            }
            if value
                .get("sparse_paths")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|paths| !paths.is_empty())
            {
                return Ok(LaneWorkdirMode::Sparse);
            }
        }
        if branch.workdir.is_some() {
            Ok(LaneWorkdirMode::NativeCow)
        } else {
            Ok(LaneWorkdirMode::Virtual)
        }
    }

    pub(crate) fn lane_requested_workdir_mode_for(
        &self,
        record: &LaneRecord,
        branch: &LaneBranch,
    ) -> Result<LaneWorkdirMode> {
        if let Some(metadata_json) = &record.metadata_json {
            let value: serde_json::Value = serde_json::from_str(metadata_json)?;
            if let Some(mode) = value
                .get("requested_workdir_mode")
                .and_then(serde_json::Value::as_str)
            {
                return parse_lane_workdir_mode(mode);
            }
        }
        self.lane_workdir_mode_for(record, branch)
    }

    pub(crate) fn lane_workdir_backend_for(
        &self,
        record: &LaneRecord,
    ) -> Result<Option<WorkdirBackend>> {
        let Some(metadata_json) = &record.metadata_json else {
            return Ok(None);
        };
        let value: serde_json::Value = serde_json::from_str(metadata_json)?;
        let Some(backend) = value.get("workdir_backend") else {
            return Ok(None);
        };
        serde_json::from_value(backend.clone())
            .map(Some)
            .map_err(Error::Json)
    }

    pub(crate) fn lane_materialization_report_for(
        &self,
        record: &LaneRecord,
    ) -> Result<Option<MaterializationReport>> {
        let Some(metadata_json) = &record.metadata_json else {
            return Ok(None);
        };
        let value: serde_json::Value = serde_json::from_str(metadata_json)?;
        let Some(report) = value.get("materialization") else {
            return Ok(None);
        };
        if report.is_null() {
            return Ok(None);
        }
        serde_json::from_value(report.clone())
            .map(Some)
            .map_err(Error::Json)
    }

    pub(crate) fn update_lane_materialization_metadata(
        &self,
        lane_id: &str,
        requested_mode: &LaneWorkdirMode,
        outcome: &MaterializationOutcome,
    ) -> Result<()> {
        let existing = self
            .conn
            .query_row(
                "SELECT metadata_json FROM lanes WHERE lane_id = ?1",
                params![lane_id],
                |row| row.get::<_, Option<String>>(0),
            )?
            .unwrap_or_else(|| "{}".to_string());
        let mut value: serde_json::Value = serde_json::from_str(&existing)?;
        let object = value.as_object_mut().ok_or_else(|| {
            Error::Corrupt(format!("lane `{lane_id}` metadata is not a JSON object"))
        })?;
        object.insert(
            "requested_workdir_mode".to_string(),
            serde_json::json!(requested_mode.as_str()),
        );
        object.insert(
            "workdir_mode".to_string(),
            serde_json::json!(outcome.resolved_mode.as_str()),
        );
        object.insert(
            "workdir_backend".to_string(),
            serde_json::json!(outcome.backend.as_str()),
        );
        object.remove("cow_backend");
        object.insert(
            "materialization".to_string(),
            serde_json::to_value(&outcome.report)?,
        );
        self.conn.execute(
            "UPDATE lanes SET metadata_json = ?1 WHERE lane_id = ?2",
            params![serde_json::to_string(&value)?, lane_id],
        )?;
        Ok(())
    }

    pub(crate) fn lane_report_sparse_paths(&self, branch: &LaneBranch) -> Result<Vec<String>> {
        if let Some(workdir) = &branch.workdir {
            if let Some(paths) = self.lane_sparse_workdir_paths(branch, Path::new(workdir))? {
                return Ok(paths);
            }
        }
        Ok(self
            .lane_sparse_paths_from_metadata(&branch.lane_id)?
            .unwrap_or_default())
    }

    pub(crate) fn lane_sparse_paths_from_metadata(
        &self,
        lane_id: &str,
    ) -> Result<Option<Vec<String>>> {
        let metadata_json = self
            .conn
            .query_row(
                "SELECT metadata_json FROM lanes WHERE lane_id = ?1",
                params![lane_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        let Some(metadata_json) = metadata_json else {
            return Ok(None);
        };
        let value: serde_json::Value = serde_json::from_str(&metadata_json)?;
        let Some(paths) = value.get("sparse_paths") else {
            return Ok(None);
        };
        let Some(paths) = paths.as_array() else {
            return Err(Error::Corrupt(format!(
                "invalid sparse path metadata for lane `{lane_id}`"
            )));
        };
        let mut normalized = BTreeSet::new();
        for path in paths {
            let Some(path) = path.as_str() else {
                return Err(Error::Corrupt(format!(
                    "invalid sparse path metadata entry for lane `{lane_id}`"
                )));
            };
            normalized.insert(normalize_relative_path(path)?);
        }
        if normalized.is_empty() {
            return Ok(None);
        }
        Ok(Some(normalized.into_iter().collect()))
    }

    pub(crate) fn write_sparse_workdir_manifest<'a, I>(&self, dir: &Path, paths: I) -> Result<()>
    where
        I: IntoIterator<Item = &'a String>,
    {
        let manifest = sparse_workdir_manifest_path(dir);
        let parent = manifest.parent().ok_or_else(|| Error::InvalidPath {
            path: manifest.to_string_lossy().to_string(),
            reason: "sparse manifest has no parent".to_string(),
        })?;
        fs::create_dir_all(parent)?;
        let mut normalized = BTreeSet::new();
        for path in paths {
            normalized.insert(normalize_relative_path(path)?);
        }
        let body = serde_json::json!({
            "version": 1,
            "materialized_paths": normalized.into_iter().collect::<Vec<_>>()
        });
        #[cfg(debug_assertions)]
        fail_sparse_selection_write_if_requested()?;
        write_file_atomic(&manifest, &serde_json::to_vec(&body)?, true)?;
        Ok(())
    }

    pub(crate) fn selected_file_entries(
        &self,
        files: &BTreeMap<String, FileEntry>,
        selected_paths: &[String],
    ) -> BTreeMap<String, FileEntry> {
        selected_file_entries(files, selected_paths)
    }

    pub(crate) fn resolve_lane_workdir_path(
        &self,
        name: &str,
        custom_workdir: Option<&Path>,
    ) -> Result<PathBuf> {
        let raw = match custom_workdir {
            Some(path) if path.is_absolute() => path.to_path_buf(),
            Some(path) => self.workspace_root.join(path),
            None => self.default_lane_workdir_path(name)?,
        };
        let normalized = normalize_workdir_path(&raw)?;
        let normalized = canonicalize_existing_workdir_prefix(&normalized)?;
        self.validate_lane_workdir_path(&normalized)?;
        Ok(normalized)
    }

    pub(crate) fn default_lane_workdir_path(&self, name: &str) -> Result<PathBuf> {
        Ok(self.default_lane_worktrees_base()?.join(name))
    }

    pub(crate) fn default_lane_worktrees_base(&self) -> Result<PathBuf> {
        let rel = normalize_relative_path(&self.config.lane.worktrees_dir)?;
        normalize_workdir_path(&self.workspace_root.join(path_from_rel(&rel)))
    }

    pub(crate) fn validate_lane_workdir_path(&self, path: &Path) -> Result<()> {
        if path == self.workspace_root {
            return Err(Error::InvalidPath {
                path: path.to_string_lossy().to_string(),
                reason: "lane workdir cannot be the workspace root".to_string(),
            });
        }
        let worktrees_base = self.default_lane_worktrees_base()?;
        if path == worktrees_base {
            return Err(Error::InvalidPath {
                path: path.to_string_lossy().to_string(),
                reason: "lane workdir must include a lane-specific directory".to_string(),
            });
        }
        if path.starts_with(&self.workspace_root) && !path.starts_with(&worktrees_base) {
            return Err(Error::InvalidPath {
                path: path.to_string_lossy().to_string(),
                reason: format!(
                    "lane workdirs inside the workspace must live under `{}`",
                    worktrees_base.display()
                ),
            });
        }
        if let Ok(metadata) = fs::symlink_metadata(path) {
            if metadata.file_type().is_symlink() {
                return Err(Error::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                    reason: "lane workdir cannot be a symlink".to_string(),
                });
            }
        }
        Ok(())
    }

    pub(crate) fn resolve_checkout_workdir_path(&self, workdir: &Path) -> Result<PathBuf> {
        let raw = if workdir.is_absolute() {
            workdir.to_path_buf()
        } else {
            self.workspace_root.join(workdir)
        };
        let normalized = normalize_workdir_path(&raw)?;
        let normalized = canonicalize_existing_workdir_prefix(&normalized)?;
        let workspace = self.workspace_root.canonicalize()?;
        if normalized == workspace {
            return Err(Error::InvalidPath {
                path: normalized.to_string_lossy().to_string(),
                reason: "checkout workdir cannot be the workspace root".to_string(),
            });
        }
        if normalized.starts_with(&workspace) {
            let db_dir = self.db_dir.canonicalize()?;
            if !normalized.starts_with(&db_dir) {
                return Err(Error::InvalidPath {
                    path: normalized.to_string_lossy().to_string(),
                    reason: format!(
                        "checkout workdir inside the workspace must live under `{}`",
                        db_dir.display()
                    ),
                });
            }
        }
        Ok(normalized)
    }
}

fn parse_lane_workdir_mode(value: &str) -> Result<LaneWorkdirMode> {
    match value {
        "overlay-cow" | "overlay_cow" => {
            return Err(Error::InvalidInput(
                "unsupported lane workdir mode `overlay-cow`; this build uses the hard-cutover modes `fuse-cow` and `dokan-cow`; remove and recreate the lane with the platform-appropriate mode"
                    .to_string(),
            ));
        }
        "full-cow" | "full_cow" => {
            return Err(Error::InvalidInput(
                "unsupported lane workdir mode `full-cow`; this mode was renamed to `native-cow` to describe filesystem-native clone/reflink materialization; remove and recreate the lane with `native-cow`"
                    .to_string(),
            ));
        }
        _ => {}
    }
    LaneWorkdirMode::parse(value).ok_or_else(|| {
        Error::InvalidInput(format!(
            "unknown lane workdir mode `{value}`; expected auto, virtual, sparse, native-cow, portable-copy, fuse-cow, nfs-cow, or dokan-cow"
        ))
    })
}

fn platform_workspace_backend(mode: &LaneWorkdirMode) -> &'static str {
    match mode {
        LaneWorkdirMode::NfsCow => "nfs",
        LaneWorkdirMode::FuseCow => "fuse",
        LaneWorkdirMode::DokanCow => "dokan",
        LaneWorkdirMode::Auto
        | LaneWorkdirMode::Sparse
        | LaneWorkdirMode::NativeCow
        | LaneWorkdirMode::PortableCopy => "clone",
        LaneWorkdirMode::Virtual => "virtual",
    }
}

fn validate_lane_workdir_mode_request(
    mode: &LaneWorkdirMode,
    custom_workdir: bool,
    sparse_paths: &[String],
) -> Result<()> {
    match mode {
        LaneWorkdirMode::Auto | LaneWorkdirMode::PortableCopy => {
            if !sparse_paths.is_empty() {
                return Err(Error::InvalidInput(format!(
                    "{} lane workdir mode cannot be combined with sparse paths",
                    mode.as_str()
                )));
            }
        }
        LaneWorkdirMode::Virtual => {
            if custom_workdir {
                return Err(Error::InvalidInput(
                    "custom lane workdir requires materialization to be enabled".to_string(),
                ));
            }
            if !sparse_paths.is_empty() {
                return Err(Error::InvalidInput(
                    "sparse lane workdir paths require materialization to be enabled".to_string(),
                ));
            }
        }
        LaneWorkdirMode::Sparse => {
            if sparse_paths.is_empty() {
                return Err(Error::InvalidInput(
                    "sparse lane workdir mode requires at least one --paths entry".to_string(),
                ));
            }
        }
        LaneWorkdirMode::NativeCow => {
            if !sparse_paths.is_empty() {
                return Err(Error::InvalidInput(
                    "native-cow lane workdir mode cannot be combined with sparse paths".to_string(),
                ));
            }
        }
        LaneWorkdirMode::FuseCow => {
            if !sparse_paths.is_empty() {
                return Err(Error::InvalidInput(
                    "fuse-cow lane workdir mode cannot be combined with sparse paths".to_string(),
                ));
            }
            #[cfg(not(any(target_os = "linux", all(target_os = "macos", feature = "macfuse"))))]
            return Err(Error::InvalidInput(
                "fuse-cow workdirs require Linux FUSE or a macOS build with --features macfuse"
                    .to_string(),
            ));
        }
        LaneWorkdirMode::DokanCow => {
            if !sparse_paths.is_empty() {
                return Err(Error::InvalidInput(
                    "dokan-cow lane workdir mode cannot be combined with sparse paths".to_string(),
                ));
            }
            #[cfg(not(target_os = "windows"))]
            return Err(Error::InvalidInput(
                "dokan-cow workdirs are currently supported only on Windows".to_string(),
            ));
        }
        LaneWorkdirMode::NfsCow => {
            if !sparse_paths.is_empty() {
                return Err(Error::InvalidInput(
                    "nfs-cow lane workdir mode cannot be combined with sparse paths".to_string(),
                ));
            }
            #[cfg(not(target_os = "macos"))]
            return Err(Error::InvalidInput(
                "nfs-cow workdirs are currently supported only on macOS".to_string(),
            ));
        }
    }
    Ok(())
}

pub(crate) fn selected_file_entries(
    files: &BTreeMap<String, FileEntry>,
    selected_paths: &[String],
) -> BTreeMap<String, FileEntry> {
    files
        .iter()
        .filter(|(path, _)| {
            selected_paths
                .iter()
                .any(|selected| path_matches_selection(path, selected))
        })
        .map(|(path, entry)| (path.clone(), entry.clone()))
        .collect()
}

fn sparse_workdir_manifest_path(dir: &Path) -> PathBuf {
    dir.join(".trail").join("sparse-selection.json")
}

#[cfg(test)]
mod hard_cutover_tests {
    use super::*;

    static AUTHORITY_TEST: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

    struct AuthorityReset;

    impl Drop for AuthorityReset {
        fn drop(&mut self) {
            crate::db::set_command_authority_override(false);
        }
    }

    fn initialized_trail() -> (tempfile::TempDir, Trail) {
        let workspace = tempfile::tempdir().unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        (workspace, db)
    }

    fn assert_lane_association_absent(db: &Trail, name: &str) {
        assert!(db.try_get_ref(&lane_ref(name)).unwrap().is_none());
        let lane_count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM lanes WHERE name=?1", [name], |row| {
                row.get(0)
            })
            .unwrap();
        let branch_count: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM lane_branches WHERE ref_name=?1",
                [lane_ref(name)],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!((lane_count, branch_count), (0, 0));
    }

    fn assert_lane_association_present(db: &Trail, name: &str) {
        assert!(db.try_get_ref(&lane_ref(name)).unwrap().is_some());
        assert!(db.lane_branch(name).is_ok());
    }

    fn materialization_journal_count(db: &Trail) -> usize {
        let journal = db.db_dir.join("materialization-operations");
        if !journal.is_dir() {
            return 0;
        }
        fs::read_dir(journal)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
            .count()
    }

    #[cfg(unix)]
    #[test]
    fn controlled_lane_prepare_is_marker_free_but_ordinary_prepare_repairs_marker() {
        use std::os::unix::fs::MetadataExt;

        let (_workspace, mut db) = initialized_trail();
        let spawned = db
            .spawn_lane("marker-free-prepare", Some("main"), true, None, None)
            .unwrap();
        let workdir = PathBuf::from(spawned.workdir.unwrap());
        let marker = workdir.join(".trail/workdir-manifest.json");
        fs::remove_file(&marker).unwrap();

        crate::db::change_ledger::prepare_materialized_lane_controlled_projection(
            &mut db,
            "marker-free-prepare",
        )
        .unwrap();
        assert!(
            !marker.exists(),
            "new controlled daemon preparation wrote its watched marker"
        );

        crate::db::change_ledger::prepare_materialized_lane_daemon(
            &db,
            "marker-free-prepare",
            true,
        )
        .unwrap();
        let ordinary_marker_inode = fs::metadata(&marker).unwrap().ino();

        crate::db::change_ledger::prepare_materialized_lane_controlled_projection(
            &mut db,
            "marker-free-prepare",
        )
        .unwrap();
        assert_eq!(
            fs::metadata(&marker).unwrap().ino(),
            ordinary_marker_inode,
            "existing controlled daemon preparation rewrote its watched marker"
        );
    }

    #[test]
    fn repeated_authoritative_materialized_spawn_and_record_setup_has_no_transient_repair() {
        let _guard = AUTHORITY_TEST
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _reset = AuthorityReset;

        for index in 0..4 {
            crate::db::set_command_authority_override(false);
            let workspace = tempfile::tempdir().unwrap();
            fs::write(workspace.path().join("README.md"), "base\n").unwrap();
            Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let mut db = Trail::open(workspace.path()).unwrap();
            crate::db::set_command_authority_override(true);
            let lane = format!("repeated-authority-{index}");
            let spawned = db
                .spawn_lane(&lane, Some("main"), true, None, None)
                .unwrap_or_else(|error| panic!("materialized spawn {index} failed: {error}"));
            let workdir = PathBuf::from(spawned.workdir.unwrap());
            fs::write(
                workdir.join("README.md"),
                format!("recorded lane contents {index}\n"),
            )
            .unwrap();
            db.record_lane_workdir(&lane, Some(format!("record setup {index}")))
                .unwrap_or_else(|error| panic!("materialized record {index} failed: {error}"));
        }
    }

    #[test]
    fn removed_cow_mode_reports_the_recreate_lifecycle() {
        let overlay_error = parse_lane_workdir_mode("overlay-cow").unwrap_err();
        let overlay_message = overlay_error.to_string();
        assert!(overlay_message.contains("hard-cutover modes `fuse-cow` and `dokan-cow`"));
        assert!(overlay_message.contains("remove and recreate the lane"));

        let native_error = parse_lane_workdir_mode("full-cow").unwrap_err();
        let native_message = native_error.to_string();
        assert!(native_message.contains("renamed to `native-cow`"));
        assert!(native_message.contains("remove and recreate the lane"));
    }

    #[test]
    fn lane_spawn_sql_association_rolls_back_at_every_boundary() {
        for boundary in ["spawn_after_ref", "spawn_after_lane", "spawn_after_branch"] {
            let (_workspace, mut db) = initialized_trail();
            set_lane_association_failure_for_current_thread(Some(boundary));
            let result = db.spawn_lane("atomic-spawn", Some("main"), false, None, None);
            set_lane_association_failure_for_current_thread(None);
            assert!(result.is_err(), "boundary {boundary} did not fail");
            assert_lane_association_absent(&db, "atomic-spawn");
        }
    }

    #[test]
    fn sparse_lane_spawn_rolls_back_publication_and_journal_at_every_sql_boundary() {
        for boundary in ["spawn_after_ref", "spawn_after_lane", "spawn_after_branch"] {
            let workspace = tempfile::tempdir().unwrap();
            fs::write(workspace.path().join("README.md"), "root contents").unwrap();
            Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let mut db = Trail::open(workspace.path()).unwrap();
            let destination = workspace.path().join(format!("sparse-{boundary}"));
            set_lane_association_failure_for_current_thread(Some(boundary));
            let result = db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                "atomic-sparse",
                Some("main"),
                LaneWorkdirMode::Sparse,
                None,
                None,
                Some(destination.clone()),
                &["README.md".to_string()],
                false,
            );
            set_lane_association_failure_for_current_thread(None);
            assert!(result.is_err(), "boundary {boundary} did not fail");
            assert_lane_association_absent(&db, "atomic-sparse");
            assert!(!destination.exists());
            let journal_dir = db.db_dir.join("materialization-operations");
            assert!(
                !journal_dir.exists()
                    || fs::read_dir(&journal_dir)
                        .unwrap()
                        .filter_map(std::result::Result::ok)
                        .all(
                            |entry| entry.path().extension().and_then(|ext| ext.to_str())
                                != Some("json")
                        )
            );
            drop(db);
            Trail::open(workspace.path()).unwrap();
            assert!(!destination.exists());
        }
    }

    #[test]
    fn turn_lane_spawn_sql_association_rolls_back_at_every_boundary() {
        for boundary in ["turn_after_ref", "turn_after_lane", "turn_after_branch"] {
            let (_workspace, mut db) = initialized_trail();
            set_lane_association_failure_for_current_thread(Some(boundary));
            let result = db.lane_branch_for_turn("atomic-turn", Some("main"), None);
            set_lane_association_failure_for_current_thread(None);
            assert!(result.is_err(), "boundary {boundary} did not fail");
            assert_lane_association_absent(&db, "atomic-turn");
        }
    }

    #[test]
    fn lane_ensure_sql_association_rolls_back_at_every_boundary() {
        for boundary in ["ensure_after_lane_metadata", "ensure_after_branch"] {
            let (workspace, mut db) = initialized_trail();
            db.spawn_lane("atomic-ensure", Some("main"), false, None, None)
                .unwrap();
            let before = db.lane_record("atomic-ensure").unwrap().metadata_json;
            let destination = workspace.path().join(format!("ensure-{boundary}"));
            set_lane_association_failure_for_current_thread(Some(boundary));
            let result =
                db.ensure_lane_workdir_materialized("atomic-ensure", Some(destination.clone()));
            set_lane_association_failure_for_current_thread(None);
            assert!(result.is_err(), "boundary {boundary} did not fail");
            let branch = db.lane_branch("atomic-ensure").unwrap();
            assert!(branch.workdir.is_none());
            assert_eq!(
                db.lane_record("atomic-ensure").unwrap().metadata_json,
                before
            );
            assert!(!destination.exists());
            assert_eq!(materialization_journal_count(&db), 0);
            drop(db);
            Trail::open(workspace.path()).unwrap();
            assert!(!destination.exists());
        }
    }

    #[test]
    fn materialized_turn_spawn_rolls_back_owned_publication_at_every_boundary() {
        for boundary in ["turn_after_ref", "turn_after_lane", "turn_after_branch"] {
            let (_workspace, mut db) = initialized_trail();
            db.config_set("lane.default_materialize", "true").unwrap();
            let destination = db
                .default_lane_workdir_path("atomic-materialized-turn")
                .unwrap();
            set_lane_association_failure_for_current_thread(Some(boundary));
            let result = db.lane_branch_for_turn("atomic-materialized-turn", Some("main"), None);
            set_lane_association_failure_for_current_thread(None);
            assert!(result.is_err(), "boundary {boundary} did not fail");
            assert_lane_association_absent(&db, "atomic-materialized-turn");
            assert_eq!(materialization_journal_count(&db), 0);
            assert!(!destination.exists());
        }
    }

    #[test]
    fn post_commit_lane_failures_are_distinct_from_rolled_back_publication() {
        let (_workspace, mut db) = initialized_trail();
        set_lane_association_failure_for_current_thread(Some("spawn_after_commit"));
        let spawn = db.spawn_lane("committed-spawn", Some("main"), false, None, None);
        set_lane_association_failure_for_current_thread(None);
        assert!(matches!(
            spawn,
            Err(Error::CommittedRepairRequired { .. })
                | Err(Error::OperationCommittedRepairRequired { .. })
        ));
        assert_lane_association_present(&db, "committed-spawn");

        set_lane_association_failure_for_current_thread(Some("turn_after_commit"));
        let turn = db.lane_branch_for_turn("committed-turn", Some("main"), None);
        set_lane_association_failure_for_current_thread(None);
        assert!(matches!(
            turn,
            Err(Error::OperationCommittedRepairRequired { .. })
        ));
        assert_lane_association_present(&db, "committed-turn");

        db.spawn_lane("committed-ensure", Some("main"), false, None, None)
            .unwrap();
        set_lane_association_failure_for_current_thread(Some("ensure_after_commit"));
        let ensure = db.ensure_lane_workdir_materialized("committed-ensure", None);
        set_lane_association_failure_for_current_thread(None);
        assert!(matches!(
            ensure,
            Err(Error::OperationCommittedRepairRequired { .. })
        ));
        assert!(db
            .lane_branch("committed-ensure")
            .unwrap()
            .workdir
            .is_some());
    }

    #[test]
    fn all_post_commit_lane_steps_preserve_committed_repair_semantics() {
        for boundary in ["spawn_ref_repair", "spawn_event"] {
            let (_workspace, mut db) = initialized_trail();
            set_lane_association_failure_for_current_thread(Some(boundary));
            let result = db.spawn_lane("committed-spawn", Some("main"), false, None, None);
            set_lane_association_failure_for_current_thread(None);
            assert!(
                matches!(
                    result,
                    Err(Error::CommittedRepairRequired { .. })
                        | Err(Error::OperationCommittedRepairRequired { .. })
                ),
                "boundary {boundary} returned {result:?}"
            );
            assert_lane_association_present(&db, "committed-spawn");
        }

        for boundary in ["spawn_journal_completion", "spawn_marker"] {
            let (_workspace, mut db) = initialized_trail();
            set_lane_association_failure_for_current_thread(Some(boundary));
            let result = db.spawn_lane("committed-spawn", Some("main"), true, None, None);
            set_lane_association_failure_for_current_thread(None);
            assert!(
                matches!(
                    result,
                    Err(Error::CommittedRepairRequired { .. })
                        | Err(Error::OperationCommittedRepairRequired { .. })
                ),
                "boundary {boundary} returned {result:?}"
            );
            assert_lane_association_present(&db, "committed-spawn");
        }

        for boundary in ["ensure_journal_completion", "ensure_event", "ensure_marker"] {
            let (_workspace, mut db) = initialized_trail();
            db.spawn_lane("committed-ensure", Some("main"), false, None, None)
                .unwrap();
            set_lane_association_failure_for_current_thread(Some(boundary));
            let result = db.ensure_lane_workdir_materialized("committed-ensure", None);
            set_lane_association_failure_for_current_thread(None);
            assert!(matches!(
                result,
                Err(Error::OperationCommittedRepairRequired { .. })
            ));
            assert!(db
                .lane_branch("committed-ensure")
                .unwrap()
                .workdir
                .is_some());
        }

        for boundary in ["turn_ref_repair", "turn_event"] {
            let (_workspace, mut db) = initialized_trail();
            set_lane_association_failure_for_current_thread(Some(boundary));
            let result = db.lane_branch_for_turn("committed-turn", Some("main"), None);
            set_lane_association_failure_for_current_thread(None);
            assert!(
                matches!(result, Err(Error::OperationCommittedRepairRequired { .. })),
                "boundary {boundary} returned {result:?}"
            );
            assert_lane_association_present(&db, "committed-turn");
        }

        for boundary in ["turn_journal_completion", "turn_marker"] {
            let (_workspace, mut db) = initialized_trail();
            db.config_set("lane.default_materialize", "true").unwrap();
            set_lane_association_failure_for_current_thread(Some(boundary));
            let result = db.lane_branch_for_turn("committed-turn", Some("main"), None);
            set_lane_association_failure_for_current_thread(None);
            assert!(matches!(
                result,
                Err(Error::OperationCommittedRepairRequired { .. })
            ));
            assert_lane_association_present(&db, "committed-turn");
        }

        for repair in [
            "journal completion",
            "marker publication",
            "workspace view publication",
            "event publication",
            "ref mirror repair",
        ] {
            let result: Result<()> = committed_lane_step(
                "operation_test",
                repair,
                Err(Error::InvalidInput("injected post-commit failure".into())),
            );
            assert!(matches!(
                result,
                Err(Error::OperationCommittedRepairRequired { .. })
            ));
        }
    }
}
