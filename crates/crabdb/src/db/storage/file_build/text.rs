use super::*;

impl CrabDb {
    pub(crate) fn build_text_content(
        &self,
        bytes: &[u8],
        change_id: &ChangeId,
        previous: Option<&[LineEntry]>,
        line_seq: &mut u64,
        similarity_threshold: f32,
    ) -> Result<TextBuildResult> {
        let new_lines = split_lines(bytes);
        let previous = previous.unwrap_or(&[]);
        let new_hashes = new_lines
            .iter()
            .map(|line| sha256_hex(&line.text))
            .collect::<Vec<_>>();
        let mut previous_by_hash: HashMap<String, Vec<usize>> = HashMap::new();
        for (idx, old) in previous.iter().enumerate() {
            previous_by_hash
                .entry(old.text_hash.clone())
                .or_default()
                .push(idx);
        }
        let mut future_hash_counts: HashMap<String, usize> = HashMap::new();
        for hash in &new_hashes {
            *future_hash_counts.entry(hash.clone()).or_default() += 1;
        }
        let mut used_old = HashSet::new();
        let mut entries = Vec::with_capacity(new_lines.len());

        for (idx, line) in new_lines.iter().enumerate() {
            let text_hash = new_hashes[idx].clone();
            decrement_hash_count(&mut future_hash_counts, &text_hash);
            let mut matched_idx = None;
            if let Some(old) = previous.get(idx) {
                if old.text_hash == text_hash && !used_old.contains(&idx) {
                    matched_idx = Some(idx);
                }
            }
            if matched_idx.is_none() {
                matched_idx = previous_by_hash.get(&text_hash).and_then(|candidates| {
                    candidates
                        .iter()
                        .copied()
                        .find(|old_idx| !used_old.contains(old_idx))
                });
            }
            if matched_idx.is_none() {
                matched_idx = previous
                    .iter()
                    .enumerate()
                    .find(|(old_idx, old)| {
                        !used_old.contains(old_idx)
                            && line_similarity(&old.text, &line.text) >= similarity_threshold
                    })
                    .map(|(old_idx, _)| old_idx);
            }
            if matched_idx.is_none() {
                if let Some(old) = previous.get(idx) {
                    let old_has_future_match = future_hash_counts
                        .get(&old.text_hash)
                        .is_some_and(|count| *count > 0)
                        || new_lines.iter().skip(idx + 1).any(|future| {
                            line_similarity(&old.text, &future.text) >= similarity_threshold
                        });
                    if !used_old.contains(&idx) && !old_has_future_match {
                        matched_idx = Some(idx);
                    }
                }
            }
            let entry = if let Some(old_idx) = matched_idx {
                used_old.insert(old_idx);
                let old = &previous[old_idx];
                LineEntry {
                    line_id: old.line_id.clone(),
                    text: line.text.clone(),
                    newline: line.newline,
                    text_hash,
                    introduced_by: old.introduced_by.clone(),
                    last_content_change: if old.text == line.text && old.newline == line.newline {
                        old.last_content_change.clone()
                    } else {
                        change_id.clone()
                    },
                    last_move_change: if old_idx == idx {
                        old.last_move_change.clone()
                    } else {
                        Some(change_id.clone())
                    },
                    flags: old.flags.clone(),
                }
            } else {
                let line_id = LineId::new(change_id.clone(), *line_seq);
                *line_seq += 1;
                LineEntry {
                    line_id,
                    text: line.text.clone(),
                    newline: line.newline,
                    text_hash,
                    introduced_by: change_id.clone(),
                    last_content_change: change_id.clone(),
                    last_move_change: None,
                    flags: LineFlags::default(),
                }
            };
            entries.push(entry);
        }

        let old_positions = previous
            .iter()
            .enumerate()
            .map(|(idx, line)| (line.line_id.clone(), (idx, line)))
            .collect::<HashMap<_, _>>();
        let new_positions = entries
            .iter()
            .enumerate()
            .map(|(idx, line)| (line.line_id.clone(), (idx, line)))
            .collect::<HashMap<_, _>>();
        let mut line_changes = Vec::new();
        for (line_id, (new_idx, new_line)) in &new_positions {
            if let Some((old_idx, old_line)) = old_positions.get(line_id) {
                if old_line.text_hash != new_line.text_hash || old_line.newline != new_line.newline
                {
                    line_changes.push(LineChange {
                        line_id: line_id.clone(),
                        kind: LineChangeKind::Modified,
                        old_line_number: Some(*old_idx as u64 + 1),
                        new_line_number: Some(*new_idx as u64 + 1),
                        before_hash: Some(old_line.text_hash.clone()),
                        after_hash: Some(new_line.text_hash.clone()),
                    });
                } else if old_idx != new_idx {
                    line_changes.push(LineChange {
                        line_id: line_id.clone(),
                        kind: LineChangeKind::Moved,
                        old_line_number: Some(*old_idx as u64 + 1),
                        new_line_number: Some(*new_idx as u64 + 1),
                        before_hash: Some(old_line.text_hash.clone()),
                        after_hash: Some(new_line.text_hash.clone()),
                    });
                }
            } else {
                line_changes.push(LineChange {
                    line_id: line_id.clone(),
                    kind: LineChangeKind::Added,
                    old_line_number: None,
                    new_line_number: Some(*new_idx as u64 + 1),
                    before_hash: None,
                    after_hash: Some(new_line.text_hash.clone()),
                });
            }
        }
        for (line_id, (old_idx, old_line)) in old_positions {
            if !new_positions.contains_key(&line_id) {
                line_changes.push(LineChange {
                    line_id,
                    kind: LineChangeKind::Deleted,
                    old_line_number: Some(old_idx as u64 + 1),
                    new_line_number: None,
                    before_hash: Some(old_line.text_hash.clone()),
                    after_hash: None,
                });
            }
        }
        line_changes.sort_by_key(|change| {
            (
                change
                    .new_line_number
                    .or(change.old_line_number)
                    .unwrap_or(u64::MAX),
                change.line_id.local_seq,
            )
        });

        let mut order_builder = BatchBuilder::new(self.store.clone(), prolly_config());
        let mut index_builder = BatchBuilder::new(self.store.clone(), prolly_config());
        for (idx, entry) in entries.iter().enumerate() {
            let key = order_key(idx as u64 + 1);
            order_builder.add(key.clone(), cbor(entry)?);
            index_builder.add(entry.line_id.encode_key(), key);
        }
        let order_tree = order_builder.build()?;
        let index_tree = index_builder.build()?;
        let content = TextContent {
            version: TEXT_OBJECT_VERSION,
            content_hash: sha256_hex(bytes),
            line_count: entries.len() as u64,
            byte_count: bytes.len() as u64,
            order_map_root: tree_root_hex(&order_tree),
            line_index_map_root: tree_root_hex(&index_tree),
            representation: TextRepresentation::TreeText,
        };
        let object_id = self.put_object(TEXT_CONTENT_KIND, TEXT_OBJECT_VERSION, &content)?;
        Ok(TextBuildResult {
            object_id,
            line_changes,
        })
    }
}

fn decrement_hash_count(counts: &mut HashMap<String, usize>, hash: &str) {
    let should_remove = if let Some(count) = counts.get_mut(hash) {
        *count = count.saturating_sub(1);
        *count == 0
    } else {
        false
    };
    if should_remove {
        counts.remove(hash);
    }
}
