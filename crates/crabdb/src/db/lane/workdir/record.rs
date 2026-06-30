use super::*;

impl CrabDb {
    pub fn preview_lane_workdir_record(&self, lane: &str) -> Result<LaneRecordPreviewReport> {
        validate_ref_segment(lane)?;
        let branch = self.lane_branch(lane)?;
        let Some(workdir) = branch.workdir.clone() else {
            return Err(Error::InvalidInput(format!(
                "lane `{lane}` does not have a materialized workdir"
            )));
        };
        let workdir_path = PathBuf::from(&workdir);
        if !workdir_path.is_dir() {
            return Err(Error::WorkspaceNotFound(workdir_path));
        }
        let head = self.get_ref(&branch.ref_name)?;
        let changed_paths =
            self.lane_workdir_record_changed_paths(&branch, &head, &workdir_path)?;
        let (ignored_paths, risky_paths) =
            self.preview_lane_workdir_path_warnings(&workdir_path)?;
        let oversized_files =
            self.lane_record_oversized_files_on_disk(&workdir_path, &changed_paths)?;
        let mut policy = self.preview_lane_record_policy(&branch, &changed_paths)?;
        if policy.error.is_none() {
            if let Err(err) = self
                .ensure_record_final_root_paths_safe_from_summaries(&head.root_id, &changed_paths)
            {
                policy.allowed = false;
                policy.error = Some(err.to_string());
            }
        }
        if !oversized_files.is_empty() && policy.error.is_none() {
            policy.allowed = false;
            policy.error = Some(lane_record_oversized_error(&oversized_files));
        }

        Ok(LaneRecordPreviewReport {
            lane_id: branch.lane_id,
            workdir,
            head_change: head.change_id,
            root_id: head.root_id,
            clean: changed_paths.is_empty(),
            changed_paths,
            ignored_paths,
            risky_paths,
            oversized_files,
            policy,
        })
    }

    pub fn record_lane_workdir(
        &mut self,
        lane: &str,
        message: Option<String>,
    ) -> Result<LaneRecordReport> {
        let _lock = self.acquire_write_lock()?;
        self.record_lane_workdir_locked(lane, message, None)
    }

    pub fn record_lane_workdir_for_turn(
        &mut self,
        lane: &str,
        turn_id: &str,
        message: Option<String>,
    ) -> Result<LaneRecordReport> {
        let _lock = self.acquire_write_lock()?;
        self.record_lane_workdir_locked(lane, message, Some(turn_id))
    }

