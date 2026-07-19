use super::*;

impl Trail {
    pub fn lane_workdir(&self, lane: &str) -> Result<LaneWorkdirReport> {
        validate_ref_segment(lane)?;
        let branch = self.lane_branch(lane)?;
        let record = self.lane_record(&branch.lane_id)?;
        let workdir_mode = self.lane_workdir_mode_for(&record, &branch)?;
        let requested_workdir_mode = self.lane_requested_workdir_mode_for(&record, &branch)?;
        let workdir_backend = self.lane_workdir_backend_for(&record)?;
        let materialization = self.lane_materialization_report_for(&record)?;
        let sparse_paths = self.lane_report_sparse_paths(&branch)?;
        Ok(LaneWorkdirReport {
            lane_id: branch.lane_id,
            workdir: branch.workdir,
            requested_workdir_mode,
            workdir_backend,
            materialization,
            sparse_paths,
            transparent_cow_available: workdir_mode.is_transparent_cow(),
            workdir_mode,
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
        let ledger_authority = crate::db::change_ledger::command_authority_enabled();
        if hydrate && ledger_authority {
            crate::db::change_ledger::materialized_lane_daemon_expected_scope(
                self,
                &branch.lane_id,
            )?;
        }
        let _lock = if hydrate && !ledger_authority {
            Some(self.acquire_write_lock()?)
        } else {
            None
        };
        let head = self.get_ref(&branch.ref_name)?;
        let mut entries =
            self.load_root_files_for_paths(&head.root_id, std::slice::from_ref(&path))?;
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
        // TRAIL_FS_PRODUCER: lane_sync LaneSync controlled
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
        let workdir_mode =
            self.lane_workdir_mode_for(&self.lane_record(&branch.lane_id)?, &branch)?;
        let requested_workdir_mode =
            self.lane_requested_workdir_mode_for(&self.lane_record(&branch.lane_id)?, &branch)?;
        if workdir_mode == LaneWorkdirMode::NfsCow {
            return self.sync_nfs_cow_lane_workdir(
                lane,
                branch,
                workdir,
                workdir_path,
                force,
                &selected_paths,
                include_neighbors,
            );
        }
        let workdir_path_metadata = match fs::symlink_metadata(&workdir_path) {
            Ok(metadata) => Some(metadata),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => return Err(Error::Io(err)),
        };
        let workdir_path_is_dir = workdir_path_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink());
        let workdir_path_is_non_dir = workdir_path_metadata.is_some() && !workdir_path_is_dir;
        if workdir_path_is_non_dir && !force {
            return Err(Error::InvalidInput(format!(
                "lane `{lane}` workdir path exists but is not a directory"
            )));
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
        let workdir_exists = workdir_path_is_dir;
        let sparse_paths = if workdir_exists {
            self.lane_sparse_workdir_paths(&branch, &workdir_path)?
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
                "lane `{lane}` workdir has unrecorded changes; run `trail lane record {lane}` or pass `--force` to sync: {preview}{suffix}"
            )));
        }
        let ledger_authority = crate::db::change_ledger::command_authority_enabled();
        let marker_before = if ledger_authority && workdir_path_is_dir {
            self.capture_materialized_lane_marker(&workdir_path)?
        } else {
            None
        };
        let expected = if ledger_authority && workdir_path_is_dir {
            Some(
                crate::db::change_ledger::prepare_materialized_lane_controlled_projection(
                    self,
                    &branch.lane_id,
                )?,
            )
        } else {
            None
        };
        let rescue_workdir = if force && workdir_path_is_non_dir {
            // TRAIL_FS_PRODUCER: lane_sync_rescue exempt_rescue_output exempt
            Some(self.rescue_replaced_lane_workdir_path(lane, &workdir, &workdir_path)?)
        } else if force && workdir_exists && !changed_paths.is_empty() {
            Some(self.rescue_dirty_lane_workdir(lane, &workdir, &workdir_path, &changed_paths)?)
        } else {
            None
        };
        // A controlled interval requires a stable directory inode before the
        // native observer is bound. A forced non-directory replacement was
        // already copied to the explicit rescue output above.
        if workdir_path_is_non_dir {
            remove_existing_lane_workdir_path(&workdir_path)?;
        }
        fs::create_dir_all(&workdir_path)?;
        let mut evidence_paths = changed_paths
            .iter()
            .flat_map(|summary| {
                std::iter::once(summary.path.as_str()).chain(summary.old_path.as_deref())
            })
            .map(crate::db::change_ledger::LedgerPath::parse)
            .collect::<Result<Vec<_>>>()?;
        if force {
            evidence_paths.extend(
                target_files
                    .keys()
                    .map(|path| crate::db::change_ledger::LedgerPath::parse(path))
                    .collect::<Result<Vec<_>>>()?,
            );
        }
        evidence_paths.sort();
        evidence_paths.dedup();
        let evidence = crate::db::change_ledger::IntentEvidence {
            exact_paths: evidence_paths,
            complete_prefixes: Vec::new(),
        };
        let mut materialization_outcome = None;
        let projected_target_files = if path_scoped {
            target_files.clone()
        } else if let Some(paths) = &sparse_paths {
            self.selected_file_entries(&target_files, paths)
        } else {
            target_files.clone()
        };
        if ledger_authority {
            let expected = expected.ok_or_else(|| Error::ChangeLedgerReconcileRequired {
                scope: crate::db::change_ledger::materialized_lane_scope_id(
                    &self.config.workspace.id.0,
                    &branch.lane_id,
                )
                .to_text(),
                state: "reconciling".into(),
                reason: "materialized lane root must be initialized before controlled sync".into(),
                command: format!("trail lane workdir ensure {}", branch.lane_id),
            })?;
            let projection_result = crate::db::change_ledger::run_projection_alignment(
                self,
                &expected,
                crate::db::change_ledger::IntentProducer::LaneSync,
                &evidence,
                crate::db::change_ledger::ProjectionAlignmentMode::Aligned,
                |db, intent| {
                    crate::db::change_ledger::with_materialized_lane_controlled_interval(
                        db,
                        &branch.lane_id,
                        intent,
                        &evidence,
                        |db| {
                            if path_scoped {
                                db.materialize_sparse_lane_workdir_paths(
                                    &workdir_path,
                                    &head.root_id,
                                    sparse_paths.clone().unwrap_or_default(),
                                    &projected_target_files,
                                    force,
                                )?;
                            } else if force || !workdir_exists {
                                db.invalidate_lane_marker_if_materialized(&branch)?;
                                materialization_outcome = db.materialize_full_lane_workdir_staged(
                                    &workdir_path,
                                    &head.root_id,
                                    &projected_target_files,
                                    sparse_paths.is_some(),
                                    &requested_workdir_mode,
                                )?;
                            } else {
                                db.invalidate_lane_marker_if_materialized(&branch)?;
                                if !changed_paths.is_empty() {
                                    db.materialize_files_at(
                                        &workdir_path,
                                        &projected_target_files,
                                        &projected_target_files,
                                    )?;
                                }
                                if sparse_paths.is_some() {
                                    db.write_sparse_workdir_manifest(
                                        &workdir_path,
                                        projected_target_files.keys(),
                                    )?;
                                }
                                db.write_clean_workdir_manifest(
                                    &workdir_path,
                                    &head.root_id,
                                    &projected_target_files,
                                    projected_target_files.keys(),
                                )?;
                            }
                            Ok(())
                        },
                        |db, policy, candidates| {
                            let comparison = db.compare_authoritative_candidates(
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
                                    reason: "lane sync pinned verification did not match its target root"
                                        .into(),
                                    command: format!("trail lane status {}", branch.lane_id),
                                })
                            }
                        },
                    )
                },
                |db| db.publish_lane_marker_if_materialized(&branch.lane_id),
            );
            if let Err(error) = projection_result {
                self.restore_materialized_lane_marker(&workdir_path, marker_before.as_deref())?;
                return Err(error);
            }
        } else {
            if path_scoped {
                self.materialize_sparse_lane_workdir_paths(
                    &workdir_path,
                    &head.root_id,
                    sparse_paths.clone().unwrap_or_default(),
                    &projected_target_files,
                    force,
                )?;
            } else if force || !workdir_exists {
                self.invalidate_lane_marker_if_materialized(&branch)?;
                materialization_outcome = self.materialize_full_lane_workdir_staged(
                    &workdir_path,
                    &head.root_id,
                    &projected_target_files,
                    sparse_paths.is_some(),
                    &requested_workdir_mode,
                )?;
            } else {
                self.invalidate_lane_marker_if_materialized(&branch)?;
                if !changed_paths.is_empty() {
                    self.materialize_files_best_effort_at(
                        &workdir_path,
                        &projected_target_files,
                        &projected_target_files,
                    )?;
                }
                if sparse_paths.is_some() {
                    self.write_sparse_workdir_manifest(
                        &workdir_path,
                        projected_target_files.keys(),
                    )?;
                }
                self.write_clean_workdir_manifest(
                    &workdir_path,
                    &head.root_id,
                    &projected_target_files,
                    projected_target_files.keys(),
                )?;
            }
            self.publish_lane_marker_if_materialized(&branch.lane_id)?;
        }
        if let Some(outcome) = &materialization_outcome {
            self.update_lane_materialization_metadata(
                &branch.lane_id,
                &requested_workdir_mode,
                outcome,
            )?;
        }
        let resolved_workdir_mode = materialization_outcome
            .as_ref()
            .map(|outcome| outcome.resolved_mode.clone())
            .unwrap_or_else(|| workdir_mode.clone());
        let workdir_backend = match materialization_outcome.as_ref() {
            Some(outcome) => Some(outcome.backend),
            None => self.lane_workdir_backend_for(&self.lane_record(&branch.lane_id)?)?,
        };
        let materialization = match materialization_outcome.as_ref() {
            Some(outcome) => Some(outcome.report.clone()),
            None => self.lane_materialization_report_for(&self.lane_record(&branch.lane_id)?)?,
        };
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
                "requested_workdir_mode": requested_workdir_mode.as_str(),
                "workdir_mode": materialization_outcome.as_ref().map(|outcome| outcome.resolved_mode.as_str()),
                "workdir_backend": materialization_outcome.as_ref().map(|outcome| outcome.backend.as_str()),
                "materialization": materialization_outcome.as_ref().map(|outcome| &outcome.report),
                "changed_paths": changed_paths.iter().map(|item| item.path.clone()).collect::<Vec<_>>()
            }),
        )?;
        Ok(LaneWorkdirSyncReport {
            lane_id: branch.lane_id,
            workdir,
            head_change: head.change_id,
            root_id: head.root_id,
            requested_workdir_mode,
            workdir_mode: resolved_workdir_mode,
            workdir_backend,
            materialization,
            forced: force,
            rescue_workdir,
            changed_paths,
        })
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "carries the fixed NFS COW sync contract"
    )]
    fn sync_nfs_cow_lane_workdir(
        &mut self,
        lane: &str,
        branch: LaneBranch,
        workdir: String,
        workdir_path: PathBuf,
        force: bool,
        selected_paths: &[String],
        include_neighbors: bool,
    ) -> Result<LaneWorkdirSyncReport> {
        if !selected_paths.is_empty() {
            return Err(Error::InvalidInput(
                "path-scoped sync-workdir is not supported for nfs-cow lanes".to_string(),
            ));
        }
        let head = self.get_ref(&branch.ref_name)?;
        let changed_paths = self
            .lane_workdir_changed_paths(&branch, &head)?
            .unwrap_or_default();
        if !changed_paths.is_empty() && !force {
            return Err(Error::DirtyWorktreeWithMessage(format!(
                "lane `{lane}` workdir has unrecorded changes; run `trail lane record {lane}` or pass `--force` to sync"
            )));
        }
        let rescue_workdir = if force && !changed_paths.is_empty() {
            Some(self.rescue_dirty_lane_workdir(lane, &workdir, &workdir_path, &changed_paths)?)
        } else {
            None
        };
        // A layered sync never recreates or erases the persisted view. Its
        // source upper is durable lane state; remount composes it with the
        // current pinned root, and a forced sync only produces a rescue copy.
        self.insert_lane_event(
            &branch.lane_id,
            "workdir_synced",
            Some(&head.change_id),
            None,
            &serde_json::json!({
                "workdir": workdir,
                "forced": force,
                "rescue_workdir": rescue_workdir,
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
            requested_workdir_mode: LaneWorkdirMode::NfsCow,
            workdir_mode: LaneWorkdirMode::NfsCow,
            workdir_backend: Some(WorkdirBackend::Nfs),
            materialization: None,
            forced: force,
            rescue_workdir,
            changed_paths,
        })
    }

    fn materialize_full_lane_workdir_staged(
        &self,
        workdir_path: &Path,
        root_id: &ObjectId,
        target_files: &BTreeMap<String, FileEntry>,
        sparse: bool,
        requested_mode: &LaneWorkdirMode,
    ) -> Result<Option<MaterializationOutcome>> {
        // A registered root may already be watched by the lane observer. Keep
        // its directory inode stable; replacing the root would silently
        // detach inotify/FSEvents authority from subsequent writes.
        if workdir_path.is_dir() {
            let disk = self.scan_files_under(workdir_path)?;
            let mut deletion_parents = BTreeSet::new();
            for path in disk
                .iter()
                .map(|file| file.path.as_str())
                .filter(|path| !target_files.contains_key(*path))
            {
                let absolute = safe_join(workdir_path, path)?;
                match fs::remove_file(absolute) {
                    Ok(()) => {
                        if let Some(parent) = Path::new(path).parent() {
                            deletion_parents.insert(workdir_path.join(parent));
                        } else {
                            deletion_parents.insert(workdir_path.to_path_buf());
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(Error::Io(error)),
                }
            }
            for parent in deletion_parents {
                File::open(parent)?.sync_all()?;
            }
            self.materialize_files_at(workdir_path, target_files, target_files)?;
            File::open(workdir_path)?.sync_all()?;
            if sparse {
                self.write_sparse_workdir_manifest(workdir_path, target_files.keys())?;
            }
            self.write_clean_workdir_manifest(
                workdir_path,
                root_id,
                target_files,
                target_files.keys(),
            )?;
            return Ok(None);
        }
        let parent = workdir_path.parent().ok_or_else(|| Error::InvalidPath {
            path: workdir_path.to_string_lossy().to_string(),
            reason: "workdir path has no parent".to_string(),
        })?;
        fs::create_dir_all(parent)?;
        if !sparse
            && matches!(
                requested_mode,
                LaneWorkdirMode::Auto | LaneWorkdirMode::NativeCow | LaneWorkdirMode::PortableCopy
            )
        {
            let replacement = create_unique_lane_workdir_replacement_target(parent, workdir_path)?;
            let policy = materialization_policy_for_mode(requested_mode);
            let outcome =
                self.materialize_lane_root_staged(root_id, &replacement, false, policy)?;
            if let Err(error) = replace_lane_workdir_with_stage(workdir_path, &replacement) {
                let _ = fs::remove_dir_all(&replacement);
                return Err(error);
            }
            self.complete_materialization_operation(&outcome.materialization_operation_id)?;
            return Ok(Some(outcome));
        }
        let stage_dir = create_unique_lane_workdir_sync_stage_dir(parent, workdir_path)?;
        let result = (|| -> Result<()> {
            let empty = BTreeMap::new();
            self.materialize_files_at(&stage_dir, &empty, target_files)?;
            if sparse {
                self.write_sparse_workdir_manifest(&stage_dir, target_files.keys())?;
            }
            self.write_clean_workdir_manifest(
                &stage_dir,
                root_id,
                target_files,
                target_files.keys(),
            )?;
            match self.cached_workdir_manifest_status(&stage_dir, root_id)? {
                CachedWorkdirManifestStatus::Clean => {}
                _ => {
                    return Err(Error::Corrupt(format!(
                        "staged lane workdir `{}` did not verify clean before replacement",
                        stage_dir.display()
                    )));
                }
            }
            replace_lane_workdir_with_stage(workdir_path, &stage_dir)
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&stage_dir);
        }
        result.map(|()| None)
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
            let destination = safe_join(&files_dir, &path)?;
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

    fn rescue_replaced_lane_workdir_path(
        &self,
        lane: &str,
        workdir: &str,
        workdir_path: &Path,
    ) -> Result<String> {
        let rescue_root = self.db_dir.join("lane-workdir-rescue");
        fs::create_dir_all(&rescue_root)?;
        let rescue_dir = create_unique_lane_workdir_rescue_dir(&rescue_root, lane)?;
        let files_dir = rescue_dir.join("files");
        fs::create_dir_all(&files_dir)?;

        let leaf = workdir_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "workdir".to_string());
        let mut copied_paths = Vec::new();
        let mut skipped_paths = Vec::new();
        let mut symlink_target = None;
        let metadata = fs::symlink_metadata(workdir_path)?;
        if metadata.is_file() && !metadata.file_type().is_symlink() {
            fs::copy(workdir_path, files_dir.join(&leaf))?;
            copied_paths.push(leaf.clone());
        } else if metadata.file_type().is_symlink() {
            match fs::read_link(workdir_path) {
                Ok(target) => {
                    symlink_target = Some(target.to_string_lossy().to_string());
                    skipped_paths.push(format!("{leaf}: symlink"));
                }
                Err(err) => {
                    skipped_paths.push(format!("{leaf}: symlink target unavailable: {err}"))
                }
            }
        } else {
            skipped_paths.push(format!("{leaf}: not a regular file"));
        }

        let manifest = serde_json::json!({
            "lane": lane,
            "workdir": workdir,
            "created_at": now_ts(),
            "replaced_workdir_path": true,
            "copied_paths": copied_paths,
            "skipped_paths": skipped_paths,
            "symlink_target": symlink_target,
        });
        fs::write(
            rescue_dir.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )?;
        Ok(rescue_dir.to_string_lossy().to_string())
    }

    pub(crate) fn hydrate_sparse_lane_workdir_paths_unlocked(
        &mut self,
        lane: &str,
        branch: &LaneBranch,
        paths: &[String],
        force: bool,
        include_neighbors: bool,
    ) -> Result<Vec<String>> {
        // TRAIL_FS_PRODUCER: sparse_hydration LaneSync controlled
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
        let Some(sparse_paths) = self.lane_sparse_workdir_paths(branch, &workdir_path)? else {
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
                "lane `{lane}` workdir has unrecorded changes; run `trail lane record {lane}` or pass `--force` to sync: {preview}{suffix}"
            )));
        }

        if !crate::db::change_ledger::command_authority_enabled() {
            self.materialize_sparse_lane_workdir_paths(
                &workdir_path,
                &head.root_id,
                sparse_paths,
                &target_files,
                force,
            )?;
            self.publish_lane_marker_if_materialized(lane)?;
            return Ok(target_files.keys().cloned().collect());
        }

        let marker_before = self.capture_materialized_lane_marker(&workdir_path)?;
        let expected = crate::db::change_ledger::prepare_materialized_lane_controlled_projection(
            self,
            &branch.lane_id,
        )?;
        let evidence = crate::db::change_ledger::IntentEvidence {
            exact_paths: target_files
                .keys()
                .map(|path| crate::db::change_ledger::LedgerPath::parse(path))
                .collect::<Result<Vec<_>>>()?,
            complete_prefixes: Vec::new(),
        };
        let projection_result = crate::db::change_ledger::run_projection_alignment(
            self,
            &expected,
            crate::db::change_ledger::IntentProducer::LaneSync,
            &evidence,
            crate::db::change_ledger::ProjectionAlignmentMode::Aligned,
            |db, intent| {
                crate::db::change_ledger::with_materialized_lane_controlled_interval(
                    db,
                    &branch.lane_id,
                    intent,
                    &evidence,
                    |db| {
                        db.materialize_sparse_lane_workdir_paths(
                            &workdir_path,
                            &head.root_id,
                            sparse_paths,
                            &target_files,
                            force,
                        )
                    },
                    |db, policy, candidates| {
                        let comparison = db.compare_authoritative_candidates(
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
                                reason: "sparse hydration pinned verification did not match its target root"
                                    .into(),
                                command: format!("trail lane status {}", branch.lane_id),
                            })
                        }
                    },
                )
            },
            |db| db.publish_lane_marker_if_materialized(&branch.lane_id),
        );
        if let Err(error) = projection_result {
            self.restore_materialized_lane_marker(&workdir_path, marker_before.as_deref())?;
            return Err(error);
        }
        Ok(target_files.keys().cloned().collect())
    }

    fn materialize_sparse_lane_workdir_paths(
        &self,
        workdir_path: &Path,
        root_id: &ObjectId,
        sparse_paths: Vec<String>,
        target_files: &BTreeMap<String, FileEntry>,
        force: bool,
    ) -> Result<()> {
        let write_files = self.sparse_hydration_write_files(workdir_path, target_files, force)?;
        let transaction = SparseHydrationTransaction::begin(workdir_path, write_files.keys())?;
        let result = (|| -> Result<()> {
            // The marker is one of the transaction snapshots, so a failure in
            // bytes or sparse-selection publication restores all three.
            if crate::db::change_ledger::command_authority_enabled() {
                self.invalidate_materialized_lane_marker(workdir_path)?;
            }
            if crate::db::change_ledger::command_authority_enabled() {
                self.materialize_files_at(workdir_path, &BTreeMap::new(), &write_files)?;
            } else {
                self.materialize_new_files_best_effort_at_with_workspace_cow(
                    workdir_path,
                    &write_files,
                )?;
            }

            let mut materialized_paths = sparse_paths;
            materialized_paths.extend(target_files.keys().cloned());
            materialized_paths.sort();
            materialized_paths.dedup();
            self.write_sparse_workdir_manifest(workdir_path, materialized_paths.iter())?;
            let clean_manifest_updated = self.update_clean_workdir_manifest_from_file_subset(
                workdir_path,
                root_id,
                root_id,
                &BTreeMap::new(),
                target_files,
            )?;
            self.verify_sparse_lane_workdir_hydration(
                workdir_path,
                root_id,
                target_files,
                clean_manifest_updated,
            )
        })();
        match result {
            Ok(()) => {
                transaction.commit();
                Ok(())
            }
            Err(err) => {
                let original = err.to_string();
                match transaction.rollback() {
                    Ok(()) => Err(err),
                    Err(rollback_err) => Err(Error::Corrupt(format!(
                        "sparse lane workdir hydration failed and rollback failed: {rollback_err}; original error: {original}"
                    ))),
                }
            }
        }
    }

    fn verify_sparse_lane_workdir_hydration(
        &self,
        workdir_path: &Path,
        root_id: &ObjectId,
        target_files: &BTreeMap<String, FileEntry>,
        clean_manifest_updated: bool,
    ) -> Result<()> {
        let target_paths = target_files.keys().cloned().collect::<Vec<_>>();
        let Some(sparse_paths) = self.sparse_workdir_paths(workdir_path)? else {
            return Err(Error::Corrupt(format!(
                "sparse lane workdir `{}` lost its sparse manifest during hydration",
                workdir_path.display()
            )));
        };
        let sparse_paths = sparse_paths.into_iter().collect::<BTreeSet<_>>();
        if let Some(path) = target_paths
            .iter()
            .find(|path| !sparse_paths.contains(path.as_str()))
        {
            return Err(Error::Corrupt(format!(
                "sparse lane workdir `{}` manifest omitted hydrated path `{path}`",
                workdir_path.display()
            )));
        }

        let disk_files = self.scan_files_under_for_paths(workdir_path, &target_paths)?;
        let disk_manifest = self.disk_manifest(&disk_files);
        let diffs =
            self.diff_file_maps_to_manifest_for_paths(target_files, &disk_manifest, &target_paths)?;
        if let Some(diff) = diffs.first() {
            let detail = sparse_hydration_diff_detail(
                diff,
                target_files.get(&diff.path),
                disk_manifest.get(&diff.path),
            );
            return Err(Error::Corrupt(format!(
                "sparse lane workdir `{}` failed hydration verification for `{}`: {detail}",
                workdir_path.display(),
                diff.path
            )));
        }

        if clean_manifest_updated
            && !self.clean_workdir_manifest_tracks_file_subset(
                workdir_path,
                root_id,
                target_files,
            )?
        {
            return Err(Error::Corrupt(format!(
                "sparse lane workdir `{}` clean manifest did not track hydrated paths",
                workdir_path.display()
            )));
        }

        Ok(())
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
        self.diff_file_maps_to_manifest_for_paths(&head_files, &disk_manifest, &candidate_paths)
    }
}

