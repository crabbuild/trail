use super::*;

const MATERIALIZE_BATCH_FILES: usize = 1024;

impl Trail {
    pub(crate) fn root_file_entry(
        &self,
        root_id: &ObjectId,
        path: &str,
    ) -> Result<Option<FileEntry>> {
        let path = normalize_relative_path(path)?;
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        let tree = root_map_tree_from_root_hex(root.path_map_root.as_deref())?;
        self.root_prolly
            .get(&tree, path.as_bytes())?
            .map(|value| from_cbor(&value))
            .transpose()
    }

    pub(crate) fn root_directory_exists(
        &self,
        root_id: &ObjectId,
        directory: &str,
    ) -> Result<bool> {
        if directory.is_empty() {
            return Ok(true);
        }
        let directory = normalize_relative_path(directory)?;
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        let tree = root_map_tree_from_root_hex(root.path_map_root.as_deref())?;
        let prefix = format!("{directory}/");
        let end = prefix_upper_bound(prefix.as_bytes());
        Ok(self
            .root_prolly
            .range(&tree, prefix.as_bytes(), end.as_deref())?
            .next()
            .transpose()?
            .is_some())
    }

    pub(crate) fn root_immediate_children(
        &self,
        root_id: &ObjectId,
        directory: &str,
    ) -> Result<Vec<RootDirectoryChild>> {
        let directory = if directory.is_empty() {
            String::new()
        } else {
            normalize_relative_path(directory)?
        };
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        let tree = root_map_tree_from_root_hex(root.path_map_root.as_deref())?;
        let prefix = if directory.is_empty() {
            String::new()
        } else {
            format!("{directory}/")
        };
        let end = if prefix.is_empty() {
            None
        } else {
            prefix_upper_bound(prefix.as_bytes())
        };
        let mut cursor = prefix.as_bytes().to_vec();
        let mut children = Vec::new();
        loop {
            let Some(item) = self
                .root_prolly
                .range(&tree, &cursor, end.as_deref())?
                .next()
            else {
                break;
            };
            let (key, value) = item?;
            let path = String::from_utf8(key.clone())
                .map_err(|err| Error::Corrupt(format!("non UTF-8 path key: {err}")))?;
            let Some(remainder) = path.strip_prefix(&prefix) else {
                break;
            };
            if remainder.is_empty() {
                cursor = key;
                cursor.push(0);
                continue;
            }
            let (name, is_directory) = match remainder.split_once('/') {
                Some((name, _)) => (name, true),
                None => (remainder, false),
            };
            let child_path = if directory.is_empty() {
                name.to_string()
            } else {
                format!("{directory}/{name}")
            };
            children.push(RootDirectoryChild {
                name: name.to_string(),
                path: child_path.clone(),
                entry: if is_directory {
                    None
                } else {
                    Some(from_cbor(&value)?)
                },
            });
            if is_directory {
                let child_prefix = format!("{child_path}/");
                let Some(next) = prefix_upper_bound(child_prefix.as_bytes()) else {
                    break;
                };
                cursor = next;
            } else {
                cursor = key;
                cursor.push(0);
            }
        }
        Ok(children)
    }

    pub(crate) fn load_root_files_for_paths(
        &self,
        root_id: &ObjectId,
        paths: &[String],
    ) -> Result<BTreeMap<String, FileEntry>> {
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        let tree = root_map_tree_from_root_hex(root.path_map_root.as_deref())?;
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
        let tree = root_map_tree_from_root_hex(root.path_map_root.as_deref())?;
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
        let tree = root_map_tree_from_root_hex(root.path_map_root.as_deref())?;
        let mut out = BTreeMap::new();
        let mut exact_sources = BTreeMap::new();
        for selection in selections {
            let selection = normalize_relative_path(selection)?;
            let exact = self.root_prolly.get(&tree, selection.as_bytes())?;
            if let Some(value) = exact {
                let entry: FileEntry = from_cbor(&value)?;
                exact_sources.insert(selection.clone(), entry.clone());
                out.insert(selection.clone(), entry);
                if let Some(parent_prefix) = parent_directory_prefix(&selection) {
                    self.load_root_files_for_prefix(&tree, &parent_prefix, &mut out)?;
                }
            }

            let prefix = format!("{selection}/");
            self.load_root_files_for_prefix(&tree, &prefix, &mut out)?;
        }
        self.load_dependency_neighbors_for_files(&tree, &exact_sources, &mut out)?;
        Ok(out)
    }

