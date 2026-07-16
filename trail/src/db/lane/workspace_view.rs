use super::workdir::{ViewIntentWriter, ViewJournalCut, ViewMutationBarrier, ViewMutationJournal};
use super::*;

const VIEW_JOURNAL_FILE: &str = "mutation-journal.jsonl";
const VIEW_UNMOUNT_REQUEST_FILE: &str = "unmount-request.json";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceViewPaths {
    pub(crate) view_id: String,
    pub(crate) view_dir: PathBuf,
    pub(crate) source_upper: PathBuf,
    pub(crate) generated_upper: PathBuf,
    pub(crate) scratch_upper: PathBuf,
    pub(crate) meta_dir: PathBuf,
    pub(crate) journal_path: PathBuf,
}

pub(crate) struct WorkspaceMountLease {
    workspace_root: PathBuf,
    db_dir: PathBuf,
    view_id: String,
    owner_start_token: String,
    released: bool,
}

impl WorkspaceMountLease {
    pub(crate) fn mark_mounted(&mut self) -> Result<()> {
        let db = Trail::open_with_db_dir(&self.workspace_root, &self.db_dir)?;
        let updated = db.conn.execute(
            "UPDATE workspace_views SET status = 'mounted', heartbeat_at = ?1, updated_at = ?1 WHERE view_id = ?2 AND owner_start_token = ?3",
            params![now_ts(), self.view_id, self.owner_start_token],
        )?;
        if updated != 1 {
            return Err(Error::InvalidInput(format!(
                "workspace view `{}` mount lease was lost before the backend became ready",
                self.view_id
            )));
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn heartbeat(&self) -> Result<()> {
        let db = Trail::open_with_db_dir(&self.workspace_root, &self.db_dir)?;
        let updated = db.conn.execute(
            "UPDATE workspace_views SET heartbeat_at = ?1, updated_at = ?1 WHERE view_id = ?2 AND owner_start_token = ?3",
            params![now_ts(), self.view_id, self.owner_start_token],
        )?;
        if updated == 0 {
            return Err(Error::InvalidInput(format!(
                "workspace view `{}` mount lease is no longer owned by this process",
                self.view_id
            )));
        }
        Ok(())
    }

    fn release(&mut self) -> Result<()> {
        if self.released {
            return Ok(());
        }
        let db = Trail::open_with_db_dir(&self.workspace_root, &self.db_dir)?;
        let view = db
            .conn
            .query_row(
                "SELECT meta_dir FROM workspace_views WHERE view_id = ?1",
                params![self.view_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        db.conn.execute(
            "UPDATE workspace_views SET status = 'unmounted', owner_pid = NULL, owner_start_token = NULL, heartbeat_at = NULL, updated_at = ?1 WHERE view_id = ?2 AND owner_start_token = ?3",
            params![now_ts(), self.view_id, self.owner_start_token],
        )?;
        if let Some(meta_dir) = view {
            let _ = fs::remove_file(Path::new(&meta_dir).join("mount.json"));
        }
        self.released = true;
        Ok(())
    }
}

impl Drop for WorkspaceMountLease {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

impl Trail {
    /// Start a mount worker owned by the current long-lived Trail process.
    /// HTTP/MCP daemons use this non-blocking entry point; the worker still
    /// owns the same foreground lifecycle and durable lease as the CLI path.
    pub fn start_lane_workspace_mount(&self, lane: &str) -> Result<WorkspaceMountReport> {
        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered workspace view"
            ))
        })?;
        if let (Some(pid), Some(token)) = (view.owner_pid, view.owner_start_token.as_deref()) {
            if process_matches_start_token(pid, token) {
                return Ok(workspace_mount_report(
                    &view,
                    Some(pid),
                    Some(token.to_string()),
                    view.status == "mounted",
                ));
            }
        }
        let workspace_root = self.workspace_root.clone();
        let db_dir = self.db_dir.clone();
        let lane = lane.to_string();
        let lane_for_worker = lane.clone();
        let (finished_tx, finished_rx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = Trail::open_with_db_dir(workspace_root, db_dir)
                .and_then(|db| db.mount_lane_workspace_until_requested(&lane_for_worker))
                .map(|_| ())
                .map_err(|err| err.to_string());
            let _ = finished_tx.send(result);
        });

        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match finished_rx.try_recv() {
                Ok(Err(message)) => return Err(Error::InvalidInput(message)),
                Ok(Ok(())) => {
                    return Err(Error::InvalidInput(format!(
                        "workspace view `{}` mount worker stopped before becoming ready",
                        view.view_id
                    )));
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    return Err(Error::InvalidInput(format!(
                        "workspace view `{}` mount worker exited unexpectedly",
                        view.view_id
                    )));
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
            let current = self.lane_workspace_view(&lane)?.ok_or_else(|| {
                Error::Corrupt(format!("workspace view for lane `{lane}` disappeared"))
            })?;
            if current.status == "mounted" {
                if let (Some(pid), Some(token)) =
                    (current.owner_pid, current.owner_start_token.clone())
                {
                    return Ok(workspace_mount_report(
                        &current,
                        Some(pid),
                        Some(token),
                        true,
                    ));
                }
            }
            if Instant::now() >= deadline {
                let stop_path = Path::new(&view.meta_dir).join(VIEW_UNMOUNT_REQUEST_FILE);
                let _ = write_file_atomic(&stop_path, b"{}", false);
                return Err(Error::InvalidInput(format!(
                    "workspace view `{}` did not mount within 15 seconds",
                    view.view_id
                )));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Own a layered mount in the foreground until another Trail process asks
    /// it to unmount. Keeping the mount handle and lease in this process makes
    /// teardown deterministic across FUSE, NFS, and Dokan.
    pub fn mount_lane_workspace_until_requested(&self, lane: &str) -> Result<WorkspaceMountReport> {
        let branch = self.lane_branch(lane)?;
        let record = self.lane_record(&branch.lane_id)?;
        let mode = self.lane_workdir_mode_for(&record, &branch)?;
        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered workspace view"
            ))
        })?;
        let stop_path = Path::new(&view.meta_dir).join(VIEW_UNMOUNT_REQUEST_FILE);
        match fs::remove_file(&stop_path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }

        match mode {
            LaneWorkdirMode::FuseCow => {
                let mount = self.mount_fuse_cow_workdir_for_lane(lane)?;
                self.wait_for_workspace_unmount_request(lane, &view, &stop_path)?;
                drop(mount);
            }
            LaneWorkdirMode::NfsCow => {
                let mount = self.mount_nfs_cow_workdir_for_lane(lane)?;
                self.wait_for_workspace_unmount_request(lane, &view, &stop_path)?;
                drop(mount);
            }
            LaneWorkdirMode::DokanCow => {
                #[cfg(target_os = "windows")]
                {
                    let mount = self.mount_dokan_cow_workdir_for_lane(lane)?;
                    self.wait_for_workspace_unmount_request(lane, &view, &stop_path)?;
                    drop(mount);
                }
                #[cfg(not(target_os = "windows"))]
                return Err(Error::InvalidInput(
                    "dokan-cow workdirs are currently supported only on Windows".to_string(),
                ));
            }
            _ => {
                return Err(Error::InvalidInput(format!(
                    "lane `{lane}` uses `{}` rather than a layered workspace view",
                    mode.as_str()
                )));
            }
        }
        let _ = fs::remove_file(&stop_path);
        self.insert_lane_event(
            &branch.lane_id,
            "workspace_view_unmounted",
            None,
            None,
            &serde_json::json!({"view_id": view.view_id, "requested": true}),
        )?;
        Ok(workspace_mount_report(&view, None, None, true))
    }

