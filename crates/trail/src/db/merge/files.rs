use super::*;

impl Trail {
    pub(crate) fn merge_root_maps_for_changed_paths(
        &self,
        base_root_id: &ObjectId,
        target_root_id: &ObjectId,
        source_root_id: &ObjectId,
        change_id: &ChangeId,
    ) -> Result<PathLocalMergeResult> {
        let changed_paths =
            self.merge_changed_paths_between_roots(base_root_id, target_root_id, source_root_id)?;
        let paths = changed_paths.into_iter().collect::<Vec<_>>();
        let base_files = self.load_root_files_for_paths(base_root_id, &paths)?;
        let target_files = self.load_root_files_for_paths(target_root_id, &paths)?;
        let source_files = self.load_root_files_for_paths(source_root_id, &paths)?;
        let (merged_files, conflicts) =
            self.merge_file_maps(&base_files, &target_files, &source_files, change_id)?;
        Ok(PathLocalMergeResult {
            target_files,
            merged_files,
            conflicts,
        })
    }

    fn merge_changed_paths_between_roots(
        &self,
        base_root_id: &ObjectId,
        target_root_id: &ObjectId,
        source_root_id: &ObjectId,
    ) -> Result<BTreeSet<String>> {
        let mut paths = BTreeSet::new();
        for summary in self.diff_root_file_summaries(base_root_id, target_root_id)? {
            if let Some(old_path) = summary.old_path {
                paths.insert(old_path);
            }
            paths.insert(summary.path);
        }
        for summary in self.diff_root_file_summaries(base_root_id, source_root_id)? {
            if let Some(old_path) = summary.old_path {
                paths.insert(old_path);
            }
            paths.insert(summary.path);
        }
        Ok(paths)
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