    fn record_lane_workdir_locked(
        &mut self,
        lane: &str,
        message: Option<String>,
        existing_turn_id: Option<&str>,
    ) -> Result<LaneRecordReport> {
        validate_ref_segment(lane)?;
        let branch = self.lane_branch(lane)?;
        ensure_lane_record_message_has_no_secrets(message.as_deref())?;
        let existing_turn = existing_turn_id
            .map(|turn_id| self.lane_turn(turn_id))
            .transpose()?;
        if let Some(turn) = &existing_turn {
            if turn.lane_id != branch.lane_id {
                return Err(Error::InvalidInput(format!(
                    "turn `{}` does not belong to lane `{lane}`",
                    turn.turn_id
                )));
            }
            if turn.ended_at.is_some() {
                return Err(Error::InvalidInput(format!(
                    "turn `{}` is already ended",
                    turn.turn_id
                )));
            }
        }
        let Some(workdir) = branch.workdir.clone() else {
            return Err(Error::InvalidInput(format!(
                "lane `{lane}` does not have a materialized workdir"
            )));
        };
        let workdir_path = PathBuf::from(&workdir);
        if !workdir_path.is_dir() {
            return Err(Error::WorkspaceNotFound(workdir_path));
        }
        let head = self.get_ref(&branch.ref_name)?;
        let sparse_paths = self.lane_sparse_workdir_paths(&branch, &workdir_path)?;
        let is_sparse = sparse_paths.is_some();
        let overlay_manifest_path = self.lane_overlay_clean_manifest_path(&branch)?;
        let cached_status = self.lane_cached_workdir_manifest_status(
            &workdir_path,
            overlay_manifest_path.as_deref(),
            &head.root_id,
        )?;
        if matches!(cached_status, CachedWorkdirManifestStatus::Clean) {
            return Ok(LaneRecordReport {
                lane_id: branch.lane_id,
                operation: None,
                root_id: head.root_id,
                changed_paths: Vec::new(),
            });
        }
        if let Some(session_id) = &branch.session_id {
            self.preflight_lane_session_owner(&branch.lane_id, session_id)?;
        }

        let actor = Actor::lane(lane);
        let change_id = self.allocate_change_id(&actor.id, "lane_record")?;
        let (built, materialized_paths, previous_files, clean_disk_manifest) = match cached_status {
            CachedWorkdirManifestStatus::Clean => unreachable!(),
            CachedWorkdirManifestStatus::Dirty {
                disk_manifest,
                candidate_paths,
            } => {
                let (summaries, previous_files, use_disk_manifest_for_clean) =
                    if let Some(mut selected_paths) = sparse_paths.clone() {
                        selected_paths.extend(disk_manifest.keys().cloned());
                        selected_paths.sort();
                        selected_paths.dedup();
                        let previous_files =
                            self.load_root_files_for_selections(&head.root_id, &selected_paths)?;
                        let summaries = self.diff_file_maps_to_manifest_for_paths(
                            &previous_files,
                            &disk_manifest,
                            &selected_paths,
                        );
                        (summaries, previous_files, false)
                    } else if let Some(candidate_paths) = candidate_paths {
                        let previous_files =
                            self.load_root_files_for_paths(&head.root_id, &candidate_paths)?;
                        let summaries = self.diff_file_maps_to_manifest_for_paths(
                            &previous_files,
                            &disk_manifest,
                            &candidate_paths,
                        );
                        (summaries, previous_files, true)
                    } else {
                        let summaries =
                            self.diff_root_to_disk_manifest(&head.root_id, &disk_manifest)?;
                        let selected_paths = summaries
                            .iter()
                            .map(|summary| summary.path.clone())
                            .collect::<Vec<_>>();
                        let previous_files =
                            self.load_root_files_for_paths(&head.root_id, &selected_paths)?;
                        (summaries, previous_files, true)
                    };
                let materialized_paths = disk_manifest.keys().cloned().collect::<Vec<_>>();
                if summaries.is_empty() {
                    if is_sparse {
                        self.write_sparse_workdir_manifest(
                            &workdir_path,
                            materialized_paths.iter(),
                        )?;
                    }
                    if use_disk_manifest_for_clean {
                        self.write_lane_clean_workdir_manifest_from_disk_manifest(
                            &workdir_path,
                            overlay_manifest_path.as_deref(),
                            &head.root_id,
                            &disk_manifest,
                            materialized_paths.iter(),
                        )?;
                    } else {
                        self.write_lane_clean_workdir_manifest(
                            &workdir_path,
                            overlay_manifest_path.as_deref(),
                            &head.root_id,
                            &previous_files,
                            materialized_paths.iter(),
                        )?;
                    }
                    return Ok(LaneRecordReport {
                        lane_id: branch.lane_id,
                        operation: None,
                        root_id: head.root_id,
                        changed_paths: Vec::new(),
                    });
                }
                let selected_paths = summaries
                    .iter()
                    .map(|summary| summary.path.clone())
                    .collect::<Vec<_>>();
                let disk_files = self.scan_files_under_for_paths(&workdir_path, &selected_paths)?;
                let built = self.build_root_for_selected_disk_files_incremental(
                    &head.root_id,
                    &previous_files,
                    &disk_files,
                    &selected_paths,
                    &change_id,
                )?;
                let clean_disk_manifest = use_disk_manifest_for_clean.then_some(disk_manifest);
                (
                    built,
                    materialized_paths,
                    previous_files,
                    clean_disk_manifest,
                )
            }
            CachedWorkdirManifestStatus::Missing => {
                let disk_files = self.scan_files_under(&workdir_path)?;
                if let Some(mut selected_paths) = sparse_paths.clone() {
                    selected_paths.extend(disk_files.iter().map(|file| file.path.clone()));
                    selected_paths.sort();
                    selected_paths.dedup();
                    let previous_files =
                        self.load_root_files_for_selections(&head.root_id, &selected_paths)?;
                    let built = self.build_root_for_selected_disk_files_incremental(
                        &head.root_id,
                        &previous_files,
                        &disk_files,
                        &selected_paths,
                        &change_id,
                    )?;
                    let materialized_paths = disk_files
                        .iter()
                        .map(|file| file.path.clone())
                        .collect::<Vec<_>>();
                    (built, materialized_paths, previous_files, None)
                } else {
                    let disk_manifest = self.disk_manifest(&disk_files);
                    let summaries =
                        self.diff_root_to_disk_manifest(&head.root_id, &disk_manifest)?;
                    let materialized_paths = disk_manifest.keys().cloned().collect::<Vec<_>>();
                    if summaries.is_empty() {
                        self.write_lane_clean_workdir_manifest_from_disk_manifest(
                            &workdir_path,
                            overlay_manifest_path.as_deref(),
                            &head.root_id,
                            &disk_manifest,
                            materialized_paths.iter(),
                        )?;
                        return Ok(LaneRecordReport {
                            lane_id: branch.lane_id,
                            operation: None,
                            root_id: head.root_id,
                            changed_paths: Vec::new(),
                        });
                    }
                    let selected_paths = summaries
                        .iter()
                        .map(|summary| summary.path.clone())
                        .collect::<Vec<_>>();
                    let previous_files =
                        self.load_root_files_for_paths(&head.root_id, &selected_paths)?;
                    let built = self.build_root_for_selected_disk_files_incremental(
                        &head.root_id,
                        &previous_files,
                        &disk_files,
                        &selected_paths,
                        &change_id,
                    )?;
                    (
                        built,
                        materialized_paths,
                        previous_files,
                        Some(disk_manifest),
                    )
                }
            }
        };
        let diff = self.diff_file_maps(&previous_files, &built.files)?;
        if diff.changes.is_empty() {
            if is_sparse {
                self.write_sparse_workdir_manifest(&workdir_path, materialized_paths.iter())?;
            }
            if let Some(disk_manifest) = &clean_disk_manifest {
                self.write_lane_clean_workdir_manifest_from_disk_manifest(
                    &workdir_path,
                    overlay_manifest_path.as_deref(),
                    &head.root_id,
                    disk_manifest,
                    materialized_paths.iter(),
                )?;
            } else {
                self.write_lane_clean_workdir_manifest(
                    &workdir_path,
                    overlay_manifest_path.as_deref(),
                    &head.root_id,
                    &built.files,
                    materialized_paths.iter(),
                )?;
            }
            return Ok(LaneRecordReport {
                lane_id: branch.lane_id,
                operation: None,
                root_id: head.root_id,
                changed_paths: Vec::new(),
            });
        }
        self.ensure_lane_record_policy(&branch, &diff.summaries)?;
        self.ensure_lane_record_file_size_policy(&built.files, &diff.summaries)?;
        if let Some(session_id) = &branch.session_id {
            self.ensure_lane_session(&branch.lane_id, session_id, None)?;
        }
        let turn_id = if let Some(turn) = &existing_turn {
            turn.turn_id.clone()
        } else {
            self.open_lane_turn(
                &branch.lane_id,
                branch.session_id.as_deref(),
                &branch.base_change,
                &head.change_id,
                Some(&serde_json::json!({
                    "kind": "workdir_record",
                    "path_count": diff.summaries.len()
                })),
            )?
        };

        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind: OperationKind::LaneRecord,
            parents: vec![head.change_id.clone()],
            before_root: Some(head.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: branch.ref_name.clone(),
            actor,
            session_id: branch.session_id.clone(),
            message: message.as_deref().map(redact_sensitive_text),
            changes: diff.changes,
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&head, &change_id, &built.root_id, &operation_id)?;
        let message_id = if let Some(message) = message {
            Some(self.store_message(
                "lane",
                &message,
                Some(&branch.lane_id),
                branch.session_id.as_deref(),
                Some(&change_id),
                operation.created_at,
            )?)
        } else {
            None
        };
        self.conn.execute(
            "UPDATE lane_branches SET head_change = ?1, head_root = ?2, updated_at = ?3 WHERE lane_id = ?4",
            params![change_id.0, built.root_id.0, now_ts(), branch.lane_id],
        )?;
        if is_sparse {
            self.write_sparse_workdir_manifest(&workdir_path, materialized_paths.iter())?;
        }
        if let Some(disk_manifest) = &clean_disk_manifest {
            self.write_lane_clean_workdir_manifest_from_disk_manifest(
                &workdir_path,
                overlay_manifest_path.as_deref(),
                &built.root_id,
                disk_manifest,
                materialized_paths.iter(),
            )?;
        } else {
            self.write_lane_clean_workdir_manifest(
                &workdir_path,
                overlay_manifest_path.as_deref(),
                &built.root_id,
                &built.files,
                materialized_paths.iter(),
            )?;
        }
        self.insert_lane_event_with_context(
            &branch.lane_id,
            branch.session_id.as_deref(),
            Some(&turn_id),
            "workdir_recorded",
            Some(&change_id),
            message_id.as_ref(),
            &serde_json::json!({
                "workdir": workdir,
                "root_id": built.root_id.0.clone(),
                "session_id": branch.session_id.clone(),
                "changed_paths": diff.summaries.iter().map(|item| item.path.clone()).collect::<Vec<_>>()
            }),
        )?;
        if existing_turn.is_some() {
            self.update_lane_turn_progress(&turn_id, "workdir_recorded", Some(&change_id))?;
        } else {
            self.finish_lane_turn(&turn_id, "completed", Some(&change_id))?;
        }
        Ok(LaneRecordReport {
            lane_id: branch.lane_id,
            operation: Some(change_id),
            root_id: built.root_id,
            changed_paths: diff.summaries,
        })
    }