fn sparse_hydration_diff_detail(
    diff: &FileDiffSummary,
    expected: Option<&FileEntry>,
    actual: Option<&DiskManifest>,
) -> String {
    let expected = expected
        .map(|entry| {
            format!(
                "expected kind={:?} executable={} hash={} size={}",
                entry.kind, entry.executable, entry.content_hash, entry.size_bytes
            )
        })
        .unwrap_or_else(|| "expected missing".to_string());
    let actual = actual
        .map(|manifest| {
            format!(
                "actual kind={:?} executable={} hash={}",
                manifest.kind, manifest.executable, manifest.content_hash
            )
        })
        .unwrap_or_else(|| "actual missing".to_string());
    format!("diff={:?}; {expected}; {actual}", diff.kind)
}

fn branch_has_sparse_workdir(db: &Trail, branch: &LaneBranch) -> Result<bool> {
    let Some(workdir) = &branch.workdir else {
        return Ok(false);
    };
    let workdir_path = PathBuf::from(workdir);
    if !workdir_path.is_dir() {
        return Ok(false);
    }
    db.lane_sparse_workdir_paths(branch, &workdir_path)
        .map(|paths| paths.is_some())
}

fn create_unique_lane_workdir_rescue_dir(rescue_root: &Path, lane: &str) -> Result<PathBuf> {
    for _ in 0..16 {
        let candidate = rescue_root.join(format!("{lane}-{}", now_nanos()));
        match fs::create_dir(&candidate) {
            Ok(()) => {
                sync_directory_strict(&candidate)?;
                sync_directory_strict(rescue_root)?;
                return Ok(candidate);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(Error::Io(err)),
        }
    }
    Err(Error::InvalidInput(
        "could not create unique lane workdir rescue directory".to_string(),
    ))
}

fn create_unique_lane_workdir_sync_stage_dir(
    parent: &Path,
    workdir_path: &Path,
) -> Result<PathBuf> {
    let leaf = workdir_path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "workdir".into());
    for _ in 0..16 {
        let candidate = parent.join(format!(".{leaf}.trail-sync-{}", now_nanos()));
        match fs::create_dir(&candidate) {
            Ok(()) => {
                sync_directory_strict(&candidate)?;
                sync_directory_strict(parent)?;
                return Ok(candidate);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(Error::Io(err)),
        }
    }
    Err(Error::InvalidInput(
        "could not create unique lane workdir sync staging directory".to_string(),
    ))
}

fn create_unique_lane_workdir_replacement_target(
    parent: &Path,
    workdir_path: &Path,
) -> Result<PathBuf> {
    let leaf = workdir_path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "workdir".into());
    for _ in 0..16 {
        let candidate = parent.join(format!(".{leaf}.trail-next-{}", now_nanos()));
        match fs::symlink_metadata(&candidate) {
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(candidate),
            Err(error) => return Err(Error::Io(error)),
        }
    }
    Err(Error::InvalidInput(
        "could not reserve a unique lane workdir replacement target".to_string(),
    ))
}

