use super::*;

impl CrabDb {
    pub fn merge_agent(&mut self, agent: &str, into: &str) -> Result<MergeReport> {
        self.merge_agent_with_options(agent, into, false)
    }

    pub fn merge_agent_with_options(
        &mut self,
        agent: &str,
        into: &str,
        dry_run: bool,
    ) -> Result<MergeReport> {
        let _lock = self.acquire_write_lock()?;
        self.merge_agent_unlocked(agent, into, dry_run, true)
    }

    pub(crate) fn merge_agent_unlocked(
        &mut self,
        agent: &str,
        into: &str,
        dry_run: bool,
        persist_conflict: bool,
    ) -> Result<MergeReport> {
        validate_ref_segment(agent)?;
        let agent_branch = self.agent_branch(agent)?;
        let source_ref = self.get_ref(&agent_branch.ref_name)?;
        self.ensure_agent_workdir_clean(&agent_branch, &source_ref)?;
        if !dry_run {
            self.ensure_agent_merge_readiness(agent)?;
        }
        let target_ref_name = branch_ref(into);
        let target_ref = self.get_ref(&target_ref_name)?;
        let base_ref = self.ref_from_change(&agent_branch.base_change)?;

        let actor = Actor::system();
        let change_id = self.allocate_change_id(&actor.id, "agent_merge")?;
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
                    source_ref: agent_branch.ref_name,
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
                    base_change: agent_branch.base_change.clone(),
                    left_change: target_ref.change_id.clone(),
                    right_change: source_ref.change_id.clone(),
                };
                let conflict_set_id = match self.existing_open_conflict_set(
                    &agent_branch.ref_name,
                    &target_ref_name,
                    &context,
                )? {
                    Some(conflict_set_id) => conflict_set_id,
                    None => self
                        .insert_merge_result_for_refs(
                            None,
                            &agent_branch.ref_name,
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
                "UPDATE agent_branches SET status = 'conflicted', updated_at = ?1 WHERE agent_id = ?2",
                params![now_ts(), agent_branch.agent_id],
            )?;
            return Err(Error::Conflict(conflict_message));
        }

        if merged.merged_files == merged.target_files {
            if !dry_run {
                self.conn.execute(
                    "UPDATE agent_branches SET status = 'merged', updated_at = ?1 WHERE agent_id = ?2",
                    params![now_ts(), agent_branch.agent_id],
                )?;
            }
            return Ok(MergeReport {
                operation: target_ref.change_id,
                source_ref: agent_branch.ref_name,
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
                source_ref: agent_branch.ref_name,
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
            kind: OperationKind::AgentMerge,
            parents: vec![target_ref.change_id.clone(), source_ref.change_id.clone()],
            before_root: Some(target_ref.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: target_ref_name.clone(),
            actor,
            session_id: agent_branch.session_id,
            message: Some(format!("Merge agent `{agent}` into `{into}`")),
            changes: diff.changes,
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&target_ref, &change_id, &built.root_id, &operation_id)?;
        self.conn.execute(
            "UPDATE agent_branches SET status = 'merged', updated_at = ?1 WHERE agent_id = ?2",
            params![now_ts(), agent_branch.agent_id],
        )?;
        Ok(MergeReport {
            operation: change_id,
            source_ref: agent_branch.ref_name,
            target_ref: target_ref_name,
            root_id: built.root_id,
            dry_run,
            changed_paths: diff.summaries,
            conflicts: Vec::new(),
        })
    }

    pub(crate) fn ensure_agent_workdir_clean(
        &self,
        branch: &AgentBranch,
        head: &RefRecord,
    ) -> Result<()> {
        let Some(changed_paths) = self.agent_workdir_changed_paths(branch, head)? else {
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
        let agent_label = branch
            .ref_name
            .strip_prefix(AGENT_REF_PREFIX)
            .unwrap_or(&branch.agent_id);
        Err(Error::DirtyWorktreeWithMessage(format!(
            "agent `{}` workdir has unrecorded changes; run `crabdb agent record {}` or discard them before merging: {}{}",
            agent_label, agent_label, preview, suffix
        )))
    }

    pub(crate) fn ensure_agent_merge_readiness(&self, agent: &str) -> Result<()> {
        let readiness = self.agent_readiness(agent)?;
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
            "agent `{}` is not merge-ready: {blockers}",
            readiness.agent.record.name
        )))
    }

    pub(crate) fn agent_workdir_changed_paths(
        &self,
        branch: &AgentBranch,
        head: &RefRecord,
    ) -> Result<Option<Vec<FileDiffSummary>>> {
        let Some(workdir) = &branch.workdir else {
            return Ok(None);
        };
        let workdir_path = PathBuf::from(workdir);
        if !workdir_path.is_dir() {
            return Err(Error::WorkspaceNotFound(workdir_path));
        }
        let cached_manifest =
            match self.cached_workdir_manifest_status(&workdir_path, &head.root_id)? {
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
        let changed_paths = if let Some(sparse_paths) = self.sparse_workdir_paths(&workdir_path)? {
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
            let head_files = self.load_root_files(&head.root_id)?;
            self.diff_file_maps_to_manifest(&head_files, &disk_manifest)
        };
        Ok(Some(changed_paths))
    }
}