    pub(crate) fn lane_overlay_clean_manifest_path(
        &self,
        branch: &LaneBranch,
    ) -> Result<Option<PathBuf>> {
        let record = self.lane_record(&branch.lane_id)?;
        if self.lane_workdir_mode_for(&record, branch)? == LaneWorkdirMode::OverlayCow {
            Ok(Some(self.overlay_clean_workdir_manifest_path_for_lane(
                &branch.lane_id,
            )?))
        } else {
            Ok(None)
        }
    }

    pub(crate) fn lane_cached_workdir_manifest_status(
        &self,
        workdir_path: &Path,
        overlay_manifest_path: Option<&Path>,
        root_id: &ObjectId,
    ) -> Result<CachedWorkdirManifestStatus> {
        if let Some(manifest_path) = overlay_manifest_path {
            self.cached_workdir_manifest_status_from_path(workdir_path, manifest_path, root_id)
        } else {
            self.cached_workdir_manifest_status(workdir_path, root_id)
        }
    }

    fn write_lane_clean_workdir_manifest<'a, I>(
        &self,
        workdir_path: &Path,
        overlay_manifest_path: Option<&Path>,
        root_id: &ObjectId,
        files: &BTreeMap<String, FileEntry>,
        expected_paths: I,
    ) -> Result<()>
    where
        I: IntoIterator<Item = &'a String>,
    {
        if let Some(manifest_path) = overlay_manifest_path {
            self.write_clean_workdir_manifest_to_path(
                workdir_path,
                manifest_path,
                root_id,
                files,
                expected_paths,
            )
        } else {
            self.write_clean_workdir_manifest(workdir_path, root_id, files, expected_paths)
        }
    }

    fn write_lane_clean_workdir_manifest_from_disk_manifest<'a, I>(
        &self,
        workdir_path: &Path,
        overlay_manifest_path: Option<&Path>,
        root_id: &ObjectId,
        disk_manifest: &BTreeMap<String, DiskManifest>,
        expected_paths: I,
    ) -> Result<()>
    where
        I: IntoIterator<Item = &'a String>,
    {
        if let Some(manifest_path) = overlay_manifest_path {
            self.write_clean_workdir_manifest_from_disk_manifest_to_path(
                workdir_path,
                manifest_path,
                root_id,
                disk_manifest,
                expected_paths,
            )
        } else {
            self.write_clean_workdir_manifest_from_disk_manifest(
                workdir_path,
                root_id,
                disk_manifest,
                expected_paths,
            )
        }
    }

    fn lane_workdir_record_changed_paths(
        &self,
        branch: &LaneBranch,
        head: &RefRecord,
        workdir_path: &Path,
    ) -> Result<Vec<FileDiffSummary>> {
        let sparse_paths = self.lane_sparse_workdir_paths(branch, workdir_path)?;
        let overlay_manifest_path = self.lane_overlay_clean_manifest_path(branch)?;
        match self.lane_cached_workdir_manifest_status(
            workdir_path,
            overlay_manifest_path.as_deref(),
            &head.root_id,
        )? {
            CachedWorkdirManifestStatus::Clean => Ok(Vec::new()),
            CachedWorkdirManifestStatus::Dirty {
                disk_manifest,
                candidate_paths,
            } => {
                if let Some(mut selected_paths) = sparse_paths {
                    selected_paths.extend(disk_manifest.keys().cloned());
                    selected_paths.sort();
                    selected_paths.dedup();
                    let previous_files =
                        self.load_root_files_for_selections(&head.root_id, &selected_paths)?;
                    Ok(self.diff_file_maps_to_manifest_for_paths(
                        &previous_files,
                        &disk_manifest,
                        &selected_paths,
                    ))
                } else if let Some(candidate_paths) = candidate_paths {
                    let previous_files =
                        self.load_root_files_for_paths(&head.root_id, &candidate_paths)?;
                    Ok(self.diff_file_maps_to_manifest_for_paths(
                        &previous_files,
                        &disk_manifest,
                        &candidate_paths,
                    ))
                } else {
                    self.diff_root_to_disk_manifest(&head.root_id, &disk_manifest)
                }
            }
            CachedWorkdirManifestStatus::Missing => {
                let disk_files = self.scan_files_under(workdir_path)?;
                let disk_manifest = self.disk_manifest(&disk_files);
                if let Some(mut selected_paths) = sparse_paths {
                    selected_paths.extend(disk_files.iter().map(|file| file.path.clone()));
                    selected_paths.sort();
                    selected_paths.dedup();
                    let previous_files =
                        self.load_root_files_for_selections(&head.root_id, &selected_paths)?;
                    Ok(self.diff_file_maps_to_manifest_for_paths(
                        &previous_files,
                        &disk_manifest,
                        &selected_paths,
                    ))
                } else {
                    self.diff_root_to_disk_manifest(&head.root_id, &disk_manifest)
                }
            }
        }
    }

    fn preview_lane_workdir_path_warnings(
        &self,
        workdir_path: &Path,
    ) -> Result<(Vec<LaneWorkdirIgnoredPath>, Vec<LaneWorkdirRisk>)> {
        let root = workdir_path.canonicalize()?;
        let matcher = lane_workdir_ignore_matcher(&root)?;
        let mut ignored_paths = Vec::new();
        let mut risky_paths = Vec::new();
        let root_metadata = fs::symlink_metadata(&root)?;
        scan_lane_workdir_preview_paths(
            &root,
            &root,
            root_metadata,
            &matcher,
            &mut ignored_paths,
            &mut risky_paths,
        )?;
        Ok((ignored_paths, risky_paths))
    }

    fn lane_record_oversized_files_on_disk(
        &self,
        workdir_path: &Path,
        summaries: &[FileDiffSummary],
    ) -> Result<Vec<LaneRecordOversizedFile>> {
        let max_file_bytes = self.config.lane.max_patch_file_bytes;
        if max_file_bytes == 0 {
            return Ok(Vec::new());
        }
        let mut oversized = BTreeMap::new();
        for summary in summaries {
            let path = normalize_relative_path(&summary.path)?;
            let abs = safe_join(workdir_path, &path)?;
            let metadata = match fs::symlink_metadata(&abs) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => return Err(Error::Io(err)),
            };
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                continue;
            }
            let size_bytes = metadata.len();
            if size_bytes > max_file_bytes {
                oversized.insert(
                    path.clone(),
                    LaneRecordOversizedFile {
                        path,
                        size_bytes,
                        limit_bytes: max_file_bytes,
                    },
                );
            }
        }
        Ok(oversized.into_values().collect())
    }

    fn ensure_lane_record_file_size_policy(
        &self,
        files: &BTreeMap<String, FileEntry>,
        summaries: &[FileDiffSummary],
    ) -> Result<()> {
        let max_file_bytes = self.config.lane.max_patch_file_bytes;
        if max_file_bytes == 0 {
            return Ok(());
        }
        let mut oversized = BTreeMap::new();
        for summary in summaries {
            let path = normalize_relative_path(&summary.path)?;
            let Some(entry) = files.get(&path) else {
                continue;
            };
            if entry.size_bytes > max_file_bytes {
                oversized.insert(
                    path.clone(),
                    LaneRecordOversizedFile {
                        path,
                        size_bytes: entry.size_bytes,
                        limit_bytes: max_file_bytes,
                    },
                );
            }
        }
        if oversized.is_empty() {
            return Ok(());
        }
        let oversized = oversized.into_values().collect::<Vec<_>>();
        Err(Error::PatchRejected(lane_record_oversized_error(
            &oversized,
        )))
    }

    pub fn watch_lane_workdir(
        &mut self,
        lane: &str,
        message: Option<String>,
        interval: Duration,
        iterations: Option<u64>,
    ) -> Result<LaneWatchReport> {
        validate_ref_segment(lane)?;
        let branch = self.lane_branch(lane)?;
        if branch.workdir.is_none() {
            return Err(Error::InvalidInput(format!(
                "lane `{lane}` does not have a materialized workdir"
            )));
        }
        let mut report = LaneWatchReport {
            lane_id: branch.lane_id,
            iterations: 0,
            recorded_operations: Vec::new(),
            changed_paths: Vec::new(),
        };
        loop {
            let record = self.record_lane_workdir(lane, message.clone())?;
            report.iterations += 1;
            if let Some(operation) = record.operation {
                report.recorded_operations.push(operation);
                report.changed_paths.extend(record.changed_paths);
            }
            if iterations.is_some_and(|limit| report.iterations >= limit) {
                break;
            }
            std::thread::sleep(interval);
        }
        Ok(report)
    }
}

