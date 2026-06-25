use super::*;

impl CrabDb {
    pub fn apply_agent_patch(
        &mut self,
        agent: &str,
        patch: PatchDocument,
    ) -> Result<AgentPatchReport> {
        let _lock = self.acquire_write_lock()?;
        self.apply_agent_patch_locked(agent, patch, None)
    }

    pub fn apply_agent_turn_patch(
        &mut self,
        turn_id: &str,
        patch: PatchDocument,
    ) -> Result<AgentPatchReport> {
        let _lock = self.acquire_write_lock()?;
        let turn = self.agent_turn(turn_id)?;
        if turn.ended_at.is_some() {
            return Err(Error::InvalidInput(format!(
                "turn `{turn_id}` is already ended"
            )));
        }
        self.apply_agent_patch_locked(&turn.agent_id, patch, Some(&turn))
    }

    pub fn end_agent_turn(&mut self, turn_id: &str, status: &str) -> Result<AgentTurnEndReport> {
        let _lock = self.acquire_write_lock()?;
        let status = parse_session_end_status(status)?;
        let turn = self.agent_turn(turn_id)?;
        if turn.ended_at.is_some() {
            return Ok(AgentTurnEndReport { turn });
        }
        let after_change = turn
            .after_change
            .as_ref()
            .unwrap_or(&turn.before_change)
            .clone();
        self.finish_agent_turn(turn_id, status, Some(&after_change))?;
        self.insert_agent_event_with_context(
            &turn.agent_id,
            turn.session_id.as_deref(),
            Some(turn_id),
            "turn_ended",
            Some(&after_change),
            None,
            &serde_json::json!({
                "turn_id": turn_id,
                "status": status
            }),
        )?;
        Ok(AgentTurnEndReport {
            turn: self.agent_turn(turn_id)?,
        })
    }