fn materialization_policy_for_mode(mode: &LaneWorkdirMode) -> MaterializationPolicy {
    match mode {
        LaneWorkdirMode::NativeCow => MaterializationPolicy::StrictNative,
        LaneWorkdirMode::PortableCopy => MaterializationPolicy::Portable,
        LaneWorkdirMode::Auto => MaterializationPolicy::Auto,
        _ => MaterializationPolicy::Portable,
    }
}

fn replace_lane_workdir_with_stage(workdir_path: &Path, stage_dir: &Path) -> Result<()> {
    let backup_path = move_existing_lane_workdir_to_backup(workdir_path)?;
    let replace_result = fs::rename(stage_dir, workdir_path).map_err(Error::Io);
    match replace_result {
        Ok(()) => {
            if let Some(backup_path) = backup_path {
                remove_existing_lane_workdir_path(&backup_path)?;
            }
        }
        Err(err) => {
            if let Some(backup_path) = backup_path {
                let _ = remove_existing_lane_workdir_path(workdir_path);
                if let Err(restore_err) = fs::rename(&backup_path, workdir_path) {
                    return Err(Error::Corrupt(format!(
                        "failed to replace lane workdir `{}` with staged directory `{}`: {err}; \
                         failed to restore previous workdir from `{}`: {restore_err}",
                        workdir_path.display(),
                        stage_dir.display(),
                        backup_path.display()
                    )));
                }
                if let Some(parent) = workdir_path.parent() {
                    sync_directory(parent);
                }
            }
            return Err(err);
        }
    }
    if let Some(parent) = workdir_path.parent() {
        sync_directory(parent);
    }
    Ok(())
}

