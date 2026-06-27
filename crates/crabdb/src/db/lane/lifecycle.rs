use super::*;

const LARGE_LANE_MATERIALIZE_FILE_THRESHOLD: u64 = 10_000;

impl CrabDb {
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
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(name)?;
        if workdir.is_some() && !materialize {
            return Err(Error::InvalidInput(
                "custom lane workdir requires materialization to be enabled".to_string(),
            ));
        }
        if !sparse_paths.is_empty() && !materialize {
            return Err(Error::InvalidInput(
                "sparse lane workdir paths require materialization to be enabled".to_string(),
            ));
        }
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
        let workdir_path = if materialize {
            Some(self.resolve_lane_workdir_path(name, workdir.as_deref())?)
        } else {
            None
        };
        let materialized_workdir = if let Some(dir) = &workdir_path {
            self.materialize_lane_workdir_at_paths_with_neighbors(
                &source.root_id,
                dir,
                workdir.is_some(),
                &sparse_paths,
                include_neighbors,
            )?;
            Some(dir.to_string_lossy().to_string())
        } else {
            None
        };
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
                Option::<String>::None
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
        self.insert_lane_event(
            &lane_id,
            "lane_spawned",
            Some(&source.change_id),
            None,
            &serde_json::json!({
                "ref_name": ref_name.clone(),
                "base_root": source.root_id.0.clone(),
                "workdir": materialized_workdir.clone()
            }),
        )?;
        Ok(LaneSpawnReport {
            lane_id,
            ref_name,
            base_change: source.change_id,
            workdir: materialized_workdir,
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
            return Ok(LaneWorkdirReport {
                lane_id: branch.lane_id,
                workdir: Some(existing),
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
        fs::write(manifest, serde_json::to_vec(&body)?)?;
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
    dir.join(".crabdb").join("sparse-workdir.json")
}

fn remove_sparse_workdir_manifest(dir: &Path) -> Result<()> {
    let manifest = sparse_workdir_manifest_path(dir);
    if manifest.exists() {
        fs::remove_file(manifest)?;
    }
    Ok(())
}
