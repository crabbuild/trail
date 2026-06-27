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
        self.show_conflict_with_limit(conflict_set_id, 50)
    }

    pub fn show_conflict_with_limit(
        &self,
        conflict_set_id: &str,
        limit: usize,
    ) -> Result<ConflictSetSummary> {
        let limit = normalize_query_limit(limit, 1000)?;
        let mut summary = self
            .conn
            .query_row(
                "SELECT conflict_set_id, merge_id, source_ref, target_ref, status, details_json, created_at \
                 FROM conflict_sets WHERE conflict_set_id = ?1",
                params![conflict_set_id],
                conflict_set_row,
            )
            .optional()?
            .ok_or_else(|| {
                Error::InvalidInput(format!("conflict set `{conflict_set_id}` not found"))
            })?;
        summary.explanation = self.conflict_explanation(&summary, limit)?;
        Ok(summary)
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
        let target_ref = self.get_ref(&pending.target_ref)?;
        if target_ref.change_id != pending.left_change {
            return Err(Error::StaleBranch(pending.target_ref));
        }
        let (kind, session_id, source_lane_id, source_at_conflict_head) =
            if let Some(lane) = pending.source_ref.strip_prefix(LANE_REF_PREFIX) {
                let branch = self.lane_branch(lane)?;
                (
                    OperationKind::LaneMerge,
                    branch.session_id,
                    Some(branch.lane_id),
                    branch.head_change == pending.right_change,
                )
            } else {
                (OperationKind::Merge, None, None, true)
            };

        let conflict_paths = conflict_paths_from_details(&summary.details)?;
        let base_root = self.pending_base_root(&pending)?;
        let target_root = self.pending_target_root(&pending)?;
        let source_root = self.pending_source_root(&pending)?;
        let base_files = self.load_root_files(&base_root)?;
        let source_files = self.load_root_files(&source_root)?;
        let target_files = self.load_root_files(&target_root)?;
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
        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind,
            parents: vec![pending.left_change.clone(), pending.right_change.clone()],
            before_root: Some(target_root.clone()),
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
        if let Some(lane_id) = source_lane_id {
            let status = if source_at_conflict_head {
                "merged"
            } else {
                "active"
            };
            self.conn.execute(
                "UPDATE lane_branches SET status = ?1, updated_at = ?2 WHERE lane_id = ?3",
                params![status, now_ts(), lane_id],
            )?;
        }
        if let Some(explanation) = &summary.explanation {
            self.record_known_conflict_resolutions(
                explanation,
                conflict_set_id,
                &pending.source_ref,
                &pending.target_ref,
                &resolution_label,
                &change_id,
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

    fn conflict_explanation(
        &self,
        summary: &ConflictSetSummary,
        limit: usize,
    ) -> Result<Option<ConflictExplanation>> {
        let pending = match self.pending_conflict_merge(&summary.conflict_set_id) {
            Ok(pending) => pending,
            Err(_) => return Ok(None),
        };
        let conflict_paths = match conflict_paths_from_details(&summary.details) {
            Ok(paths) => paths,
            Err(_) => return Ok(None),
        };
        let base_root = self.pending_base_root(&pending)?;
        let target_root = self.pending_target_root(&pending)?;
        let source_root = self.pending_source_root(&pending)?;
        let paths = conflict_paths.into_iter().collect::<Vec<_>>();
        let base_files = self.load_root_files_for_paths(&base_root, &paths)?;
        let target_files = self.load_root_files_for_paths(&target_root, &paths)?;
        let source_files = self.load_root_files_for_paths(&source_root, &paths)?;
        let needs_rename_lookup = paths.iter().any(|path| {
            matches!(
                (
                    base_files.get(path),
                    target_files.get(path),
                    source_files.get(path)
                ),
                (Some(_), None, Some(_)) | (Some(_), Some(_), None)
            )
        });
        let target_lookup_files;
        let source_lookup_files;
        let target_lookup = if needs_rename_lookup {
            target_lookup_files = self.load_root_files(&target_root)?;
            &target_lookup_files
        } else {
            &target_files
        };
        let source_lookup = if needs_rename_lookup {
            source_lookup_files = self.load_root_files(&source_root)?;
            &source_lookup_files
        } else {
            &source_files
        };
        let mut explanations = Vec::new();
        let mut recommendations = Vec::new();
        for path in paths.into_iter().take(limit) {
            let explanation = self.explain_conflict_path(
                &path,
                base_files.get(&path),
                target_files.get(&path),
                source_files.get(&path),
                &base_files,
                target_lookup,
                source_lookup,
                &pending,
                limit,
            )?;
            recommendations.push(explanation.recommendation.clone());
            explanations.push(explanation);
        }
        Ok(Some(ConflictExplanation {
            merge: ConflictMergeContext {
                merge_id: pending.merge_id,
                queue_id: pending.queue_id,
                source_ref: pending.source_ref,
                target_ref: pending.target_ref,
                base_change: pending.base_change,
                target_change: pending.left_change,
                source_change: pending.right_change,
                base_root: Some(base_root),
                target_root: Some(target_root),
                source_root: Some(source_root),
            },
            paths: explanations,
            recommendations,
            next_steps: vec![
                "Inspect each conflicted path and choose source, target, or manual content."
                    .to_string(),
                format!(
                    "Resolve with `crabdb conflicts resolve {} --take source`, `--take target`, or `--manual <JSON_FILE>`.",
                    summary.conflict_set_id
                ),
                "Run status, diff, and the relevant test/eval gates after resolution.".to_string(),
            ],
        }))
    }

    fn explain_conflict_path(
        &self,
        path: &str,
        base_entry: Option<&FileEntry>,
        target_entry: Option<&FileEntry>,
        source_entry: Option<&FileEntry>,
        base_files: &BTreeMap<String, FileEntry>,
        target_files: &BTreeMap<String, FileEntry>,
        source_files: &BTreeMap<String, FileEntry>,
        pending: &PendingConflictMerge,
        limit: usize,
    ) -> Result<ConflictPathExplanation> {
        let target_changed = entry_hash(base_entry) != entry_hash(target_entry);
        let source_changed = entry_hash(base_entry) != entry_hash(source_entry);
        let target = self.side_provenance(
            "target",
            target_entry
                .map(|entry| entry.last_content_change.clone())
                .unwrap_or_else(|| pending.left_change.clone()),
        );
        let source = self.side_provenance(
            "source",
            source_entry
                .map(|entry| entry.last_content_change.clone())
                .unwrap_or_else(|| pending.right_change.clone()),
        );
        let lines = self.explain_conflict_lines(base_entry, target_entry, source_entry, limit)?;
        let same_insertion_gap =
            self.same_insertion_gap_conflict(base_entry, target_entry, source_entry)?;
        let conflict_class = conflict_class_for_path(
            path,
            base_entry,
            target_entry,
            source_entry,
            base_files,
            target_files,
            source_files,
            &lines,
            same_insertion_gap,
        );
        let signature = conflict_path_signature(
            path,
            &conflict_class,
            base_entry,
            target_entry,
            source_entry,
        );
        let known_resolutions = self.known_conflict_resolutions(&signature)?;
        let reason = if !lines.is_empty() {
            "both sides changed the same logical line differently".to_string()
        } else if !matches!(
            (base_entry, target_entry, source_entry),
            (Some(_), Some(_), Some(_))
        ) {
            "one side changed or deleted a path that the other side also changed".to_string()
        } else {
            "both sides changed the same path and CrabDB could not prove a safe line merge"
                .to_string()
        };
        let recommendation = match (target_changed, source_changed, lines.is_empty()) {
            (false, true, _) => ConflictResolutionCandidate {
                resolution: "source".to_string(),
                confidence: "high".to_string(),
                reason: "Only source differs from the recorded base for this path.".to_string(),
            },
            (true, false, _) => ConflictResolutionCandidate {
                resolution: "target".to_string(),
                confidence: "high".to_string(),
                reason: "Only target differs from the recorded base for this path.".to_string(),
            },
            (_, _, false) => ConflictResolutionCandidate {
                resolution: "manual".to_string(),
                confidence: "high".to_string(),
                reason: "Manual resolution is safest because both sides edited the same logical content."
                    .to_string(),
            },
            _ => ConflictResolutionCandidate {
                resolution: "manual".to_string(),
                confidence: "medium".to_string(),
                reason: "Review source and target content before choosing a side.".to_string(),
            },
        };
        let summary = match (target_changed, source_changed) {
            (true, true) => "target and source both changed this path".to_string(),
            (true, false) => "target changed this path".to_string(),
            (false, true) => "source changed this path".to_string(),
            (false, false) => {
                "path is listed in the conflict, but no content delta was detected".to_string()
            }
        };
        Ok(ConflictPathExplanation {
            path: path.to_string(),
            conflict_class,
            summary,
            reason,
            target,
            source,
            lines,
            recommendation,
            known_resolutions,
            signature,
        })
    }

    fn known_conflict_resolutions(&self, signature: &str) -> Result<Vec<ConflictKnownResolution>> {
        let mut stmt = self.conn.prepare(
            "SELECT resolution, conflict_set_id, operation, created_at \
             FROM conflict_resolution_suggestions \
             WHERE signature = ?1 \
             ORDER BY created_at DESC, suggestion_id DESC \
             LIMIT 5",
        )?;
        let rows = stmt.query_map(params![signature], |row| {
            let resolution: String = row.get(0)?;
            let conflict_set_id: String = row.get(1)?;
            Ok(ConflictKnownResolution {
                confidence: "known".to_string(),
                reason: format!(
                    "A previous conflict with the same path/content signature was resolved with `{resolution}`."
                ),
                resolution,
                conflict_set_id,
                operation: ChangeId(row.get(2)?),
                created_at: row.get(3)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    fn record_known_conflict_resolutions(
        &self,
        explanation: &ConflictExplanation,
        conflict_set_id: &str,
        source_ref: &str,
        target_ref: &str,
        resolution: &str,
        operation: &ChangeId,
    ) -> Result<()> {
        let created_at = now_ts();
        for path in &explanation.paths {
            if path.signature.is_empty() {
                continue;
            }
            let suggestion_id = format!(
                "known_conflict_{}",
                &sha256_hex(
                    format!(
                        "{}:{}:{}:{}",
                        path.signature, conflict_set_id, resolution, operation.0
                    )
                    .as_bytes()
                )[..16]
            );
            self.conn.execute(
                "INSERT OR REPLACE INTO conflict_resolution_suggestions \
                 (suggestion_id, signature, path, conflict_class, resolution, conflict_set_id, operation, source_ref, target_ref, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    suggestion_id,
                    path.signature,
                    path.path,
                    path.conflict_class,
                    resolution,
                    conflict_set_id,
                    operation.0,
                    source_ref,
                    target_ref,
                    created_at
                ],
            )?;
        }
        Ok(())
    }

    fn pending_base_root(&self, pending: &PendingConflictMerge) -> Result<ObjectId> {
        match &pending.base_root {
            Some(root) => Ok(root.clone()),
            None => Ok(self.ref_from_change(&pending.base_change)?.root_id),
        }
    }

    fn pending_target_root(&self, pending: &PendingConflictMerge) -> Result<ObjectId> {
        match &pending.left_root {
            Some(root) => Ok(root.clone()),
            None => Ok(self.ref_from_change(&pending.left_change)?.root_id),
        }
    }

    fn pending_source_root(&self, pending: &PendingConflictMerge) -> Result<ObjectId> {
        match &pending.right_root {
            Some(root) => Ok(root.clone()),
            None => Ok(self.ref_from_change(&pending.right_change)?.root_id),
        }
    }

    fn explain_conflict_lines(
        &self,
        base_entry: Option<&FileEntry>,
        target_entry: Option<&FileEntry>,
        source_entry: Option<&FileEntry>,
        limit: usize,
    ) -> Result<Vec<ConflictLineExplanation>> {
        let (Some(base_entry), Some(target_entry), Some(source_entry)) =
            (base_entry, target_entry, source_entry)
        else {
            return Ok(Vec::new());
        };
        if base_entry.kind != FileKind::Text
            || target_entry.kind != FileKind::Text
            || source_entry.kind != FileKind::Text
            || base_entry.file_id != target_entry.file_id
            || base_entry.file_id != source_entry.file_id
        {
            return Ok(Vec::new());
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
            return Ok(Vec::new());
        };
        let base_lines = self.load_text_lines(base_text)?;
        let target_lines = self.load_text_lines(target_text)?;
        let source_lines = self.load_text_lines(source_text)?;
        let target_by_id = line_map_by_id(&target_lines);
        let source_by_id = line_map_by_id(&source_lines);
        let mut out = Vec::new();
        for base_line in &base_lines {
            let line_id = base_line.line_id_key();
            let target_line = target_by_id.get(&line_id).copied();
            let source_line = source_by_id.get(&line_id).copied();
            let target_changed = !line_content_equal(Some(base_line), target_line);
            let source_changed = !line_content_equal(Some(base_line), source_line);
            if target_changed && source_changed && !line_content_equal(target_line, source_line) {
                out.push(ConflictLineExplanation {
                    line_id,
                    base: Some(line_preview(base_line)),
                    target: target_line.map(line_preview),
                    source: source_line.map(line_preview),
                    target_change: target_line.and_then(|line| {
                        self.side_provenance("target", line.last_content_change.clone())
                    }),
                    source_change: source_line.and_then(|line| {
                        self.side_provenance("source", line.last_content_change.clone())
                    }),
                    reason: "target and source changed this stable line differently".to_string(),
                });
                if out.len() >= limit {
                    break;
                }
            }
        }
        Ok(out)
    }

    fn same_insertion_gap_conflict(
        &self,
        base_entry: Option<&FileEntry>,
        target_entry: Option<&FileEntry>,
        source_entry: Option<&FileEntry>,
    ) -> Result<bool> {
        let (Some(base_entry), Some(target_entry), Some(source_entry)) =
            (base_entry, target_entry, source_entry)
        else {
            return Ok(false);
        };
        if base_entry.kind != FileKind::Text
            || target_entry.kind != FileKind::Text
            || source_entry.kind != FileKind::Text
            || base_entry.file_id != target_entry.file_id
            || base_entry.file_id != source_entry.file_id
        {
            return Ok(false);
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
            return Ok(false);
        };
        let base_lines = self.load_text_lines(base_text)?;
        let target_lines = self.load_text_lines(target_text)?;
        let source_lines = self.load_text_lines(source_text)?;
        let base_order = base_lines
            .iter()
            .map(LineEntryExt::line_id_key)
            .collect::<Vec<_>>();
        let base_keys = base_order.iter().cloned().collect::<HashSet<_>>();
        let target_inserted_gaps = inserted_line_gaps(&target_lines, &base_keys);
        let source_inserted_groups = inserted_line_groups(&source_lines, &base_keys);
        Ok(source_inserted_groups
            .iter()
            .any(|(gap, _)| target_inserted_gaps.contains(gap)))
    }

    fn side_provenance(&self, side: &str, change_id: ChangeId) -> Option<ConflictSideProvenance> {
        let operation = self.operation(&change_id).ok()?;
        Some(ConflictSideProvenance {
            side: side.to_string(),
            change_id: operation.change_id,
            kind: format!("{:?}", operation.kind),
            branch: operation.branch,
            actor_id: operation.actor.id,
            session_id: operation.session_id,
            message: operation.message,
            created_at: operation.created_at,
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

fn line_preview(line: &LineEntry) -> String {
    let mut value = String::from_utf8_lossy(&line.text).to_string();
    match line.newline {
        NewlineKind::None => {}
        NewlineKind::Lf => value.push('\n'),
        NewlineKind::Crlf => value.push_str("\r\n"),
    }
    const MAX_PREVIEW_CHARS: usize = 160;
    if value.chars().count() > MAX_PREVIEW_CHARS {
        value = value.chars().take(MAX_PREVIEW_CHARS - 3).collect();
        value.push_str("...");
    }
    value
}

fn conflict_class_for_path(
    path: &str,
    base_entry: Option<&FileEntry>,
    target_entry: Option<&FileEntry>,
    source_entry: Option<&FileEntry>,
    _base_files: &BTreeMap<String, FileEntry>,
    target_files: &BTreeMap<String, FileEntry>,
    source_files: &BTreeMap<String, FileEntry>,
    lines: &[ConflictLineExplanation],
    same_insertion_gap: bool,
) -> String {
    if same_insertion_gap {
        return "same_insertion_gap".to_string();
    }
    if has_binary_entry(base_entry)
        || has_binary_entry(target_entry)
        || has_binary_entry(source_entry)
    {
        return "binary".to_string();
    }
    if let (Some(base), None, Some(_)) = (base_entry, target_entry, source_entry) {
        if file_id_at_different_path(target_files, &base.file_id, path) {
            return "rename/modify".to_string();
        }
        return "delete/modify".to_string();
    }
    if let (Some(base), Some(_), None) = (base_entry, target_entry, source_entry) {
        if file_id_at_different_path(source_files, &base.file_id, path) {
            return "rename/modify".to_string();
        }
        return "delete/modify".to_string();
    }
    if let (Some(_), Some(target), Some(source)) = (base_entry, target_entry, source_entry) {
        if target.executable != source.executable || target.mode != source.mode {
            return "mode".to_string();
        }
        if !lines.is_empty() {
            return "modify/modify".to_string();
        }
        return "modify/modify".to_string();
    }
    "modify/modify".to_string()
}

fn has_binary_entry(entry: Option<&FileEntry>) -> bool {
    entry.is_some_and(|entry| matches!(entry.kind, FileKind::Binary | FileKind::OpaqueText))
}

fn file_id_at_different_path(
    files: &BTreeMap<String, FileEntry>,
    file_id: &FileId,
    path: &str,
) -> bool {
    files
        .iter()
        .any(|(candidate_path, entry)| candidate_path != path && &entry.file_id == file_id)
}

fn conflict_path_signature(
    path: &str,
    conflict_class: &str,
    base_entry: Option<&FileEntry>,
    target_entry: Option<&FileEntry>,
    source_entry: Option<&FileEntry>,
) -> String {
    let payload = format!(
        "v1\0{path}\0{conflict_class}\0{}\0{}\0{}",
        conflict_entry_signature(base_entry),
        conflict_entry_signature(target_entry),
        conflict_entry_signature(source_entry)
    );
    sha256_hex(payload.as_bytes())
}

fn conflict_entry_signature(entry: Option<&FileEntry>) -> String {
    match entry {
        Some(entry) => format!(
            "{}:{}:{}:{:?}",
            entry.content_hash, entry.executable, entry.mode, entry.kind
        ),
        None => "deleted".to_string(),
    }
}