fn move_existing_lane_workdir_to_backup(workdir_path: &Path) -> Result<Option<PathBuf>> {
    match fs::symlink_metadata(workdir_path) {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(Error::Io(err)),
    }
    let backup_path = create_unique_lane_workdir_replacement_backup_path(workdir_path)?;
    fs::rename(workdir_path, &backup_path)?;
    Ok(Some(backup_path))
}

fn create_unique_lane_workdir_replacement_backup_path(workdir_path: &Path) -> Result<PathBuf> {
    let parent = workdir_path.parent().ok_or_else(|| Error::InvalidPath {
        path: workdir_path.to_string_lossy().to_string(),
        reason: "workdir path has no parent".to_string(),
    })?;
    let leaf = workdir_path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "workdir".into());
    for _ in 0..16 {
        let candidate = parent.join(format!(".{leaf}.trail-replace-{}", now_nanos()));
        match fs::symlink_metadata(&candidate) {
            Ok(_) => continue,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(candidate),
            Err(err) => return Err(Error::Io(err)),
        }
    }
    Err(Error::InvalidInput(
        "could not create unique lane workdir replacement backup path".to_string(),
    ))
}

fn remove_existing_lane_workdir_path(workdir_path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(workdir_path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(Error::Io(err)),
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(workdir_path)?;
    } else {
        fs::remove_file(workdir_path)?;
    }
    Ok(())
}

