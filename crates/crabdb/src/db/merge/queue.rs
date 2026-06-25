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

    pub fn enqueue_merge(
        &mut self,
        source: &str,
        target: &str,
        priority: i64,
    ) -> Result<MergeQueueAddReport> {
        let _lock = self.acquire_write_lock()?;
        let source_ref = self.normalize_merge_queue_source_ref(source)?;
        let target_ref = self.normalize_merge_queue_target_ref(target)?;
        if let Some(entry) = self
            .conn
            .query_row(
                "SELECT queue_id, source_ref, target_ref, status, priority, created_at, updated_at \
                 FROM merge_queue \
                 WHERE source_ref = ?1 AND target_ref = ?2 AND status IN ('queued', 'running') \
                 ORDER BY created_at LIMIT 1",
                params![source_ref, target_ref],
                merge_queue_row,
            )
            .optional()?
        {
            return Ok(MergeQueueAddReport { entry });
        }

        let now = now_ts();
        let seed = format!("{source_ref}:{target_ref}:{priority}:{now}");
        let hash = sha256_hex(seed.as_bytes());
        let queue_id = format!("mq_{}", &hash[..16]);
        self.conn.execute(
            "INSERT INTO merge_queue \
             (queue_id, source_ref, target_ref, status, priority, created_at, updated_at) \
             VALUES (?1, ?2, ?3, 'queued', ?4, ?5, ?5)",
            params![queue_id, source_ref, target_ref, priority, now],
        )?;

        Ok(MergeQueueAddReport {
            entry: MergeQueueEntry {
                queue_id,
                source_ref,
                target_ref,
                status: "queued".to_string(),
                priority,
                created_at: now,
                updated_at: now,
            },
        })
    }

    pub fn list_merge_queue(&self) -> Result<Vec<MergeQueueEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT queue_id, source_ref, target_ref, status, priority, created_at, updated_at \
             FROM merge_queue ORDER BY status = 'queued' DESC, priority DESC, created_at ASC",
        )?;
        let rows = stmt.query_map([], merge_queue_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub fn remove_merge_queue(&mut self, selector: &str) -> Result<MergeQueueRemoveReport> {
        let _lock = self.acquire_write_lock()?;
        let agent_candidate = agent_ref(selector);
        let branch_candidate = branch_ref(selector);
        let entry = self
            .conn
            .query_row(
                "SELECT queue_id, source_ref, target_ref, status, priority, created_at, updated_at \
                 FROM merge_queue \
                 WHERE (queue_id = ?1 OR source_ref = ?1 OR source_ref = ?2 OR source_ref = ?3) \
                   AND status NOT IN ('merged', 'cancelled') \
                 ORDER BY priority DESC, created_at ASC LIMIT 1",
                params![selector, agent_candidate, branch_candidate],
                merge_queue_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("merge queue item `{selector}` not found")))?;
        let now = now_ts();
        self.conn.execute(
            "UPDATE merge_queue SET status = 'cancelled', updated_at = ?1 WHERE queue_id = ?2",
            params![now, entry.queue_id],
        )?;
        Ok(MergeQueueRemoveReport {
            entry: MergeQueueEntry {
                status: "cancelled".to_string(),
                updated_at: now,
                ..entry
            },
        })
    }

    pub fn run_merge_queue(&mut self, limit: Option<usize>) -> Result<MergeQueueRunReport> {
        let _lock = self.acquire_write_lock()?;
        let entries = self.queued_merge_entries(limit)?;
        let mut processed = Vec::new();
        let mut stopped_on_conflict = false;
        let mut stopped_on_failure = false;

        for entry in entries {
            self.set_merge_queue_status(&entry.queue_id, "running")?;
            let context = match self.merge_queue_context(&entry.source_ref, &entry.target_ref) {
                Ok(context) => context,
                Err(err) => {
                    self.set_merge_queue_status(&entry.queue_id, "failed")?;
                    processed.push(MergeQueueRunItem {
                        queue_id: entry.queue_id,
                        source_ref: entry.source_ref,
                        target_ref: entry.target_ref,
                        status: "failed".to_string(),
                        operation: None,
                        changed_paths: Vec::new(),
                        error: Some(err.to_string()),
                    });
                    stopped_on_failure = true;
                    break;
                }
            };

            match self.merge_queue_entry(&entry) {
                Ok(report) => {
                    self.set_merge_queue_status(&entry.queue_id, "merged")?;
                    self.insert_merge_result(
                        &entry,
                        &context,
                        Some(&report.operation),
                        "merged",
                        None,
                    )?;
                    processed.push(MergeQueueRunItem {
                        queue_id: entry.queue_id,
                        source_ref: report.source_ref,
                        target_ref: report.target_ref,
                        status: "merged".to_string(),
                        operation: Some(report.operation),
                        changed_paths: report.changed_paths,
                        error: None,
                    });
                }
                Err(err) => {
                    let is_conflict = matches!(err, Error::Conflict(_));
                    let status = if is_conflict { "conflicted" } else { "failed" };
                    let message = err.to_string();
                    self.set_merge_queue_status(&entry.queue_id, status)?;
                    self.insert_merge_result(
                        &entry,
                        &context,
                        None,
                        status,
                        is_conflict.then_some(message.as_str()),
                    )?;
                    processed.push(MergeQueueRunItem {
                        queue_id: entry.queue_id,
                        source_ref: entry.source_ref,
                        target_ref: entry.target_ref,
                        status: status.to_string(),
                        operation: None,
                        changed_paths: Vec::new(),
                        error: Some(message),
                    });
                    if is_conflict {
                        stopped_on_conflict = true;
                    } else {
                        stopped_on_failure = true;
                    }
                    break;
                }
            }
        }

        Ok(MergeQueueRunReport {
            processed,
            stopped_on_conflict,
            stopped_on_failure,
        })
    }

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
                    let built = self.build_file_entry(
                        &path,
                        content.into_bytes(),
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

        let base_files = self.load_root_files(&base_ref.root_id)?;
        let source_files = self.load_root_files(&source_ref.root_id)?;
        let target_files = self.load_root_files(&target_ref.root_id)?;
        let actor = Actor::system();
        let change_id = self.allocate_change_id(&actor.id, "agent_merge")?;
        let (merged_files, conflicts) =
            self.merge_file_maps(&base_files, &target_files, &source_files, &change_id)?;
        if !conflicts.is_empty() {
            if dry_run {
                return Ok(MergeReport {
                    operation: change_id,
                    source_ref: agent_branch.ref_name,
                    target_ref: target_ref_name,
                    root_id: target_ref.root_id,
                    dry_run,
                    changed_paths: Vec::new(),
                    conflicts,
                });
            }
            let detail = conflicts.join("; ");
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

        let built = self.build_root_from_file_entries(merged_files, &change_id)?;
        let diff = self.diff_file_maps(&target_files, &built.files)?;
        if diff.changes.is_empty() {
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
        let head_files = self.load_root_files(&head.root_id)?;
        let disk_files = self.scan_files_under(&workdir_path)?;
        let disk_manifest = self.disk_manifest(&disk_files);
        Ok(Some(
            self.diff_file_maps_to_manifest(&head_files, &disk_manifest),
        ))
    }

    pub(crate) fn merge_file_maps(
        &self,
        base: &BTreeMap<String, FileEntry>,
        target: &BTreeMap<String, FileEntry>,
        source: &BTreeMap<String, FileEntry>,
        change_id: &ChangeId,
    ) -> Result<(BTreeMap<String, FileEntry>, Vec<String>)> {
        let mut merged = target.clone();
        let mut conflicts = Vec::new();
        let mut pending_text_merges = Vec::new();
        let mut paths = BTreeSet::new();
        paths.extend(base.keys().cloned());
        paths.extend(target.keys().cloned());
        paths.extend(source.keys().cloned());
        for path in paths {
            let base_entry = base.get(&path);
            let target_entry = target.get(&path);
            let source_entry = source.get(&path);
            let target_changed = entry_hash(base_entry) != entry_hash(target_entry);
            let source_changed = entry_hash(base_entry) != entry_hash(source_entry);
            match (target_changed, source_changed) {
                (false, true) => match source_entry {
                    Some(entry) => {
                        merged.insert(path.clone(), entry.clone());
                    }
                    None => {
                        merged.remove(&path);
                    }
                },
                (true, true) => {
                    if entry_hash(target_entry) == entry_hash(source_entry) {
                        continue;
                    }
                    match self.plan_line_merge(&path, base_entry, target_entry, source_entry)? {
                        Some(plan) => pending_text_merges.push(plan),
                        None => conflicts.push(format!("both changed `{path}` differently")),
                    }
                }
                _ => {}
            }
        }

        if conflicts.is_empty() {
            for plan in pending_text_merges {
                let entry =
                    self.file_entry_from_merged_lines(&plan.target_entry, &plan.lines, change_id)?;
                merged.insert(plan.path, entry);
            }
        }
        Ok((merged, conflicts))
    }

    pub(crate) fn plan_line_merge(
        &self,
        path: &str,
        base_entry: Option<&FileEntry>,
        target_entry: Option<&FileEntry>,
        source_entry: Option<&FileEntry>,
    ) -> Result<Option<PendingLineMerge>> {
        let (Some(base_entry), Some(target_entry), Some(source_entry)) =
            (base_entry, target_entry, source_entry)
        else {
            return Ok(None);
        };
        if base_entry.kind != FileKind::Text
            || target_entry.kind != FileKind::Text
            || source_entry.kind != FileKind::Text
            || base_entry.file_id != target_entry.file_id
            || base_entry.file_id != source_entry.file_id
            || target_entry.executable != source_entry.executable
            || target_entry.mode != source_entry.mode
        {
            return Ok(None);
        }
        let (
            FileContentRef::Text(base_text),
            FileContentRef::Text(target_text),
            FileContentRef::Text(source_text),
        ) = (
            &base_entry.content,
            &target_entry.content,
            &source_entry.content,
        )
        else {
            return Ok(None);
        };
        let base_lines = self.load_text_lines(base_text)?;
        let target_lines = self.load_text_lines(target_text)?;
        let source_lines = self.load_text_lines(source_text)?;
        let base_order = base_lines
            .iter()
            .map(LineEntryExt::line_id_key)
            .collect::<Vec<_>>();
        if !preserves_base_line_order(&base_order, &target_lines)
            || !preserves_base_line_order(&base_order, &source_lines)
        {
            return Ok(None);
        }

        let base_keys = base_order.iter().cloned().collect::<HashSet<_>>();
        let target_inserted_gaps = inserted_line_gaps(&target_lines, &base_keys);
        let source_inserted_groups = inserted_line_groups(&source_lines, &base_keys);
        if source_inserted_groups
            .iter()
            .any(|(gap, _)| target_inserted_gaps.contains(gap))
        {
            return Ok(None);
        }

        let base_by_id = line_map_by_id(&base_lines);
        let target_by_id = line_map_by_id(&target_lines);
        let source_by_id = line_map_by_id(&source_lines);
        let mut merged_lines = target_lines.clone();
        for line_id in &base_order {
            let base_line = base_by_id.get(line_id).copied();
            let target_line = target_by_id.get(line_id).copied();
            let source_line = source_by_id.get(line_id).copied();
            let target_changed = !line_content_equal(base_line, target_line);
            let source_changed = !line_content_equal(base_line, source_line);
            match (target_changed, source_changed) {
                (true, true) if !line_content_equal(target_line, source_line) => return Ok(None),
                (false, true) => match source_line {
                    Some(line) => replace_or_insert_line(&mut merged_lines, line_id, line.clone()),
                    None => remove_line(&mut merged_lines, line_id),
                },
                _ => {}
            }
        }
        for (gap, lines) in source_inserted_groups {
            insert_lines_at_gap(&mut merged_lines, &gap, lines);
        }
        Ok(Some(PendingLineMerge {
            path: path.to_string(),
            target_entry: target_entry.clone(),
            lines: merged_lines,
        }))
    }

    pub(crate) fn file_entry_from_merged_lines(
        &self,
        target_entry: &FileEntry,
        lines: &[LineEntry],
        change_id: &ChangeId,
    ) -> Result<FileEntry> {
        let bytes = materialize_lines(lines);
        let text_id = self.put_text_content_from_lines(lines)?;
        let mut entry = target_entry.clone();
        entry.content = FileContentRef::Text(text_id);
        entry.size_bytes = bytes.len() as u64;
        entry.content_hash = sha256_hex(&bytes);
        entry.last_content_change = change_id.clone();
        Ok(entry)
    }
}
