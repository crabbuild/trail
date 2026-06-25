use super::*;

const MATERIALIZE_BATCH_FILES: usize = 1024;

impl CrabDb {
    pub(crate) fn load_root_files_for_paths(
        &self,
        root_id: &ObjectId,
        paths: &[String],
    ) -> Result<BTreeMap<String, FileEntry>> {
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        let tree = worktree_root_map_tree_from_root_hex(root.path_map_root.as_deref())?;
        let mut out = BTreeMap::new();
        for path in paths {
            let path = normalize_relative_path(path)?;
            if let Some(value) = self.root_prolly.get(&tree, path.as_bytes())? {
                out.insert(path, from_cbor(&value)?);
            }
        }
        Ok(out)
    }

    pub(crate) fn load_root_files_for_selections(
        &self,
        root_id: &ObjectId,
        selections: &[String],
    ) -> Result<BTreeMap<String, FileEntry>> {
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        let tree = worktree_root_map_tree_from_root_hex(root.path_map_root.as_deref())?;
        let mut out = BTreeMap::new();
        for selection in selections {
            let selection = normalize_relative_path(selection)?;
            if let Some(value) = self.root_prolly.get(&tree, selection.as_bytes())? {
                out.insert(selection.clone(), from_cbor(&value)?);
            }

            let prefix = format!("{selection}/");
            let end = prefix_upper_bound(prefix.as_bytes());
            let iter = self
                .root_prolly
                .range(&tree, prefix.as_bytes(), end.as_deref())?;
            for item in iter {
                let (key, value) = item?;
                let path = String::from_utf8(key)
                    .map_err(|err| Error::Corrupt(format!("non UTF-8 path key: {err}")))?;
                if path_matches_selection(&path, &selection) {
                    out.insert(path, from_cbor(&value)?);
                }
            }
        }
        Ok(out)
    }

    pub(crate) fn load_root_files_for_selections_with_neighbors(
        &self,
        root_id: &ObjectId,
        selections: &[String],
    ) -> Result<BTreeMap<String, FileEntry>> {
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        let tree = worktree_root_map_tree_from_root_hex(root.path_map_root.as_deref())?;
        let mut out = BTreeMap::new();
        for selection in selections {
            let selection = normalize_relative_path(selection)?;
            let exact = self.root_prolly.get(&tree, selection.as_bytes())?;
            if let Some(value) = exact {
                let entry: FileEntry = from_cbor(&value)?;
                self.load_dependency_neighbors_for_file(&tree, &selection, &entry, &mut out)?;
                out.insert(selection.clone(), entry);
                if let Some(parent_prefix) = parent_directory_prefix(&selection) {
                    self.load_root_files_for_prefix(&tree, &parent_prefix, &mut out)?;
                }
            }

            let prefix = format!("{selection}/");
            self.load_root_files_for_prefix(&tree, &prefix, &mut out)?;
        }
        Ok(out)
    }

    fn load_dependency_neighbors_for_file(
        &self,
        tree: &Tree,
        source_path: &str,
        source_entry: &FileEntry,
        out: &mut BTreeMap<String, FileEntry>,
    ) -> Result<()> {
        let bytes = self.materialize_entry_bytes(source_entry)?;
        let Ok(source) = std::str::from_utf8(&bytes) else {
            return Ok(());
        };
        for candidate in dependency_neighbor_candidates(source_path, source)? {
            if out.contains_key(&candidate) {
                continue;
            }
            if let Some(value) = self.root_prolly.get(tree, candidate.as_bytes())? {
                out.insert(candidate, from_cbor(&value)?);
            }
        }
        Ok(())
    }