struct SparseHydrationTransaction {
    backup_dir: PathBuf,
    snapshots: Vec<SparseHydrationSnapshot>,
}

struct SparseHydrationSnapshot {
    target: PathBuf,
    state: SparseHydrationSnapshotState,
}

enum SparseHydrationSnapshotState {
    Missing,
    File {
        backup_path: PathBuf,
        permissions: fs::Permissions,
    },
    #[cfg(unix)]
    Symlink {
        target: PathBuf,
    },
    Directory,
    Other,
}

impl SparseHydrationTransaction {
    fn begin<'a, I>(workdir_path: &Path, write_paths: I) -> Result<Self>
    where
        I: IntoIterator<Item = &'a String>,
    {
        let parent = workdir_path.parent().ok_or_else(|| Error::InvalidPath {
            path: workdir_path.to_string_lossy().to_string(),
            reason: "workdir path has no parent".to_string(),
        })?;
        fs::create_dir_all(parent)?;
        let backup_dir = create_unique_lane_workdir_sync_stage_dir(parent, workdir_path)?;
        let backup_dir_for_cleanup = backup_dir.clone();
        let result = (|| -> Result<Self> {
            let mut paths = BTreeSet::new();
            paths.insert(".trail/sparse-selection.json".to_string());
            paths.insert(".trail/workdir-manifest.json".to_string());
            paths.extend(write_paths.into_iter().cloned());

            let mut snapshots = Vec::new();
            for (index, path) in paths.iter().enumerate() {
                let target = safe_join(workdir_path, path)?;
                let backup_path = backup_dir.join(format!("snapshot-{index}"));
                let state = snapshot_sparse_hydration_target(path, &target, backup_path)?;
                snapshots.push(SparseHydrationSnapshot { target, state });
            }

            Ok(Self {
                backup_dir,
                snapshots,
            })
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&backup_dir_for_cleanup);
        }
        result
    }

