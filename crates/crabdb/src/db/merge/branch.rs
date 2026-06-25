use super::*;

impl CrabDb {
    pub fn merge_branches(&mut self, source: &str, target: &str) -> Result<MergeReport> {
        self.merge_branches_with_options(source, target, false)
    }

    pub fn merge_branches_with_options(
        &mut self,
        source: &str,
        target: &str,
        dry_run: bool,
    ) -> Result<MergeReport> {
        let _lock = self.acquire_write_lock()?;
        self.merge_branches_unlocked(source, target, dry_run)
    }

    pub(crate) fn merge_branches_unlocked(
        &mut self,
        source: &str,
        target: &str,
        dry_run: bool,
    ) -> Result<MergeReport> {
        let source_ref_name = branch_ref(source);
        let target_ref_name = branch_ref(target);
        let source_ref = self.get_ref(&source_ref_name)?;
        let target_ref = self.get_ref(&target_ref_name)?;
        let base_change = self.common_parent_hint(&source_ref.change_id, &target_ref.change_id)?;
        let base_ref = self.ref_from_change(&base_change)?;
        let base_files = self.load_root_files(&base_ref.root_id)?;
        let source_files = self.load_root_files(&source_ref.root_id)?;
        let target_files = self.load_root_files(&target_ref.root_id)?;
        let actor = Actor::human();
        let change_id = self.allocate_change_id(&actor.id, "merge")?;
        let (merged_files, conflicts) =
            self.merge_file_maps(&base_files, &target_files, &source_files, &change_id)?;
        if !conflicts.is_empty() {
            if dry_run {
                return Ok(MergeReport {
                    operation: change_id,
                    source_ref: source_ref_name,
                    target_ref: target_ref_name,
                    root_id: target_ref.root_id,
                    dry_run,
                    changed_paths: Vec::new(),
                    conflicts,
                });
            }
            return Err(Error::Conflict(conflicts.join("; ")));
        }
        let built = self.build_root_from_file_entries(merged_files, &change_id)?;
        let diff = self.diff_file_maps(&target_files, &built.files)?;
        if dry_run {
            return Ok(MergeReport {
                operation: change_id,
                source_ref: source_ref_name,
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
            kind: OperationKind::Merge,
            parents: vec![target_ref.change_id.clone(), source_ref.change_id.clone()],
            before_root: Some(target_ref.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: target_ref_name.clone(),
            actor,
            session_id: None,
            message: Some(format!("Merge `{source}` into `{target}`")),
            changes: diff.changes,
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&target_ref, &change_id, &built.root_id, &operation_id)?;
        Ok(MergeReport {
            operation: change_id,
            source_ref: source_ref_name,
            target_ref: target_ref_name,
            root_id: built.root_id,
            dry_run,
            changed_paths: diff.summaries,
            conflicts: Vec::new(),
        })
    }

    pub(crate) fn normalize_merge_queue_source_ref(&self, source: &str) -> Result<String> {
        if source.starts_with("refs/") {
            self.get_ref(source)?;
            return Ok(source.to_string());
        }
        let agent_ref_name = agent_ref(source);
        if self.try_get_ref(&agent_ref_name)?.is_some() {
            return Ok(agent_ref_name);
        }
        let branch_ref_name = branch_ref(source);
        self.get_ref(&branch_ref_name)?;
        Ok(branch_ref_name)
    }

    pub(crate) fn normalize_merge_queue_target_ref(&self, target: &str) -> Result<String> {
        let target_ref_name = branch_ref(target);
        if !target_ref_name.starts_with(MAIN_REF_PREFIX) {
            return Err(Error::InvalidInput(
                "merge queue target must be a branch ref".to_string(),
            ));
        }
        self.get_ref(&target_ref_name)?;
        Ok(target_ref_name)
    }

    pub(crate) fn queued_merge_entries(
        &self,
        limit: Option<usize>,
    ) -> Result<Vec<MergeQueueEntry>> {
        let sql =
            "SELECT queue_id, source_ref, target_ref, status, priority, created_at, updated_at \
                   FROM merge_queue WHERE status = 'queued' \
                   ORDER BY priority DESC, created_at ASC";
        match limit {
            Some(limit) => {
                let mut stmt = self.conn.prepare(&format!("{sql} LIMIT ?1"))?;
                let rows = stmt.query_map(params![limit as i64], merge_queue_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
            None => {
                let mut stmt = self.conn.prepare(sql)?;
                let rows = stmt.query_map([], merge_queue_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
        }
    }

    pub(crate) fn set_merge_queue_status(&self, queue_id: &str, status: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE merge_queue SET status = ?1, updated_at = ?2 WHERE queue_id = ?3",
            params![status, now_ts(), queue_id],
        )?;
        Ok(())
    }

    pub(crate) fn merge_queue_entry(&mut self, entry: &MergeQueueEntry) -> Result<MergeReport> {
        let target = entry
            .target_ref
            .strip_prefix(MAIN_REF_PREFIX)
            .unwrap_or(&entry.target_ref);
        if let Some(agent) = entry.source_ref.strip_prefix(AGENT_REF_PREFIX) {
            return self.merge_agent_unlocked(agent, target, false, false);
        }
        if let Some(source) = entry.source_ref.strip_prefix(MAIN_REF_PREFIX) {
            return self.merge_branches_unlocked(source, target, false);
        }
        Err(Error::InvalidInput(format!(
            "merge queue source `{}` must be an agent or branch ref",
            entry.source_ref
        )))
    }

    pub(crate) fn merge_queue_context(
        &self,
        source_ref_name: &str,
        target_ref_name: &str,
    ) -> Result<MergeContext> {
        let source_ref = self.get_ref(source_ref_name)?;
        let target_ref = self.get_ref(target_ref_name)?;
        let base_change = if let Some(agent) = source_ref_name.strip_prefix(AGENT_REF_PREFIX) {
            self.agent_branch(agent)?.base_change
        } else {
            self.common_parent_hint(&source_ref.change_id, &target_ref.change_id)?
        };
        Ok(MergeContext {
            base_change,
            left_change: target_ref.change_id,
            right_change: source_ref.change_id,
        })
    }

    pub(crate) fn pending_conflict_merge(
        &self,
        conflict_set_id: &str,
    ) -> Result<PendingConflictMerge> {
        self.conn
            .query_row(
                "SELECT merge_id, queue_id, source_ref, target_ref, base_change, left_change, right_change \
                 FROM merge_results WHERE conflict_set = ?1 ORDER BY created_at DESC LIMIT 1",
                params![conflict_set_id],
                |row| {
                    Ok(PendingConflictMerge {
                        merge_id: row.get(0)?,
                        queue_id: row.get(1)?,
                        source_ref: row.get(2)?,
                        target_ref: row.get(3)?,
                        base_change: ChangeId(row.get(4)?),
                        left_change: ChangeId(row.get(5)?),
                        right_change: ChangeId(row.get(6)?),
                    })
                },
            )
            .optional()?
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "conflict set `{conflict_set_id}` is not linked to a merge result"
                ))
            })
    }

    pub(crate) fn insert_merge_result(
        &self,
        entry: &MergeQueueEntry,
        context: &MergeContext,
        result_change: Option<&ChangeId>,
        status: &str,
        conflict_detail: Option<&str>,
    ) -> Result<()> {
        self.insert_merge_result_for_refs(
            Some(&entry.queue_id),
            &entry.source_ref,
            &entry.target_ref,
            context,
            result_change,
            status,
            conflict_detail,
        )?;
        Ok(())
    }

    pub(crate) fn insert_merge_result_for_refs(
        &self,
        queue_id: Option<&str>,
        source_ref: &str,
        target_ref: &str,
        context: &MergeContext,
        result_change: Option<&ChangeId>,
        status: &str,
        conflict_detail: Option<&str>,
    ) -> Result<Option<String>> {
        let created_at = now_ts();
        let seed = format!(
            "{}:{}:{}:{}:{}",
            queue_id.unwrap_or("direct"),
            source_ref,
            target_ref,
            status,
            created_at
        );
        let hash = sha256_hex(seed.as_bytes());
        let merge_id = format!("merge_{}", &hash[..16]);
        let conflict_set = conflict_detail.map(|detail| {
            let conflict_hash = sha256_hex(format!("{merge_id}:{detail}").as_bytes());
            format!("conflict_{}", &conflict_hash[..16])
        });
        let conflict_details_json = conflict_detail
            .map(|detail| {
                let details = detail
                    .split("; ")
                    .filter(|item| !item.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                serde_json::to_string(&details)
            })
            .transpose()?;
        let result_change = result_change.map(|change| change.0.clone());
        self.conn.execute(
            "INSERT INTO merge_results \
             (merge_id, queue_id, source_ref, target_ref, base_change, left_change, right_change, result_change, status, conflict_set, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                merge_id,
                queue_id,
                source_ref,
                target_ref,
                context.base_change.0,
                context.left_change.0,
                context.right_change.0,
                result_change,
                status,
                conflict_set,
                created_at
            ],
        )?;
        if let Some(conflict_set_id) = &conflict_set {
            self.conn.execute(
                "INSERT INTO conflict_sets \
                 (conflict_set_id, merge_id, source_ref, target_ref, status, details_json, created_at) \
                 VALUES (?1, ?2, ?3, ?4, 'open', ?5, ?6)",
                params![
                    conflict_set_id,
                    merge_id,
                    source_ref,
                    target_ref,
                    conflict_details_json,
                    created_at
                ],
            )?;
        }
        Ok(conflict_set)
    }

    pub(crate) fn existing_open_conflict_set(
        &self,
        source_ref: &str,
        target_ref: &str,
        context: &MergeContext,
    ) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT mr.conflict_set \
                 FROM merge_results mr \
                 JOIN conflict_sets cs ON cs.conflict_set_id = mr.conflict_set \
                 WHERE mr.source_ref = ?1 \
                   AND mr.target_ref = ?2 \
                   AND mr.base_change = ?3 \
                   AND mr.left_change = ?4 \
                   AND mr.right_change = ?5 \
                   AND mr.status = 'conflicted' \
                   AND cs.status = 'open' \
                 ORDER BY mr.created_at DESC LIMIT 1",
                params![
                    source_ref,
                    target_ref,
                    context.base_change.0,
                    context.left_change.0,
                    context.right_change.0
                ],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::from)
    }