    fn load_root_files_for_prefix(
        &self,
        tree: &Tree,
        prefix: &str,
        out: &mut BTreeMap<String, FileEntry>,
    ) -> Result<()> {
        let end = prefix_upper_bound(prefix.as_bytes());
        let iter = self
            .root_prolly
            .range(tree, prefix.as_bytes(), end.as_deref())?;
        for item in iter {
            let (key, value) = item?;
            let path = String::from_utf8(key)
                .map_err(|err| Error::Corrupt(format!("non UTF-8 path key: {err}")))?;
            out.insert(path, from_cbor(&value)?);
        }
        Ok(())
    }

    pub(crate) fn load_root_files(
        &self,
        root_id: &ObjectId,
    ) -> Result<BTreeMap<String, FileEntry>> {
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        let tree = worktree_root_map_tree_from_root_hex(root.path_map_root.as_deref())?;
        let iter = self.root_prolly.range(&tree, &[], None)?;
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
        self.load_text_content_lines(text_id, content)
    }

    fn load_text_content_lines(
        &self,
        text_id: &ObjectId,
        content: TextContent,
    ) -> Result<Vec<LineEntry>> {
        match content.representation {
            TextRepresentation::SmallText { lines } => Ok(lines),
            TextRepresentation::SmallTextTable { table } => decode_small_text_table(&table),
            TextRepresentation::TreeText => {
                let tree = tree_from_root_hex(content.order_map_root.as_deref())?;
                let iter = self.prolly.range(&tree, &[], None)?;
                let mut out = Vec::new();
                for item in iter {
                    let (_, value) = item?;
                    out.push(from_cbor(&value)?);
                }
                Ok(out)
            }
            TextRepresentation::OpaqueText { blob_id, .. } => {
                let blob: Blob = self.get_object(BLOB_KIND, &blob_id)?;
                Ok(split_lines(&blob.bytes)
                    .into_iter()
                    .enumerate()
                    .map(|(idx, line)| {
                        let line_id = LineId::new(ChangeId(text_id.0.clone()), idx as u64 + 1);
                        let text_hash = sha256_hex(&line.text);
                        LineEntry {
                            line_id,
                            text: line.text,
                            newline: line.newline,
                            text_hash,
                            introduced_by: ChangeId(text_id.0.clone()),
                            last_content_change: ChangeId(text_id.0.clone()),
                            last_move_change: None,
                            flags: LineFlags::default(),
                        }
                    })
                    .collect())
            }
        }
    }

    pub(crate) fn load_text_bytes(&self, text_id: &ObjectId) -> Result<Vec<u8>> {
        let content: TextContent = self.get_object(TEXT_CONTENT_KIND, text_id)?;
        if let Some(blob_id) = &content.full_bytes_blob_id {
            let blob: Blob = self.get_object(BLOB_KIND, blob_id)?;
            validate_full_text_blob(&content, &blob)?;
            return Ok(blob.bytes);
        }
        let lines = self.load_text_content_lines(text_id, content)?;
        Ok(materialize_lines(&lines))
    }

    pub(crate) fn materialize_entry_bytes(&self, entry: &FileEntry) -> Result<Vec<u8>> {
        match &entry.content {
            FileContentRef::Text(text_id) => self.load_text_bytes(text_id),
            FileContentRef::Opaque(blob_id) | FileContentRef::Binary(blob_id) => {
                let blob: Blob = self.get_object(BLOB_KIND, blob_id)?;
                Ok(blob.bytes)
            }
        }
    }