    fn commit(self) {
        let _ = fs::remove_dir_all(&self.backup_dir);
    }

    fn rollback(self) -> Result<()> {
        let mut result = Ok(());
        for snapshot in self.snapshots.iter().rev() {
            if let Err(err) = restore_sparse_hydration_snapshot(snapshot) {
                result = Err(err);
                break;
            }
        }
        let cleanup = fs::remove_dir_all(&self.backup_dir).map_err(Error::Io);
        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(err), _) => Err(err),
            (Ok(()), Err(err)) => Err(err),
        }
    }
}

fn snapshot_sparse_hydration_target(
    rel: &str,
    target: &Path,
    backup_path: PathBuf,
) -> Result<SparseHydrationSnapshotState> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SparseHydrationSnapshotState::Missing);
        }
        Err(err) => return Err(Error::Io(err)),
    };
    if metadata.file_type().is_symlink() {
        #[cfg(unix)]
        {
            return Ok(SparseHydrationSnapshotState::Symlink {
                target: fs::read_link(target)?,
            });
        }
        #[cfg(not(unix))]
        {
            return Err(Error::InvalidPath {
                path: rel.to_string(),
                reason: "cannot snapshot symlink on this platform".to_string(),
            });
        }
    }
    if metadata.is_file() {
        fs::copy(target, &backup_path)?;
        return Ok(SparseHydrationSnapshotState::File {
            backup_path,
            permissions: metadata.permissions(),
        });
    }
    if metadata.is_dir() {
        return Ok(SparseHydrationSnapshotState::Directory);
    }
    let _ = rel;
    Ok(SparseHydrationSnapshotState::Other)
}

