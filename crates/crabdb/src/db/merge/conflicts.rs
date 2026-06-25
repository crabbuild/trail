use super::*;

impl CrabDb {
    pub fn list_conflicts(&self) -> Result<Vec<ConflictSetSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT conflict_set_id, merge_id, source_ref, target_ref, status, details_json, created_at \
             FROM conflict_sets ORDER BY created_at DESC, conflict_set_id DESC",
        )?;
        let rows = stmt.query_map([], conflict_set_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub fn show_conflict(&self, conflict_set_id: &str) -> Result<ConflictSetSummary> {
        self.conn
            .query_row(
                "SELECT conflict_set_id, merge_id, source_ref, target_ref, status, details_json, created_at \
                 FROM conflict_sets WHERE conflict_set_id = ?1",
                params![conflict_set_id],
                conflict_set_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("conflict set `{conflict_set_id}` not found")))
    }

    pub fn resolve_conflict(
        &mut self,
        conflict_set_id: &str,
        take: &str,
    ) -> Result<ConflictResolveReport> {
        let _lock = self.acquire_write_lock()?;
        let take = parse_conflict_take(take)?;
        self.resolve_conflict_unlocked(conflict_set_id, ConflictResolution::Take(take))
    }

    pub fn resolve_conflict_manual(
        &mut self,
        conflict_set_id: &str,
        manual: ConflictManualResolution,
    ) -> Result<ConflictResolveReport> {
        let _lock = self.acquire_write_lock()?;
        self.resolve_conflict_unlocked(conflict_set_id, ConflictResolution::Manual(manual))
    }

    pub(crate) fn resolve_conflict_unlocked(
        &mut self,
        conflict_set_id: &str,
        resolution: ConflictResolution,
    ) -> Result<ConflictResolveReport> {
        let summary = self.show_conflict(conflict_set_id)?;
        if summary.status != "open" {
            return Err(Error::InvalidInput(format!(
                "conflict set `{conflict_set_id}` is {}",
                summary.status
            )));
        }
        let pending = self.pending_conflict_merge(conflict_set_id)?;
        let source_ref = self.get_ref(&pending.source_ref)?;
        let target_ref = self.get_ref(&pending.target_ref)?;
        if source_ref.change_id != pending.right_change {
            return Err(Error::StaleBranch(pending.source_ref));
        }
        if target_ref.change_id != pending.left_change {
            return Err(Error::StaleBranch(pending.target_ref));
        }

        let conflict_paths = conflict_paths_from_details(&summary.details)?;
        let base_ref = self.ref_from_change(&pending.base_change)?;
        let base_files = self.load_root_files(&base_ref.root_id)?;
        let source_files = self.load_root_files(&source_ref.root_id)?;
        let target_files = self.load_root_files(&target_ref.root_id)?;
        let manual_files = match &resolution {
            ConflictResolution::Take(_) => None,
            ConflictResolution::Manual(manual) => Some(normalize_manual_conflict_files(
                manual.clone(),
                &conflict_paths,
            )?),
        };

        let actor = Actor::system();
        let change_id = self.allocate_change_id(&actor.id, "conflict_resolve")?;
        let (merged_files, resolution_label) = match resolution {
            ConflictResolution::Take(take) => {
                let merged_files = merge_files_with_resolution(
                    &base_files,
                    &target_files,
                    &source_files,
                    &conflict_paths,
                    take,
                )?;
                let resolution = match take {
                    ConflictTake::Source => "source",
                    ConflictTake::Target => "target",
                };
                (merged_files, resolution.to_string())
            }
            ConflictResolution::Manual(_) => {
                let mut merged_files = merge_files_with_resolution(
                    &base_files,
                    &target_files,
                    &source_files,
                    &conflict_paths,
                    ConflictTake::Target,
                )?;
                self.apply_manual_conflict_files(
                    &mut merged_files,
                    &base_files,
                    &target_files,
                    &source_files,
                    manual_files.unwrap_or_default(),
                    &change_id,
                )?;
                (merged_files, "manual".to_string())
            }
        };
        let built = self.build_root_from_file_entries(merged_files, &change_id)?;
        let diff = self.diff_file_maps(&target_files, &built.files)?;
        let (kind, session_id) =
            if let Some(agent) = pending.source_ref.strip_prefix(AGENT_REF_PREFIX) {
                let branch = self.agent_branch(agent)?;
                (OperationKind::AgentMerge, branch.session_id)
            } else {
                (OperationKind::Merge, None)
            };
        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind,
            parents: vec![target_ref.change_id.clone(), source_ref.change_id.clone()],
            before_root: Some(target_ref.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: pending.target_ref.clone(),
            actor,
            session_id,
            message: Some(format!(
                "Resolve conflict `{conflict_set_id}` with {resolution_label}"
            )),
            changes: diff.changes,
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&target_ref, &change_id, &built.root_id, &operation_id)?;
        self.conn.execute(
            "UPDATE merge_results SET status = 'resolved', result_change = ?1 WHERE merge_id = ?2",
            params![change_id.0, pending.merge_id],
        )?;
        self.conn.execute(
            "UPDATE conflict_sets SET status = 'resolved' WHERE conflict_set_id = ?1",
            params![conflict_set_id],
        )?;
        if let Some(queue_id) = pending.queue_id {
            self.conn.execute(
                "UPDATE merge_queue SET status = 'merged', updated_at = ?1 WHERE queue_id = ?2",
                params![now_ts(), queue_id],
            )?;
        }
        if let Some(agent) = pending.source_ref.strip_prefix(AGENT_REF_PREFIX) {
            self.conn.execute(
                "UPDATE agent_branches SET status = 'merged', updated_at = ?1 WHERE agent_id = ?2",
                params![now_ts(), self.agent_branch(agent)?.agent_id],
            )?;
        }
        Ok(ConflictResolveReport {
            conflict_set_id: conflict_set_id.to_string(),
            resolution: resolution_label,
            operation: change_id,
            target_ref: pending.target_ref,
            root_id: built.root_id,
            changed_paths: diff.summaries,
        })
    }

    pub(crate) fn apply_manual_conflict_files(
        &self,
        merged_files: &mut BTreeMap<String, FileEntry>,
        base_files: &BTreeMap<String, FileEntry>,
        target_files: &BTreeMap<String, FileEntry>,
        source_files: &BTreeMap<String, FileEntry>,
        manual_files: BTreeMap<String, ConflictManualFile>,
        change_id: &ChangeId,
    ) -> Result<()> {
        let mut file_seq = 1;
        let mut line_seq = 1;
        for (path, file) in manual_files {
            let previous = target_files
                .get(&path)
                .or_else(|| source_files.get(&path))
                .or_else(|| base_files.get(&path));
            let default_executable = previous.is_some_and(|entry| entry.executable);
            match manual_conflict_file_payload(file, default_executable)? {
                ManualConflictPayload::Delete => {
                    merged_files.remove(&path);
                }
                ManualConflictPayload::Text {
                    content,
                    executable,
                } => {
                    let bytes = content.into_bytes();
                    let content_hash = sha256_hex(&bytes);
                    let built = self.build_file_entry(
                        &path,
                        bytes,
                        content_hash,
                        executable,
                        change_id,
                        previous,
                        &mut file_seq,
                        &mut line_seq,
                    )?;
                    merged_files.insert(path, built.entry);
                }
            }
        }
        Ok(())
    }
}
