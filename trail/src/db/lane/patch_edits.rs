use super::*;

pub(crate) type ManualLineChange = (String, FileId, LineChange);

impl Trail {
    pub(crate) fn preflight_replace_line_batch(
        &self,
        edits: &[PatchEdit],
        files: &BTreeMap<String, FileEntry>,
    ) -> Result<()> {
        if !edits
            .iter()
            .all(|edit| matches!(edit, PatchEdit::ReplaceLine { .. }))
        {
            return Ok(());
        }

        let mut lines_by_path: BTreeMap<String, Vec<LineEntry>> = BTreeMap::new();
        for edit in edits {
            let PatchEdit::ReplaceLine {
                path,
                line_id,
                expected_text,
                new_text,
            } = edit
            else {
                continue;
            };
            let path = normalize_relative_path(path)?;
            if !lines_by_path.contains_key(&path) {
                let Some(entry) = files.get(&path) else {
                    return Err(Error::PatchRejected(format!(
                        "replace_line path `{path}` is absent"
                    )));
                };
                let FileContentRef::Text(text_id) = &entry.content else {
                    return Err(Error::PatchRejected(format!(
                        "replace_line path `{path}` is not text"
                    )));
                };
                lines_by_path.insert(path.clone(), self.load_text_lines(text_id)?);
            }
            let lines = lines_by_path.get_mut(&path).expect("path was loaded");
            let parsed_line_id = parse_line_id_key(line_id)
                .map_err(|error| Error::PatchRejected(error.to_string()))?;
            let storage_line_id = line_id_key_value(&parsed_line_id);
            let Some(line_idx) = lines
                .iter()
                .position(|line| line.line_id_key() == storage_line_id)
            else {
                return Err(Error::PatchRejected(format!(
                    "replace_line line_id `{line_id}` not found in `{path}`"
                )));
            };
            let Some(expected_text) = expected_text else {
                return Err(Error::PatchRejected(format!(
                    "replace_line for `{path}` requires expected_text; include the current line text so stale edits are rejected"
                )));
            };
            let actual = String::from_utf8_lossy(&lines[line_idx].text);
            if actual != *expected_text {
                return Err(Error::PatchRejected(format!(
                    "replace_line expected text mismatch for `{path}` {line_id}"
                )));
            }
            lines[line_idx].text = new_text.as_bytes().to_vec();
            lines[line_idx].text_hash = sha256_hex(&lines[line_idx].text);
        }
        Ok(())
    }

    pub(crate) fn patch_touched_paths(&self, edits: &[PatchEdit]) -> Result<Vec<String>> {
        let mut paths = BTreeSet::new();
        for edit in edits {
            match edit {
                PatchEdit::Write { path, .. }
                | PatchEdit::WriteBytes { path, .. }
                | PatchEdit::ReplaceLine { path, .. }
                | PatchEdit::Delete { path } => {
                    paths.insert(normalize_relative_path(path)?);
                }
                PatchEdit::Rename { from, to } => {
                    paths.insert(normalize_relative_path(from)?);
                    paths.insert(normalize_relative_path(to)?);
                }
            }
        }
        Ok(paths.into_iter().collect())
    }

    pub(crate) fn apply_patch_edit_to_files(
        &self,
        edit: PatchEdit,
        files: &mut BTreeMap<String, FileEntry>,
        change_id: &ChangeId,
        file_seq: &mut u64,
        line_seq: &mut u64,
        manual_line_changes: &mut Vec<ManualLineChange>,
    ) -> Result<()> {
        match edit {
            PatchEdit::Write {
                path,
                content,
                executable,
            } => {
                let path = normalize_relative_path(&path)?;
                let previous = files.get(&path);
                let bytes = content.into_bytes();
                let content_hash = sha256_hex(&bytes);
                let built = self.build_file_entry(
                    &path,
                    bytes,
                    content_hash,
                    executable,
                    change_id,
                    previous,
                    file_seq,
                    line_seq,
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
                let content_hash = sha256_hex(&bytes);
                let built = self.build_file_entry(
                    &path,
                    bytes,
                    content_hash,
                    executable,
                    change_id,
                    previous,
                    file_seq,
                    line_seq,
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
            } => self.apply_replace_line_edit(
                files,
                change_id,
                manual_line_changes,
                path,
                line_id,
                expected_text,
                new_text,
            )?,
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
        Ok(())
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "mirrors the replace-line patch operation schema"
    )]
    fn apply_replace_line_edit(
        &self,
        files: &mut BTreeMap<String, FileEntry>,
        change_id: &ChangeId,
        manual_line_changes: &mut Vec<ManualLineChange>,
        path: String,
        line_id: String,
        expected_text: Option<String>,
        new_text: String,
    ) -> Result<()> {
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
        let parsed_line_id =
            parse_line_id_key(&line_id).map_err(|error| Error::PatchRejected(error.to_string()))?;
        let storage_line_id = line_id_key_value(&parsed_line_id);
        let Some(line_idx) = lines
            .iter()
            .position(|line| line.line_id_key() == storage_line_id)
        else {
            return Err(Error::PatchRejected(format!(
                "replace_line line_id `{line_id}` not found in `{path}`"
            )));
        };
        let Some(expected_text) = expected_text else {
            return Err(Error::PatchRejected(format!(
                "replace_line for `{path}` requires expected_text; include the current line text so stale edits are rejected"
            )));
        };
        let actual = String::from_utf8_lossy(&lines[line_idx].text);
        if actual != expected_text {
            return Err(Error::PatchRejected(format!(
                "replace_line expected text mismatch for `{path}` {line_id}"
            )));
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
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn internal_replace_line_paths_require_expected_text() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "one\ntwo\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let head = db.get_ref("refs/branches/main").unwrap();
        let files = db.load_root_files(&head.root_id).unwrap();
        let why = db.why("README.md:1", Some("main")).unwrap();
        let line_id = why.line_id.alias();
        let edit = PatchEdit::ReplaceLine {
            path: "README.md".to_string(),
            line_id,
            expected_text: None,
            new_text: "uno".to_string(),
        };

        let err = db
            .preflight_replace_line_batch(std::slice::from_ref(&edit), &files)
            .unwrap_err();
        assert!(
            matches!(err, Error::PatchRejected(ref message) if message.contains("requires expected_text")),
            "expected preflight guard rejection, got {err:?}"
        );

        let mut files = files;
        let mut manual_line_changes = Vec::new();
        let mut file_seq = 1;
        let mut line_seq = 1;
        let err = db
            .apply_patch_edit_to_files(
                edit,
                &mut files,
                &ChangeId("change_internal_replace_line_guard".to_string()),
                &mut file_seq,
                &mut line_seq,
                &mut manual_line_changes,
            )
            .unwrap_err();
        assert!(
            matches!(err, Error::PatchRejected(ref message) if message.contains("requires expected_text")),
            "expected applicator guard rejection, got {err:?}"
        );
        assert!(manual_line_changes.is_empty());
    }
}