fn lane_workdir_ignore_matcher(root: &Path) -> Result<ignore::gitignore::Gitignore> {
    let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
    for filename in [".crabignore", ".gitignore"] {
        let path = root.join(filename);
        if path.exists() {
            if let Some(err) = builder.add(path) {
                return Err(Error::InvalidInput(err.to_string()));
            }
        }
    }
    builder
        .build()
        .map_err(|err| Error::InvalidInput(err.to_string()))
}

fn scan_lane_workdir_preview_paths(
    root: &Path,
    dir: &Path,
    root_metadata: fs::Metadata,
    matcher: &ignore::gitignore::Gitignore,
    ignored_paths: &mut Vec<LaneWorkdirIgnoredPath>,
    risky_paths: &mut Vec<LaneWorkdirRisk>,
) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        let rel = path
            .strip_prefix(root)
            .map_err(|err| Error::InvalidInput(err.to_string()))?;
        let rel = normalize_relative_path(&rel.to_string_lossy())?;

        if rel == ".crabdb" {
            continue;
        }

        append_lane_workdir_risks(&rel, &metadata, &root_metadata, risky_paths);
        if let Some(source) = lane_workdir_ignore_source(&rel, &metadata, matcher) {
            ignored_paths.push(LaneWorkdirIgnoredPath { path: rel, source });
            if metadata.is_dir() {
                continue;
            }
        }

        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() && !is_external_mount(&metadata, &root_metadata) {
            scan_lane_workdir_preview_paths(
                root,
                &path,
                root_metadata.clone(),
                matcher,
                ignored_paths,
                risky_paths,
            )?;
        }
    }
    Ok(())
}