    pub(crate) fn materialize_entries_bytes(
        &self,
        entries: &BTreeMap<String, FileEntry>,
    ) -> Result<BTreeMap<String, Vec<u8>>> {
        let mut text_ids = Vec::new();
        let mut blob_ids = Vec::new();
        for entry in entries.values() {
            match &entry.content {
                FileContentRef::Text(text_id) => text_ids.push(text_id.clone()),
                FileContentRef::Opaque(blob_id) | FileContentRef::Binary(blob_id) => {
                    blob_ids.push(blob_id.clone());
                }
            }
        }

        let text_contents: HashMap<ObjectId, TextContent> =
            self.get_objects(TEXT_CONTENT_KIND, &text_ids)?;
        for content in text_contents.values() {
            if let Some(blob_id) = &content.full_bytes_blob_id {
                blob_ids.push(blob_id.clone());
            } else if let TextRepresentation::OpaqueText { blob_id, .. } = &content.representation {
                blob_ids.push(blob_id.clone());
            }
        }
        let blobs: HashMap<ObjectId, Blob> = self.get_objects(BLOB_KIND, &blob_ids)?;

        let mut out = BTreeMap::new();
        for (path, entry) in entries {
            let bytes = match &entry.content {
                FileContentRef::Text(text_id) => {
                    let content =
                        text_contents
                            .get(text_id)
                            .ok_or_else(|| Error::ObjectNotFound {
                                kind: TEXT_CONTENT_KIND,
                                id: text_id.0.clone(),
                            })?;
                    self.materialize_loaded_text_bytes(text_id, content, &blobs)?
                }
                FileContentRef::Opaque(blob_id) | FileContentRef::Binary(blob_id) => blobs
                    .get(blob_id)
                    .ok_or_else(|| Error::ObjectNotFound {
                        kind: BLOB_KIND,
                        id: blob_id.0.clone(),
                    })?
                    .bytes
                    .clone(),
            };
            out.insert(path.clone(), bytes);
        }
        Ok(out)
    }

    fn materialize_loaded_text_bytes(
        &self,
        text_id: &ObjectId,
        content: &TextContent,
        blobs: &HashMap<ObjectId, Blob>,
    ) -> Result<Vec<u8>> {
        if let Some(blob_id) = &content.full_bytes_blob_id {
            let blob = blobs.get(blob_id).ok_or_else(|| Error::ObjectNotFound {
                kind: BLOB_KIND,
                id: blob_id.0.clone(),
            })?;
            validate_full_text_blob(content, blob)?;
            return Ok(blob.bytes.clone());
        }
        match &content.representation {
            TextRepresentation::SmallText { lines } => Ok(materialize_lines(lines)),
            TextRepresentation::SmallTextTable { table } => {
                Ok(materialize_lines(&decode_small_text_table(table)?))
            }
            TextRepresentation::TreeText => self.load_text_bytes(text_id),
            TextRepresentation::OpaqueText { blob_id, .. } => {
                let blob = blobs.get(blob_id).ok_or_else(|| Error::ObjectNotFound {
                    kind: BLOB_KIND,
                    id: blob_id.0.clone(),
                })?;
                validate_full_text_blob(content, blob)?;
                Ok(blob.bytes.clone())
            }
        }
    }

    pub(crate) fn materialize_files(
        &self,
        previous: &BTreeMap<String, FileEntry>,
        target: &BTreeMap<String, FileEntry>,
    ) -> Result<()> {
        self.materialize_files_at(&self.workspace_root, previous, target)
    }

    pub(crate) fn materialize_files_at(
        &self,
        output_root: &Path,
        previous: &BTreeMap<String, FileEntry>,
        target: &BTreeMap<String, FileEntry>,
    ) -> Result<()> {
        materialize_into_batched(
            &self.workspace_root,
            output_root,
            previous,
            target,
            MATERIALIZE_BATCH_FILES,
            |batch| self.materialize_entries_bytes(batch),
        )
    }

    pub(crate) fn materialize_files_best_effort_at(
        &self,
        output_root: &Path,
        previous: &BTreeMap<String, FileEntry>,
        target: &BTreeMap<String, FileEntry>,
    ) -> Result<()> {
        materialize_into_batched_best_effort(
            &self.workspace_root,
            output_root,
            previous,
            target,
            MATERIALIZE_BATCH_FILES,
            |batch| self.materialize_entries_bytes(batch),
        )
    }
}

fn prefix_upper_bound(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut end = prefix.to_vec();
    for idx in (0..end.len()).rev() {
        if end[idx] != u8::MAX {
            end[idx] += 1;
            end.truncate(idx + 1);
            return Some(end);
        }
    }
    None
}