fn restore_sparse_hydration_snapshot(snapshot: &SparseHydrationSnapshot) -> Result<()> {
    match &snapshot.state {
        SparseHydrationSnapshotState::Missing => remove_sparse_hydration_target(&snapshot.target),
        SparseHydrationSnapshotState::File {
            backup_path,
            permissions,
        } => {
            if sparse_hydration_file_matches_snapshot(&snapshot.target, backup_path, permissions)? {
                return Ok(());
            }
            remove_sparse_hydration_target(&snapshot.target)?;
            if let Some(parent) = snapshot.target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(backup_path, &snapshot.target)?;
            fs::set_permissions(&snapshot.target, permissions.clone())?;
            Ok(())
        }
        #[cfg(unix)]
        SparseHydrationSnapshotState::Symlink { target } => {
            if sparse_hydration_symlink_matches_snapshot(&snapshot.target, target)? {
                return Ok(());
            }
            remove_sparse_hydration_target(&snapshot.target)?;
            if let Some(parent) = snapshot.target.parent() {
                fs::create_dir_all(parent)?;
            }
            symlink_file(target, &snapshot.target)?;
            Ok(())
        }
        SparseHydrationSnapshotState::Directory | SparseHydrationSnapshotState::Other => Ok(()),
    }
}

fn sparse_hydration_file_matches_snapshot(
    target: &Path,
    backup_path: &Path,
    permissions: &fs::Permissions,
) -> Result<bool> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => metadata,
        Ok(_) => return Ok(false),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(Error::Io(err)),
    };
    if fs::read(target)? != fs::read(backup_path)? {
        return Ok(false);
    }
    if sparse_hydration_permissions_match(&metadata.permissions(), permissions) {
        return Ok(true);
    }
    fs::set_permissions(target, permissions.clone())?;
    Ok(true)
}

