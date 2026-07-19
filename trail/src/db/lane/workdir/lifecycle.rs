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
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(lane)?;
        let branch = self.lane_branch(lane)?;
        let preserved_view = self.lane_workspace_view(lane)?;
        if let Some(view) = &preserved_view
            && let (Some(pid), Some(token)) = (view.owner_pid, view.owner_start_token.as_deref())
            && process_matches_start_token(pid, token)
        {
            return Err(Error::InvalidInput(format!(
                        "lane `{lane}` has an active workspace writer in process {pid}; unmount or stop it before removal"
                    )));
        }
        let preserved_space = preserved_view
            .as_ref()
            .map(|_| self.lane_workspace_space(lane))
            .transpose()?;
        if branch.status != "merged" && branch.head_change != branch.base_change && !force {
            return Err(Error::InvalidInput(format!(
                "lane `{lane}` has unmerged changes; pass --force to remove"
            )));
        }
        let mut owners = vec![branch.lane_id.as_str(), lane];
        if let Some(view) = &preserved_view {
            owners.push(view.view_id.as_str());
        }
        let roots = branch.workdir.as_deref().into_iter().collect::<Vec<_>>();
        let retired_segments = retire_deletion_scopes(
            &self.conn,
            &self.sqlite_path,
            &owners,
            &roots,
            &[branch.ref_name.as_str()],
        )?;
        remove_retired_segments(&self.conn, &retired_segments)?;
        remove_ref_file(&self.db_dir, &branch.ref_name)?;
        if let Some(workdir) = &branch.workdir {
            let path = PathBuf::from(workdir);
            if path.exists() {
                fs::remove_dir_all(&path)?;
            }
        }
        for backend in ["fuse-cow", "nfs-cow", "dokan-cow"] {
            let state = self.db_dir.join(backend).join(lane);
            if state.exists() {
                fs::remove_dir_all(state)?;
            }
        }
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
                "preserved_view_id": preserved_view.as_ref().map(|view| view.view_id.as_str()),
                "preserved_source_bytes": preserved_space.as_ref().map(|space| space.uncheckpointed_source_bytes),
                "preserved_generated_bytes": preserved_space.as_ref().map(|space| space.generated_upper_bytes),
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
            Ok(())
        })();
        match removal {
            Ok(()) => self.conn.execute_batch("COMMIT;")?,
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                return Err(error);
            }
        }
        Ok(LaneRemoveReport {
            lane_id: branch.lane_id,
            ref_name: branch.ref_name,
            removed_workdir: branch.workdir,
            forced: force,
        })
    }
}