fn lane_workdir_ignore_source(
    rel: &str,
    metadata: &fs::Metadata,
    matcher: &ignore::gitignore::Gitignore,
) -> Option<String> {
    if is_default_ignored(rel) {
        return Some("hardcoded".to_string());
    }
    matcher
        .matched_path_or_any_parents(path_from_rel(rel), metadata.is_dir())
        .is_ignore()
        .then(|| "workdir".to_string())
}

fn append_lane_workdir_risks(
    rel: &str,
    metadata: &fs::Metadata,
    root_metadata: &fs::Metadata,
    risky_paths: &mut Vec<LaneWorkdirRisk>,
) {
    if path_has_component(rel, ".git") {
        risky_paths.push(LaneWorkdirRisk {
            path: rel.to_string(),
            kind: "nested_git".to_string(),
            message:
                "nested .git content is ignored by record and can hide unrelated repository state"
                    .to_string(),
        });
    }
    if rel != ".crabdb" && path_has_component(rel, ".crabdb") {
        risky_paths.push(LaneWorkdirRisk {
            path: rel.to_string(),
            kind: "nested_crabdb".to_string(),
            message: "nested .crabdb content is ignored by record and can hide CrabDB metadata"
                .to_string(),
        });
    }
    if metadata.file_type().is_symlink() {
        risky_paths.push(LaneWorkdirRisk {
            path: rel.to_string(),
            kind: "symlink".to_string(),
            message: "symlinks are skipped by workdir record".to_string(),
        });
    }
    if metadata.is_file() && has_multiple_hardlinks(metadata) {
        risky_paths.push(LaneWorkdirRisk {
            path: rel.to_string(),
            kind: "hardlink".to_string(),
            message: "file has multiple hardlinks; recording it can hide writes made through another path".to_string(),
        });
    }
    if is_external_mount(metadata, root_metadata) {
        risky_paths.push(LaneWorkdirRisk {
            path: rel.to_string(),
            kind: "external_mount".to_string(),
            message: "path is on a different filesystem device from the lane workdir root"
                .to_string(),
        });
    }
}

