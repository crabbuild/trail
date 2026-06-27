use super::*;

impl CrabDb {
    pub fn lane_workdir(&self, lane: &str) -> Result<LaneWorkdirReport> {
        validate_ref_segment(lane)?;
        let branch = self.lane_branch(lane)?;
        Ok(LaneWorkdirReport {
            lane_id: branch.lane_id,
            workdir: branch.workdir,
        })
    }

    pub fn read_lane_file(
        &mut self,
        lane: &str,
        path: &str,
        hydrate: bool,
        force: bool,
        include_neighbors: bool,
    ) -> Result<LaneFileReadReport> {
        self.read_lane_file_with_hydration(lane, path, Some(hydrate), force, include_neighbors)
    }

    pub fn read_lane_file_with_hydration(
        &mut self,
        lane: &str,
        path: &str,
        hydrate: Option<bool>,
        force: bool,
        include_neighbors: bool,
    ) -> Result<LaneFileReadReport> {
        validate_ref_segment(lane)?;
        let path = normalize_relative_path(path)?;
        let branch = self.lane_branch(lane)?;
        let hydrate = match hydrate {
            Some(hydrate) => hydrate,
            None => branch_has_sparse_workdir(self, &branch)?,
        };
        let _lock = if hydrate {
            Some(self.acquire_write_lock()?)
        } else {
            None
        };
        let head = self.get_ref(&branch.ref_name)?;
        let mut entries = self.load_root_files_for_paths(&head.root_id, &[path.clone()])?;
        let entry = entries
            .remove(&path)
            .ok_or_else(|| Error::InvalidInput(format!("lane `{lane}` has no file `{path}`")))?;
        let bytes = self.materialize_entry_bytes(&entry)?;
        let byte_count = bytes.len() as u64;
        let content = match String::from_utf8(bytes) {
            Ok(text) => (text, "utf-8".to_string()),
            Err(err) => (hex::encode(err.into_bytes()), "hex".to_string()),
        };
        let hydrated_paths = if hydrate {
            self.hydrate_sparse_lane_workdir_paths_unlocked(
                lane,
                &branch,
                std::slice::from_ref(&path),
                force,
                include_neighbors,
            )?
        } else {
            Vec::new()
        };

        Ok(LaneFileReadReport {
            lane_id: branch.lane_id,
            ref_name: branch.ref_name,
            root_id: head.root_id.0,
            path,
            kind: entry.kind,
            byte_count,
            content_hash: entry.content_hash,
            content_encoding: content.1,
            content: content.0,
            hydrated_paths,
        })
    }

    pub fn sync_lane_workdir(&mut self, lane: &str, force: bool) -> Result<LaneWorkdirSyncReport> {
        self.sync_lane_workdir_with_paths(lane, force, &[])
    }

    pub fn sync_lane_workdir_with_paths(
        &mut self,
        lane: &str,
        force: bool,
        paths: &[String],
    ) -> Result<LaneWorkdirSyncReport> {
        self.sync_lane_workdir_with_paths_and_neighbors(lane, force, paths, false)
    }

