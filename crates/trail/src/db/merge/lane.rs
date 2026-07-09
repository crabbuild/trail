use super::*;

impl Trail {
    pub fn merge_lane(&mut self, lane: &str, into: &str) -> Result<MergeReport> {
        self.merge_lane_with_options(lane, into, false)
    }

    pub fn preview_lane_refresh(
        &self,
        lane: &str,
        target_branch: &str,
    ) -> Result<LaneRefreshPreviewReport> {
        validate_ref_segment(lane)?;
        let lane_branch = self.lane_branch(lane)?;
        let lane_head = self.get_ref(&lane_branch.ref_name)?;
        self.ensure_lane_workdir_clean(&lane_branch, &lane_head)?;
        let target_ref_name = branch_ref(target_branch);
        let target_ref = self.get_ref(&target_ref_name)?;
        let base_ref = self.ref_from_change(&lane_branch.base_change)?;
        let operations_behind =
            self.first_parent_distance(&target_ref.change_id, &lane_branch.base_change)?;

        let actor = Actor::system();
        let change_id = self.allocate_change_id(&actor.id, "lane_refresh_preview")?;
        let merged = self.merge_root_maps_for_changed_paths(
            &base_ref.root_id,
            &lane_head.root_id,
            &target_ref.root_id,
            &change_id,
        )?;
        let conflicted = !merged.conflicts.is_empty();
        let changed_paths = if conflicted || merged.merged_files == merged.target_files {
            Vec::new()
        } else {
            self.diff_file_maps(&merged.target_files, &merged.merged_files)?
                .summaries
        };
        let clean = !conflicted && changed_paths.is_empty();
        let next_steps = lane_refresh_preview_next_steps(lane, target_branch, clean, conflicted);
        Ok(LaneRefreshPreviewReport {
            lane_id: lane_branch.lane_id,
            ref_name: lane_branch.ref_name,
            base_change: lane_branch.base_change,
            lane_head_change: lane_head.change_id,
            lane_head_root: lane_head.root_id,
            target_ref: target_ref_name,
            target_change: target_ref.change_id,
            target_root: target_ref.root_id,
            operations_behind,
            clean,
            conflicted,
            changed_paths,
            conflicts: merged.conflicts,
            next_steps,
        })
    }

    pub fn merge_lane_with_options(
        &mut self,
        lane: &str,
        into: &str,
        dry_run: bool,
    ) -> Result<MergeReport> {
        let _lock = self.acquire_write_lock()?;
        self.merge_lane_unlocked(lane, into, dry_run, true)
    }

    pub fn merge_lane_user_with_options(
        &mut self,
        lane: &str,
        into: &str,
        dry_run: bool,
        direct: bool,
    ) -> Result<MergeReport> {
        let _lock = self.acquire_write_lock()?;
        self.ensure_direct_lane_merge_allowed(lane, into, dry_run, direct)?;
        self.merge_lane_unlocked(lane, into, dry_run, true)
    }

