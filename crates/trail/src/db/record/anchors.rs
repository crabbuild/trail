use super::*;

impl Trail {
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