    pub fn sync_lane_workdir_with_paths_and_neighbors(
        &mut self,
        lane: &str,
        force: bool,
        paths: &[String],
        include_neighbors: bool,
    ) -> Result<LaneWorkdirSyncReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(lane)?;
        let selected_paths = normalize_record_paths(paths)?;
        let path_scoped = !selected_paths.is_empty();
        let branch = self.lane_branch(lane)?;
        let Some(workdir) = branch.workdir.clone() else {
            return Err(Error::InvalidInput(format!(
                "lane `{lane}` does not have a materialized workdir"
            )));
        };
        let workdir_path = PathBuf::from(&workdir);
        if workdir_path.exists() && !workdir_path.is_dir() {
            if force {
                fs::remove_file(&workdir_path)?;
            } else {
                return Err(Error::InvalidInput(format!(
                    "lane `{lane}` workdir path exists but is not a directory"
                )));
            }
        }
        let head = self.get_ref(&branch.ref_name)?;
        let target_files = if path_scoped {
            let target_files = if include_neighbors {
                self.load_root_files_for_selections_with_neighbors(&head.root_id, &selected_paths)?
            } else {
                self.load_root_files_for_selections(&head.root_id, &selected_paths)?
            };
            if target_files.is_empty() {
                return Err(Error::InvalidInput(format!(
                    "no files in lane `{lane}` branch match requested sync paths"
                )));
            }
            target_files
        } else {
            self.load_root_files(&head.root_id)?
        };
        let workdir_exists = workdir_path.is_dir();
        let sparse_paths = if workdir_exists {
            self.sparse_workdir_paths(&workdir_path)?
        } else {
            None
        };
        if path_scoped && workdir_exists && sparse_paths.is_none() {
            return Err(Error::InvalidInput(
                "path-scoped sync-workdir is only supported for sparse lane workdirs".to_string(),
            ));
        }
        let changed_paths = if path_scoped {
            if workdir_exists {
                self.sparse_hydration_changed_paths(
                    &workdir_path,
                    sparse_paths.as_deref().unwrap_or_default(),
                    &target_files,
                )?
            } else {
                Vec::new()
            }
        } else if workdir_exists {
            self.lane_workdir_changed_paths(&branch, &head)?
                .unwrap_or_default()
        } else {
            self.diff_file_maps(&BTreeMap::new(), &target_files)?
                .summaries
        };
        if workdir_exists && !changed_paths.is_empty() && !force {
            let preview = changed_paths
                .iter()
                .take(5)
                .map(|path| format!("{:?} {}", path.kind, path.path))
                .collect::<Vec<_>>()
                .join(", ");
            let suffix = if changed_paths.len() > 5 {
                format!(", ... {} more", changed_paths.len() - 5)
            } else {
                String::new()
            };
            return Err(Error::DirtyWorktreeWithMessage(format!(
                "lane `{lane}` workdir has unrecorded changes; run `crabdb lane record {lane}` or pass `--force` to sync: {preview}{suffix}"
            )));
        }
        let rescue_workdir = if force && workdir_exists && !changed_paths.is_empty() {
            Some(self.rescue_dirty_lane_workdir(lane, &workdir, &workdir_path, &changed_paths)?)
        } else {
            None
        };
        if force && !path_scoped && workdir_path.exists() {
            fs::remove_dir_all(&workdir_path)?;
        }
        fs::create_dir_all(&workdir_path)?;
        if path_scoped {
            let write_files =
                self.sparse_hydration_write_files(&workdir_path, &target_files, force)?;
            self.materialize_new_files_best_effort_at_with_workspace_cow(
                &workdir_path,
                &write_files,
            )?;
            let mut materialized_paths = sparse_paths.unwrap_or_default();
            materialized_paths.extend(target_files.keys().cloned());
            materialized_paths.sort();
            materialized_paths.dedup();
            self.write_sparse_workdir_manifest(&workdir_path, materialized_paths.iter())?;
            self.update_clean_workdir_manifest_from_file_subset(
                &workdir_path,
                &head.root_id,
                &head.root_id,
                &BTreeMap::new(),
                &target_files,
            )?;
        } else {
            let target_files = if let Some(paths) = &sparse_paths {
                self.selected_file_entries(&target_files, paths)
            } else {
                target_files
            };
            let previous = if force || !workdir_exists {
                BTreeMap::new()
            } else {
                target_files.clone()
            };
            if force || !workdir_exists || !changed_paths.is_empty() {
                self.materialize_files_best_effort_at(&workdir_path, &previous, &target_files)?;
            }
            if sparse_paths.is_some() {
                self.write_sparse_workdir_manifest(&workdir_path, target_files.keys())?;
            }
            self.write_clean_workdir_manifest(
                &workdir_path,
                &head.root_id,
                &target_files,
                target_files.keys(),
            )?;
        }
        self.insert_lane_event(
            &branch.lane_id,
            "workdir_synced",
            Some(&head.change_id),
            None,
            &serde_json::json!({
                "workdir": workdir.clone(),
                "forced": force,
                "rescue_workdir": rescue_workdir.clone(),
                "paths": selected_paths,
                "include_neighbors": include_neighbors,
                "changed_paths": changed_paths.iter().map(|item| item.path.clone()).collect::<Vec<_>>()
            }),
        )?;
        Ok(LaneWorkdirSyncReport {
            lane_id: branch.lane_id,
            workdir,
            head_change: head.change_id,
            root_id: head.root_id,
            forced: force,
            rescue_workdir,
            changed_paths,
        })
    }

    fn rescue_dirty_lane_workdir(
        &self,
        lane: &str,
        workdir: &str,
        workdir_path: &Path,
        changed_paths: &[FileDiffSummary],
    ) -> Result<String> {
        let rescue_root = self.db_dir.join("lane-workdir-rescue");
        fs::create_dir_all(&rescue_root)?;
        let rescue_dir = create_unique_lane_workdir_rescue_dir(&rescue_root, lane)?;
        let files_dir = rescue_dir.join("files");
        fs::create_dir_all(&files_dir)?;

        let mut copied_paths = Vec::new();
        let mut skipped_paths = Vec::new();
        let mut candidate_paths = BTreeSet::new();
        for changed_path in changed_paths {
            candidate_paths.insert(changed_path.path.clone());
            if let Some(old_path) = &changed_path.old_path {
                candidate_paths.insert(old_path.clone());
            }
        }

        for path in candidate_paths {
            let source = match safe_join(workdir_path, &path) {
                Ok(source) => source,
                Err(err) => {
                    skipped_paths.push(format!("{path}: {err}"));
                    continue;
                }
            };
            let metadata = match fs::symlink_metadata(&source) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    skipped_paths.push(format!("{path}: missing"));
                    continue;
                }
                Err(err) => return Err(Error::Io(err)),
            };
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                skipped_paths.push(format!("{path}: not a regular file"));
                continue;
            }
            let destination = files_dir.join(path_from_rel(&path));
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&source, &destination)?;
            copied_paths.push(path);
        }

        let manifest = serde_json::json!({
            "lane": lane,
            "workdir": workdir,
            "created_at": now_ts(),
            "changed_paths": changed_paths,
            "copied_paths": copied_paths,
            "skipped_paths": skipped_paths,
        });
        fs::write(
            rescue_dir.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )?;
        Ok(rescue_dir.to_string_lossy().to_string())
    }

    pub(crate) fn hydrate_sparse_lane_workdir_paths_unlocked(
        &self,
        lane: &str,
        branch: &LaneBranch,
        paths: &[String],
        force: bool,
        include_neighbors: bool,
    ) -> Result<Vec<String>> {
        let selected_paths = normalize_record_paths(paths)?;
        if selected_paths.is_empty() {
            return Ok(Vec::new());
        }
        let Some(workdir) = branch.workdir.clone() else {
            return Ok(Vec::new());
        };
        let workdir_path = PathBuf::from(&workdir);
        if !workdir_path.is_dir() {
            return Ok(Vec::new());
        }
        let Some(sparse_paths) = self.sparse_workdir_paths(&workdir_path)? else {
            return Ok(Vec::new());
        };
        let head = self.get_ref(&branch.ref_name)?;
        let target_files = if include_neighbors {
            self.load_root_files_for_selections_with_neighbors(&head.root_id, &selected_paths)?
        } else {
            self.load_root_files_for_selections(&head.root_id, &selected_paths)?
        };
        if target_files.is_empty() {
            return Ok(Vec::new());
        }

        let changed_paths =
            self.sparse_hydration_changed_paths(&workdir_path, &sparse_paths, &target_files)?;
        if !changed_paths.is_empty() && !force {
            let preview = changed_paths
                .iter()
                .take(5)
                .map(|path| format!("{:?} {}", path.kind, path.path))
                .collect::<Vec<_>>()
                .join(", ");
            let suffix = if changed_paths.len() > 5 {
                format!(", ... {} more", changed_paths.len() - 5)
            } else {
                String::new()
            };
            return Err(Error::DirtyWorktreeWithMessage(format!(
                "lane `{lane}` workdir has unrecorded changes; run `crabdb lane record {lane}` or pass `--force` to sync: {preview}{suffix}"
            )));
        }

        let write_files = self.sparse_hydration_write_files(&workdir_path, &target_files, force)?;
        self.materialize_new_files_best_effort_at_with_workspace_cow(&workdir_path, &write_files)?;
        let mut materialized_paths = sparse_paths;
        materialized_paths.extend(target_files.keys().cloned());
        materialized_paths.sort();
        materialized_paths.dedup();
        self.write_sparse_workdir_manifest(&workdir_path, materialized_paths.iter())?;
        self.update_clean_workdir_manifest_from_file_subset(
            &workdir_path,
            &head.root_id,
            &head.root_id,
            &BTreeMap::new(),
            &target_files,
        )?;
        Ok(target_files.keys().cloned().collect())
    }

    fn sparse_hydration_write_files(
        &self,
        workdir_path: &Path,
        target_files: &BTreeMap<String, FileEntry>,
        force: bool,
    ) -> Result<BTreeMap<String, FileEntry>> {
        if force {
            return Ok(target_files.clone());
        }
        let mut write_files = BTreeMap::new();
        for (path, entry) in target_files {
            let abs = safe_join(workdir_path, path)?;
            match fs::symlink_metadata(&abs) {
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {}
                Ok(_) => {
                    write_files.insert(path.clone(), entry.clone());
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    write_files.insert(path.clone(), entry.clone());
                }
                Err(err) => return Err(Error::Io(err)),
            }
        }
        Ok(write_files)
    }

    fn sparse_hydration_changed_paths(
        &self,
        workdir_path: &Path,
        sparse_paths: &[String],
        target_files: &BTreeMap<String, FileEntry>,
    ) -> Result<Vec<FileDiffSummary>> {
        let target_paths = target_files.keys().cloned().collect::<Vec<_>>();
        if target_paths.is_empty() {
            return Ok(Vec::new());
        }

        let disk_files = self.scan_files_under_for_paths(workdir_path, &target_paths)?;
        let disk_paths = disk_files
            .iter()
            .map(|file| file.path.clone())
            .collect::<BTreeSet<_>>();
        let sparse_paths = sparse_paths.iter().cloned().collect::<BTreeSet<_>>();
        let candidate_paths = target_paths
            .into_iter()
            .filter(|path| sparse_paths.contains(path) || disk_paths.contains(path))
            .collect::<Vec<_>>();
        if candidate_paths.is_empty() {
            return Ok(Vec::new());
        }

        let head_files = self.selected_file_entries(target_files, &candidate_paths);
        let disk_manifest = self.disk_manifest(&disk_files);
        Ok(
            self.diff_file_maps_to_manifest_for_paths(
                &head_files,
                &disk_manifest,
                &candidate_paths,
            ),
        )
    }
}

fn branch_has_sparse_workdir(db: &CrabDb, branch: &LaneBranch) -> Result<bool> {
    let Some(workdir) = &branch.workdir else {
        return Ok(false);
    };
    let workdir_path = PathBuf::from(workdir);
    if !workdir_path.is_dir() {
        return Ok(false);
    }
    db.sparse_workdir_paths(&workdir_path)
        .map(|paths| paths.is_some())
}

fn create_unique_lane_workdir_rescue_dir(rescue_root: &Path, lane: &str) -> Result<PathBuf> {
    for _ in 0..16 {
        let candidate = rescue_root.join(format!("{lane}-{}", now_nanos()));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(Error::Io(err)),
        }
    }
    Err(Error::InvalidInput(
        "could not create unique lane workdir rescue directory".to_string(),
    ))
}