    pub fn export_patch(&self, range: &str) -> Result<String> {
        let summary = self.diff_range(range, true)?;
        let mut out = String::new();
        for file in summary.files {
            if let Some(patch) = file.patch {
                out.push_str(&patch);
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
        }
        Ok(out)
    }

    pub fn git_export_commit(&mut self, range: &str, message: &str) -> Result<GitExportReport> {
        let _lock = self.acquire_write_lock()?;
        let message = message.trim();
        if message.is_empty() {
            return Err(Error::InvalidInput(
                "git export commit message cannot be empty".to_string(),
            ));
        }
        let Some(git_state) = self.current_git_state()? else {
            return Err(Error::Git(format!(
                "git export requires a Git working tree at {}",
                self.workspace_root.display()
            )));
        };
        let (left, right) = parse_range(range)?;
        let left_ref = self.resolve_refish(left)?;
        let right_ref = self.resolve_refish(right)?;
        if !self
            .ancestor_set(&right_ref.change_id)?
            .contains(&left_ref.change_id.0)
        {
            return Err(Error::InvalidInput(format!(
                "range `{range}` is not an ancestor range"
            )));
        }
        let files = self.load_root_files(&right_ref.root_id)?;
        let tree_oid = self.git_write_tree(&files)?;
        let commit = self.git_commit_tree(&tree_oid, git_state.head.as_deref(), message)?;
        let operation = self.operation(&right_ref.change_id)?;
        let branch = operation.branch.clone();
        let mapping = self.insert_git_mapping_for_state(
            "export",
            &branch,
            &right_ref.change_id,
            &right_ref.root_id,
            Some(commit.clone()),
            git_state.dirty,
        )?;
        Ok(GitExportReport {
            range: range.to_string(),
            branch,
            operation: right_ref.change_id,
            root_id: right_ref.root_id,
            commit,
            parent: git_state.head,
            mapping,
        })
    }

    pub fn fsck(&self) -> Result<FsckReport> {
        let mut report = FsckReport {
            checked_refs: 0,
            checked_roots: 0,
            checked_texts: 0,
            errors: Vec::new(),
        };
        let refs = self.all_refs()?;
        for reference in refs {
            report.checked_refs += 1;
            if self.operation(&reference.change_id).is_err() {
                report.errors.push(format!(
                    "ref {} points to missing operation {}",
                    reference.name, reference.change_id.0
                ));
            }
            match self.get_object::<WorktreeRoot>(WORKTREE_ROOT_KIND, &reference.root_id) {
                Ok(root) => {
                    report.checked_roots += 1;
                    if let Err(err) = self.validate_worktree_root(&root) {
                        report
                            .errors
                            .push(format!("root {} invalid: {err}", reference.root_id.0));
                    }
                    if let Ok(files) = self.load_root_files(&reference.root_id) {
                        for entry in files.values() {
                            if let FileContentRef::Text(text_id) = &entry.content {
                                report.checked_texts += 1;
                                if let Err(err) = self.validate_text_content(text_id) {
                                    report
                                        .errors
                                        .push(format!("text {} invalid: {err}", text_id.0));
                                }
                            }
                        }
                    }
                }
                Err(err) => report.errors.push(format!(
                    "ref {} points to missing root {}: {err}",
                    reference.name, reference.root_id.0
                )),
            }
        }
        Ok(report)
    }

    pub fn write_patch_to(&self, range: &str, output: &Path) -> Result<()> {
        let patch = self.export_patch(range)?;
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(output, patch)?;
        Ok(())
    }

    pub(crate) fn insert_git_mapping(
        &self,
        direction: &str,
        branch: &str,
        change_id: &ChangeId,
        root_id: &ObjectId,
    ) -> Result<Option<GitMapping>> {
        let Some(state) = self.current_git_state()? else {
            return Ok(None);
        };
        self.insert_git_mapping_for_state(
            direction,
            branch,
            change_id,
            root_id,
            state.head,
            state.dirty,
        )
    }

    pub(crate) fn insert_git_mapping_for_state(
        &self,
        direction: &str,
        branch: &str,
        change_id: &ChangeId,
        root_id: &ObjectId,
        git_head: Option<String>,
        git_dirty: bool,
    ) -> Result<Option<GitMapping>> {
        let created_at = now_ts();
        let seed = format!(
            "{direction}:{branch}:{:?}:{}:{}:{created_at}",
            git_head, change_id.0, root_id.0
        );
        let hash = sha256_hex(seed.as_bytes());
        let mapping_id = format!("gitmap_{}", &hash[..16]);
        self.conn.execute(
            "INSERT INTO git_mappings \
             (mapping_id, direction, branch, git_head, git_dirty, crab_change, crab_root, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                mapping_id,
                direction,
                branch,
                git_head.as_deref(),
                if git_dirty { 1_i64 } else { 0_i64 },
                change_id.0,
                root_id.0,
                created_at
            ],
        )?;
        Ok(Some(GitMapping {
            mapping_id,
            direction: direction.to_string(),
            branch: branch.to_string(),
            git_head,
            git_dirty,
            crab_change: change_id.clone(),
            crab_root: root_id.clone(),
            created_at,
        }))
    }
}
