use super::*;

impl CrabDb {
    pub fn diff_range(&self, spec: &str, patches: bool) -> Result<DiffSummary> {
        self.diff_range_with_options(spec, patches, false)
    }

    pub fn diff_range_with_options(
        &self,
        spec: &str,
        patches: bool,
        line_changes: bool,
    ) -> Result<DiffSummary> {
        let (left, right) = parse_range(spec)?;
        self.diff_refs_with_options(left, right, patches, line_changes)
    }

    pub fn diff_refs(&self, left: &str, right: &str, patches: bool) -> Result<DiffSummary> {
        self.diff_refs_with_options(left, right, patches, false)
    }

    pub fn diff_refs_with_options(
        &self,
        left: &str,
        right: &str,
        patches: bool,
        line_changes: bool,
    ) -> Result<DiffSummary> {
        let left_ref = self.resolve_refish(left)?;
        let right_ref = self.resolve_refish(right)?;
        let left_files = self.load_root_files(&left_ref.root_id)?;
        let right_files = self.load_root_files(&right_ref.root_id)?;
        self.diff_files(
            left.to_string(),
            right.to_string(),
            &left_files,
            &right_files,
            patches,
            line_changes,
        )
    }

    pub fn diff_roots(&self, spec: &str, patches: bool, line_changes: bool) -> Result<DiffSummary> {
        let (left, right) = parse_range(spec)?;
        let left_id = ObjectId(left.to_string());
        let right_id = ObjectId(right.to_string());
        let left_files = self.load_root_files(&left_id)?;
        let right_files = self.load_root_files(&right_id)?;
        self.diff_files(
            left.to_string(),
            right.to_string(),
            &left_files,
            &right_files,
            patches,
            line_changes,
        )
    }

    pub fn diff_dirty(&mut self, patches: bool, line_changes: bool) -> Result<DiffSummary> {
        let _lock = self.acquire_write_lock()?;
        let branch = self.current_branch()?;
        let head = self.resolve_branch_ref(&branch)?;
        let previous_files = self.load_root_files(&head.root_id)?;
        let disk_files = self.scan_worktree_files()?;
        let change_id = self.allocate_change_id("crabdb", "dirty-diff")?;
        let built =
            self.build_root_from_disk_files(&disk_files, &change_id, Some(&previous_files))?;
        self.diff_files(
            branch,
            "dirty".to_string(),
            &previous_files,
            &built.files,
            patches,
            line_changes,
        )
    }

    pub fn checkout(&mut self, change_or_ref: &str, force: bool) -> Result<CheckoutReport> {
        self.checkout_with_options(change_or_ref, force, false, None, false)
    }

    pub fn checkout_with_options(
        &mut self,
        change_or_ref: &str,
        force: bool,
        dry_run: bool,
        workdir: Option<&Path>,
        record_dirty: bool,
    ) -> Result<CheckoutReport> {
        let _lock = self.acquire_write_lock()?;
        if dry_run && record_dirty {
            return Err(Error::InvalidInput(
                "checkout --record-dirty cannot be combined with --dry-run".to_string(),
            ));
        }
        let mut recorded_dirty = None;
        if record_dirty {
            let current_branch = self.current_branch()?;
            let report = self.record_with_options_unlocked(
                Some(&current_branch),
                Some(format!(
                    "Record dirty worktree before checkout `{change_or_ref}`"
                )),
                Actor::human(),
                RecordOptions {
                    kind: Some(OperationKind::Checkout),
                    ..RecordOptions::default()
                },
            )?;
            recorded_dirty = report.operation;
        }
        let current = self.resolve_branch_ref(&self.current_branch()?)?;
        if !dry_run && workdir.is_none() && !force && !record_dirty {
            let status = self.status(None)?;
            if status.worktree_state != WorktreeState::Clean {
                return Err(Error::DirtyWorktree);
            }
        }
        let target = self.resolve_refish(change_or_ref)?;
        let current_files = self.load_root_files(&current.root_id)?;
        let target_files = self.load_root_files(&target.root_id)?;
        let diff = self.diff_file_maps(&current_files, &target_files)?;
        let output_root = workdir
            .map(|path| self.resolve_checkout_workdir_path(path))
            .transpose()?;
        if !dry_run {
            if let Some(output_root) = &output_root {
                prepare_checkout_workdir(output_root)?;
                materialize_into(
                    &self.workspace_root,
                    output_root,
                    &BTreeMap::new(),
                    &target_files,
                    |entry| self.materialize_entry_bytes(entry),
                )?;
            } else {
                self.materialize_files(&current_files, &target_files)?;
            }
        }
        Ok(CheckoutReport {
            change_id: target.change_id,
            root_id: target.root_id,
            written_files: if dry_run {
                0
            } else {
                target_files.len() as u64
            },
            dry_run,
            recorded_dirty,
            output_root: output_root.map(|path| path.to_string_lossy().to_string()),
            changed_paths: diff.summaries,
        })
    }