    fn load_dependency_neighbors_for_files(
        &self,
        tree: &Tree,
        source_entries: &BTreeMap<String, FileEntry>,
        out: &mut BTreeMap<String, FileEntry>,
    ) -> Result<()> {
        let source_bytes = self.materialize_entries_bytes(source_entries)?;
        for (source_path, bytes) in source_bytes {
            let Ok(source) = std::str::from_utf8(&bytes) else {
                continue;
            };
            for candidate in dependency_neighbor_candidates(&source_path, source)? {
                if out.contains_key(&candidate) {
                    continue;
                }
                if let Some(value) = self.root_prolly.get(tree, candidate.as_bytes())? {
                    out.insert(candidate, from_cbor(&value)?);
                }
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
        let mut out = BTreeMap::new();
        self.for_each_root_file_chunk(root_id, MATERIALIZE_BATCH_FILES, |chunk| {
            out.extend(chunk);
            Ok(())
        })?;
        Ok(out)
    }

    pub(crate) fn load_root_paths(&self, root_id: &ObjectId) -> Result<Vec<String>> {
        self.note_full_root_path_load();
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        let tree = root_map_tree_from_root_hex(root.path_map_root.as_deref())?;
        let iter = self.root_prolly.range(&tree, &[], None)?;
        let mut paths = Vec::new();
        for item in iter {
            let (key, _) = item?;
            let path = String::from_utf8(key)
                .map_err(|err| Error::Corrupt(format!("non UTF-8 path key: {err}")))?;
            paths.push(path);
        }
        Ok(paths)
    }

    pub(crate) fn for_each_root_file_chunk<F>(
        &self,
        root_id: &ObjectId,
        chunk_size: usize,
        mut f: F,
    ) -> Result<()>
    where
        F: FnMut(BTreeMap<String, FileEntry>) -> Result<()>,
    {
        self.note_full_root_path_load();
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        let tree = root_map_tree_from_root_hex(root.path_map_root.as_deref())?;
        let iter = self.root_prolly.range(&tree, &[], None)?;
        let chunk_size = chunk_size.max(1);
        let mut chunk = BTreeMap::new();
        for item in iter {
            let (key, value) = item?;
            let path = String::from_utf8(key)
                .map_err(|err| Error::Corrupt(format!("non UTF-8 path key: {err}")))?;
            let entry: FileEntry = from_cbor(&value)?;
            chunk.insert(path, entry);
            if chunk.len() >= chunk_size {
                f(std::mem::take(&mut chunk))?;
            }
        }
        if !chunk.is_empty() {
            f(chunk)?;
        }
        Ok(())
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
        match &content.representation {
            TextRepresentation::SmallText { lines } => Ok(lines.clone()),
            TextRepresentation::SmallTextTable { table } => decode_small_text_table(table),
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
            TextRepresentation::LazyText {
                blob_id,
                introduced_by,
            } => {
                let blob: Blob = self.get_object(BLOB_KIND, blob_id)?;
                validate_full_text_blob(&content, &blob)?;
                Ok(lazy_text_lines(&blob.bytes, introduced_by))
            }
            TextRepresentation::OpaqueText { blob_id, .. } => {
                let blob: Blob = self.get_object(BLOB_KIND, blob_id)?;
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

    pub(crate) fn project_entry_file(&self, entry: &FileEntry) -> Result<PathBuf> {
        if entry.content_hash.len() != 64
            || !entry
                .content_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(Error::Corrupt(format!(
                "file entry has invalid content hash `{}`",
                entry.content_hash
            )));
        }
        let path = self
            .db_dir
            .join("cache")
            .join("blobs")
            .join(&entry.content_hash[..2])
            .join(&entry.content_hash);
        match fs::symlink_metadata(&path) {
            Ok(metadata)
                if metadata.is_file()
                    && !metadata.file_type().is_symlink()
                    && metadata.len() == entry.size_bytes
                    && sha256_projection_file(&path)? == entry.content_hash =>
            {
                return Ok(path);
            }
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(Error::Corrupt(format!(
                    "blob projection `{}` is not a regular file",
                    path.display()
                )));
            }
            Ok(_) => {
                make_projection_writable(&path)?;
                fs::remove_file(&path)?;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(Error::Io(err)),
        }
        let bytes = self.materialize_entry_bytes(entry)?;
        if bytes.len() as u64 != entry.size_bytes || sha256_hex(&bytes) != entry.content_hash {
            return Err(Error::Corrupt(format!(
                "materialized bytes do not match file entry hash {}",
                entry.content_hash
            )));
        }
        write_file_atomic(&path, &bytes, false)?;
        let mut permissions = fs::metadata(&path)?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions)?;
        Ok(path)
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
            TextRepresentation::LazyText { blob_id, .. } => {
                let blob = blobs.get(blob_id).ok_or_else(|| Error::ObjectNotFound {
                    kind: BLOB_KIND,
                    id: blob_id.0.clone(),
                })?;
                validate_full_text_blob(content, blob)?;
                Ok(blob.bytes.clone())
            }
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
        self.materialize_files_at_report(output_root, previous, target)
            .map(|_| ())
    }

    pub(crate) fn materialize_files_at_report(
        &self,
        output_root: &Path,
        previous: &BTreeMap<String, FileEntry>,
        target: &BTreeMap<String, FileEntry>,
    ) -> Result<MaterializedWorkdir> {
        materialize_into_batched_report(
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
        self.materialize_files_best_effort_at_report(output_root, previous, target)
            .map(|_| ())
    }

    pub(crate) fn materialize_files_best_effort_at_report(
        &self,
        output_root: &Path,
        previous: &BTreeMap<String, FileEntry>,
        target: &BTreeMap<String, FileEntry>,
    ) -> Result<MaterializedWorkdir> {
        materialize_into_batched_best_effort_report(
            &self.workspace_root,
            output_root,
            previous,
            target,
            MATERIALIZE_BATCH_FILES,
            |batch| self.materialize_entries_bytes(batch),
        )
    }

    pub(crate) fn materialize_new_files_best_effort_at_with_workspace_cow(
        &self,
        output_root: &Path,
        target: &BTreeMap<String, FileEntry>,
    ) -> Result<()> {
        self.materialize_new_files_best_effort_at_with_workspace_cow_report(output_root, target)
            .map(|_| ())
    }

    pub(crate) fn materialize_new_files_best_effort_at_with_workspace_cow_report(
        &self,
        output_root: &Path,
        target: &BTreeMap<String, FileEntry>,
    ) -> Result<MaterializedWorkdir> {
        if target.is_empty() {
            return Ok(MaterializedWorkdir::default());
        }
        reject_case_insensitive_collisions(output_root, target)?;
        let mut remaining = BTreeMap::new();
        let mut report = MaterializedWorkdir::default();
        let mut cow_available = true;
        for (path, entry) in target {
            if cow_available {
                match materialize_workspace_file_cow_status_if_matching(
                    &self.workspace_root,
                    output_root,
                    path,
                    entry,
                )? {
                    WorkspaceCowMaterializeStatus::Cloned(stamp) => {
                        report.insert_stamp(path.clone(), stamp);
                        continue;
                    }
                    WorkspaceCowMaterializeStatus::Skipped => {}
                    WorkspaceCowMaterializeStatus::Unavailable(_) => {
                        cow_available = false;
                    }
                }
            }
            remaining.insert(path.clone(), entry.clone());
        }
        report.extend(self.materialize_files_best_effort_at_report(
            output_root,
            &BTreeMap::new(),
            &remaining,
        )?);
        Ok(report)
    }

    pub(crate) fn materialize_root_files_at_streaming(
        &self,
        root_id: &ObjectId,
        output_root: &Path,
        durable: bool,
    ) -> Result<RootMaterializationReport> {
        self.validate_streaming_root_case_collisions(root_id, output_root)?;
        let empty = BTreeMap::new();
        let baseline = self.worktree_index_baseline_root()?;
        let clean_index_available =
            self.clean_baseline_matches_visible_root(baseline.as_ref(), root_id);
        let mut report = RootMaterializationReport::default();
        self.for_each_root_file_chunk(root_id, MATERIALIZE_BATCH_FILES, |chunk| {
            let mut chunk_report = None;
            if clean_index_available {
                if let Some(source_stamps) =
                    self.workspace_file_stamps_if_clean_index_matches(root_id, &chunk)?
                {
                    chunk_report = materialize_from_workspace_cow_report(
                        &self.workspace_root,
                        output_root,
                        &chunk,
                        &source_stamps,
                        durable,
                    )?;
                }
            }

            let chunk_report = match chunk_report {
                Some(report) => report,
                None if durable => self.materialize_files_at_report(output_root, &empty, &chunk)?,
                None => {
                    self.materialize_files_best_effort_at_report(output_root, &empty, &chunk)?
                }
            };

            report.file_count += chunk.len() as u64;
            for (path, entry) in &chunk {
                report.disk_manifest.insert(
                    path.clone(),
                    DiskManifest {
                        kind: entry.kind.clone(),
                        executable: entry.executable,
                        content_hash: entry.content_hash.clone(),
                    },
                );
            }
            report.materialized.extend(chunk_report);
            Ok(())
        })?;
        Ok(report)
    }

    fn validate_streaming_root_case_collisions(
        &self,
        root_id: &ObjectId,
        output_root: &Path,
    ) -> Result<()> {
        if !is_case_insensitive_filesystem(output_root)? {
            return Ok(());
        }
        let mut seen = HashMap::new();
        self.for_each_root_file_chunk(root_id, MATERIALIZE_BATCH_FILES, |chunk| {
            for path in chunk.keys() {
                let folded = path.to_lowercase();
                if let Some(previous) = seen.insert(folded, path.clone()) {
                    if previous != *path {
                        return Err(Error::InvalidPath {
                            path: path.clone(),
                            reason: format!("case-insensitive path collision with `{previous}`"),
                        });
                    }
                }
            }
            Ok(())
        })
    }
}

fn sha256_projection_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn make_projection_writable(path: &Path) -> Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    if permissions.readonly() {
        permissions.set_readonly(false);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

pub(crate) fn lazy_text_lines(bytes: &[u8], introduced_by: &ChangeId) -> Vec<LineEntry> {
    split_lines(bytes)
        .into_iter()
        .enumerate()
        .map(|(idx, line)| {
            let text_hash = sha256_hex(&line.text);
            LineEntry {
                line_id: LineId::new(introduced_by.clone(), idx as u64 + 1),
                text: line.text,
                newline: line.newline,
                text_hash,
                introduced_by: introduced_by.clone(),
                last_content_change: introduced_by.clone(),
                last_move_change: None,
                flags: LineFlags::default(),
            }
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn streaming_root_materialization_fixture() -> (tempfile::TempDir, Trail, RefRecord) {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("b.txt"), "b1\n").unwrap();
        fs::write(workspace.path().join("a.txt"), "a1\n").unwrap();
        fs::create_dir(workspace.path().join("src")).unwrap();
        fs::write(workspace.path().join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let head = db.resolve_branch_ref("main").unwrap();
        (workspace, db, head)
    }

    #[test]
    fn streaming_root_materialization_chunks_visit_sorted_files_once() {
        let (_workspace, db, head) = streaming_root_materialization_fixture();
        let mut chunks = Vec::new();

        db.for_each_root_file_chunk(&head.root_id, 2, |chunk| {
            chunks.push(chunk.keys().cloned().collect::<Vec<_>>());
            Ok(())
        })
        .unwrap();

        assert_eq!(
            chunks,
            vec![
                vec!["a.txt".to_string(), "b.txt".to_string()],
                vec!["src/lib.rs".to_string()]
            ]
        );
    }

    #[test]
    fn streaming_root_materialization_writes_files_and_stamps() {
        let (_workspace, db, head) = streaming_root_materialization_fixture();
        let output = tempfile::tempdir().unwrap();

        let report = db
            .materialize_root_files_at_streaming(&head.root_id, output.path(), false)
            .unwrap();

        assert_eq!(report.file_count, 3);
        assert_eq!(report.disk_manifest.len(), 3);
        assert_eq!(report.materialized.stamps.len(), 3);
        assert_eq!(
            fs::read_to_string(output.path().join("a.txt")).unwrap(),
            "a1\n"
        );
        assert_eq!(
            fs::read_to_string(output.path().join("b.txt")).unwrap(),
            "b1\n"
        );
        assert_eq!(
            fs::read_to_string(output.path().join("src/lib.rs")).unwrap(),
            "pub fn lib() {}\n"
        );
    }

    #[test]
    fn lazy_root_lookup_lists_only_immediate_children() {
        let (_workspace, db, head) = streaming_root_materialization_fixture();

        let root = db.root_immediate_children(&head.root_id, "").unwrap();
        assert_eq!(
            root.iter()
                .map(|child| (
                    child.name.as_str(),
                    child.path.as_str(),
                    child.entry.is_some()
                ))
                .collect::<Vec<_>>(),
            vec![
                ("a.txt", "a.txt", true),
                ("b.txt", "b.txt", true),
                ("src", "src", false),
            ]
        );
        let src = db.root_immediate_children(&head.root_id, "src").unwrap();
        assert_eq!(src.len(), 1);
        assert_eq!(src[0].path, "src/lib.rs");
        assert!(src[0].entry.is_some());
        assert!(db.root_directory_exists(&head.root_id, "src").unwrap());
        assert!(!db.root_directory_exists(&head.root_id, "missing").unwrap());
        assert!(db
            .root_file_entry(&head.root_id, "a.txt")
            .unwrap()
            .is_some());
    }

    #[test]
    fn blob_projection_is_content_addressed_and_reused() {
        let (_workspace, db, head) = streaming_root_materialization_fixture();
        let entry = db.root_file_entry(&head.root_id, "a.txt").unwrap().unwrap();

        let first = db.project_entry_file(&entry).unwrap();
        let second = db.project_entry_file(&entry).unwrap();

        assert_eq!(first, second);
        assert_eq!(fs::read(first).unwrap(), b"a1\n");
    }

    #[test]
    fn blob_projection_repairs_same_size_corruption_before_reuse() {
        let (_workspace, db, head) = streaming_root_materialization_fixture();
        let entry = db.root_file_entry(&head.root_id, "a.txt").unwrap().unwrap();
        let path = db.project_entry_file(&entry).unwrap();
        make_projection_writable(&path).unwrap();
        fs::write(&path, b"bad").unwrap();

        let repaired = db.project_entry_file(&entry).unwrap();
        assert_eq!(repaired, path);
        assert_eq!(fs::read(repaired).unwrap(), b"a1\n");
    }
}

pub(crate) fn validate_full_text_blob(content: &TextContent, blob: &Blob) -> Result<()> {
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
