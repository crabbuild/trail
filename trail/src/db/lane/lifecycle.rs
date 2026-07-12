use super::*;

const LARGE_LANE_MATERIALIZE_FILE_THRESHOLD: u64 = 10_000;

impl Trail {
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
            self.resolve_automatic_lane_workdir_mode(from)?
        } else if let Some(requested_mode) = requested_mode {
            parse_lane_workdir_mode(requested_mode)?
        } else if no_materialize || materialize == Some(false) {
            LaneWorkdirMode::Virtual
        } else if !sparse_paths.is_empty() {
            LaneWorkdirMode::Sparse
        } else if custom_workdir || materialize == Some(true) {
            LaneWorkdirMode::FullCow
        } else if self.default_lane_materialize_for_ref(from)? {
            LaneWorkdirMode::FullCow
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

    fn resolve_automatic_lane_workdir_mode(&self, from: Option<&str>) -> Result<LaneWorkdirMode> {
        #[cfg(target_os = "windows")]
        {
            let _ = from;
            Ok(LaneWorkdirMode::DokanCow)
        }
        #[cfg(not(target_os = "windows"))]
        {
            #[cfg(target_os = "macos")]
            {
                if Path::new("/sbin/mount_nfs").is_file() {
                    return Ok(LaneWorkdirMode::NfsCow);
                }
            }
            #[cfg(target_os = "linux")]
            {
                if Path::new("/dev/fuse").exists() {
                    return Ok(LaneWorkdirMode::FuseCow);
                }
            }
            let source = match from {
                Some(refish) => self.resolve_refish(refish)?,
                None => self.resolve_branch_ref(&self.current_branch()?)?,
            };
            let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, &source.root_id)?;
            if root.file_count > LARGE_LANE_MATERIALIZE_FILE_THRESHOLD {
                return Err(Error::InvalidInput(format!(
                    "automatic layered workdir selection found no available mount backend for a {}-path root; install/enable FUSE, use nfs-cow on macOS, Dokan on Windows, or explicitly accept full-cow",
                    root.file_count
                )));
            }
            Ok(LaneWorkdirMode::FullCow)
        }
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
                LaneWorkdirMode::FullCow
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
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(name)?;
        validate_lane_workdir_mode_request(&workdir_mode, workdir.is_some(), sparse_paths)?;
        let sparse_paths = normalize_record_paths(sparse_paths)?;
        let source = match from {
            Some(refish) => self.resolve_refish(refish)?,
            None => self.resolve_branch_ref(&self.current_branch()?)?,
        };
        let lane_id = format!("lane_{}", crate::ids::short_hash(name.as_bytes(), 8));
        let ref_name = lane_ref(name);
        if self.try_get_ref(&ref_name)?.is_some() {
            return Err(Error::InvalidInput(format!("lane `{name}` already exists")));
        }
        let workdir_path = if workdir_mode.materializes() {
            Some(self.resolve_lane_workdir_path(name, workdir.as_deref())?)
        } else {
            None
        };
        let transparent_cow_available = workdir_mode.is_transparent_cow();
        let mut sparse_policy_paths = None;
        let materialized_workdir = if let Some(dir) = &workdir_path {
            match &workdir_mode {
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
                LaneWorkdirMode::Sparse | LaneWorkdirMode::FullCow => {
                    self.materialize_lane_workdir_at_paths_with_neighbors(
                        &source.root_id,
                        dir,
                        workdir.is_some(),
                        &sparse_paths,
                        include_neighbors,
                    )?;
                    if !sparse_paths.is_empty() {
                        sparse_policy_paths = self.sparse_workdir_paths(dir)?;
                    }
                }
                LaneWorkdirMode::Virtual => {}
            }
            Some(dir.to_string_lossy().to_string())
        } else {
            None
        };
        let sparse_paths_for_report = sparse_policy_paths.clone().unwrap_or_default();
        let metadata_json = serde_json::to_string(&serde_json::json!({
            "workdir_mode": workdir_mode.as_str(),
            "cow_backend": workdir_mode.cow_backend(),
            "sparse_paths": sparse_paths_for_report,
            "include_neighbors": include_neighbors,
            "transparent_cow_available": transparent_cow_available
        }))?;
        self.set_ref(
            &ref_name,
            &source.change_id,
            &source.root_id,
            &source.operation_id,
        )?;
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO lanes (lane_id, name, kind, provider, model, created_at, metadata_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                lane_id,
                name,
                "coding-lane",
                provider,
                model,
                now,
                metadata_json
            ],
        )?;
        self.conn.execute(
            "INSERT INTO lane_branches \
             (lane_id, ref_name, base_change, head_change, base_root, head_root, session_id, workdir, status, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'active', ?9, ?9)",
            params![
                lane_id,
                ref_name,
                source.change_id.0,
                source.change_id.0,
                source.root_id.0,
                source.root_id.0,
                Option::<String>::None,
                materialized_workdir,
                now
            ],
        )?;
        if workdir_mode.is_transparent_cow() {
            let mountpoint = materialized_workdir.as_deref().ok_or_else(|| {
                Error::Corrupt("transparent COW lane has no mountpoint".to_string())
            })?;
            self.create_workspace_view(
                &lane_id,
                &source.change_id,
                &source.root_id,
                platform_workspace_backend(&workdir_mode),
                Path::new(mountpoint),
            )?;
        }
        self.insert_lane_event(
            &lane_id,
            "lane_spawned",
            Some(&source.change_id),
            None,
            &serde_json::json!({
                "ref_name": ref_name.clone(),
                "base_root": source.root_id.0.clone(),
                "workdir": materialized_workdir.clone(),
                "workdir_mode": workdir_mode.as_str(),
                "cow_backend": workdir_mode.cow_backend(),
                "sparse_paths": sparse_policy_paths.clone().unwrap_or_default(),
                "include_neighbors": include_neighbors,
                "transparent_cow_available": transparent_cow_available
            }),
        )?;
        Ok(LaneSpawnReport {
            lane_id,
            ref_name,
            base_change: source.change_id,
            workdir: materialized_workdir,
            cow_backend: workdir_mode.cow_backend().map(str::to_string),
            sparse_paths: sparse_policy_paths.unwrap_or_default(),
            transparent_cow_available,
            workdir_mode,
        })
    }

    pub fn ensure_lane_workdir_materialized(
        &mut self,
        lane: &str,
        workdir: Option<PathBuf>,
    ) -> Result<LaneWorkdirReport> {
        let _lock = self.acquire_write_lock()?;
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
            let sparse_paths = self.lane_report_sparse_paths(&branch)?;
            let transparent_cow_available = workdir_mode.is_transparent_cow();
            return Ok(LaneWorkdirReport {
                lane_id: branch.lane_id,
                workdir: Some(existing),
                cow_backend: workdir_mode.cow_backend().map(str::to_string),
                sparse_paths,
                transparent_cow_available,
                workdir_mode,
            });
        }

        let head = self.get_ref(&branch.ref_name)?;
        let dir = self.materialize_lane_workdir(lane, &head.root_id, workdir.as_deref())?;
        let workdir = dir.to_string_lossy().to_string();
        self.conn.execute(
            "UPDATE lane_branches SET workdir = ?1, updated_at = ?2 WHERE lane_id = ?3",
            params![workdir, now_ts(), branch.lane_id],
        )?;
        self.insert_lane_event(
            &branch.lane_id,
            "workdir_materialized",
            Some(&head.change_id),
            None,
            &serde_json::json!({
                "workdir": workdir,
                "root_id": head.root_id.0
            }),
        )?;
        Ok(LaneWorkdirReport {
            lane_id: branch.lane_id,
            workdir: Some(dir.to_string_lossy().to_string()),
            workdir_mode: LaneWorkdirMode::FullCow,
            cow_backend: (LaneWorkdirMode::FullCow).cow_backend().map(str::to_string),
            sparse_paths: Vec::new(),
            transparent_cow_available: false,
        })
    }

    pub(crate) fn materialize_lane_workdir(
        &self,
        name: &str,
        root_id: &ObjectId,
        custom_workdir: Option<&Path>,
    ) -> Result<PathBuf> {
        let dir = self.resolve_lane_workdir_path(name, custom_workdir)?;
        self.materialize_lane_workdir_at(root_id, &dir, custom_workdir.is_some())?;
        Ok(dir)
    }

    pub(crate) fn materialize_lane_workdir_at(
        &self,
        root_id: &ObjectId,
        dir: &Path,
        custom_workdir: bool,
    ) -> Result<()> {
        self.materialize_lane_workdir_at_paths(root_id, dir, custom_workdir, &[])
    }

    pub(crate) fn materialize_lane_workdir_at_paths(
        &self,
        root_id: &ObjectId,
        dir: &Path,
        custom_workdir: bool,
        sparse_paths: &[String],
    ) -> Result<()> {
        self.materialize_lane_workdir_at_paths_with_neighbors(
            root_id,
            dir,
            custom_workdir,
            sparse_paths,
            false,
        )
    }

    pub(crate) fn materialize_lane_workdir_at_paths_with_neighbors(
        &self,
        root_id: &ObjectId,
        dir: &Path,
        custom_workdir: bool,
        sparse_paths: &[String],
        include_neighbors: bool,
    ) -> Result<()> {
        prepare_lane_workdir(dir, custom_workdir)?;
        if sparse_paths.is_empty() {
            let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
            if root.file_count > LARGE_LANE_MATERIALIZE_FILE_THRESHOLD {
                return self.materialize_lane_workdir_at_streaming(root_id, dir);
            }
        }
        let empty = BTreeMap::new();
        let files = if sparse_paths.is_empty() {
            self.load_root_files(root_id)?
        } else if include_neighbors {
            self.load_root_files_for_selections_with_neighbors(root_id, sparse_paths)?
        } else {
            self.load_root_files_for_selections(root_id, sparse_paths)?
        };
        let mut materialized = None;
        let cloned_from_workspace = if sparse_paths.is_empty() {
            let source_stamps =
                match self.workspace_file_stamps_if_clean_index_matches(root_id, &files)? {
                    Some(stamps) => Some(stamps),
                    None => self.workspace_file_stamps_if_entries_match(&files)?,
                };
            if let Some(source_stamps) = source_stamps {
                if let Some(report) = materialize_from_workspace_cow_report(
                    &self.workspace_root,
                    dir,
                    &files,
                    &source_stamps,
                    false,
                )? {
                    materialized = Some(report);
                    true
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };
        if !cloned_from_workspace {
            materialized = Some(if sparse_paths.is_empty() {
                self.materialize_files_best_effort_at_report(dir, &empty, &files)?
            } else {
                self.materialize_new_files_best_effort_at_with_workspace_cow_report(dir, &files)?
            });
        }
        if sparse_paths.is_empty() {
            remove_sparse_workdir_manifest(dir)?;
        } else {
            self.write_sparse_workdir_manifest(dir, files.keys())?;
        }
        if let Some(report) = materialized {
            self.write_clean_workdir_manifest_from_stamps(
                dir,
                root_id,
                &files,
                files.keys(),
                report.stamps,
            )?;
        } else {
            self.write_clean_workdir_manifest(dir, root_id, &files, files.keys())?;
        }
        Ok(())
    }

    fn materialize_lane_workdir_at_streaming(&self, root_id: &ObjectId, dir: &Path) -> Result<()> {
        let report = self.materialize_root_files_at_streaming(root_id, dir, false)?;
        remove_sparse_workdir_manifest(dir)?;
        self.write_clean_workdir_manifest_from_disk_manifest_and_stamps(
            dir,
            root_id,
            &report.disk_manifest,
            report.disk_manifest.keys(),
            report.materialized.stamps,
        )
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
            Ok(LaneWorkdirMode::FullCow)
        } else {
            Ok(LaneWorkdirMode::Virtual)
        }
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
        write_file_atomic(&manifest, &serde_json::to_vec(&body)?, false)?;
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
        _ => {}
    }
    LaneWorkdirMode::parse(value).ok_or_else(|| {
        Error::InvalidInput(format!(
            "unknown lane workdir mode `{value}`; expected auto, virtual, sparse, full-cow, fuse-cow, nfs-cow, or dokan-cow"
        ))
    })
}