fn path_has_component(path: &str, needle: &str) -> bool {
    path.split('/').any(|component| component == needle)
}

#[cfg(unix)]
fn has_multiple_hardlinks(metadata: &fs::Metadata) -> bool {
    std::os::unix::fs::MetadataExt::nlink(metadata) > 1
}

#[cfg(not(unix))]
fn has_multiple_hardlinks(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn is_external_mount(metadata: &fs::Metadata, root_metadata: &fs::Metadata) -> bool {
    std::os::unix::fs::MetadataExt::dev(metadata)
        != std::os::unix::fs::MetadataExt::dev(root_metadata)
}

#[cfg(not(unix))]
fn is_external_mount(_metadata: &fs::Metadata, _root_metadata: &fs::Metadata) -> bool {
    false
}

fn lane_record_oversized_error(files: &[LaneRecordOversizedFile]) -> String {
    let limit = files
        .first()
        .map(|file| file.limit_bytes)
        .unwrap_or_default();
    let paths = files
        .iter()
        .map(|file| format!("{} ({} bytes)", file.path, file.size_bytes))
        .collect::<Vec<_>>()
        .join(", ");
    format!("lane record file(s) exceed lane.max_patch_file_bytes {limit}: {paths}")
}

fn ensure_lane_record_message_has_no_secrets(message: Option<&str>) -> Result<()> {
    let Some(message) = message else {
        return Ok(());
    };
    if contains_sensitive_text(message) {
        return Err(Error::PatchRejected(
            "secret scan rejected lane record message; remove credentials from the record metadata"
                .to_string(),
        ));
    }
    Ok(())
}