fn parent_directory_prefix(path: &str) -> Option<String> {
    path.rsplit_once('/')
        .map(|(parent, _)| format!("{parent}/"))
}

fn dependency_neighbor_candidates(source_path: &str, source: &str) -> Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    for line in source.lines() {
        for spec in quoted_strings(line) {
            if spec.starts_with('.') {
                add_relative_dependency_candidates(source_path, &spec, &mut out);
            }
        }
        if let Some(module) = rust_module_declaration(line) {
            add_rust_module_candidates(source_path, &module, &mut out);
        }
    }
    Ok(out)
}

fn quoted_strings(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let mut idx = 0;
    while idx < bytes.len() {
        let quote = bytes[idx];
        if quote != b'\'' && quote != b'"' {
            idx += 1;
            continue;
        }
        let start = idx + 1;
        idx = start;
        while idx < bytes.len() {
            if bytes[idx] == b'\\' {
                idx += 2;
                continue;
            }
            if bytes[idx] == quote {
                out.push(String::from_utf8_lossy(&bytes[start..idx]).to_string());
                idx += 1;
                break;
            }
            idx += 1;
        }
    }
    out
}

fn rust_module_declaration(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let trimmed = trimmed.strip_prefix("pub ").unwrap_or(trimmed);
    let rest = trimmed.strip_prefix("mod ")?;
    let module = rest.strip_suffix(';')?.trim();
    if module
        .chars()
        .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
    {
        Some(module.to_string())
    } else {
        None
    }
}

fn add_rust_module_candidates(source_path: &str, module: &str, out: &mut BTreeSet<String>) {
    let Some(parent) = source_parent(source_path) else {
        return;
    };
    push_dependency_candidate(out, &format!("{parent}/{module}.rs"));
    push_dependency_candidate(out, &format!("{parent}/{module}/mod.rs"));
}

fn add_relative_dependency_candidates(source_path: &str, spec: &str, out: &mut BTreeSet<String>) {
    let Some(base) = resolve_relative_dependency_path(source_path, spec) else {
        return;
    };
    push_dependency_candidate(out, &base);
    if path_has_extension(&base) {
        return;
    }
    for extension in ["rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py"] {
        push_dependency_candidate(out, &format!("{base}.{extension}"));
    }
    for index in ["mod.rs", "index.ts", "index.tsx", "index.js", "__init__.py"] {
        push_dependency_candidate(out, &format!("{base}/{index}"));
    }
}

fn resolve_relative_dependency_path(source_path: &str, spec: &str) -> Option<String> {
    let mut parts = source_parent(source_path)
        .map(|parent| parent.split('/').map(str::to_string).collect::<Vec<_>>())
        .unwrap_or_default();
    for part in spec.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part.to_string()),
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

fn source_parent(source_path: &str) -> Option<&str> {
    source_path.rsplit_once('/').map(|(parent, _)| parent)
}

fn path_has_extension(path: &str) -> bool {
    path.rsplit_once('/')
        .map(|(_, leaf)| leaf)
        .unwrap_or(path)
        .rsplit_once('.')
        .is_some_and(|(_, extension)| !extension.is_empty())
}

fn push_dependency_candidate(out: &mut BTreeSet<String>, path: &str) {
    if let Ok(path) = normalize_relative_path(path) {
        out.insert(path);
    }
}

fn validate_full_text_blob(content: &TextContent, blob: &Blob) -> Result<()> {
    if blob.content_hash != content.content_hash {
        return Err(Error::Corrupt(format!(
            "full text blob hash {} does not match text content hash {}",
            blob.content_hash, content.content_hash
        )));
    }
    if blob.bytes.len() as u64 != content.byte_count {
        return Err(Error::Corrupt(format!(
            "full text blob byte_count {} does not match text content byte_count {}",
            blob.bytes.len(),
            content.byte_count
        )));
    }
    Ok(())
}