    pub fn create_branch(&mut self, name: &str, from: Option<&str>) -> Result<BranchReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(name)?;
        let source = match from {
            Some(refish) => self.resolve_refish(refish)?,
            None => self.resolve_branch_ref(&self.current_branch()?)?,
        };
        let ref_name = branch_ref(name);
        if self.try_get_ref(&ref_name)?.is_some() {
            return Err(Error::InvalidInput(format!(
                "branch `{name}` already exists"
            )));
        }
        self.set_ref(
            &ref_name,
            &source.change_id,
            &source.root_id,
            &source.operation_id,
        )?;
        Ok(BranchReport {
            name: name.to_string(),
            from: source.change_id,
            root_id: source.root_id,
        })
    }

    pub fn list_branches(&self) -> Result<Vec<BranchListEntry>> {
        let current = self.current_branch()?;
        let mut stmt = self.conn.prepare(
            "SELECT name, change_id, root_id, operation_id, generation, updated_at \
             FROM refs WHERE name LIKE 'refs/branches/%' ORDER BY name",
        )?;
        let rows = stmt.query_map([], ref_row)?;
        let refs = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        Ok(refs
            .into_iter()
            .map(|record| {
                let name = record
                    .name
                    .strip_prefix(MAIN_REF_PREFIX)
                    .unwrap_or(&record.name)
                    .to_string();
                BranchListEntry {
                    is_current: name == current || record.name == current,
                    name,
                    ref_name: record.name,
                    change_id: record.change_id,
                    root_id: record.root_id,
                    generation: record.generation,
                }
            })
            .collect())
    }

    pub fn delete_branch(&mut self, name: &str) -> Result<BranchDeleteReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(name)?;
        let current = self.current_branch()?;
        let ref_name = branch_ref(name);
        let short_name = ref_name.strip_prefix(MAIN_REF_PREFIX).unwrap_or(name);
        if short_name == current || ref_name == current {
            return Err(Error::InvalidInput(format!(
                "cannot delete current branch `{short_name}`"
            )));
        }
        self.get_ref(&ref_name)?;
        self.conn
            .execute("DELETE FROM refs WHERE name = ?1", params![ref_name])?;
        remove_ref_file(&self.db_dir, &ref_name)?;
        Ok(BranchDeleteReport {
            name: short_name.to_string(),
            ref_name,
        })
    }

    pub fn rename_branch(&mut self, old_name: &str, new_name: &str) -> Result<BranchRenameReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(old_name)?;
        validate_ref_segment(new_name)?;
        let old_ref = branch_ref(old_name);
        let new_ref = branch_ref(new_name);
        let record = self.get_ref(&old_ref)?;
        if self.try_get_ref(&new_ref)?.is_some() {
            return Err(Error::InvalidInput(format!(
                "branch `{new_name}` already exists"
            )));
        }
        self.conn.execute(
            "UPDATE refs SET name = ?1, updated_at = ?2 WHERE name = ?3",
            params![new_ref, now_ts(), old_ref],
        )?;
        remove_ref_file(&self.db_dir, &old_ref)?;
        write_ref_file(
            &self.db_dir,
            &new_ref,
            &record.change_id,
            &record.root_id,
            &record.operation_id,
            record.generation,
        )?;
        let current = self.current_branch()?;
        let old_short = old_ref.strip_prefix(MAIN_REF_PREFIX).unwrap_or(old_name);
        let new_short = new_ref.strip_prefix(MAIN_REF_PREFIX).unwrap_or(new_name);
        if current == old_short || current == old_ref {
            fs::write(self.db_dir.join(HEAD_FILE), format!("{new_short}\n"))?;
        }
        Ok(BranchRenameReport {
            old_name: old_short.to_string(),
            new_name: new_short.to_string(),
            change_id: record.change_id,
            root_id: record.root_id,
        })
    }

    pub fn why(&self, path_line: &str, branch: Option<&str>) -> Result<WhyResult> {
        let (path, line_number) = parse_path_line(path_line)?;
        let head = self.resolve_why_ref(branch)?;
        let files = self.load_root_files(&head.root_id)?;
        let entry = files
            .get(&path)
            .ok_or_else(|| Error::InvalidInput(format!("path `{path}` is not tracked")))?;
        let FileContentRef::Text(text_id) = &entry.content else {
            return Err(Error::InvalidInput(format!(
                "path `{path}` is not line-tracked text"
            )));
        };
        let lines = self.load_text_lines(text_id)?;
        let Some(line) = lines.get(line_number.saturating_sub(1) as usize) else {
            return Err(Error::InvalidInput(format!(
                "line {line_number} is outside `{path}`"
            )));
        };
        self.why_from_line(path, line_number, entry, line)
    }

    pub fn why_line_id(&self, line_id: &str, branch: Option<&str>) -> Result<WhyResult> {
        let parsed = parse_line_id_key(line_id)?;
        let line_id_key = line_id_key_value(&parsed);
        let head = self.resolve_why_ref(branch)?;
        let files = self.load_root_files(&head.root_id)?;
        for (path, entry) in &files {
            let FileContentRef::Text(text_id) = &entry.content else {
                continue;
            };
            let lines = self.load_text_lines(text_id)?;
            for (index, line) in lines.iter().enumerate() {
                if line.line_id_key() == line_id_key {
                    return self.why_from_line(path.clone(), index as u64 + 1, entry, line);
                }
            }
        }
        Err(Error::InvalidInput(format!(
            "line id `{line_id}` is not present in the selected root"
        )))
    }

    pub(crate) fn resolve_why_ref(&self, refish: Option<&str>) -> Result<RefRecord> {
        match refish {
            Some(refish) => self.resolve_refish(refish),
            None => self.resolve_branch_ref(&self.current_branch()?),
        }
    }

    pub(crate) fn why_from_line(
        &self,
        path: String,
        line_number: u64,
        entry: &FileEntry,
        line: &LineEntry,
    ) -> Result<WhyResult> {
        let mut stmt = self.conn.prepare(
            "SELECT change_id, path, line_number, kind, text_hash, created_at \
             FROM line_history WHERE line_id = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![line.line_id_key()], |row| {
            Ok(LineHistoryEntry {
                change_id: ChangeId(row.get(0)?),
                path: row.get(1)?,
                line_number: row.get::<_, Option<i64>>(2)?.map(|n| n as u64),
                kind: parse_line_change_kind(&row.get::<_, String>(3)?),
                text_hash: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        let history = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        Ok(WhyResult {
            path,
            line_number,
            file_id: entry.file_id.clone(),
            line_id: line.line_id.clone(),
            current_text: String::from_utf8_lossy(&line.text).into_owned(),
            introduced_by: line.introduced_by.clone(),
            last_content_change: line.last_content_change.clone(),
            last_move_change: line.last_move_change.clone(),
            history,
        })
    }

    pub fn create_anchor(
        &mut self,
        path_line: &str,
        label: impl Into<String>,
        branch: Option<&str>,
    ) -> Result<AnchorCreateReport> {
        let _lock = self.acquire_write_lock()?;
        let label = label.into();
        if label.trim().is_empty() {
            return Err(Error::InvalidInput(
                "anchor label cannot be empty".to_string(),
            ));
        }
        let why = self.why(path_line, branch)?;
        let anchor = Anchor {
            version: ANCHOR_OBJECT_VERSION,
            id: AnchorId::new(&why.file_id, &why.line_id, &label),
            label,
            file_id: why.file_id,
            line_id: why.line_id,
            created_path: why.path,
            created_line: why.line_number,
            created_change: why.last_content_change,
            created_at: now_ts(),
        };
        let object_id = self.put_object(ANCHOR_KIND, ANCHOR_OBJECT_VERSION, &anchor)?;
        self.index_anchor(&anchor, &object_id)?;
        Ok(AnchorCreateReport { anchor, object_id })
    }

    pub fn list_anchors(&self) -> Result<Vec<Anchor>> {
        let mut stmt = self
            .conn
            .prepare("SELECT object_id FROM anchors ORDER BY created_at ASC, anchor_id ASC")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let object_ids = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        object_ids
            .into_iter()
            .map(|object_id| self.get_object(ANCHOR_KIND, &ObjectId(object_id)))
            .collect()
    }

    pub fn resolve_anchor(
        &self,
        anchor_id: &str,
        branch: Option<&str>,
    ) -> Result<AnchorResolveReport> {
        let anchor = self.anchor(anchor_id)?;
        let branch = branch.map(str::to_string).unwrap_or(self.current_branch()?);
        let head = self.resolve_refish(&branch)?;
        let files = self.load_root_files(&head.root_id)?;
        let Some((path, entry)) = files
            .iter()
            .find(|(_, entry)| entry.file_id == anchor.file_id)
        else {
            return Ok(AnchorResolveReport {
                anchor,
                branch,
                status: "missing_file".to_string(),
                path: None,
                line_number: None,
                text: None,
            });
        };
        let FileContentRef::Text(text_id) = &entry.content else {
            return Ok(AnchorResolveReport {
                anchor,
                branch,
                status: "non_text".to_string(),
                path: Some(path.clone()),
                line_number: None,
                text: None,
            });
        };
        let lines = self.load_text_lines(text_id)?;
        for (idx, line) in lines.iter().enumerate() {
            if line.line_id == anchor.line_id {
                return Ok(AnchorResolveReport {
                    anchor,
                    branch,
                    status: "found".to_string(),
                    path: Some(path.clone()),
                    line_number: Some(idx as u64 + 1),
                    text: Some(String::from_utf8_lossy(&line.text).into_owned()),
                });
            }
        }
        Ok(AnchorResolveReport {
            anchor,
            branch,
            status: "missing_line".to_string(),
            path: Some(path.clone()),
            line_number: None,
            text: None,
        })
    }

    pub fn delete_anchor(&mut self, anchor_id: &str) -> Result<AnchorDeleteReport> {
        let _lock = self.acquire_write_lock()?;
        let anchor = self.anchor(anchor_id)?;
        self.conn.execute(
            "DELETE FROM anchors WHERE anchor_id = ?1",
            params![anchor.id.0],
        )?;
        Ok(AnchorDeleteReport {
            anchor_id: anchor.id,
        })
    }
}