    pub(crate) fn merge_lane_unlocked(
        &mut self,
        lane: &str,
        into: &str,
        dry_run: bool,
        persist_conflict: bool,
    ) -> Result<MergeReport> {
        validate_ref_segment(lane)?;
        let lane_branch = self.lane_branch(lane)?;
        let source_ref = self.get_ref(&lane_branch.ref_name)?;
        self.ensure_lane_workdir_clean(&lane_branch, &source_ref)?;
        if !dry_run {
            self.ensure_lane_merge_readiness(lane)?;
        }
        let target_ref_name = branch_ref(into);
        let target_ref = self.get_ref(&target_ref_name)?;
        let base_ref = self.ref_from_change(&lane_branch.base_change)?;

        let actor = Actor::system();
        let change_id = self.allocate_change_id(&actor.id, "lane_merge")?;
        let merged = self.merge_root_maps_for_changed_paths(
            &base_ref.root_id,
            &target_ref.root_id,
            &source_ref.root_id,
            &change_id,
        )?;
        if !merged.conflicts.is_empty() {
            if dry_run {
                return Ok(MergeReport {
                    operation: change_id,
                    source_ref: lane_branch.ref_name,
                    target_ref: target_ref_name,
                    root_id: target_ref.root_id,
                    dry_run,
                    changed_paths: Vec::new(),
                    conflicts: merged.conflicts,
                });
            }
            let detail = merged.conflicts.join("; ");
            let conflict_message = if persist_conflict {
                let context = MergeContext {
                    base_change: lane_branch.base_change.clone(),
                    left_change: target_ref.change_id.clone(),
                    right_change: source_ref.change_id.clone(),
                    base_root: base_ref.root_id.clone(),
                    left_root: target_ref.root_id.clone(),
                    right_root: source_ref.root_id.clone(),
                };
                let conflict_set_id = match self.existing_open_conflict_set(
                    &lane_branch.ref_name,
                    &target_ref_name,
                    &context,
                )? {
                    Some(conflict_set_id) => conflict_set_id,
                    None => self
                        .insert_merge_result_for_refs(
                            None,
                            &lane_branch.ref_name,
                            &target_ref_name,
                            &context,
                            None,
                            "conflicted",
                            Some(&detail),
                        )?
                        .ok_or_else(|| {
                            Error::Corrupt(
                                "conflicted merge result did not create a conflict set".to_string(),
                            )
                        })?,
                };
                format!("recorded {conflict_set_id}: {detail}")
            } else {
                detail
            };
            self.conn.execute(
                "UPDATE lane_branches SET status = 'conflicted', updated_at = ?1 WHERE lane_id = ?2",
                params![now_ts(), lane_branch.lane_id],
            )?;
            return Err(Error::Conflict(conflict_message));
        }

        if merged.merged_files == merged.target_files {
            if !dry_run {
                self.conn.execute(
                    "UPDATE lane_branches SET status = 'merged', updated_at = ?1 WHERE lane_id = ?2",
                    params![now_ts(), lane_branch.lane_id],
                )?;
            }
            return Ok(MergeReport {
                operation: target_ref.change_id,
                source_ref: lane_branch.ref_name,
                target_ref: target_ref_name,
                root_id: target_ref.root_id,
                dry_run,
                changed_paths: Vec::new(),
                conflicts: Vec::new(),
            });
        }
        let built = self.build_root_from_touched_file_entries_incremental(
            &target_ref.root_id,
            &merged.target_files,
            &merged.merged_files,
            &change_id,
        )?;
        let diff = self.diff_file_maps(&merged.target_files, &merged.merged_files)?;
        if dry_run {
            return Ok(MergeReport {
                operation: change_id,
                source_ref: lane_branch.ref_name,
                target_ref: target_ref_name,
                root_id: built.root_id,
                dry_run,
                changed_paths: diff.summaries,
                conflicts: Vec::new(),
            });
        }

        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind: OperationKind::LaneMerge,
            parents: vec![target_ref.change_id.clone(), source_ref.change_id.clone()],
            before_root: Some(target_ref.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: target_ref_name.clone(),
            actor,
            session_id: lane_branch.session_id,
            message: Some(format!("Merge lane `{lane}` into `{into}`")),
            changes: diff.changes,
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&target_ref, &change_id, &built.root_id, &operation_id)?;
        self.conn.execute(
            "UPDATE lane_branches SET status = 'merged', updated_at = ?1 WHERE lane_id = ?2",
            params![now_ts(), lane_branch.lane_id],
        )?;
        Ok(MergeReport {
            operation: change_id,
            source_ref: lane_branch.ref_name,
            target_ref: target_ref_name,
            root_id: built.root_id,
            dry_run,
            changed_paths: diff.summaries,
            conflicts: Vec::new(),
        })
    }

    pub(crate) fn ensure_lane_workdir_clean(
        &self,
        branch: &LaneBranch,
        head: &RefRecord,
    ) -> Result<()> {
        let Some(changed_paths) = self.lane_workdir_changed_paths(branch, head)? else {
            return Ok(());
        };
        if changed_paths.is_empty() {
            return Ok(());
        }
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
        let lane_label = branch
            .ref_name
            .strip_prefix(LANE_REF_PREFIX)
            .unwrap_or(&branch.lane_id);
        Err(Error::DirtyWorktreeWithMessage(format!(
            "lane `{}` workdir has unrecorded changes; run `trail lane record {}` or discard them before merging: {}{}",
            lane_label, lane_label, preview, suffix
        )))
    }