    /// Ask the foreground mount owner to drop its backend handle, then wait
    /// for the lease release to become visible. This avoids force-unmounting a
    /// live writer from an unrelated process.
    pub fn request_lane_workspace_unmount(&self, lane: &str) -> Result<WorkspaceMountReport> {
        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered workspace view"
            ))
        })?;
        let active = match (view.owner_pid, view.owner_start_token.as_deref()) {
            (Some(pid), Some(token)) => process_matches_start_token(pid, token),
            _ => false,
        };
        if !active {
            return Ok(workspace_mount_report(&view, None, None, true));
        }
        let stop_path = Path::new(&view.meta_dir).join(VIEW_UNMOUNT_REQUEST_FILE);
        write_file_atomic(
            &stop_path,
            &serde_json::to_vec_pretty(&serde_json::json!({
                "view_id": view.view_id,
                "requested_at": now_ts(),
                "requester_pid": std::process::id(),
            }))?,
            false,
        )?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let current = self.lane_workspace_view(lane)?.ok_or_else(|| {
                Error::Corrupt(format!("workspace view for lane `{lane}` disappeared"))
            })?;
            if current.owner_pid.is_none() {
                return Ok(workspace_mount_report(&current, None, None, true));
            }
            if Instant::now() >= deadline {
                return Err(Error::InvalidInput(format!(
                    "workspace view `{}` did not unmount within 30 seconds; owner process {} may be stuck",
                    current.view_id,
                    current.owner_pid.unwrap_or_default()
                )));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn wait_for_workspace_unmount_request(
        &self,
        lane: &str,
        view: &LaneWorkspaceViewReport,
        stop_path: &Path,
    ) -> Result<()> {
        loop {
            if stop_path.is_file() {
                return Ok(());
            }
            let current = self.lane_workspace_view(lane)?.ok_or_else(|| {
                Error::Corrupt(format!("workspace view for lane `{lane}` disappeared"))
            })?;
            let Some(token) = current.owner_start_token else {
                return Err(Error::InvalidInput(format!(
                    "workspace view `{}` lost its mount lease",
                    view.view_id
                )));
            };
            let updated = self.conn.execute(
                "UPDATE workspace_views SET heartbeat_at = ?1, updated_at = ?1 WHERE view_id = ?2 AND owner_pid = ?3 AND owner_start_token = ?4",
                params![now_ts(), view.view_id, std::process::id(), token],
            )?;
            if updated == 0 {
                return Err(Error::InvalidInput(format!(
                    "workspace view `{}` mount lease is no longer owned by this process",
                    view.view_id
                )));
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    pub(crate) fn workspace_view_paths_for_lane_id(&self, lane_id: &str) -> WorkspaceViewPaths {
        let view_id = format!(
            "view_{}",
            crate::ids::short_hash(format!("workspace-view:{lane_id}").as_bytes(), 12)
        );
        let view_dir = self.db_dir.join("views").join(&view_id);
        let meta_dir = view_dir.join("meta");
        WorkspaceViewPaths {
            view_id,
            source_upper: view_dir.join("source-upper"),
            generated_upper: view_dir.join("generated-upper"),
            scratch_upper: view_dir.join("scratch-upper"),
            journal_path: meta_dir.join(VIEW_JOURNAL_FILE),
            meta_dir,
            view_dir,
        }
    }

    pub(crate) fn workspace_view_paths_for_lane_name(&self, lane: &str) -> WorkspaceViewPaths {
        let lane_id = format!("lane_{}", crate::ids::short_hash(lane.as_bytes(), 8));
        self.workspace_view_paths_for_lane_id(&lane_id)
    }

    pub(crate) fn prepare_workspace_view_storage_for_lane_name(
        &self,
        lane: &str,
    ) -> Result<WorkspaceViewPaths> {
        let paths = self.workspace_view_paths_for_lane_name(lane);
        if paths.view_dir.exists() && view_dir_contains_files(&paths.view_dir)? {
            return Err(Error::InvalidInput(format!(
                "workspace view `{}` contains recoverable state; refusing to erase it while preparing lane `{lane}`",
                paths.view_id
            )));
        }
        for dir in [
            &paths.source_upper,
            &paths.generated_upper,
            &paths.scratch_upper,
            &paths.meta_dir,
        ] {
            fs::create_dir_all(dir)?;
        }
        Ok(paths)
    }

    pub(crate) fn create_workspace_view(
        &self,
        lane_id: &str,
        base_change: &ChangeId,
        base_root: &ObjectId,
        backend: &str,
        mountpoint: &Path,
    ) -> Result<LaneWorkspaceViewReport> {
        if let Some(existing) = self.workspace_view_by_lane_id(lane_id)? {
            if existing.base_change == *base_change
                && existing.base_root == *base_root
                && existing.mountpoint == mountpoint.to_string_lossy()
            {
                return Ok(existing);
            }
            return Err(Error::InvalidInput(format!(
                "lane `{lane_id}` already has workspace view `{}`; recover or remove that view before replacing it",
                existing.view_id
            )));
        }
        let paths = self.workspace_view_paths_for_lane_id(lane_id);
        if paths.view_dir.exists() && view_dir_contains_files(&paths.view_dir)? {
            return Err(Error::InvalidInput(format!(
                "workspace view directory `{}` already contains recoverable state; refusing to overwrite it",
                paths.view_dir.display()
            )));
        }
        for dir in [
            &paths.source_upper,
            &paths.generated_upper,
            &paths.scratch_upper,
            &paths.meta_dir,
        ] {
            fs::create_dir_all(dir)?;
        }
        ViewMutationJournal::initialize_storage(&paths.source_upper)?;
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO workspace_views \
             (view_id, lane_id, base_change, base_root, backend, mountpoint, source_upper, generated_upper, scratch_upper, meta_dir, journal_path, generation, checkpoint_seq, checkpoint_root, status, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, 0, ?4, 'unmounted', ?12, ?12)",
            params![
                paths.view_id,
                lane_id,
                base_change.0,
                base_root.0,
                backend,
                mountpoint.to_string_lossy(),
                paths.source_upper.to_string_lossy(),
                paths.generated_upper.to_string_lossy(),
                paths.scratch_upper.to_string_lossy(),
                paths.meta_dir.to_string_lossy(),
                paths.journal_path.to_string_lossy(),
                now,
            ],
        )?;
        let report = self
            .workspace_view_by_lane_id(lane_id)?
            .ok_or_else(|| Error::Corrupt("workspace view insert was not visible".to_string()))?;
        write_file_atomic(
            &paths.meta_dir.join("view.json"),
            &serde_json::to_vec_pretty(&report)?,
            false,
        )?;
        Ok(report)
    }

    pub(crate) fn acquire_workspace_mount_lease(
        &self,
        lane: &str,
        backend: &str,
    ) -> Result<WorkspaceMountLease> {
        let _lock = self.acquire_write_lock()?;
        let initial = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered workspace view"
            ))
        })?;
        let _barrier = ViewMutationBarrier::shared(Path::new(&initial.meta_dir))?;
        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` lost its layered workspace view while acquiring its mount lease"
            ))
        })?;
        if view.view_id != initial.view_id
            || view.meta_dir != initial.meta_dir
            || view.generation != initial.generation
            || view.base_root != initial.base_root
            || view.base_change != initial.base_change
        {
            return Err(Error::InvalidInput(format!(
                "workspace view for lane `{lane}` changed while acquiring its mount lease; retry the mount"
            )));
        }
        if view.backend != backend {
            return Err(Error::InvalidInput(format!(
                "workspace view `{}` uses backend `{}`; `{backend}` cannot mount it",
                view.view_id, view.backend
            )));
        }
        if let (Some(pid), Some(token)) = (view.owner_pid, view.owner_start_token.as_deref()) {
            if process_matches_start_token(pid, token) {
                return Err(Error::InvalidInput(format!(
                    "workspace view `{}` already has an active writer in process {pid}",
                    view.view_id
                )));
            }
            self.conn.execute(
                "UPDATE workspace_views SET status = 'recovered', owner_pid = NULL, owner_start_token = NULL, heartbeat_at = NULL, updated_at = ?1 WHERE view_id = ?2",
                params![now_ts(), view.view_id],
            )?;
            self.insert_lane_event(
                &view.lane_id,
                "workspace_view_recovered",
                None,
                None,
                &serde_json::json!({"view_id": view.view_id, "stale_owner_pid": pid}),
            )?;
        }
        for path in [
            &view.source_upper,
            &view.generated_upper,
            &view.scratch_upper,
            &view.meta_dir,
        ] {
            let path = Path::new(path);
            if !path.is_dir() {
                return Err(Error::InvalidInput(format!(
                    "workspace view `{}` is unhealthy because `{}` is missing",
                    view.view_id,
                    path.display()
                )));
            }
        }
        let owner_start_token = current_process_start_token();
        let installed = self.conn.execute(
            "UPDATE workspace_views SET status = 'mounting', owner_pid = ?1, owner_start_token = ?2, heartbeat_at = ?3, updated_at = ?3 \
             WHERE view_id = ?4 AND generation = ?5 AND base_root = ?6 AND base_change = ?7 AND owner_pid IS NULL",
            params![
                std::process::id(),
                owner_start_token,
                now_ts(),
                view.view_id,
                view.generation as i64,
                view.base_root.0,
                view.base_change.0,
            ],
        )?;
        if installed != 1 {
            return Err(Error::InvalidInput(format!(
                "workspace view `{}` changed before its mount lease could be installed; retry the mount",
                view.view_id
            )));
        }
        write_file_atomic(
            &Path::new(&view.meta_dir).join("mount.json"),
            &serde_json::to_vec_pretty(&serde_json::json!({
                "view_id": view.view_id,
                "owner_pid": std::process::id(),
                "owner_start_token": owner_start_token,
                "backend": backend,
                "mountpoint": view.mountpoint,
                "generation": view.generation,
                "heartbeat_at": now_ts(),
            }))?,
            false,
        )?;
        Ok(WorkspaceMountLease {
            workspace_root: self.workspace_root.clone(),
            db_dir: self.db_dir.clone(),
            view_id: view.view_id,
            owner_start_token,
            released: false,
        })
    }

    pub fn recover_workspace_views(&self) -> Result<Vec<String>> {
        self.recover_workspace_lane_heads()?;
        self.recover_workspace_checkpoint_markers()?;
        let mut stmt = self.conn.prepare(
            "SELECT view_id, owner_pid, owner_start_token, meta_dir FROM workspace_views WHERE owner_pid IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)? as u32,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut recovered = Vec::new();
        for row in rows {
            let (view_id, pid, token, meta_dir) = row?;
            if process_matches_start_token(pid, &token) {
                continue;
            }
            self.conn.execute(
                "UPDATE workspace_views SET status = 'recovered', owner_pid = NULL, owner_start_token = NULL, heartbeat_at = NULL, updated_at = ?1 WHERE view_id = ?2",
                params![now_ts(), view_id],
            )?;
            let _ = fs::remove_file(Path::new(&meta_dir).join("mount.json"));
            recovered.push(view_id);
        }
        Ok(recovered)
    }

    fn recover_workspace_lane_heads(&self) -> Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT b.lane_id, b.ref_name, b.head_change, b.head_root \
             FROM lane_branches b JOIN workspace_views v ON v.lane_id = b.lane_id \
             WHERE b.status != 'removed'",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (lane_id, ref_name, head_change, head_root) in rows {
            let head = self.get_ref(&ref_name)?;
            if head.change_id.0 != head_change || head.root_id.0 != head_root {
                self.conn.execute(
                    "UPDATE lane_branches SET head_change = ?1, head_root = ?2, updated_at = ?3 WHERE lane_id = ?4",
                    params![head.change_id.0, head.root_id.0, now_ts(), lane_id],
                )?;
            }
        }
        Ok(())
    }

    fn recover_workspace_checkpoint_markers(&self) -> Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT v.view_id, v.meta_dir, v.source_upper, v.checkpoint_seq, v.generation, v.checkpoint_root, b.ref_name \
             FROM workspace_views v JOIN lane_branches b ON b.lane_id = v.lane_id \
             WHERE b.status != 'removed'",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?.max(0) as u64,
                    row.get::<_, i64>(4)?.max(0) as u64,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (
            view_id,
            meta_dir,
            source_upper,
            checkpoint_seq,
            generation,
            checkpoint_root,
            ref_name,
        ) in rows
        {
            let path = Path::new(&meta_dir).join("clean-checkpoint.json");
            let read_barrier = ViewMutationBarrier::shared(Path::new(&meta_dir))?;
            let marker = fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
            let mirror_matches = marker.as_ref().is_some_and(|marker| {
                marker["view_id"].as_str() == Some(view_id.as_str())
                    && marker["root_id"].as_str() == Some(checkpoint_root.as_str())
                    && marker["journal_sequence"].as_u64() == Some(checkpoint_seq)
                    && marker["generation"].as_u64() == Some(generation)
                    && marker["recovery_qualified"].as_bool() == Some(true)
            });
            if mirror_matches
                && read_barrier.checkpoint_sequence() == checkpoint_seq
                && read_barrier.checkpoint_generation() == generation
            {
                continue;
            }
            drop(read_barrier);

            let mut barrier = ViewMutationBarrier::exclusive(Path::new(&meta_dir))?;
            let journal = ViewMutationJournal::open(Path::new(&source_upper))?;
            if !journal.recovery_is_qualified() {
                return Err(Error::ChangeLedgerReconcileRequired {
                    scope: view_id,
                    state: "unqualified_view_journal".into(),
                    reason:
                        "workspace checkpoint recovery cannot qualify journal generation evidence"
                            .into(),
                    command: "trail ledger reconcile".into(),
                });
            }
            if checkpoint_seq > journal.last_sequence() {
                return Err(Error::Corrupt(format!(
                    "SQLite workspace checkpoint {} is ahead of durable journal sequence {} for `{}`",
                    checkpoint_seq,
                    journal.last_sequence(),
                    view_id,
                )));
            }
            let head = self.get_ref(&ref_name)?;
            if head.root_id.0 != checkpoint_root {
                return Err(Error::Corrupt(format!(
                    "workspace checkpoint root `{checkpoint_root}` is not lane head `{}` for `{view_id}`",
                    head.root_id.0
                )));
            }
            let marker = fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
            let mirror_matches = marker.as_ref().is_some_and(|marker| {
                marker["view_id"].as_str() == Some(view_id.as_str())
                    && marker["root_id"].as_str() == Some(checkpoint_root.as_str())
                    && marker["journal_sequence"].as_u64() == Some(checkpoint_seq)
                    && marker["generation"].as_u64() == Some(generation)
                    && marker["recovery_qualified"].as_bool() == Some(true)
            });
            if !mirror_matches {
                let checkpoint = serde_json::json!({
                    "view_id": view_id,
                    "root_id": checkpoint_root,
                    "operation": serde_json::Value::Null,
                    "journal_sequence": checkpoint_seq,
                    "journal_qualified": journal.is_qualified(),
                    "recovery_qualified": true,
                    "generation": generation,
                    "completed_at": now_ts(),
                });
                write_file_atomic(&path, &serde_json::to_vec_pretty(&checkpoint)?, false)?;
            }
            ViewMutationJournal::rotate_after_checkpoint(
                Path::new(&source_upper),
                checkpoint_seq,
                generation,
            )?;
            barrier.record_checkpoint_cut(checkpoint_seq, generation)?;
        }
        Ok(())
    }

    pub fn exec_lane_workspace(
        &self,
        lane: &str,
        command: &[String],
    ) -> Result<WorkspaceExecReport> {
        if command.is_empty() {
            return Err(Error::InvalidInput(
                "lane workspace exec requires a command".to_string(),
            ));
        }
        let branch = self.lane_branch(lane)?;
        let record = self.lane_record(&branch.lane_id)?;
        let mode = self.lane_workdir_mode_for(&record, &branch)?;
        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered workspace view"
            ))
        })?;
        let head = self.get_ref(&branch.ref_name)?;
        let run = || self.run_workspace_command(&view, &head.root_id, command);
        let exit_code = match mode {
            LaneWorkdirMode::FuseCow => {
                let mount = self.mount_fuse_cow_workdir_for_lane(lane)?;
                let result = run();
                drop(mount);
                result?
            }
            LaneWorkdirMode::NfsCow => {
                let mount = self.mount_nfs_cow_workdir_for_lane(lane)?;
                let result = run();
                drop(mount);
                result?
            }
            LaneWorkdirMode::DokanCow => {
                #[cfg(target_os = "windows")]
                {
                    let mount = self.mount_dokan_cow_workdir_for_lane(lane)?;
                    let result = run();
                    drop(mount);
                    result?
                }
                #[cfg(not(target_os = "windows"))]
                return Err(Error::InvalidInput(
                    "dokan-cow workdirs are currently supported only on Windows".to_string(),
                ));
            }
            _ => {
                return Err(Error::InvalidInput(format!(
                    "lane `{lane}` uses `{}` rather than a layered workspace view",
                    mode.as_str()
                )));
            }
        };
        let environment_generation = self
            .conn
            .query_row(
                "SELECT generation_id FROM environment_view_generations WHERE view_id = ?1",
                params![&view.view_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        self.insert_lane_event(
            &branch.lane_id,
            "workspace_view_exec_completed",
            Some(&head.change_id),
            None,
            &serde_json::json!({
                "view_id": view.view_id,
                "source_root": head.root_id.0,
                "generation": view.generation,
                "environment_generation": environment_generation,
                "command_fingerprint": sha256_hex(&serde_json::to_vec(command)?),
                "exit_code": exit_code,
            }),
        )?;
        Ok(WorkspaceExecReport {
            view_id: view.view_id,
            lane_id: branch.lane_id,
            source_root: head.root_id,
            generation: view.generation,
            environment_generation,
            backend: view.backend,
            command: command.to_vec(),
            exit_code,
        })
    }

    pub fn lane_workspace_environment(&self, lane: &str) -> Result<Vec<(String, String)>> {
        let branch = self.lane_branch(lane)?;
        let head = self.get_ref(&branch.ref_name)?;
        if let Some(view) = self.lane_workspace_view(lane)? {
            return self.workspace_command_environment(&view, &head.root_id);
        }
        Ok(vec![
            (
                "TRAIL_WORKSPACE".to_string(),
                self.workspace_root.to_string_lossy().into_owned(),
            ),
            ("TRAIL_LANE".to_string(), branch.lane_id),
            ("TRAIL_SOURCE_ROOT".to_string(), head.root_id.0),
        ])
    }

    fn run_workspace_command(
        &self,
        view: &LaneWorkspaceViewReport,
        source_root: &ObjectId,
        command: &[String],
    ) -> Result<i32> {
        let environment = self.workspace_command_environment(view, source_root)?;
        let mut process = Command::new(&command[0]);
        process
            .args(&command[1..])
            .current_dir(&view.mountpoint)
            .envs(environment);
        let status = process.status()?;
        Ok(status.code().unwrap_or(128))
    }

    pub(crate) fn workspace_command_environment(
        &self,
        view: &LaneWorkspaceViewReport,
        source_root: &ObjectId,
    ) -> Result<Vec<(String, String)>> {
        let cargo_home = self.db_dir.join("cache/tool-home/cargo");
        let node_cache = self.db_dir.join("cache/tool-home/node/npm");
        let sccache_dir = self.db_dir.join("cache/tool-home/sccache");
        let target_dir = Path::new(&view.mountpoint).join("target");
        for path in [&cargo_home, &node_cache, &sccache_dir] {
            fs::create_dir_all(path)?;
        }
        let mut environment = vec![
            (
                "TRAIL_WORKSPACE".to_string(),
                self.workspace_root.to_string_lossy().into_owned(),
            ),
            ("TRAIL_LANE".to_string(), view.lane_id.clone()),
            ("TRAIL_VIEW".to_string(), view.view_id.clone()),
            ("TRAIL_SOURCE_ROOT".to_string(), source_root.0.clone()),
            (
                "TRAIL_VIEW_GENERATION".to_string(),
                view.generation.to_string(),
            ),
            (
                "CARGO_HOME".to_string(),
                cargo_home.to_string_lossy().into_owned(),
            ),
            (
                "CARGO_TARGET_DIR".to_string(),
                target_dir.to_string_lossy().into_owned(),
            ),
            (
                "SCCACHE_DIR".to_string(),
                sccache_dir.to_string_lossy().into_owned(),
            ),
            (
                "npm_config_cache".to_string(),
                node_cache.to_string_lossy().into_owned(),
            ),
        ];
        if let Some(generation_id) = self
            .conn
            .query_row(
                "SELECT generation_id FROM environment_view_generations WHERE view_id = ?1",
                params![&view.view_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            environment.push(("TRAIL_ENVIRONMENT_GENERATION".to_string(), generation_id));
        }
        let mut runtime_stmt = self.conn.prepare(
            "SELECT r.component_id, r.resource_name, r.host_port, r.status, r.health_status
             FROM environment_view_generations active
             JOIN environment_generation_runtime_resources r
               ON r.generation_id = active.generation_id
             WHERE active.view_id = ?1
             ORDER BY r.component_id, r.resource_name",
        )?;
        let runtime_resources = runtime_stmt
            .query_map(params![&view.view_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<u16>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if !runtime_resources.is_empty() {
            let mut alias_counts = BTreeMap::<String, usize>::new();
            for (_, resource_name, _, _, _) in &runtime_resources {
                *alias_counts
                    .entry(runtime_service_environment_segment(resource_name))
                    .or_default() += 1;
            }
            let mut services = serde_json::Map::new();
            for (component_id, resource_name, host_port, status, health_status) in runtime_resources
            {
                let host_port = host_port.filter(|_| {
                    status == "running" && health_status == "healthy"
                }).ok_or_else(|| {
                    Error::InvalidInput(format!(
                        "runtime service `{component_id}/{resource_name}` is {status}/{health_status}; run `trail env runtime reconcile {}` before executing lane commands",
                        view.lane_id
                    ))
                })?;
                let address = format!("127.0.0.1:{host_port}");
                services.insert(
                    format!("{component_id}/{resource_name}"),
                    serde_json::json!({
                        "host": "127.0.0.1",
                        "port": host_port,
                        "address": address,
                    }),
                );
                let alias = runtime_service_environment_segment(&resource_name);
                if alias_counts.get(&alias) == Some(&1) {
                    let prefix = format!("TRAIL_SERVICE_{alias}");
                    environment.extend([
                        (format!("{prefix}_HOST"), "127.0.0.1".to_string()),
                        (format!("{prefix}_PORT"), host_port.to_string()),
                        (format!("{prefix}_ADDRESS"), address),
                    ]);
                }
            }
            environment.push((
                "TRAIL_SERVICES_JSON".to_string(),
                serde_json::to_string(&services)?,
            ));
        }
        if command_available("sccache") {
            environment.extend([
                ("RUSTC_WRAPPER".to_string(), "sccache".to_string()),
                ("CARGO_INCREMENTAL".to_string(), "0".to_string()),
            ]);
        } else {
            environment.push(("CARGO_INCREMENTAL".to_string(), "1".to_string()));
        }
        if let Some(shadow) = self.ensure_workspace_git_shadow(view, source_root)? {
            environment.extend([
                ("GIT_DIR".to_string(), shadow.git_dir.clone()),
                ("GIT_WORK_TREE".to_string(), shadow.work_tree.clone()),
                (
                    "GIT_INDEX_FILE".to_string(),
                    Path::new(&shadow.git_dir)
                        .join("index")
                        .to_string_lossy()
                        .into_owned(),
                ),
                ("TRAIL_GIT_SHADOW_HEAD".to_string(), shadow.pinned_head),
            ]);
        }
        Ok(environment)
    }

    pub fn lane_workspace_view(&self, lane: &str) -> Result<Option<LaneWorkspaceViewReport>> {
        validate_ref_segment(lane)?;
        self.conn
            .query_row(
                "SELECT v.view_id, v.lane_id, v.base_change, v.base_root, v.backend, v.mountpoint, v.source_upper, v.generated_upper, v.scratch_upper, v.meta_dir, v.journal_path, v.generation, v.checkpoint_seq, v.checkpoint_root, v.status, v.owner_pid, v.owner_start_token, v.heartbeat_at, v.created_at, v.updated_at \
                 FROM workspace_views v JOIN lanes l ON l.lane_id = v.lane_id WHERE l.name = ?1",
                params![lane],
                workspace_view_from_row,
            )
            .optional()
            .map_err(Error::from)
    }

    pub(crate) fn workspace_view_by_lane_id(
        &self,
        lane_id: &str,
    ) -> Result<Option<LaneWorkspaceViewReport>> {
        self.conn
            .query_row(
                "SELECT view_id, lane_id, base_change, base_root, backend, mountpoint, source_upper, generated_upper, scratch_upper, meta_dir, journal_path, generation, checkpoint_seq, checkpoint_root, status, owner_pid, owner_start_token, heartbeat_at, created_at, updated_at \
                 FROM workspace_views WHERE lane_id = ?1",
                params![lane_id],
                workspace_view_from_row,
            )
            .optional()
            .map_err(Error::from)
    }

    pub(crate) fn workspace_view_paths_for_lane(&self, lane: &str) -> Result<WorkspaceViewPaths> {
        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered workspace view"
            ))
        })?;
        Ok(WorkspaceViewPaths {
            view_id: view.view_id,
            view_dir: PathBuf::from(&view.meta_dir)
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| {
                    Error::Corrupt("workspace view meta path has no parent".to_string())
                })?,
            source_upper: PathBuf::from(view.source_upper),
            generated_upper: PathBuf::from(view.generated_upper),
            scratch_upper: PathBuf::from(view.scratch_upper),
            meta_dir: PathBuf::from(view.meta_dir),
            journal_path: PathBuf::from(view.journal_path),
        })
    }

    pub fn lane_workspace_space(&self, lane: &str) -> Result<WorkspaceSpaceReport> {
        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered workspace view"
            ))
        })?;
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, &view.base_root)?;
        let source = directory_usage(Path::new(&view.source_upper))?;
        let generated = directory_usage(Path::new(&view.generated_upper))?;
        let scratch = directory_usage(Path::new(&view.scratch_upper))?;
        let blobs = directory_usage(&self.db_dir.join("cache/blobs"))?;
        let layers = directory_usage(&self.db_dir.join("cache/layers"))?;
        Ok(WorkspaceSpaceReport {
            view_id: view.view_id,
            logical_visible_bytes: root
                .total_text_bytes
                .saturating_add(source.logical_bytes)
                .saturating_add(generated.logical_bytes)
                .saturating_add(scratch.logical_bytes),
            shared_physical_bytes: blobs.physical_bytes.saturating_add(layers.physical_bytes),
            lane_exclusive_physical_bytes: source
                .physical_bytes
                .saturating_add(generated.physical_bytes)
                .saturating_add(scratch.physical_bytes),
            shared_extent_bytes: None,
            reclaimable_cache_bytes: self.workspace_reclaimable_cache_bytes()?,
            uncheckpointed_source_bytes: source.logical_bytes,
            generated_upper_bytes: generated.logical_bytes,
            scratch_upper_bytes: scratch.logical_bytes,
            physical_accounting: if cfg!(unix) {
                "allocated-blocks".to_string()
            } else {
                "file-size-estimate".to_string()
            },
        })
    }

    pub fn workspace_quota_status(&self, lane: &str) -> Result<WorkspaceQuotaReport> {
        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered workspace view"
            ))
        })?;
        let source = directory_usage(Path::new(&view.source_upper))?;
        let generated = directory_usage(Path::new(&view.generated_upper))?;
        let scratch = directory_usage(Path::new(&view.scratch_upper))?;
        let upper_logical_bytes = source
            .logical_bytes
            .saturating_add(generated.logical_bytes)
            .saturating_add(scratch.logical_bytes);
        let upper_file_count = source
            .file_count
            .saturating_add(generated.file_count)
            .saturating_add(scratch.file_count);
        let largest_file_bytes = source
            .largest_file_bytes
            .max(generated.largest_file_bytes)
            .max(scratch.largest_file_bytes);
        let journal_bytes = fs::metadata(&view.journal_path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let cache_physical_bytes = directory_usage(&self.db_dir.join("cache"))?.physical_bytes;
        let limits = &self.config().workspace_views;
        let mut exceeded = Vec::new();
        if limits.upper_logical_bytes > 0 && upper_logical_bytes > limits.upper_logical_bytes {
            exceeded.push("upper_logical_bytes".to_string());
        }
        if limits.upper_file_count > 0 && upper_file_count > limits.upper_file_count {
            exceeded.push("upper_file_count".to_string());
        }
        if limits.single_file_bytes > 0 && largest_file_bytes > limits.single_file_bytes {
            exceeded.push("single_file_bytes".to_string());
        }
        if limits.journal_bytes > 0 && journal_bytes > limits.journal_bytes {
            exceeded.push("journal_bytes".to_string());
        }
        if limits.cache_max_bytes > 0 && cache_physical_bytes > limits.cache_max_bytes {
            exceeded.push("cache_max_bytes".to_string());
        }
        Ok(WorkspaceQuotaReport {
            view_id: view.view_id,
            upper_logical_bytes,
            upper_file_count,
            largest_file_bytes,
            journal_bytes,
            cache_physical_bytes,
            exceeded,
        })
    }

    pub(crate) fn complete_workspace_checkpoint(
        &self,
        lane: &str,
        root_id: &ObjectId,
        operation: Option<&ChangeId>,
        barrier: &mut ViewMutationBarrier,
    ) -> Result<u64> {
        // TRAIL_FS_PRODUCER: cow_checkpoint CowPublication controlled
        let Some(view) = self.lane_workspace_view(lane)? else {
            return Ok(0);
        };
        let cut = checkpoint_view(Path::new(&view.source_upper))?;
        if !cut.recovery_qualified {
            return Err(Error::ChangeLedgerReconcileRequired {
                scope: view.view_id,
                state: "unqualified_view_journal".into(),
                reason: "workspace checkpoint has neither a qualified changed-path journal nor a qualified independent whiteout journal".into(),
                command: "trail ledger reconcile".into(),
            });
        }
        let sequence = cut.sequence;
        let next_generation = cut.generation.saturating_add(1);
        self.conn.execute(
            "UPDATE workspace_views SET checkpoint_seq = ?1, checkpoint_root = ?2, generation = ?3, updated_at = ?4 WHERE view_id = ?5",
            params![sequence as i64, root_id.0, next_generation as i64, now_ts(), view.view_id],
        )?;
        self.write_workspace_checkpoint_mirror(
            &view,
            root_id,
            operation,
            sequence,
            next_generation,
            cut.qualified,
            barrier,
        )?;
        Ok(sequence)
    }

    pub(crate) fn repair_workspace_checkpoint_mirror(
        &self,
        lane: &str,
        root_id: &ObjectId,
        operation: Option<&ChangeId>,
        sequence: u64,
        generation: u64,
        journal_qualified: bool,
        barrier: &mut ViewMutationBarrier,
    ) -> Result<()> {
        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::Corrupt(format!(
                "layered lane `{lane}` has no persisted workspace view"
            ))
        })?;
        if view.checkpoint_seq != sequence
            || view.checkpoint_root.as_ref() != Some(root_id)
            || view.generation != generation
        {
            return Err(Error::Corrupt(format!(
                "workspace view `{}` checkpoint publication is not the committed SQLite generation",
                view.view_id
            )));
        }
        self.write_workspace_checkpoint_mirror(
            &view,
            root_id,
            operation,
            sequence,
            generation,
            journal_qualified,
            barrier,
        )
    }

    fn write_workspace_checkpoint_mirror(
        &self,
        view: &LaneWorkspaceViewReport,
        root_id: &ObjectId,
        operation: Option<&ChangeId>,
        sequence: u64,
        generation: u64,
        journal_qualified: bool,
        barrier: &mut ViewMutationBarrier,
    ) -> Result<()> {
        let checkpoint = serde_json::json!({
            "view_id": view.view_id,
            "root_id": root_id.0,
            "operation": operation.map(|value| value.0.as_str()),
            "journal_sequence": sequence,
            "journal_qualified": journal_qualified,
            "recovery_qualified": true,
            "generation": generation,
            "completed_at": now_ts(),
        });
        write_file_atomic(
            &Path::new(&view.meta_dir).join("clean-checkpoint.json"),
            &serde_json::to_vec_pretty(&checkpoint)?,
            false,
        )?;
        test_crash_point("checkpoint_after_clean_marker");
        ViewMutationJournal::rotate_after_checkpoint(
            Path::new(&view.source_upper),
            sequence,
            generation,
        )?;
        barrier.record_checkpoint_cut(sequence, generation)?;
        Ok(())
    }

    pub(crate) fn workspace_view_last_journal_sequence(
        &self,
        view: &LaneWorkspaceViewReport,
    ) -> Result<u64> {
        Ok(ViewMutationJournal::open(Path::new(&view.source_upper))?.last_sequence())
    }

    pub fn checkpoint_lane_workspace(
        &mut self,
        lane: &str,
        message: Option<String>,
    ) -> Result<WorkspaceCheckpointReport> {
        let metrics = self.operation_metrics.clone();
        profile_operation_metrics(
            metrics.as_ref(),
            OperationMetricsKind::CowCheckpoint,
            || {
                let record = self.record_lane_workdir(lane, message)?;
                let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
                    Error::InvalidInput(format!(
                        "lane `{lane}` does not have a layered workspace view"
                    ))
                })?;
                let sequence = view.checkpoint_seq;
                let report = WorkspaceCheckpointReport {
                    view_id: view.view_id,
                    operation: record.operation,
                    root_id: record.root_id,
                    journal_sequence: sequence,
                    source_paths: record
                        .changed_paths
                        .into_iter()
                        .map(|item| item.path)
                        .collect(),
                    generated_dirty_paths: record.generated_dirty_paths,
                    generated_path_accounting: "journal_interval".into(),
                    upper_recovery_walks: record.upper_recovery_walks,
                };
                Ok(report)
            },
        )
    }
}

fn checkpoint_view(source_upper: &Path) -> Result<ViewJournalCut> {
    let journal: ViewIntentWriter = ViewMutationJournal::open(source_upper)?;
    Ok(journal.cut())
}

fn runtime_service_environment_segment(name: &str) -> String {
    let mut segment = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    if segment.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        segment.insert(0, '_');
    }
    segment
}

fn workspace_view_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LaneWorkspaceViewReport> {
    let owner_pid = row
        .get::<_, Option<i64>>(15)?
        .and_then(|value| u32::try_from(value).ok());
    Ok(LaneWorkspaceViewReport {
        view_id: row.get(0)?,
        lane_id: row.get(1)?,
        base_change: ChangeId(row.get(2)?),
        base_root: ObjectId(row.get(3)?),
        backend: row.get(4)?,
        mountpoint: row.get(5)?,
        source_upper: row.get(6)?,
        generated_upper: row.get(7)?,
        scratch_upper: row.get(8)?,
        meta_dir: row.get(9)?,
        journal_path: row.get(10)?,
        generation: row.get::<_, i64>(11)?.max(0) as u64,
        checkpoint_seq: row.get::<_, i64>(12)?.max(0) as u64,
        checkpoint_root: row.get::<_, Option<String>>(13)?.map(ObjectId),
        status: row.get(14)?,
        owner_pid,
        owner_start_token: row.get(16)?,
        heartbeat_at: row.get(17)?,
        created_at: row.get(18)?,
        updated_at: row.get(19)?,
    })
}

fn workspace_mount_report(
    view: &LaneWorkspaceViewReport,
    owner_pid: Option<u32>,
    owner_start_token: Option<String>,
    healthy: bool,
) -> WorkspaceMountReport {
    WorkspaceMountReport {
        view_id: view.view_id.clone(),
        backend: view.backend.clone(),
        mountpoint: view.mountpoint.clone(),
        generation: view.generation,
        owner_pid,
        owner_start_token,
        healthy,
    }
}

fn view_dir_contains_files(path: &Path) -> Result<bool> {
    for entry in walkdir::WalkDir::new(path).follow_links(false) {
        let entry = entry.map_err(|err| Error::InvalidInput(err.to_string()))?;
        if entry.file_type().is_file() || entry.file_type().is_symlink() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn command_available(command: &str) -> bool {
    let candidate = Path::new(command);
    if candidate.components().count() > 1 {
        return candidate.is_file();
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .any(|directory| {
            let path = directory.join(command);
            if path.is_file() {
                return true;
            }
            #[cfg(windows)]
            {
                return directory.join(format!("{command}.exe")).is_file();
            }
            #[cfg(not(windows))]
            false
        })
}

#[derive(Clone, Copy, Debug, Default)]
struct DirectoryUsage {
    logical_bytes: u64,
    physical_bytes: u64,
    file_count: u64,
    largest_file_bytes: u64,
}

fn directory_usage(path: &Path) -> Result<DirectoryUsage> {
    if !path.exists() {
        return Ok(DirectoryUsage::default());
    }
    let mut usage = DirectoryUsage::default();
    for entry in walkdir::WalkDir::new(path).follow_links(false) {
        let entry = entry.map_err(|err| Error::InvalidInput(err.to_string()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let metadata = entry.metadata().map_err(|err| Error::Io(err.into()))?;
        usage.logical_bytes = usage.logical_bytes.saturating_add(metadata.len());
        usage.file_count = usage.file_count.saturating_add(1);
        usage.largest_file_bytes = usage.largest_file_bytes.max(metadata.len());
        usage.physical_bytes = usage
            .physical_bytes
            .saturating_add(file_physical_bytes(&metadata));
    }
    Ok(usage)
}

#[cfg(unix)]
fn file_physical_bytes(metadata: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.blocks().saturating_mul(512)
}

#[cfg(not(unix))]
fn file_physical_bytes(metadata: &fs::Metadata) -> u64 {
    metadata.len()
}

#[cfg(test)]
mod tests {
    use super::super::workdir::{
        ViewCore, ViewMutationBarrier, ViewMutationJournal, ViewUpperLayout, VIEW_ROOT_INO,
    };
    use super::*;
    use std::process::Stdio;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn checkpoint_crash_helper() {
        let Some(workspace) = std::env::var_os("TRAIL_TEST_CRASH_WORKSPACE") else {
            return;
        };
        let workspace = PathBuf::from(workspace);
        let mut db = Trail::open(&workspace).unwrap();
        let branch = db.lane_branch("checkpoint-crash").unwrap();
        let head = db.get_ref(&branch.ref_name).unwrap();
        let paths = db
            .workspace_view_paths_for_lane("checkpoint-crash")
            .unwrap();
        let mut core = ViewCore::new_lazy(
            Trail::open(&workspace).unwrap(),
            paths.source_upper,
            head.root_id,
        )
        .unwrap();
        let readme = core.lookup(VIEW_ROOT_INO, "README.md").unwrap();
        core.setattr(readme, Some(0), None).unwrap();
        core.write(readme, 0, b"durable after kill\n").unwrap();
        drop(core);
        let _ = db.checkpoint_lane_workspace("checkpoint-crash", Some("crash".to_string()));
        panic!("checkpoint crash helper passed its requested crash point");
    }

    #[test]
    fn killing_checkpoint_at_each_commit_phase_preserves_and_recovers_source() {
        for phase in [
            "checkpoint_after_source_sync",
            "checkpoint_after_ref_advance",
            "checkpoint_after_lane_head_update",
            "checkpoint_after_clean_marker",
        ] {
            let workspace = tempfile::tempdir().unwrap();
            fs::write(workspace.path().join("README.md"), "baseline\n").unwrap();
            Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let mut db = Trail::open(workspace.path()).unwrap();
            let mode = if cfg!(target_os = "macos") {
                LaneWorkdirMode::NfsCow
            } else if cfg!(target_os = "windows") {
                LaneWorkdirMode::DokanCow
            } else {
                LaneWorkdirMode::FuseCow
            };
            db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                "checkpoint-crash",
                Some("main"),
                mode,
                None,
                None,
                None,
                &[],
                false,
            )
            .unwrap();
            let source_upper = db
                .workspace_view_paths_for_lane("checkpoint-crash")
                .unwrap()
                .source_upper;
            drop(db);

            let ready = workspace
                .path()
                .join(".trail/tmp")
                .join(format!("{phase}.ready"));
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "db::lane::workspace_view::tests::checkpoint_crash_helper",
                    "--nocapture",
                ])
                .env("RUST_TEST_THREADS", "1")
                .env("TRAIL_TEST_CRASH_AT", phase)
                .env("TRAIL_TEST_CRASH_READY", &ready)
                .env("TRAIL_TEST_CRASH_WORKSPACE", workspace.path())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            wait_for_checkpoint_crash_handshake(&mut child, &ready, phase);
            child.kill().unwrap();
            let _ = child.wait().unwrap();

            assert_eq!(
                fs::read_to_string(source_upper.join("README.md")).unwrap(),
                "durable after kill\n"
            );
            let mut reopened = Trail::open(workspace.path()).unwrap();
            if phase == "checkpoint_after_clean_marker" {
                let view = reopened
                    .lane_workspace_view("checkpoint-crash")
                    .unwrap()
                    .unwrap();
                let journal = ViewMutationJournal::open(&source_upper).unwrap();
                assert_eq!(journal.generation(), view.generation);
                assert_eq!(journal.last_sequence(), view.checkpoint_seq);
                let barrier = ViewMutationBarrier::shared(Path::new(&view.meta_dir)).unwrap();
                assert_eq!(barrier.checkpoint_sequence(), view.checkpoint_seq);
                assert_eq!(barrier.checkpoint_generation(), view.generation);
            }
            let recovered = reopened
                .checkpoint_lane_workspace("checkpoint-crash", Some("recover".to_string()))
                .unwrap();
            let entry = reopened
                .root_file_entry(&recovered.root_id, "README.md")
                .unwrap()
                .unwrap();
            assert_eq!(
                reopened.materialize_entry_bytes(&entry).unwrap(),
                b"durable after kill\n"
            );
            let view = reopened
                .lane_workspace_view("checkpoint-crash")
                .unwrap()
                .unwrap();
            assert_eq!(view.checkpoint_root, Some(recovered.root_id));
            assert!(!reopened
                .lane_readiness("checkpoint-crash")
                .unwrap()
                .blockers
                .iter()
                .any(|issue| issue.code == "uncheckpointed_source_changes"));

            if phase == "checkpoint_after_clean_marker" {
                for index in 0..6 {
                    let branch = reopened.lane_branch("checkpoint-crash").unwrap();
                    let head = reopened.get_ref(&branch.ref_name).unwrap();
                    let mut core = ViewCore::new_lazy(
                        Trail::open(workspace.path()).unwrap(),
                        source_upper.clone(),
                        head.root_id,
                    )
                    .unwrap();
                    let name = format!("post-recovery-{index}.txt");
                    let file = core.create(VIEW_ROOT_INO, &name, 0o644, true).unwrap();
                    core.write(file.ino, 0, b"generation recovery\n").unwrap();
                    drop(core);
                    reopened
                        .checkpoint_lane_workspace(
                            "checkpoint-crash",
                            Some(format!("post-recovery-{index}")),
                        )
                        .unwrap();
                }

                let view = reopened
                    .lane_workspace_view("checkpoint-crash")
                    .unwrap()
                    .unwrap();
                let journal = ViewMutationJournal::open(&source_upper).unwrap();
                assert_eq!(journal.generation(), view.generation);
                assert_eq!(journal.last_sequence(), view.checkpoint_seq);
                let layout = ViewUpperLayout::from_source_upper(source_upper.clone());
                let journal_files = fs::read_dir(layout.meta_dir)
                    .unwrap()
                    .filter_map(|entry| entry.ok())
                    .filter(|entry| entry.file_name().to_string_lossy().contains("journal.g"))
                    .count();
                assert!(
                    journal_files <= 4,
                    "recovered journal generations were not compacted"
                );
            }
        }
    }

    #[test]
    fn checkpoint_waits_for_inflight_mutation_and_does_not_mark_later_edits_clean() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "baseline\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "barrier",
            Some("main"),
            mode,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let branch = db.lane_branch("barrier").unwrap();
        let head = db.get_ref(&branch.ref_name).unwrap();
        let paths = db.workspace_view_paths_for_lane("barrier").unwrap();
        let mut core = ViewCore::new_lazy(
            Trail::open(workspace.path()).unwrap(),
            paths.source_upper.clone(),
            head.root_id,
        )
        .unwrap();
        let readme = core.lookup(VIEW_ROOT_INO, "README.md").unwrap();
        core.setattr(readme, Some(0), None).unwrap();
        core.write(readme, 0, b"first checkpoint\n").unwrap();
        drop(core);
        drop(db);

        let inflight = ViewMutationBarrier::shared(&paths.meta_dir).unwrap();
        let root = workspace.path().to_path_buf();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (completed_tx, completed_rx) = std::sync::mpsc::channel();
        let checkpoint = std::thread::spawn(move || {
            let mut db = Trail::open(root).unwrap();
            started_tx.send(()).unwrap();
            let report = db
                .checkpoint_lane_workspace("barrier", Some("barrier".to_string()))
                .unwrap();
            completed_tx.send(report).unwrap();
        });
        started_rx.recv().unwrap();
        assert!(
            completed_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "checkpoint crossed an active mutation barrier"
        );
        drop(inflight);
        let report = completed_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        checkpoint.join().unwrap();

        let db = Trail::open(workspace.path()).unwrap();
        let view = db.lane_workspace_view("barrier").unwrap().unwrap();
        assert_eq!(view.checkpoint_seq, report.journal_sequence);
        assert_eq!(view.checkpoint_root, Some(report.root_id.clone()));
        let entry = db
            .root_file_entry(&report.root_id, "README.md")
            .unwrap()
            .unwrap();
        assert_eq!(
            db.materialize_entry_bytes(&entry).unwrap(),
            b"first checkpoint\n"
        );

        let branch = db.lane_branch("barrier").unwrap();
        let head = db.get_ref(&branch.ref_name).unwrap();
        let mut core = ViewCore::new_lazy(
            Trail::open(workspace.path()).unwrap(),
            paths.source_upper,
            head.root_id,
        )
        .unwrap();
        let readme = core.lookup(VIEW_ROOT_INO, "README.md").unwrap();
        core.setattr(readme, Some(0), None).unwrap();
        core.write(readme, 0, b"later edit\n").unwrap();
        drop(core);

        let reopened = Trail::open(workspace.path()).unwrap();
        let later_sequence = reopened
            .workspace_view_last_journal_sequence(
                &reopened.lane_workspace_view("barrier").unwrap().unwrap(),
            )
            .unwrap();
        assert!(
            later_sequence > report.journal_sequence,
            "later journal sequence {later_sequence} did not advance past clean checkpoint {}",
            report.journal_sequence
        );
        assert!(reopened
            .lane_readiness("barrier")
            .unwrap()
            .blockers
            .iter()
            .any(|issue| issue.code == "uncheckpointed_source_changes"));
    }

    fn wait_for_checkpoint_crash_handshake(
        child: &mut std::process::Child,
        ready: &Path,
        phase: &str,
    ) {
        for _ in 0..1_000 {
            if ready.is_file() {
                return;
            }
            if let Some(status) = child.try_wait().unwrap() {
                panic!("checkpoint crash helper exited at {phase} before handshake: {status}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = child.kill();
        panic!("timed out waiting for checkpoint crash helper at {phase}");
    }

    #[test]
    fn workspace_view_is_persisted_and_empty_view_has_negligible_exclusive_space() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "layered",
            Some("main"),
            mode,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();

        let view = db.lane_workspace_view("layered").unwrap().unwrap();
        assert_eq!(view.status, "unmounted");
        assert!(Path::new(&view.source_upper).is_dir());
        assert!(Path::new(&view.generated_upper).is_dir());
        assert!(Path::new(&view.scratch_upper).is_dir());
        let space = db.lane_workspace_space("layered").unwrap();
        assert!(space.lane_exclusive_physical_bytes < 64 * 1024);

        drop(db);
        let reopened = Trail::open(temp.path()).unwrap();
        assert_eq!(
            reopened
                .lane_workspace_view("layered")
                .unwrap()
                .unwrap()
                .view_id,
            view.view_id
        );
    }

    #[test]
    fn offline_checkpoint_reads_only_source_upper_and_excludes_generated_files() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "checkpoint",
            Some("main"),
            mode,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let branch = db.lane_branch("checkpoint").unwrap();
        let head = db.get_ref(&branch.ref_name).unwrap();
        let paths = db.workspace_view_paths_for_lane("checkpoint").unwrap();
        let mut core = ViewCore::new_lazy(
            Trail::open(temp.path()).unwrap(),
            paths.source_upper.clone(),
            head.root_id,
        )
        .unwrap();
        let readme = core.lookup(VIEW_ROOT_INO, "README.md").unwrap();
        core.setattr(readme, Some(0), None).unwrap();
        core.write(readme, 0, b"changed\n").unwrap();
        let target = core.mkdir(VIEW_ROOT_INO, "target", 0o755).unwrap();
        let artifact = core.create(target.ino, "artifact", 0o644, true).unwrap();
        core.write(artifact.ino, 0, b"generated\n").unwrap();
        drop(core);

        let checkpoint = db
            .checkpoint_lane_workspace("checkpoint", Some("source only".to_string()))
            .unwrap();
        assert_eq!(checkpoint.source_paths, vec!["README.md"]);
        assert!(checkpoint.generated_dirty_paths >= 1);
        assert_eq!(checkpoint.generated_path_accounting, "journal_interval");
        assert!(checkpoint.journal_sequence > 0);
        let entry = db
            .root_file_entry(&checkpoint.root_id, "README.md")
            .unwrap()
            .unwrap();
        assert_eq!(db.materialize_entry_bytes(&entry).unwrap(), b"changed\n");
        assert!(db
            .root_file_entry(&checkpoint.root_id, "target/artifact")
            .unwrap()
            .is_none());
        let persisted = db.lane_workspace_view("checkpoint").unwrap().unwrap();
        assert_eq!(persisted.checkpoint_seq, checkpoint.journal_sequence);
        assert_eq!(persisted.checkpoint_root, Some(checkpoint.root_id));
    }

    #[test]
    fn source_upper_checkpoint_builds_exactly_the_native_directory_root() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::write(temp.path().join("README.md"), "baseline\n").unwrap();
        fs::write(temp.path().join("src/lower.rs"), "lower\n").unwrap();
        fs::write(temp.path().join("untouched.txt"), "untouched\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let root = db.resolve_branch_ref("main").unwrap().root_id;
        let view_dir = temp.path().join("equivalence-view");
        let source_upper = view_dir.join("source-upper");
        let mut view = ViewCore::new_lazy(
            Trail::open(temp.path()).unwrap(),
            source_upper.clone(),
            root.clone(),
        )
        .unwrap();
        let readme = view.lookup(VIEW_ROOT_INO, "README.md").unwrap();
        view.setattr(readme, Some(0), None).unwrap();
        view.write(readme, 0, b"changed\n").unwrap();
        let src = view.lookup(VIEW_ROOT_INO, "src").unwrap();
        let new_file = view.create(src, "new.rs", 0o644, true).unwrap();
        view.write(new_file.ino, 0, b"new\n").unwrap();
        view.remove(src, "lower.rs").unwrap();
        let candidates = view.checkpoint_candidates().unwrap();
        drop(view);

        let selected = candidates.paths.into_iter().collect::<Vec<_>>();
        assert_eq!(
            selected,
            vec![
                "README.md".to_string(),
                "src/lower.rs".to_string(),
                "src/new.rs".to_string(),
            ]
        );
        assert!(!selected.iter().any(|path| path == "untouched.txt"));
        let previous = db.load_root_files_for_paths(&root, &selected).unwrap();
        let upper_files = db
            .scan_files_under_for_paths(&source_upper, &selected)
            .unwrap();

        let native = temp.path().join("native-reference");
        fs::create_dir_all(&native).unwrap();
        db.materialize_root_files_at_streaming(&root, &native, false)
            .unwrap();
        fs::write(native.join("README.md"), "changed\n").unwrap();
        fs::write(native.join("src/new.rs"), "new\n").unwrap();
        fs::remove_file(native.join("src/lower.rs")).unwrap();
        let native_files = db.scan_files_under_for_paths(&native, &selected).unwrap();
        assert_eq!(
            upper_files
                .iter()
                .map(|file| (&file.path, &file.bytes, file.executable))
                .collect::<Vec<_>>(),
            native_files
                .iter()
                .map(|file| (&file.path, &file.bytes, file.executable))
                .collect::<Vec<_>>()
        );

        let change = ChangeId("change_layered_native_equivalence".to_string());
        let layered = db
            .build_root_for_selected_disk_files_incremental(
                &root,
                &previous,
                &upper_files,
                &selected,
                &change,
            )
            .unwrap();
        let native = db
            .build_root_for_selected_disk_files_incremental(
                &root,
                &previous,
                &native_files,
                &selected,
                &change,
            )
            .unwrap();
        assert_eq!(layered.root_id, native.root_id);
        assert_eq!(layered.files, native.files);
    }

    #[test]
    fn checkpoint_marker_recovery_preserves_newer_uncheckpointed_source_edits() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "crash",
            Some("main"),
            mode,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let branch = db.lane_branch("crash").unwrap();
        let head = db.get_ref(&branch.ref_name).unwrap();
        let paths = db.workspace_view_paths_for_lane("crash").unwrap();
        let mut core = ViewCore::new_lazy(
            Trail::open(temp.path()).unwrap(),
            paths.source_upper.clone(),
            head.root_id,
        )
        .unwrap();
        let readme = core.lookup(VIEW_ROOT_INO, "README.md").unwrap();
        core.setattr(readme, Some(0), None).unwrap();
        core.write(readme, 0, b"checkpointed\n").unwrap();
        drop(core);
        let checkpoint = db.checkpoint_lane_workspace("crash", None).unwrap();

        let head = db.get_ref(&branch.ref_name).unwrap();
        let mut core = ViewCore::new_lazy(
            Trail::open(temp.path()).unwrap(),
            paths.source_upper.clone(),
            head.root_id,
        )
        .unwrap();
        let newer = core
            .create(VIEW_ROOT_INO, "after-crash.rs", 0o644, true)
            .unwrap();
        core.write(newer.ino, 0, b"uncheckpointed\n").unwrap();
        drop(core);
        let view = db.lane_workspace_view("crash").unwrap().unwrap();
        fs::write(
            Path::new(&view.meta_dir).join("clean-checkpoint.json"),
            b"stale postcommit mirror",
        )
        .unwrap();
        drop(db);

        let reopened = Trail::open(temp.path()).unwrap();
        reopened.recover_workspace_views().unwrap();
        let recovered = reopened.lane_workspace_view("crash").unwrap().unwrap();
        assert_eq!(recovered.checkpoint_seq, checkpoint.journal_sequence);
        assert_eq!(recovered.checkpoint_root, Some(checkpoint.root_id));
        assert!(
            fs::read_to_string(paths.source_upper.join("after-crash.rs"))
                .unwrap()
                .contains("uncheckpointed")
        );
        let readiness = reopened.lane_readiness("crash").unwrap();
        assert!(readiness
            .blockers
            .iter()
            .any(|issue| issue.code == "uncheckpointed_source_changes"));
    }

    #[test]
    fn checkpoint_retries_safely_after_ref_advance_before_clean_marker() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "retry",
            Some("main"),
            mode,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let branch = db.lane_branch("retry").unwrap();
        let head = db.get_ref(&branch.ref_name).unwrap();
        let paths = db.workspace_view_paths_for_lane("retry").unwrap();
        let mut core = ViewCore::new_lazy(
            Trail::open(temp.path()).unwrap(),
            paths.source_upper.clone(),
            head.root_id,
        )
        .unwrap();
        let readme = core.lookup(VIEW_ROOT_INO, "README.md").unwrap();
        core.setattr(readme, Some(0), None).unwrap();
        core.write(readme, 0, b"durable\n").unwrap();
        drop(core);
        let recorded = db.record_lane_workdir("retry", None).unwrap();
        assert!(recorded.operation.is_some());
        let view = db.lane_workspace_view("retry").unwrap().unwrap();
        fs::remove_file(Path::new(&view.meta_dir).join("clean-checkpoint.json")).unwrap();
        drop(db);

        let mut reopened = Trail::open(temp.path()).unwrap();
        let retried = reopened.checkpoint_lane_workspace("retry", None).unwrap();
        assert!(retried.operation.is_none());
        assert_eq!(retried.root_id, recorded.root_id);
        assert_eq!(
            fs::read_to_string(paths.source_upper.join("README.md")).unwrap(),
            "durable\n"
        );
        let recovered = reopened.lane_workspace_view("retry").unwrap().unwrap();
        assert_eq!(recovered.checkpoint_root, Some(recorded.root_id));
        assert!(Path::new(&recovered.meta_dir)
            .join("clean-checkpoint.json")
            .is_file());
    }

    #[test]
    fn workspace_mount_lease_allows_one_writer_and_recovers_stale_owner() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "lease",
            Some("main"),
            mode,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let backend = db.lane_workspace_view("lease").unwrap().unwrap().backend;
        let mut lease = db.acquire_workspace_mount_lease("lease", &backend).unwrap();
        assert!(db.acquire_workspace_mount_lease("lease", &backend).is_err());
        lease.mark_mounted().unwrap();
        lease.heartbeat().unwrap();
        assert_eq!(
            db.lane_workspace_view("lease").unwrap().unwrap().status,
            "mounted"
        );
        let readiness = db.lane_readiness("lease").unwrap();
        assert!(readiness
            .blockers
            .iter()
            .any(|issue| issue.code == "workspace_view_active_writers"));
        drop(lease);
        assert_eq!(
            db.lane_workspace_view("lease").unwrap().unwrap().status,
            "unmounted"
        );

        let view = db.lane_workspace_view("lease").unwrap().unwrap();
        db.conn
            .execute(
                "UPDATE workspace_views SET owner_pid = ?1, owner_start_token = 'dead', status = 'mounted' WHERE view_id = ?2",
                params![u32::MAX as i64, view.view_id],
            )
            .unwrap();
        assert_eq!(db.recover_workspace_views().unwrap(), vec![view.view_id]);
        assert_eq!(
            db.lane_workspace_view("lease").unwrap().unwrap().status,
            "recovered"
        );
    }

    #[test]
    fn mark_mounted_fails_when_mount_lease_cas_is_lost() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "lease-cas",
            Some("main"),
            mode,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let view = db.lane_workspace_view("lease-cas").unwrap().unwrap();
        let mut lease = db
            .acquire_workspace_mount_lease("lease-cas", &view.backend)
            .unwrap();
        db.conn
            .execute(
                "UPDATE workspace_views SET owner_start_token = 'replacement-owner' WHERE view_id = ?1",
                params![view.view_id],
            )
            .unwrap();

        assert!(lease.mark_mounted().is_err());
        assert_ne!(
            db.lane_workspace_view("lease-cas").unwrap().unwrap().status,
            "mounted"
        );
    }

    #[test]
    fn workspace_mount_lease_waits_for_exclusive_view_barrier() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "barrier-lease",
            Some("main"),
            mode,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let view = db.lane_workspace_view("barrier-lease").unwrap().unwrap();
        let meta_dir = PathBuf::from(&view.meta_dir);
        let root = temp.path().to_path_buf();
        let backend = view.backend;
        drop(db);
        let (opened_tx, opened_rx) = mpsc::channel();
        let (go_tx, go_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let db = Trail::open(root).unwrap();
            opened_tx.send(()).unwrap();
            go_rx.recv().unwrap();
            let lease = db
                .acquire_workspace_mount_lease("barrier-lease", &backend)
                .unwrap();
            acquired_tx.send(()).unwrap();
            drop(lease);
        });
        opened_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let barrier = ViewMutationBarrier::exclusive(&meta_dir).unwrap();
        go_tx.send(()).unwrap();
        assert!(acquired_rx
            .recv_timeout(Duration::from_millis(150))
            .is_err());
        drop(barrier);
        acquired_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn unmount_request_is_acknowledged_by_the_mount_owner_before_lease_release() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "unmount-control",
            Some("main"),
            mode,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let view = db.lane_workspace_view("unmount-control").unwrap().unwrap();
        let mut lease = db
            .acquire_workspace_mount_lease("unmount-control", &view.backend)
            .unwrap();
        lease.mark_mounted().unwrap();
        let stop_path = Path::new(&view.meta_dir).join(VIEW_UNMOUNT_REQUEST_FILE);
        let owner = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !stop_path.is_file() {
                assert!(
                    Instant::now() < deadline,
                    "unmount request was not published"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            drop(lease);
        });
        let report = db
            .request_lane_workspace_unmount("unmount-control")
            .unwrap();
        owner.join().unwrap();
        assert!(report.healthy);
        assert_eq!(report.owner_pid, None);
        assert_eq!(
            db.lane_workspace_view("unmount-control")
                .unwrap()
                .unwrap()
                .status,
            "unmounted"
        );
    }

    #[test]
    fn rust_environment_shares_downloads_and_compiler_cache_but_not_target_state() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname='x'\nversion='0.1.0'\n",
        )
        .unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::write(temp.path().join("src/lib.rs"), "pub fn x() {}\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        for lane in ["cargo-a", "cargo-b"] {
            db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                lane,
                Some("main"),
                mode.clone(),
                None,
                None,
                None,
                &[],
                false,
            )
            .unwrap();
        }
        let environment = |lane: &str| {
            let view = db.lane_workspace_view(lane).unwrap().unwrap();
            let root = db
                .get_ref(&db.lane_branch(lane).unwrap().ref_name)
                .unwrap()
                .root_id;
            db.workspace_command_environment(&view, &root)
                .unwrap()
                .into_iter()
                .collect::<BTreeMap<_, _>>()
        };
        let a = environment("cargo-a");
        let b = environment("cargo-b");
        assert_eq!(a["CARGO_HOME"], b["CARGO_HOME"]);
        assert_eq!(a["SCCACHE_DIR"], b["SCCACHE_DIR"]);
        assert_ne!(a["CARGO_TARGET_DIR"], b["CARGO_TARGET_DIR"]);
        assert_eq!(
            Path::new(&a["CARGO_TARGET_DIR"]).parent(),
            Some(Path::new(
                &db.lane_workspace_view("cargo-a")
                    .unwrap()
                    .unwrap()
                    .mountpoint
            ))
        );
        assert_eq!(
            Path::new(&b["CARGO_TARGET_DIR"]).parent(),
            Some(Path::new(
                &db.lane_workspace_view("cargo-b")
                    .unwrap()
                    .unwrap()
                    .mountpoint
            ))
        );
        assert_eq!(
            Path::new(&a["CARGO_TARGET_DIR"]).file_name(),
            Some(std::ffi::OsStr::new("target"))
        );
        assert_eq!(
            Path::new(&b["CARGO_TARGET_DIR"]).file_name(),
            Some(std::ffi::OsStr::new("target"))
        );
    }

    #[test]
    fn configured_upper_quota_rejects_mutation_and_blocks_readiness() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "quota",
            Some("main"),
            mode,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        db.config_set("workspace_views.upper_logical_bytes", "4")
            .unwrap();
        let branch = db.lane_branch("quota").unwrap();
        let root = db.get_ref(&branch.ref_name).unwrap().root_id;
        let paths = db.workspace_view_paths_for_lane("quota").unwrap();
        let mut core = ViewCore::new_lazy(
            Trail::open(temp.path()).unwrap(),
            paths.source_upper.clone(),
            root,
        )
        .unwrap();
        let readme = core.lookup(VIEW_ROOT_INO, "README.md").unwrap();
        assert_eq!(core.write(readme, 0, b"too large").unwrap_err(), 28);
        drop(core);

        // Out-of-band writes are still diagnosed and prevent landing.
        fs::write(paths.source_upper.join("oversized.rs"), "12345").unwrap();
        let quota = db.workspace_quota_status("quota").unwrap();
        assert_eq!(quota.exceeded, vec!["upper_logical_bytes"]);
        let readiness = db.lane_readiness("quota").unwrap();
        assert!(readiness
            .blockers
            .iter()
            .any(|issue| issue.code == "workspace_quota_exceeded"));
    }

    #[test]
    fn million_path_twenty_view_scale_acceptance() {
        if std::env::var_os("TRAIL_RUN_MILLION_PATH_VIEW_TEST").is_none() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("seed.txt"), "seed\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let original = db.resolve_branch_ref("main").unwrap();
        let mut entry = db
            .root_file_entry(&original.root_id, "seed.txt")
            .unwrap()
            .unwrap();
        let change = original.change_id.clone();
        let started = Instant::now();
        let mut paths = SortedBatchBuilder::new(db.store.clone(), root_map_prolly_config());
        let mut file_index = BatchBuilder::new(db.store.clone(), root_map_prolly_config());
        let mut case_fold = SortedBatchBuilder::new(db.store.clone(), root_map_prolly_config());
        const PATHS: u64 = 1_000_000;
        for index in 0..PATHS {
            let path = format!("tree/{:04}/{:08}.txt", index / 10_000, index);
            entry.file_id = FileId::new(change.clone(), index + 1);
            paths
                .add(path.as_bytes().to_vec(), cbor(&entry).unwrap())
                .unwrap();
            file_index.add(entry.file_id.encode_key(), path.as_bytes().to_vec());
            case_fold
                .add(
                    case_insensitive_path_key(&path).into_bytes(),
                    path.as_bytes().to_vec(),
                )
                .unwrap();
        }
        let path_tree = paths.build().unwrap();
        let file_index_tree = file_index.build().unwrap();
        let case_fold_tree = case_fold.build().unwrap();
        let root = WorktreeRoot {
            version: ROOT_OBJECT_VERSION,
            path_map_root: tree_root_hex(&path_tree),
            file_index_map_root: tree_root_hex(&file_index_tree),
            case_fold_map_root: tree_root_hex(&case_fold_tree),
            file_count: PATHS,
            total_text_bytes: PATHS * entry.size_bytes,
            created_by: change.clone(),
        };
        let root_id = db
            .put_object(WORKTREE_ROOT_KIND, ROOT_OBJECT_VERSION, &root)
            .unwrap();
        db.set_ref(
            "refs/branches/main",
            &change,
            &root_id,
            &original.operation_id,
        )
        .unwrap();
        let root_build_ms = started.elapsed().as_millis();
        let rss_before = process_resident_bytes();
        let view_started = Instant::now();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        let mut cores = Vec::new();
        let mut exclusive_bytes = 0_u64;
        for index in 0..20 {
            let lane = format!("scale-{index:02}");
            db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                &lane,
                Some("main"),
                mode.clone(),
                None,
                None,
                None,
                &[],
                false,
            )
            .unwrap();
            let paths = db.workspace_view_paths_for_lane(&lane).unwrap();
            let core = ViewCore::new_lazy(
                Trail::open(temp.path()).unwrap(),
                paths.source_upper,
                root_id.clone(),
            )
            .unwrap();
            assert_eq!(core.indexed_path_count(), 1);
            cores.push(core);
            exclusive_bytes = exclusive_bytes.saturating_add(
                db.lane_workspace_space(&lane)
                    .unwrap()
                    .lane_exclusive_physical_bytes,
            );
        }
        let view_create_ms = view_started.elapsed().as_millis();
        let rss_after = process_resident_bytes();
        assert_eq!(cores.len(), 20);
        assert!(exclusive_bytes < 20 * 10 * 1024 * 1024);
        println!(
            "{}",
            serde_json::json!({
                "tracked_paths": PATHS,
                "views": 20,
                "root_build_ms": root_build_ms,
                "view_create_ms": view_create_ms,
                "rss_delta_bytes": rss_after.saturating_sub(rss_before),
                "exclusive_physical_bytes": exclusive_bytes,
                "indexed_paths_per_view": 1,
            })
        );
    }
}

#[cfg(all(test, target_os = "linux"))]
fn process_resident_bytes() -> u64 {
    fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|value| value.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map(|pages| pages.saturating_mul(4096))
        .unwrap_or(0)
}

#[cfg(all(test, target_os = "macos"))]
fn process_resident_bytes() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0 {
        unsafe { usage.assume_init().ru_maxrss as u64 }
    } else {
        0
    }
}

#[cfg(all(test, not(any(target_os = "linux", target_os = "macos"))))]
fn process_resident_bytes() -> u64 {
    0
}