fn sparse_hydration_permissions_match(
    current: &fs::Permissions,
    expected: &fs::Permissions,
) -> bool {
    #[cfg(unix)]
    {
        current.mode() == expected.mode()
    }
    #[cfg(not(unix))]
    {
        current.readonly() == expected.readonly()
    }
}

#[cfg(unix)]
fn sparse_hydration_symlink_matches_snapshot(target: &Path, expected: &Path) -> Result<bool> {
    match fs::symlink_metadata(target) {
        Ok(metadata) if metadata.file_type().is_symlink() => Ok(fs::read_link(target)? == expected),
        Ok(_) => Ok(false),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(Error::Io(err)),
    }
}

fn remove_sparse_hydration_target(target: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(Error::Io(err)),
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(target)?;
    } else {
        fs::remove_file(target)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_lane_workdir_with_stage_replaces_existing_directory() {
        let temp = tempfile::tempdir().unwrap();
        let workdir = temp.path().join("lane-workdir");
        fs::create_dir(&workdir).unwrap();
        fs::write(workdir.join("old.txt"), "old").unwrap();

        let stage = temp.path().join(".lane-workdir.trail-sync-test");
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("new.txt"), "new").unwrap();

        replace_lane_workdir_with_stage(&workdir, &stage).unwrap();

        assert_eq!(fs::read_to_string(workdir.join("new.txt")).unwrap(), "new");
        assert!(!workdir.join("old.txt").exists());
        assert!(!stage.exists());
        assert_no_replacement_backups(temp.path());
    }

    #[test]
    fn replace_lane_workdir_with_stage_replaces_existing_file() {
        let temp = tempfile::tempdir().unwrap();
        let workdir = temp.path().join("lane-workdir");
        fs::write(&workdir, "old-file").unwrap();

        let stage = temp.path().join(".lane-workdir.trail-sync-test");
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("new.txt"), "new").unwrap();

        replace_lane_workdir_with_stage(&workdir, &stage).unwrap();

        assert!(workdir.is_dir());
        assert_eq!(fs::read_to_string(workdir.join("new.txt")).unwrap(), "new");
        assert!(!stage.exists());
        assert_no_replacement_backups(temp.path());
    }

    #[test]
    fn replace_lane_workdir_with_stage_handles_missing_target() {
        let temp = tempfile::tempdir().unwrap();
        let workdir = temp.path().join("lane-workdir");
        let stage = temp.path().join(".lane-workdir.trail-sync-test");
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("new.txt"), "new").unwrap();

        replace_lane_workdir_with_stage(&workdir, &stage).unwrap();

        assert_eq!(fs::read_to_string(workdir.join("new.txt")).unwrap(), "new");
        assert!(!stage.exists());
        assert_no_replacement_backups(temp.path());
    }

    #[test]
    fn replace_lane_workdir_with_stage_restores_backup_when_stage_rename_fails() {
        let temp = tempfile::tempdir().unwrap();
        let workdir = temp.path().join("lane-workdir");
        fs::create_dir(&workdir).unwrap();
        fs::write(workdir.join("old.txt"), "old").unwrap();
        let missing_stage = temp.path().join(".missing-stage");

        let err = replace_lane_workdir_with_stage(&workdir, &missing_stage).unwrap_err();

        assert!(matches!(err, Error::Io(_)));
        assert_eq!(fs::read_to_string(workdir.join("old.txt")).unwrap(), "old");
        assert_no_replacement_backups(temp.path());
    }

    fn assert_no_replacement_backups(parent: &Path) {
        for entry in fs::read_dir(parent).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            assert!(
                !name.contains(".trail-replace-"),
                "unexpected replacement backup left behind: {name}"
            );
        }
    }
}
