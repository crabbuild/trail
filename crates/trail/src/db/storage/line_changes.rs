use super::*;

impl Trail {
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
}
