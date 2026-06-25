use super::*;

impl CrabDb {
    pub(crate) fn insert_agent_event(
        &self,
        agent_id: &str,
        event_type: &str,
        change_id: Option<&ChangeId>,
        message_id: Option<&MessageId>,
        payload: &serde_json::Value,
    ) -> Result<String> {
        self.insert_agent_event_with_context(
            agent_id, None, None, event_type, change_id, message_id, payload,
        )
    }

    pub(crate) fn insert_agent_event_with_context(
        &self,
        agent_id: &str,
        session_id: Option<&str>,
        turn_id: Option<&str>,
        event_type: &str,
        change_id: Option<&ChangeId>,
        message_id: Option<&MessageId>,
        payload: &serde_json::Value,
    ) -> Result<String> {
        let event_seed = format!(
            "{}:{}:{}:{}:{}:{}:{}",
            agent_id,
            session_id.unwrap_or("none"),
            turn_id.unwrap_or("none"),
            event_type,
            change_id.map(|id| id.0.as_str()).unwrap_or("none"),
            message_id.map(|id| id.0.as_str()).unwrap_or("none"),
            now_nanos()
        );
        let event_id = format!("evt_{}", crate::ids::short_hash(event_seed.as_bytes(), 16));
        let payload = redact_sensitive_json(payload.clone());
        self.conn.execute(
            "INSERT INTO agent_events \
             (event_id, agent_id, turn_id, session_id, event_type, change_id, message_id, payload_json, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                event_id,
                agent_id,
                turn_id,
                session_id,
                event_type,
                change_id.map(|id| id.0.clone()),
                message_id.map(|id| id.0.clone()),
                serde_json::to_string(&payload)?,
                now_ts()
            ],
        )?;
        Ok(event_id)
    }

    pub(crate) fn messages_for_change(&self, change_id: &ChangeId) -> Result<Vec<Message>> {
        let mut stmt = self.conn.prepare(
            "SELECT object_id FROM messages WHERE change_id = ?1 ORDER BY created_at, rowid",
        )?;
        let rows = stmt.query_map(params![change_id.0], |row| row.get::<_, String>(0))?;
        let object_ids = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        object_ids
            .into_iter()
            .map(|object_id| self.get_object(MESSAGE_KIND, &ObjectId(object_id)))
            .collect()
    }

    pub(crate) fn message(&self, message_id: &str) -> Result<Message> {
        let object_id: Option<String> = self
            .conn
            .query_row(
                "SELECT object_id FROM messages WHERE message_id = ?1",
                params![message_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(object_id) = object_id else {
            return Err(Error::InvalidInput(format!(
                "message `{message_id}` not found"
            )));
        };
        self.get_object(MESSAGE_KIND, &ObjectId(object_id))
    }

    pub(crate) fn object_info(&self, object_id: &str) -> Result<ObjectInfo> {
        self.conn
            .query_row(
                "SELECT object_id, kind, version, size_bytes, created_at FROM objects WHERE object_id = ?1",
                params![object_id],
                |row| {
                    Ok(ObjectInfo {
                        object_id: ObjectId(row.get(0)?),
                        kind: row.get(1)?,
                        version: row.get::<_, i64>(2)? as u16,
                        size_bytes: row.get::<_, i64>(3)? as u64,
                        created_at: row.get(4)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("object `{object_id}` not found")))
    }

    pub(crate) fn file_history_by_path(&self, path: &str) -> Result<Vec<FileHistoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT file_id, change_id, path, old_path, kind, before_hash, after_hash, created_at \
             FROM file_history WHERE path = ?1 OR old_path = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![path], file_history_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn file_history_by_file_id(&self, file_id: &str) -> Result<Vec<FileHistoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT file_id, change_id, path, old_path, kind, before_hash, after_hash, created_at \
             FROM file_history WHERE file_id = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![file_id], file_history_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn line_history_by_line_id(&self, line_id: &str) -> Result<Vec<LineHistoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT change_id, path, line_number, kind, text_hash, created_at \
             FROM line_history WHERE line_id = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![line_id], line_history_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn session_change_ids(&self, session_id: &str) -> Result<Vec<ChangeId>> {
        let mut stmt = self.conn.prepare(
            "SELECT change_id FROM operations WHERE session_id = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| Ok(ChangeId(row.get(0)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn agent_change_ids(&self, agent: &str) -> Result<Vec<ChangeId>> {
        let branch = self.agent_branch(agent)?;
        let mut stmt = self.conn.prepare(
            "SELECT change_id FROM operations \
             WHERE branch = ?1 OR actor_id = ?2 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![branch.ref_name, branch.agent_id], |row| {
            Ok(ChangeId(row.get(0)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn operation(&self, change_id: &ChangeId) -> Result<Operation> {
        let object_id: Option<String> = self
            .conn
            .query_row(
                "SELECT operation_id FROM operations WHERE change_id = ?1",
                params![change_id.0],
                |row| row.get(0),
            )
            .optional()?;
        let Some(object_id) = object_id else {
            return Err(Error::OperationNotFound(change_id.0.clone()));
        };
        self.get_object(OPERATION_KIND, &ObjectId(object_id))
    }

    pub(crate) fn set_ref(
        &self,
        name: &str,
        change_id: &ChangeId,
        root_id: &ObjectId,
        operation_id: &ObjectId,
    ) -> Result<()> {
        let now = now_ts();
        let generation = self
            .try_get_ref(name)?
            .map(|record| record.generation + 1)
            .unwrap_or(1);
        self.conn.execute(
            "INSERT INTO refs (name, change_id, root_id, operation_id, generation, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(name) DO UPDATE SET \
                change_id = excluded.change_id, root_id = excluded.root_id, \
                operation_id = excluded.operation_id, generation = excluded.generation, \
                updated_at = excluded.updated_at",
            params![
                name,
                change_id.0,
                root_id.0,
                operation_id.0,
                generation,
                now
            ],
        )?;
        write_ref_file(
            &self.db_dir,
            name,
            change_id,
            root_id,
            operation_id,
            generation,
        )?;
        Ok(())
    }

    pub(crate) fn advance_ref_cas(
        &self,
        expected: &RefRecord,
        change_id: &ChangeId,
        root_id: &ObjectId,
        operation_id: &ObjectId,
    ) -> Result<()> {
        let generation = expected.generation + 1;
        let now = now_ts();
        let updated = self.conn.execute(
            "UPDATE refs SET change_id = ?1, root_id = ?2, operation_id = ?3, generation = ?4, updated_at = ?5 \
             WHERE name = ?6 AND generation = ?7 AND change_id = ?8",
            params![
                change_id.0,
                root_id.0,
                operation_id.0,
                generation,
                now,
                expected.name.clone(),
                expected.generation,
                expected.change_id.0.clone()
            ],
        )?;
        if updated != 1 {
            return Err(Error::StaleBranch(expected.name.clone()));
        }
        write_ref_file(
            &self.db_dir,
            &expected.name,
            change_id,
            root_id,
            operation_id,
            generation,
        )?;
        Ok(())
    }

    pub(crate) fn get_ref(&self, name: &str) -> Result<RefRecord> {
        self.try_get_ref(name)?
            .ok_or_else(|| Error::RefNotFound(name.to_string()))
    }

    pub(crate) fn try_get_ref(&self, name: &str) -> Result<Option<RefRecord>> {
        self.conn
            .query_row(
                "SELECT name, change_id, root_id, operation_id, generation, updated_at FROM refs WHERE name = ?1",
                params![name],
                ref_row,
            )
            .optional()
            .map_err(Error::from)
    }

    pub(crate) fn all_refs(&self) -> Result<Vec<RefRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, change_id, root_id, operation_id, generation, updated_at FROM refs ORDER BY name",
        )?;
        let rows = stmt.query_map([], ref_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn resolve_branch_ref(&self, branch: &str) -> Result<RefRecord> {
        if branch.starts_with("refs/") {
            self.get_ref(branch)
        } else {
            self.get_ref(&branch_ref(branch))
        }
    }

    pub(crate) fn resolve_refish(&self, refish: &str) -> Result<RefRecord> {
        if let Some(branch) = refish.strip_prefix("branch:") {
            return self.resolve_branch_ref(branch);
        }
        if let Some(agent) = refish.strip_prefix("agent:") {
            return self.get_ref(&agent_ref(agent));
        }
        if let Some(root_id) = refish.strip_prefix("root:") {
            return self.ref_from_root(&ObjectId(root_id.to_string()));
        }
        if refish.starts_with("ch_") {
            return self.ref_from_change(&ChangeId(refish.to_string()));
        }
        if refish.starts_with("refs/") {
            return self.get_ref(refish);
        }
        if let Ok(record) = self.get_ref(&branch_ref(refish)) {
            return Ok(record);
        }
        if let Ok(record) = self.get_ref(&agent_ref(refish)) {
            return Ok(record);
        }
        if refish.starts_with("obj_") {
            return self.ref_from_root(&ObjectId(refish.to_string()));
        }
        Err(Error::RefNotFound(refish.to_string()))
    }

    pub(crate) fn ref_from_change(&self, change_id: &ChangeId) -> Result<RefRecord> {
        let op = self.operation(change_id)?;
        let operation_id: String = self.conn.query_row(
            "SELECT operation_id FROM operations WHERE change_id = ?1",
            params![change_id.0],
            |row| row.get(0),
        )?;
        Ok(RefRecord {
            name: format!("changes/{}", change_id.0),
            change_id: change_id.clone(),
            root_id: op.after_root,
            operation_id: ObjectId(operation_id),
            generation: 0,
            updated_at: op.created_at,
        })
    }

    pub(crate) fn ref_from_root(&self, root_id: &ObjectId) -> Result<RefRecord> {
        let _: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        let row: Option<(String, String, i64)> = self
            .conn
            .query_row(
                "SELECT change_id, operation_id, created_at \
                 FROM operations WHERE after_root = ?1 \
                 ORDER BY created_at DESC, rowid DESC LIMIT 1",
                params![root_id.0],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((change_id, operation_id, created_at)) = row else {
            return Err(Error::InvalidInput(format!(
                "root `{}` is not associated with a recorded operation",
                root_id.0
            )));
        };
        Ok(RefRecord {
            name: format!("roots/{}", root_id.0),
            change_id: ChangeId(change_id),
            root_id: root_id.clone(),
            operation_id: ObjectId(operation_id),
            generation: 0,
            updated_at: created_at,
        })
    }

    pub(crate) fn common_parent_hint(
        &self,
        source: &ChangeId,
        target: &ChangeId,
    ) -> Result<ChangeId> {
        let source_ancestors = self.ancestor_set(source)?;
        let mut cursor = Some(target.clone());
        while let Some(change) = cursor {
            if source_ancestors.contains(&change.0) {
                return Ok(change);
            }
            cursor = self.first_parent(&change)?;
        }
        Err(Error::Conflict(
            "branches do not have a recorded common ancestor".to_string(),
        ))
    }

    pub(crate) fn ancestor_set(&self, change_id: &ChangeId) -> Result<HashSet<String>> {
        let mut out = HashSet::new();
        let mut stack = vec![change_id.clone()];
        while let Some(change) = stack.pop() {
            if !out.insert(change.0.clone()) {
                continue;
            }
            let parents = self.parents(&change)?;
            stack.extend(parents);
        }
        Ok(out)
    }

    pub(crate) fn first_parent(&self, change_id: &ChangeId) -> Result<Option<ChangeId>> {
        self.conn
            .query_row(
                "SELECT parent_change_id FROM operation_parents WHERE change_id = ?1 ORDER BY position LIMIT 1",
                params![change_id.0],
                |row| Ok(ChangeId(row.get(0)?)),
            )
            .optional()
            .map_err(Error::from)
    }

    pub(crate) fn parents(&self, change_id: &ChangeId) -> Result<Vec<ChangeId>> {
        let mut stmt = self.conn.prepare(
            "SELECT parent_change_id FROM operation_parents WHERE change_id = ?1 ORDER BY position",
        )?;
        let rows = stmt.query_map(params![change_id.0], |row| Ok(ChangeId(row.get(0)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn load_root_files(
        &self,
        root_id: &ObjectId,
    ) -> Result<BTreeMap<String, FileEntry>> {
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        let tree = tree_from_root_hex(root.path_map_root.as_deref())?;
        let iter = self.prolly.range(&tree, &[], None)?;
        let mut out = BTreeMap::new();
        for item in iter {
            let (key, value) = item?;
            let path = String::from_utf8(key)
                .map_err(|err| Error::Corrupt(format!("non UTF-8 path key: {err}")))?;
            let entry: FileEntry = from_cbor(&value)?;
            out.insert(path, entry);
        }
        Ok(out)
    }

    pub(crate) fn load_text_lines(&self, text_id: &ObjectId) -> Result<Vec<LineEntry>> {
        let content: TextContent = self.get_object(TEXT_CONTENT_KIND, text_id)?;
        let tree = tree_from_root_hex(content.order_map_root.as_deref())?;
        let iter = self.prolly.range(&tree, &[], None)?;
        let mut out = Vec::new();
        for item in iter {
            let (_, value) = item?;
            out.push(from_cbor(&value)?);
        }
        Ok(out)
    }

    pub(crate) fn materialize_entry_bytes(&self, entry: &FileEntry) -> Result<Vec<u8>> {
        match &entry.content {
            FileContentRef::Text(text_id) => {
                let lines = self.load_text_lines(text_id)?;
                Ok(materialize_lines(&lines))
            }
            FileContentRef::Opaque(blob_id) | FileContentRef::Binary(blob_id) => {
                let blob: Blob = self.get_object(BLOB_KIND, blob_id)?;
                Ok(blob.bytes)
            }
        }
    }

    pub(crate) fn materialize_files(
        &self,
        previous: &BTreeMap<String, FileEntry>,
        target: &BTreeMap<String, FileEntry>,
    ) -> Result<()> {
        materialize_into(
            &self.workspace_root,
            &self.workspace_root,
            previous,
            target,
            |entry| self.materialize_entry_bytes(entry),
        )
    }

    pub(crate) fn diff_files(
        &self,
        from: String,
        to: String,
        left: &BTreeMap<String, FileEntry>,
        right: &BTreeMap<String, FileEntry>,
        patches: bool,
        include_line_changes: bool,
    ) -> Result<DiffSummary> {
        let mut diff = self.diff_file_maps(left, right)?;
        if include_line_changes {
            attach_line_changes(&diff.changes, &mut diff.summaries);
        }
        if patches {
            self.attach_patches(left, right, &mut diff.summaries)?;
        }
        Ok(DiffSummary {
            from,
            to,
            files: diff.summaries,
        })
    }

    pub(crate) fn diff_file_maps(
        &self,
        left: &BTreeMap<String, FileEntry>,
        right: &BTreeMap<String, FileEntry>,
    ) -> Result<RootDiff> {
        let mut paths = BTreeSet::new();
        paths.extend(left.keys().cloned());
        paths.extend(right.keys().cloned());
        let mut changes = Vec::new();
        let mut summaries = Vec::new();
        let mut removed_by_hash: HashMap<String, Vec<(String, FileEntry)>> = HashMap::new();
        for (path, entry) in left {
            if !right.contains_key(path) {
                removed_by_hash
                    .entry(entry.content_hash.clone())
                    .or_default()
                    .push((path.clone(), entry.clone()));
            }
        }

        let mut handled_renames = HashSet::new();
        for path in paths {
            let old = left.get(&path);
            let new = right.get(&path);
            match (old, new) {
                (None, Some(new_entry)) => {
                    let rename = removed_by_hash
                        .get(&new_entry.content_hash)
                        .and_then(|candidates| candidates.first());
                    if let Some((old_path, old_entry)) = rename {
                        if old_entry.file_id == new_entry.file_id {
                            handled_renames.insert(old_path.clone());
                            let change = FileChange {
                                path: path.clone(),
                                old_path: Some(old_path.clone()),
                                file_id: Some(new_entry.file_id.clone()),
                                kind: FileChangeKind::Renamed,
                                before_hash: Some(old_entry.content_hash.clone()),
                                after_hash: Some(new_entry.content_hash.clone()),
                                line_changes: Vec::new(),
                            };
                            summaries.push(FileDiffSummary {
                                path: path.clone(),
                                old_path: Some(old_path.clone()),
                                kind: FileChangeKind::Renamed,
                                before_hash: Some(old_entry.content_hash.clone()),
                                after_hash: Some(new_entry.content_hash.clone()),
                                additions: 0,
                                deletions: 0,
                                line_changes: Vec::new(),
                                patch: None,
                            });
                            changes.push(change);
                            continue;
                        }
                    }
                    let line_changes = self.added_line_changes(&path, new_entry)?;
                    let (adds, dels) = count_line_delta(&line_changes);
                    changes.push(FileChange {
                        path: path.clone(),
                        old_path: None,
                        file_id: Some(new_entry.file_id.clone()),
                        kind: FileChangeKind::Added,
                        before_hash: None,
                        after_hash: Some(new_entry.content_hash.clone()),
                        line_changes,
                    });
                    summaries.push(FileDiffSummary {
                        path,
                        old_path: None,
                        kind: FileChangeKind::Added,
                        before_hash: None,
                        after_hash: Some(new_entry.content_hash.clone()),
                        additions: adds,
                        deletions: dels,
                        line_changes: Vec::new(),
                        patch: None,
                    });
                }
                (Some(old_entry), None) => {
                    if handled_renames.contains(&path) {
                        continue;
                    }
                    let line_changes = self.deleted_line_changes(&path, old_entry)?;
                    let (adds, dels) = count_line_delta(&line_changes);
                    changes.push(FileChange {
                        path: path.clone(),
                        old_path: None,
                        file_id: Some(old_entry.file_id.clone()),
                        kind: FileChangeKind::Deleted,
                        before_hash: Some(old_entry.content_hash.clone()),
                        after_hash: None,
                        line_changes,
                    });
                    summaries.push(FileDiffSummary {
                        path,
                        old_path: None,
                        kind: FileChangeKind::Deleted,
                        before_hash: Some(old_entry.content_hash.clone()),
                        after_hash: None,
                        additions: adds,
                        deletions: dels,
                        line_changes: Vec::new(),
                        patch: None,
                    });
                }
                (Some(old_entry), Some(new_entry)) => {
                    if old_entry.content_hash == new_entry.content_hash
                        && old_entry.executable == new_entry.executable
                        && old_entry.kind == new_entry.kind
                    {
                        continue;
                    }
                    let line_changes = self.modified_line_changes(old_entry, new_entry)?;
                    let (adds, dels) = count_line_delta(&line_changes);
                    let kind = if old_entry.kind != new_entry.kind {
                        FileChangeKind::TypeChanged
                    } else {
                        FileChangeKind::Modified
                    };
                    changes.push(FileChange {
                        path: path.clone(),
                        old_path: None,
                        file_id: Some(new_entry.file_id.clone()),
                        kind: kind.clone(),
                        before_hash: Some(old_entry.content_hash.clone()),
                        after_hash: Some(new_entry.content_hash.clone()),
                        line_changes,
                    });
                    summaries.push(FileDiffSummary {
                        path,
                        old_path: None,
                        kind,
                        before_hash: Some(old_entry.content_hash.clone()),
                        after_hash: Some(new_entry.content_hash.clone()),
                        additions: adds,
                        deletions: dels,
                        line_changes: Vec::new(),
                        patch: None,
                    });
                }
                (None, None) => {}
            }
        }
        Ok(RootDiff { changes, summaries })
    }

    pub(crate) fn added_line_changes(
        &self,
        _path: &str,
        entry: &FileEntry,
    ) -> Result<Vec<LineChange>> {
        let FileContentRef::Text(text_id) = &entry.content else {
            return Ok(Vec::new());
        };
        Ok(self
            .load_text_lines(text_id)?
            .into_iter()
            .enumerate()
            .map(|(idx, line)| LineChange {
                line_id: line.line_id,
                kind: LineChangeKind::Added,
                old_line_number: None,
                new_line_number: Some(idx as u64 + 1),
                before_hash: None,
                after_hash: Some(line.text_hash),
            })
            .collect())
    }

    pub(crate) fn deleted_line_changes(
        &self,
        _path: &str,
        entry: &FileEntry,
    ) -> Result<Vec<LineChange>> {
        let FileContentRef::Text(text_id) = &entry.content else {
            return Ok(Vec::new());
        };
        Ok(self
            .load_text_lines(text_id)?
            .into_iter()
            .enumerate()
            .map(|(idx, line)| LineChange {
                line_id: line.line_id,
                kind: LineChangeKind::Deleted,
                old_line_number: Some(idx as u64 + 1),
                new_line_number: None,
                before_hash: Some(line.text_hash),
                after_hash: None,
            })
            .collect())
    }

    pub(crate) fn modified_line_changes(
        &self,
        old_entry: &FileEntry,
        new_entry: &FileEntry,
    ) -> Result<Vec<LineChange>> {
        let (FileContentRef::Text(old_text), FileContentRef::Text(new_text)) =
            (&old_entry.content, &new_entry.content)
        else {
            return Ok(Vec::new());
        };
        let old_lines = self.load_text_lines(old_text)?;
        let new_lines = self.load_text_lines(new_text)?;
        let old_positions = old_lines
            .iter()
            .enumerate()
            .map(|(idx, line)| (line.line_id.clone(), (idx, line)))
            .collect::<HashMap<_, _>>();
        let new_positions = new_lines
            .iter()
            .enumerate()
            .map(|(idx, line)| (line.line_id.clone(), (idx, line)))
            .collect::<HashMap<_, _>>();
        let mut out = Vec::new();
        for (line_id, (new_idx, new_line)) in &new_positions {
            match old_positions.get(line_id) {
                Some((old_idx, old_line)) if old_line.text_hash != new_line.text_hash => {
                    out.push(LineChange {
                        line_id: line_id.clone(),
                        kind: LineChangeKind::Modified,
                        old_line_number: Some(*old_idx as u64 + 1),
                        new_line_number: Some(*new_idx as u64 + 1),
                        before_hash: Some(old_line.text_hash.clone()),
                        after_hash: Some(new_line.text_hash.clone()),
                    });
                }
                Some((old_idx, old_line)) if old_idx != new_idx => {
                    out.push(LineChange {
                        line_id: line_id.clone(),
                        kind: LineChangeKind::Moved,
                        old_line_number: Some(*old_idx as u64 + 1),
                        new_line_number: Some(*new_idx as u64 + 1),
                        before_hash: Some(old_line.text_hash.clone()),
                        after_hash: Some(new_line.text_hash.clone()),
                    });
                }
                Some(_) => {}
                None => out.push(LineChange {
                    line_id: line_id.clone(),
                    kind: LineChangeKind::Added,
                    old_line_number: None,
                    new_line_number: Some(*new_idx as u64 + 1),
                    before_hash: None,
                    after_hash: Some(new_line.text_hash.clone()),
                }),
            }
        }
        for (line_id, (old_idx, old_line)) in old_positions {
            if !new_positions.contains_key(&line_id) {
                out.push(LineChange {
                    line_id,
                    kind: LineChangeKind::Deleted,
                    old_line_number: Some(old_idx as u64 + 1),
                    new_line_number: None,
                    before_hash: Some(old_line.text_hash.clone()),
                    after_hash: None,
                });
            }
        }
        out.sort_by_key(|change| {
            (
                change
                    .new_line_number
                    .or(change.old_line_number)
                    .unwrap_or(u64::MAX),
                change.line_id.local_seq,
            )
        });
        Ok(out)
    }

    pub(crate) fn attach_patches(
        &self,
        left: &BTreeMap<String, FileEntry>,
        right: &BTreeMap<String, FileEntry>,
        summaries: &mut [FileDiffSummary],
    ) -> Result<()> {
        for summary in summaries {
            let old = summary
                .old_path
                .as_ref()
                .and_then(|path| left.get(path))
                .or_else(|| left.get(&summary.path));
            let new = right.get(&summary.path);
            let old_text = old
                .map(|entry| self.materialize_entry_bytes(entry))
                .transpose()?
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_default();
            let new_text = new
                .map(|entry| self.materialize_entry_bytes(entry))
                .transpose()?
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_default();
            summary.patch = Some(unified_patch(
                summary.old_path.as_deref().unwrap_or(&summary.path),
                &summary.path,
                &old_text,
                &new_text,
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_worktree_root(&self, root: &WorktreeRoot) -> Result<()> {
        let path_tree = tree_from_root_hex(root.path_map_root.as_deref())?;
        let index_tree = tree_from_root_hex(root.file_index_map_root.as_deref())?;
        let path_entries = self
            .prolly
            .range(&path_tree, &[], None)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut count = 0;
        for (path, value) in path_entries {
            count += 1;
            let entry: FileEntry = from_cbor(&value)?;
            let indexed = self.prolly.get(&index_tree, &entry.file_id.encode_key())?;
            if indexed.as_deref() != Some(path.as_slice()) {
                return Err(Error::Corrupt(format!(
                    "file index mismatch for {}",
                    String::from_utf8_lossy(&path)
                )));
            }
        }
        if count != root.file_count {
            return Err(Error::Corrupt(format!(
                "root file_count {} but path map has {}",
                root.file_count, count
            )));
        }
        Ok(())
    }

    pub(crate) fn validate_text_content(&self, text_id: &ObjectId) -> Result<()> {
        let content: TextContent = self.get_object(TEXT_CONTENT_KIND, text_id)?;
        let order_tree = tree_from_root_hex(content.order_map_root.as_deref())?;
        let index_tree = tree_from_root_hex(content.line_index_map_root.as_deref())?;
        let mut count = 0;
        for item in self.prolly.range(&order_tree, &[], None)? {
            let (order_key, value) = item?;
            count += 1;
            let entry: LineEntry = from_cbor(&value)?;
            let indexed = self.prolly.get(&index_tree, &entry.line_id.encode_key())?;
            if indexed.as_deref() != Some(order_key.as_slice()) {
                return Err(Error::Corrupt(format!(
                    "line index mismatch for {}",
                    entry.line_id.local_seq
                )));
            }
        }
        if count != content.line_count {
            return Err(Error::Corrupt(format!(
                "text line_count {} but order map has {}",
                content.line_count, count
            )));
        }
        Ok(())
    }

    pub(crate) fn agent_branch(&self, agent: &str) -> Result<AgentBranch> {
        self.conn
            .query_row(
                "SELECT agent_id, ref_name, base_change, head_change, base_root, head_root, session_id, workdir, status, created_at, updated_at \
                 FROM agent_branches WHERE agent_id = ?1 OR ref_name = ?2 OR agent_id IN (SELECT agent_id FROM agents WHERE name = ?1)",
                params![agent, agent_ref(agent)],
                |row| {
                    Ok(AgentBranch {
                        agent_id: row.get(0)?,
                        ref_name: row.get(1)?,
                        base_change: ChangeId(row.get(2)?),
                        head_change: ChangeId(row.get(3)?),
                        base_root: ObjectId(row.get(4)?),
                        head_root: ObjectId(row.get(5)?),
                        session_id: row.get(6)?,
                        workdir: row.get(7)?,
                        status: row.get(8)?,
                        created_at: row.get(9)?,
                        updated_at: row.get(10)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::RefNotFound(agent_ref(agent)))
    }

    pub(crate) fn agent_record(&self, agent_id: &str) -> Result<AgentRecord> {
        self.conn
            .query_row(
                "SELECT agent_id, name, kind, provider, model, created_at, metadata_json \
                 FROM agents WHERE agent_id = ?1 OR name = ?1",
                params![agent_id],
                |row| {
                    Ok(AgentRecord {
                        agent_id: row.get(0)?,
                        name: row.get(1)?,
                        kind: row.get(2)?,
                        provider: row.get(3)?,
                        model: row.get(4)?,
                        created_at: row.get(5)?,
                        metadata_json: row.get(6)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::RefNotFound(agent_id.to_string()))
    }

    pub(crate) fn lease(&self, lease_id: &str) -> Result<LeaseRecord> {
        self.conn
            .query_row(
                "SELECT lease_id, agent_id, ref_name, path, file_id, mode, expires_at, created_at \
                 FROM leases WHERE lease_id = ?1",
                params![lease_id],
                lease_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("lease `{lease_id}` not found")))
    }

    pub(crate) fn existing_active_lease(
        &self,
        agent_id: &str,
        path: Option<&str>,
        mode: &str,
    ) -> Result<Option<LeaseRecord>> {
        self.conn
            .query_row(
                "SELECT lease_id, agent_id, ref_name, path, file_id, mode, expires_at, created_at \
                 FROM leases WHERE agent_id = ?1 AND COALESCE(path, '') = COALESCE(?2, '') \
                   AND mode = ?3 AND expires_at > ?4 ORDER BY expires_at DESC LIMIT 1",
                params![agent_id, path, mode, now_ts()],
                lease_row,
            )
            .optional()
            .map_err(Error::from)
    }

    pub(crate) fn conflicting_active_leases(
        &self,
        agent_id: &str,
        path: Option<&str>,
        mode: &str,
    ) -> Result<Vec<LeaseRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT lease_id, agent_id, ref_name, path, file_id, mode, expires_at, created_at \
             FROM leases WHERE agent_id != ?1 AND COALESCE(path, '') = COALESCE(?2, '') \
               AND expires_at > ?3 ORDER BY expires_at ASC, created_at ASC",
        )?;
        let rows = stmt.query_map(params![agent_id, path, now_ts()], lease_row)?;
        let leases = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        Ok(leases
            .into_iter()
            .filter(|lease| mode == "write" || lease.mode == "write")
            .collect())
    }
}