    pub(crate) fn apply_agent_patch_locked(
        &mut self,
        agent: &str,
        patch: PatchDocument,
        api_turn: Option<&AgentTurn>,
    ) -> Result<AgentPatchReport> {
        validate_ref_segment(agent)?;
        let agent_row = self.agent_branch(agent)?;
        let ref_name = agent_row.ref_name.clone();
        let head = self.get_ref(&ref_name)?;
        if let Some(turn) = api_turn {
            if turn.agent_id != agent_row.agent_id {
                return Err(Error::InvalidInput(format!(
                    "turn `{}` belongs to another agent",
                    turn.turn_id
                )));
            }
            if turn.before_change != head.change_id {
                return Err(Error::StaleBranch(ref_name));
            }
        }
        if let Some(base_change) = &patch.base_change {
            if base_change != &head.change_id.0 {
                return Err(Error::PatchRejected(format!(
                    "patch base {base_change} does not match agent head {}",
                    head.change_id.0
                )));
            }
        }
        for edit in &patch.edits {
            self.ensure_patch_edit_allowed(edit, patch.allow_ignored)?;
        }

        let previous_files = self.load_root_files(&head.root_id)?;
        let actor = Actor::agent(agent);
        let change_id = self.allocate_change_id(&actor.id, "agent_patch")?;
        let mut files = previous_files.clone();
        let mut manual_line_changes = Vec::new();
        let mut file_seq = 1;
        let mut line_seq = 1;

        for edit in patch.edits {
            match edit {
                PatchEdit::Write {
                    path,
                    content,
                    executable,
                } => {
                    let path = normalize_relative_path(&path)?;
                    let previous = files.get(&path);
                    let built = self.build_file_entry(
                        &path,
                        content.into_bytes(),
                        executable,
                        &change_id,
                        previous,
                        &mut file_seq,
                        &mut line_seq,
                    )?;
                    manual_line_changes.extend(
                        built
                            .line_changes
                            .iter()
                            .map(|line| (path.clone(), built.entry.file_id.clone(), line.clone())),
                    );
                    files.insert(path, built.entry);
                }
                PatchEdit::WriteBytes {
                    path,
                    bytes_hex,
                    executable,
                } => {
                    let path = normalize_relative_path(&path)?;
                    let bytes = hex::decode(bytes_hex).map_err(|err| {
                        Error::PatchRejected(format!("invalid bytes_hex for `{path}`: {err}"))
                    })?;
                    let previous = files.get(&path);
                    let built = self.build_file_entry(
                        &path,
                        bytes,
                        executable,
                        &change_id,
                        previous,
                        &mut file_seq,
                        &mut line_seq,
                    )?;
                    manual_line_changes.extend(
                        built
                            .line_changes
                            .iter()
                            .map(|line| (path.clone(), built.entry.file_id.clone(), line.clone())),
                    );
                    files.insert(path, built.entry);
                }
                PatchEdit::ReplaceLine {
                    path,
                    line_id,
                    expected_text,
                    new_text,
                } => {
                    let path = normalize_relative_path(&path)?;
                    let Some(entry) = files.get(&path).cloned() else {
                        return Err(Error::PatchRejected(format!(
                            "replace_line path `{path}` is absent"
                        )));
                    };
                    let FileContentRef::Text(text_id) = &entry.content else {
                        return Err(Error::PatchRejected(format!(
                            "replace_line path `{path}` is not text"
                        )));
                    };
                    let mut lines = self.load_text_lines(text_id)?;
                    let Some(line_idx) =
                        lines.iter().position(|line| line.line_id_key() == line_id)
                    else {
                        return Err(Error::PatchRejected(format!(
                            "replace_line line_id `{line_id}` not found in `{path}`"
                        )));
                    };
                    if let Some(expected_text) = expected_text {
                        let actual = String::from_utf8_lossy(&lines[line_idx].text);
                        if actual != expected_text {
                            return Err(Error::PatchRejected(format!(
                                "replace_line expected text mismatch for `{path}` {line_id}"
                            )));
                        }
                    }
                    let before_hash = lines[line_idx].text_hash.clone();
                    lines[line_idx].text = new_text.into_bytes();
                    lines[line_idx].text_hash = sha256_hex(&lines[line_idx].text);
                    lines[line_idx].last_content_change = change_id.clone();
                    let text_id = self.put_text_content_from_lines(&lines)?;
                    let bytes = materialize_lines(&lines);
                    let mut next_entry = entry.clone();
                    next_entry.content = FileContentRef::Text(text_id);
                    next_entry.size_bytes = bytes.len() as u64;
                    next_entry.content_hash = sha256_hex(&bytes);
                    next_entry.last_content_change = change_id.clone();
                    manual_line_changes.push((
                        path.clone(),
                        next_entry.file_id.clone(),
                        LineChange {
                            line_id: lines[line_idx].line_id.clone(),
                            kind: LineChangeKind::Modified,
                            old_line_number: Some(line_idx as u64 + 1),
                            new_line_number: Some(line_idx as u64 + 1),
                            before_hash: Some(before_hash),
                            after_hash: Some(lines[line_idx].text_hash.clone()),
                        },
                    ));
                    files.insert(path, next_entry);
                }
                PatchEdit::Delete { path } => {
                    let path = normalize_relative_path(&path)?;
                    if files.remove(&path).is_none() {
                        return Err(Error::PatchRejected(format!(
                            "delete path `{path}` is absent"
                        )));
                    }
                }
                PatchEdit::Rename { from, to } => {
                    let from = normalize_relative_path(&from)?;
                    let to = normalize_relative_path(&to)?;
                    if files.contains_key(&to) {
                        return Err(Error::PatchRejected(format!(
                            "rename destination `{to}` already exists"
                        )));
                    }
                    let Some(mut entry) = files.remove(&from) else {
                        return Err(Error::PatchRejected(format!(
                            "rename source `{from}` is absent"
                        )));
                    };
                    entry.last_path_change = Some(change_id.clone());
                    files.insert(to, entry);
                }
            }
        }

        let built = self.build_root_from_file_entries(files, &change_id)?;
        let mut diff = self.diff_file_maps(&previous_files, &built.files)?;
        for (path, file_id, line) in manual_line_changes {
            if let Some(change) = diff
                .changes
                .iter_mut()
                .find(|change| change.path == path && change.file_id.as_ref() == Some(&file_id))
            {
                if !change
                    .line_changes
                    .iter()
                    .any(|existing| existing.line_id == line.line_id)
                {
                    change.line_changes.push(line);
                }
            }
        }
        if diff.changes.is_empty() {
            return Err(Error::PatchRejected(
                "patch produced no changes".to_string(),
            ));
        }

        let patch_message = patch.message.as_deref().map(redact_sensitive_text);
        let patch_session_id = if let Some(turn) = api_turn {
            if patch.session_id.is_some() && patch.session_id != turn.session_id {
                return Err(Error::InvalidInput(format!(
                    "patch session does not match turn `{}`",
                    turn.turn_id
                )));
            }
            turn.session_id.clone()
        } else {
            patch.session_id.clone().or(agent_row.session_id.clone())
        };
        if let Some(session_id) = &patch_session_id {
            self.ensure_agent_session(&agent_row.agent_id, session_id, None)?;
        }
        let turn_id = if let Some(turn) = api_turn {
            turn.turn_id.clone()
        } else {
            self.open_agent_turn(
                &agent_row.agent_id,
                patch_session_id.as_deref(),
                &agent_row.base_change,
                &head.change_id,
                Some(&serde_json::json!({
                    "kind": "patch",
                    "path_count": diff.summaries.len()
                })),
            )?
        };
        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind: OperationKind::AgentPatch,
            parents: vec![head.change_id.clone()],
            before_root: Some(head.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: ref_name.clone(),
            actor,
            session_id: patch_session_id.clone(),
            message: patch_message.clone(),
            changes: diff.changes,
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&head, &change_id, &built.root_id, &operation_id)?;
        let message_id = if let Some(message) = patch_message {
            Some(self.store_message(
                "agent",
                &message,
                Some(&agent_row.agent_id),
                patch_session_id.as_deref(),
                Some(&change_id),
                operation.created_at,
            )?)
        } else {
            None
        };
        self.insert_agent_event_with_context(
                &agent_row.agent_id,
                patch_session_id.as_deref(),
                Some(&turn_id),
                "patch_applied",
            Some(&change_id),
            message_id.as_ref(),
                &serde_json::json!({
                    "ref_name": ref_name.clone(),
                    "root_id": built.root_id.0.clone(),
                    "session_id": patch_session_id.clone(),
                    "allow_ignored": patch.allow_ignored,
                    "changed_paths": diff.summaries.iter().map(|item| item.path.clone()).collect::<Vec<_>>()
                }),
            )?;
        self.conn.execute(
            "UPDATE agent_branches SET head_change = ?1, head_root = ?2, session_id = COALESCE(?3, session_id), updated_at = ?4 \
             WHERE agent_id = ?5",
            params![
                change_id.0,
                built.root_id.0,
                patch_session_id,
                now_ts(),
                agent_row.agent_id
            ],
        )?;
        if let Some(workdir) = agent_row.workdir {
            let previous = self.load_root_files(&head.root_id)?;
            materialize_into(
                &self.workspace_root,
                Path::new(&workdir),
                &previous,
                &built.files,
                |entry| self.materialize_entry_bytes(entry),
            )?;
        }
        if api_turn.is_some() {
            self.update_agent_turn_progress(&turn_id, "patch_applied", Some(&change_id))?;
        } else {
            self.finish_agent_turn(&turn_id, "patch_applied", Some(&change_id))?;
        }
        Ok(AgentPatchReport {
            agent_id: agent_row.agent_id,
            operation: change_id,
            root_id: built.root_id,
            changed_paths: diff.summaries,
        })
    }

