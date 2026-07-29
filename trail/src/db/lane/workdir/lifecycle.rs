use super::*;
use crate::db::change_ledger::{remove_retired_segments, retire_deletion_scopes};
use crate::db::lane::initialization::lane_initialization_record;

pub(crate) struct NativeCowSpaceContext {
    pub(crate) initialization_id: String,
    pub(crate) workdir: PathBuf,
    pub(crate) backend: String,
    pub(crate) clone_count: u64,
}

impl Trail {
    pub(crate) fn native_cow_space_context(&self, lane: &str) -> Result<NativeCowSpaceContext> {
        let branch = self.lane_branch(lane)?;
        let record = self.lane_record(&branch.lane_id)?;
        let mode = self.lane_workdir_mode_for(&record, &branch)?;
        if mode != LaneWorkdirMode::NativeCow {
            return Err(Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered or native-COW workspace"
            )));
        }
        let backend = self.lane_workdir_backend_for(&record)?.ok_or_else(|| {
            Error::Corrupt(format!(
                "native-COW lane `{lane}` has no durable workdir backend"
            ))
        })?;
        if backend != WorkdirBackend::Clone {
            return Err(Error::Corrupt(format!(
                "native-COW lane `{lane}` has incompatible durable backend `{}`",
                backend.as_str()
            )));
        }
        let initialization = lane_initialization_record(&self.conn, lane)?.ok_or_else(|| {
            Error::Corrupt(format!(
                "native-COW lane `{lane}` has no durable initialization"
            ))
        })?;
        let encoded = initialization
            .materialization_json
            .as_deref()
            .ok_or_else(|| {
                Error::Corrupt(format!(
                    "native-COW lane `{lane}` has no durable materialization report"
                ))
            })?;
        let materialization: MaterializationReport = serde_json::from_str(encoded)?;
        if materialization.copied_files != 0 {
            return Err(Error::Corrupt(format!(
                "native-COW lane `{lane}` durable materialization contains copied files"
            )));
        }
        let workdir = branch
            .workdir
            .map(PathBuf::from)
            .ok_or_else(|| Error::Corrupt(format!("native-COW lane `{lane}` has no workdir")))?;
        if initialization.workdir.as_ref() != Some(&workdir) {
            return Err(Error::Corrupt(format!(
                "native-COW lane `{lane}` workdir does not match its durable initialization"
            )));
        }
        Ok(NativeCowSpaceContext {
            initialization_id: initialization.initialization_id,
            workdir,
            backend: mode.as_str().to_string(),
            clone_count: materialization.cloned_files,
        })
    }

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
        let lane_id = self.lane_branch(lane).ok().map(|branch| branch.lane_id);
        match self.remove_lane_inner(lane, force) {
            Ok(report) => Ok(report),
            Err(Error::RefNotFound(_)) => {
                let Some(retirement) = self.lane_retirement(lane)? else {
                    return Err(Error::RefNotFound(format!("refs/lanes/{lane}")));
                };
                if retirement.phase != LaneRetirementPhase::Completed {
                    return Err(Error::RefNotFound(format!("refs/lanes/{lane}")));
                }
                Ok(LaneRemoveReport {
                    lane_id: retirement.lane_id,
                    ref_name: retirement.provenance.ref_name,
                    removed_workdir: None,
                    forced: retirement.forced,
                })
            }
            Err(error) => {
                if let Some(lane_id) = lane_id {
                    let _ = self.mark_lane_retirement_repair_required(&lane_id, &error);
                }
                Err(error)
            }
        }
    }

    fn remove_lane_inner(&mut self, lane: &str, force: bool) -> Result<LaneRemoveReport> {
        let ledger_authority = crate::db::change_ledger::command_authority_enabled();
        let mut write_lock = Some(self.acquire_write_lock()?);
        validate_ref_segment(lane)?;
        let branch = self.lane_branch(lane)?;
        let mut existing_retirement = self.lane_retirement(&branch.lane_id)?;
        if let Some(retirement) = existing_retirement.as_ref()
            && retirement.phase == LaneRetirementPhase::RepairRequired
        {
            let resume_phase = retirement.resume_phase.ok_or_else(|| {
                Error::Corrupt(format!(
                    "repair-required retirement `{}` has no resume phase",
                    retirement.retirement_id
                ))
            })?;
            let changed = self.conn.execute(
                "UPDATE lane_retirements
                 SET phase=?1,resume_phase=NULL,last_error_code=NULL,last_error_message=NULL,
                     updated_at=?2
                 WHERE retirement_id=?3 AND phase='repair_required'",
                params![
                    super::super::retirement::lane_retirement_phase_name(resume_phase),
                    now_ts(),
                    &retirement.retirement_id
                ],
            )?;
            if changed != 1 {
                return Err(Error::WorkspaceLocked(format!(
                    "lane `{lane}` retirement changed while retrying repair"
                )));
            }
            existing_retirement = self.lane_retirement(&branch.lane_id)?;
        }
        let preserved_view = self.lane_workspace_view(lane)?;
        if let Some(view) = &preserved_view
            && let (Some(pid), Some(token)) = (view.owner_pid, view.owner_start_token.as_deref())
            && process_matches_start_token(pid, token)
        {
            return Err(Error::InvalidInput(format!(
                        "lane `{lane}` has an active workspace writer in process {pid}; unmount or stop it before removal"
                    )));
        }
        let private_cleanup_started = existing_retirement.as_ref().is_some_and(|retirement| {
            matches!(
                retirement.phase,
                LaneRetirementPhase::BindingsRetired
                    | LaneRetirementPhase::PrivateDeleted
                    | LaneRetirementPhase::Completed
            )
        });
        let preserved_space = if private_cleanup_started {
            None
        } else {
            preserved_view
                .as_ref()
                .map(|_| self.lane_workspace_space(lane))
                .transpose()?
        };
        if branch.status != "merged" && branch.head_change != branch.base_change && !force {
            return Err(Error::InvalidInput(format!(
                "lane `{lane}` has unmerged changes; pass --force to remove"
            )));
        }
        let retirement_id = self.prepare_lane_removal(
            lane,
            &branch,
            preserved_view.as_ref(),
            preserved_space.as_ref(),
            force,
        )?;
        test_crash_point("lane_retirement_after_prepared");
        let resume_phase = self
            .lane_retirement(&branch.lane_id)?
            .ok_or_else(|| Error::Corrupt("prepared lane retirement disappeared".into()))?
            .phase;
        if resume_phase == LaneRetirementPhase::Prepared {
            if self.lane_retirement_has_runtime_resources(&branch.lane_id)? {
                drop(write_lock.take());
                self.stop_workspace_environment_runtime(lane)?;
                write_lock = Some(self.acquire_write_lock()?);
                if self.lane_branch(lane)?.lane_id != branch.lane_id {
                    return Err(Error::WorkspaceLocked(format!(
                        "lane `{lane}` changed while its runtime was stopping"
                    )));
                }
            }
            self.mark_lane_retirement_runtime_stopped(&retirement_id)?;
            test_crash_point("lane_retirement_after_runtime_stopped");
        }
        let mut owners = vec![branch.lane_id.as_str(), lane];
        if let Some(view) = &preserved_view {
            owners.push(view.view_id.as_str());
        }
        let roots = branch.workdir.as_deref().into_iter().collect::<Vec<_>>();
        let should_retire_bindings = matches!(
            resume_phase,
            LaneRetirementPhase::Prepared | LaneRetirementPhase::RuntimeStopped
        );
        let retired_segments = if should_retire_bindings {
            let retirement = retire_deletion_scopes(
                &self.conn,
                &self.sqlite_path,
                &owners,
                &roots,
                &[branch.ref_name.as_str()],
            );
            if ledger_authority {
                let retry_retirement = matches!(retirement, Err(Error::WorkspaceLocked(_)));
                if retry_retirement || retirement.is_ok() {
                    drop(write_lock.take());
                    crate::db::change_ledger::retire_materialized_lane_daemon(self, lane)?;
                    write_lock = Some(self.acquire_write_lock()?);
                    if self.lane_branch(lane)?.lane_id != branch.lane_id {
                        return Err(Error::WorkspaceLocked(format!(
                            "lane `{lane}` changed while its observer was shutting down"
                        )));
                    }
                }
                match retirement {
                    Err(Error::WorkspaceLocked(_)) => retire_deletion_scopes(
                        &self.conn,
                        &self.sqlite_path,
                        &owners,
                        &roots,
                        &[branch.ref_name.as_str()],
                    )?,
                    result => result?,
                }
            } else {
                retirement?
            }
        } else {
            Vec::new()
        };
        let mut held_write_lock = write_lock;
        if should_retire_bindings {
            remove_retired_segments(&self.conn, &retired_segments)?;
            self.retire_lane_environment_bindings(&retirement_id, preserved_view.as_ref())?;
            test_crash_point("lane_retirement_after_bindings_retired");
        }
        let should_delete_private = matches!(
            resume_phase,
            LaneRetirementPhase::Prepared
                | LaneRetirementPhase::RuntimeStopped
                | LaneRetirementPhase::BindingsRetired
        );
        if should_delete_private {
            if let Some(view) = preserved_view.as_ref() {
                drop(held_write_lock.take());
                self.cleanup_retired_workspace_environment_runtime_for_view(&view.view_id)?;
                held_write_lock = Some(self.acquire_write_lock()?);
                if self.lane_branch(lane)?.lane_id != branch.lane_id {
                    return Err(Error::WorkspaceLocked(format!(
                        "lane `{lane}` changed while retired runtime resources were being removed"
                    )));
                }
            }
            self.delete_lane_retirement_private_paths(&retirement_id, &branch)?;
            test_crash_point("lane_retirement_after_private_deleted");
        }
        let _write_lock = held_write_lock;
        remove_ref_file(&self.db_dir, &branch.ref_name).map_err(|error| {
            Error::Corrupt(format!(
                "lane retirement could not remove ref mirror `{}`: {error}",
                branch.ref_name
            ))
        })?;
        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let removal = (|| -> Result<()> {
            self.conn
                .execute("DELETE FROM refs WHERE name = ?1", params![branch.ref_name])?;
            let removed_at = now_ts();
            let retired_ref = format!("retired/{}/{}", branch.lane_id, removed_at);
            self.conn.execute(
                "UPDATE lane_branches
             SET status='removed',ref_name=?1,updated_at=?2 WHERE lane_id=?3",
                params![retired_ref, removed_at, branch.lane_id],
            )?;
            self.insert_lane_event(
            &branch.lane_id,
            "lane_removed",
            Some(&branch.head_change),
            None,
            &serde_json::json!({
                "ref_name": branch.ref_name.clone(),
                "forced": force,
                "retirement_id": retirement_id,
                "disposed_view_id": preserved_view.as_ref().map(|view| view.view_id.as_str()),
                "disposed_source_bytes": preserved_space.as_ref().map(|space| space.uncheckpointed_source_bytes),
                "disposed_generated_bytes": preserved_space.as_ref().map(|space| space.generated_upper_bytes),
                "disposed_scratch_bytes": preserved_space.as_ref().map(|space| space.scratch_upper_bytes),
            }),
        )?;
            self.conn.execute(
                "DELETE FROM lane_initializations WHERE lane_id=?1",
                params![branch.lane_id],
            )?;
            self.conn.execute(
                "UPDATE lanes SET name=?1 WHERE lane_id=?2",
                params![
                    format!("retired/{}/{}", lane, branch.lane_id),
                    branch.lane_id
                ],
            )?;
            self.compact_lane_retirement_in_transaction(
                &retirement_id,
                preserved_view.as_ref(),
                removed_at,
            )?;
            Ok(())
        })();
        match removal {
            Ok(()) => self.conn.execute_batch("COMMIT;")?,
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                return Err(error);
            }
        }
        test_crash_point("lane_retirement_after_completed");
        Ok(LaneRemoveReport {
            lane_id: branch.lane_id,
            ref_name: branch.ref_name,
            removed_workdir: branch.workdir,
            forced: force,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    #[test]
    fn lane_retirement_crash_helper() {
        let Some(workspace) = std::env::var_os("TRAIL_TEST_RETIREMENT_WORKSPACE") else {
            return;
        };
        let mut db = Trail::open(PathBuf::from(workspace)).unwrap();
        let _ = db.remove_lane("retirement-crash", true);
        panic!("lane retirement crash helper passed its requested crash point");
    }

    #[test]
    fn killing_lane_removal_at_every_durable_phase_recovers_to_completion() {
        for phase in [
            "lane_retirement_after_prepared",
            "lane_retirement_after_runtime_stopped",
            "lane_retirement_after_bindings_retired",
            "lane_retirement_after_private_deleted",
            "lane_retirement_after_completed",
        ] {
            let workspace = tempfile::tempdir().unwrap();
            fs::write(workspace.path().join("README.md"), b"retirement crash\n").unwrap();
            Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let mut db = Trail::open(workspace.path()).unwrap();
            let mode = if cfg!(target_os = "macos") {
                LaneWorkdirMode::NfsCow
            } else if cfg!(target_os = "windows") {
                LaneWorkdirMode::DokanCow
            } else {
                LaneWorkdirMode::FuseCow
            };
            let spawned = db
                .spawn_lane_with_workdir_mode_paths_and_neighbors(
                    "retirement-crash",
                    Some("main"),
                    mode,
                    None,
                    None,
                    None,
                    &[],
                    false,
                )
                .unwrap();
            let view = db.lane_workspace_view("retirement-crash").unwrap().unwrap();
            fs::write(
                Path::new(&view.generated_upper).join("artifact"),
                b"discard",
            )
            .unwrap();
            drop(db);

            let ready = workspace
                .path()
                .join(".trail/tmp")
                .join(format!("{phase}.ready"));
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "db::lane::workdir::lifecycle::tests::lane_retirement_crash_helper",
                    "--nocapture",
                ])
                .env("RUST_TEST_THREADS", "1")
                .env("TRAIL_TEST_CRASH_AT", phase)
                .env("TRAIL_TEST_CRASH_READY", &ready)
                .env("TRAIL_TEST_RETIREMENT_WORKSPACE", workspace.path())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while !ready.is_file() && Instant::now() < deadline {
                if child.try_wait().unwrap().is_some() {
                    panic!("retirement helper exited before `{phase}`");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(ready.is_file(), "retirement helper did not reach `{phase}`");
            child.kill().unwrap();
            let _ = child.wait().unwrap();

            let recovered = Trail::open(workspace.path())
                .unwrap_or_else(|error| panic!("recovery after `{phase}` failed: {error}"));
            let retirement = recovered
                .lane_retirement(&spawned.lane_id)
                .unwrap()
                .expect("recovery must retain compact retirement provenance");
            assert_eq!(retirement.phase, LaneRetirementPhase::Completed);
            assert!(!Path::new(&view.generated_upper).exists());
            assert!(recovered.lane_branch("retirement-crash").is_err());
        }
    }
}