    pub(crate) fn ensure_lane_merge_readiness(&self, lane: &str) -> Result<()> {
        let readiness = self.lane_readiness(lane)?;
        if readiness.ready {
            return Ok(());
        }
        let blockers = readiness
            .blockers
            .iter()
            .filter(|issue| issue.code != "open_conflicts" && issue.code != "dirty_workdir")
            .map(|issue| format!("{}: {}", issue.code, issue.message))
            .collect::<Vec<_>>();
        if blockers.is_empty() {
            return Ok(());
        }
        let blockers = blockers.join("; ");
        Err(Error::InvalidInput(format!(
            "lane `{}` is not merge-ready: {blockers}",
            readiness.lane.record.name
        )))
    }

    pub(crate) fn lane_workdir_changed_paths(
        &self,
        branch: &LaneBranch,
        head: &RefRecord,
    ) -> Result<Option<Vec<FileDiffSummary>>> {
        let Some(workdir) = &branch.workdir else {
            return Ok(None);
        };
        let workdir_path = PathBuf::from(workdir);
        if !workdir_path.is_dir() {
            return Err(Error::WorkspaceNotFound(workdir_path));
        }
        let overlay_manifest_path = self.lane_overlay_clean_manifest_path(branch)?;
        let cached_manifest = match self.lane_cached_workdir_manifest_status(
            &workdir_path,
            overlay_manifest_path.as_deref(),
            &head.root_id,
        )? {
            CachedWorkdirManifestStatus::Clean => return Ok(Some(Vec::new())),
            CachedWorkdirManifestStatus::Dirty {
                disk_manifest,
                candidate_paths,
            } => Some((disk_manifest, candidate_paths)),
            CachedWorkdirManifestStatus::Missing => None,
        };
        let disk_files;
        let (disk_manifest, candidate_paths) = if let Some(cached_manifest) = cached_manifest {
            cached_manifest
        } else {
            disk_files = self.scan_files_under(&workdir_path)?;
            (self.disk_manifest(&disk_files), None)
        };
        let changed_paths = if let Some(sparse_paths) =
            self.lane_sparse_workdir_paths(branch, &workdir_path)?
        {
            let mut selected_paths = sparse_paths;
            selected_paths.extend(disk_manifest.keys().cloned());
            selected_paths.sort();
            selected_paths.dedup();
            let head_files = self.load_root_files_for_selections(&head.root_id, &selected_paths)?;
            self.diff_file_maps_to_manifest_for_paths(&head_files, &disk_manifest, &selected_paths)
        } else if let Some(candidate_paths) = candidate_paths {
            let head_files = self.load_root_files_for_paths(&head.root_id, &candidate_paths)?;
            self.diff_file_maps_to_manifest_for_paths(&head_files, &disk_manifest, &candidate_paths)
        } else {
            self.diff_root_to_disk_manifest(&head.root_id, &disk_manifest)?
        };
        Ok(Some(changed_paths))
    }

    fn ensure_direct_lane_merge_allowed(
        &self,
        lane: &str,
        into: &str,
        dry_run: bool,
        direct: bool,
    ) -> Result<()> {
        if dry_run || direct || into != self.config.workspace.default_branch {
            return Ok(());
        }
        Err(Error::InvalidInput(format!(
            "direct merge into shared target `{into}` is disabled by default; run `trail merge-queue add {lane} --into {into}` and `trail merge-queue run`, or pass `--direct` to merge immediately"
        )))
    }
}

fn lane_refresh_preview_next_steps(
    lane: &str,
    target_branch: &str,
    clean: bool,
    conflicted: bool,
) -> Vec<String> {
    if conflicted {
        return vec![format!(
            "Resolve these refresh conflicts before merging `{lane}` into `{target_branch}`."
        )];
    }
    if clean {
        return vec![format!(
            "`{lane}` already incorporates `{target_branch}` or has no refresh changes to apply."
        )];
    }
    vec![format!(
        "Review the changed paths, then merge via `trail merge-queue add {lane} --into {target_branch}` when ready."
    )]
}
