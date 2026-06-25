use super::*;

impl CrabDb {
    pub(crate) fn build_file_entry(
        &self,
        path: &str,
        bytes: Vec<u8>,
        content_hash: String,
        executable: bool,
        change_id: &ChangeId,
        previous: Option<&FileEntry>,
        file_seq: &mut u64,
        line_seq: &mut u64,
    ) -> Result<FileBuildResult> {
        let file_id = previous
            .map(|entry| entry.file_id.clone())
            .unwrap_or_else(|| {
                let id = FileId::new(change_id.clone(), *file_seq);
                *file_seq += 1;
                id
            });
        let created_by = previous
            .map(|entry| entry.created_by.clone())
            .unwrap_or_else(|| change_id.clone());
        let previous_text = previous.and_then(|entry| match &entry.content {
            FileContentRef::Text(text_id) => self.load_text_lines(text_id).ok(),
            _ => None,
        });
        let should_store_small_text = self.config.text.small_text_max_bytes > 0
            && bytes.len() as u64 <= self.config.text.small_text_max_bytes;
        let below_tree_text_threshold = (bytes.len() as u64) < self.config.text.tree_text_min_bytes;
        let should_store_lazy_text =
            below_tree_text_threshold && !should_store_small_text && previous_text.is_none();
        let (kind, content, line_changes) = if looks_binary(&bytes) {
            let blob_id = self.put_blob(bytes.clone())?;
            (
                FileKind::Binary,
                FileContentRef::Binary(blob_id),
                Vec::new(),
            )
        } else if std::str::from_utf8(&bytes).is_err() {
            let blob_id = self.put_blob(bytes.clone())?;
            (
                FileKind::OpaqueText,
                FileContentRef::Opaque(blob_id),
                Vec::new(),
            )
        } else if bytes.len() as u64 > self.config.text.opaque_text_max_bytes {
            let blob_id = self.put_blob(bytes.clone())?;
            (
                FileKind::OpaqueText,
                FileContentRef::Opaque(blob_id),
                Vec::new(),
            )
        } else if max_line_len(&bytes) as u64 > self.config.text.max_line_bytes {
            let blob_id = self.put_blob(bytes.clone())?;
            (
                FileKind::OpaqueText,
                FileContentRef::Opaque(blob_id),
                Vec::new(),
            )
        } else {
            let built_text = self.build_text_content(
                &bytes,
                change_id,
                previous_text.as_deref(),
                line_seq,
                self.config.text.preserve_similarity,
                should_store_small_text,
                should_store_lazy_text,
            )?;
            (
                FileKind::Text,
                FileContentRef::Text(built_text.object_id),
                built_text.line_changes,
            )
        };
        let last_content_change =
            if previous.is_some_and(|entry| entry.content_hash == content_hash) {
                previous
                    .map(|entry| entry.last_content_change.clone())
                    .unwrap_or_else(|| change_id.clone())
            } else {
                change_id.clone()
            };
        let entry = FileEntry {
            file_id,
            kind,
            mode: if executable { 0o755 } else { 0o644 },
            executable,
            content,
            size_bytes: bytes.len() as u64,
            content_hash,
            created_by,
            last_content_change,
            last_path_change: previous.and_then(|entry| entry.last_path_change.clone()),
        };
        let _ = path;
        let disk_manifest = DiskManifest {
            kind: entry.kind.clone(),
            executable: entry.executable,
            content_hash: entry.content_hash.clone(),
        };
        Ok(FileBuildResult {
            entry,
            line_changes,
            disk_manifest,
        })
    }
}