    pub(crate) fn ensure_patch_edit_allowed(
        &self,
        edit: &PatchEdit,
        allow_ignored: bool,
    ) -> Result<()> {
        match edit {
            PatchEdit::Write { path, .. }
            | PatchEdit::WriteBytes { path, .. }
            | PatchEdit::ReplaceLine { path, .. }
            | PatchEdit::Delete { path } => {
                let path = normalize_relative_path(path)?;
                self.ensure_patch_path_allowed(&path, allow_ignored)
            }
            PatchEdit::Rename { from, to } => {
                let from = normalize_relative_path(from)?;
                let to = normalize_relative_path(to)?;
                self.ensure_patch_path_allowed(&from, allow_ignored)?;
                self.ensure_patch_path_allowed(&to, allow_ignored)
            }
        }
    }

    pub(crate) fn ensure_patch_path_allowed(&self, path: &str, allow_ignored: bool) -> Result<()> {
        if is_internal_path(path) {
            return Err(Error::IgnoredPath(path.to_string()));
        }
        if allow_ignored {
            return Ok(());
        }
        let report = self.ignore_check(path)?;
        if report.ignored {
            return Err(Error::IgnoredPath(path.to_string()));
        }
        Ok(())
    }

    pub fn diff_agent(&self, agent: &str, patches: bool) -> Result<DiffSummary> {
        self.diff_agent_with_options(agent, patches, false)
    }

    pub fn diff_agent_with_options(
        &self,
        agent: &str,
        patches: bool,
        line_changes: bool,
    ) -> Result<DiffSummary> {
        let agent_branch = self.agent_branch(agent)?;
        let source = self.get_ref(&agent_branch.ref_name)?;
        let base = self.ref_from_change(&agent_branch.base_change)?;
        let left_files = self.load_root_files(&base.root_id)?;
        let right_files = self.load_root_files(&source.root_id)?;
        self.diff_files(
            agent_branch.base_change.0,
            source.change_id.0,
            &left_files,
            &right_files,
            patches,
            line_changes,
        )
    }
}