fn platform_workspace_backend(mode: &LaneWorkdirMode) -> &'static str {
    match mode {
        LaneWorkdirMode::NfsCow => "nfs",
        LaneWorkdirMode::FuseCow => "fuse",
        LaneWorkdirMode::DokanCow => "dokan",
        LaneWorkdirMode::Sparse | LaneWorkdirMode::FullCow => "clone",
        LaneWorkdirMode::Virtual => "virtual",
    }
}

fn validate_lane_workdir_mode_request(
    mode: &LaneWorkdirMode,
    custom_workdir: bool,
    sparse_paths: &[String],
) -> Result<()> {
    match mode {
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
        LaneWorkdirMode::FullCow => {
            if !sparse_paths.is_empty() {
                return Err(Error::InvalidInput(
                    "full-cow lane workdir mode cannot be combined with sparse paths".to_string(),
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
    dir.join(".trail").join("sparse-workdir.json")
}

fn remove_sparse_workdir_manifest(dir: &Path) -> Result<()> {
    let manifest = sparse_workdir_manifest_path(dir);
    if manifest.exists() {
        fs::remove_file(manifest)?;
    }
    Ok(())
}

#[cfg(test)]
mod hard_cutover_tests {
    use super::*;

    #[test]
    fn removed_cow_mode_reports_the_recreate_lifecycle() {
        let overlay_error = parse_lane_workdir_mode("overlay-cow").unwrap_err();
        let overlay_message = overlay_error.to_string();
        assert!(overlay_message.contains("hard-cutover modes `fuse-cow` and `dokan-cow`"));
        assert!(overlay_message.contains("remove and recreate the lane"));
    }
}
