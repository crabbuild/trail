use super::*;

pub(crate) fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<u64> {
    if !source.exists() {
        return Ok(0);
    }
    fs::create_dir_all(destination)?;
    let mut bytes = 0;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.file_type().is_symlink() {
            #[cfg(unix)]
            {
                let target = fs::read_link(&source_path)?;
                symlink_file(target, destination_path)?;
            }
            #[cfg(not(unix))]
            {
                return Err(Error::InvalidInput(format!(
                    "cannot copy symlink `{}` on this platform",
                    source_path.display()
                )));
            }
        } else if metadata.is_dir() {
            bytes += copy_dir_recursive(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            bytes += fs::copy(&source_path, &destination_path)?;
        }
    }
    Ok(bytes)
}

pub(crate) fn materialize_into<F>(
    workspace_root: &Path,
    output_root: &Path,
    previous: &BTreeMap<String, FileEntry>,
    target: &BTreeMap<String, FileEntry>,
    bytes_for: F,
) -> Result<()>
where
    F: Fn(&FileEntry) -> Result<Vec<u8>>,
{
    reject_case_insensitive_collisions(output_root, target)?;
    for path in previous.keys() {
        if !target.contains_key(path) {
            let abs = safe_join(output_root, path)?;
            if abs.exists() {
                fs::remove_file(abs)?;
            }
        }
    }
    for (path, entry) in target {
        let abs = safe_join(output_root, path)?;
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = bytes_for(entry)?;
        write_materialized_file(&abs, path, &bytes, entry.executable)?;
    }
    let _ = workspace_root;
    Ok(())
}

pub(crate) fn reject_case_insensitive_collisions(
    output_root: &Path,
    target: &BTreeMap<String, FileEntry>,
) -> Result<()> {
    if !is_case_insensitive_filesystem(output_root)? {
        return Ok(());
    }
    validate_no_case_fold_collisions(target.keys())
}

pub(crate) fn validate_no_case_fold_collisions<'a, I>(paths: I) -> Result<()>
where
    I: IntoIterator<Item = &'a String>,
{
    let mut seen = HashMap::new();
    for path in paths {
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
}

pub(crate) fn is_case_insensitive_filesystem(root: &Path) -> Result<bool> {
    let root = root.canonicalize()?;
    for _ in 0..16 {
        let lower_name = format!(".crabdb-case-probe-{}-a", now_nanos());
        let upper_name = lower_name.to_ascii_uppercase();
        let lower = root.join(&lower_name);
        let upper = root.join(&upper_name);
        match OpenOptions::new().write(true).create_new(true).open(&lower) {
            Ok(_) => {
                let insensitive = upper.exists();
                let _ = fs::remove_file(&lower);
                return Ok(insensitive);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(Error::from(err)),
        }
    }
    Err(Error::InvalidInput(
        "could not create filesystem case-sensitivity probe".to_string(),
    ))
}

pub(crate) fn safe_join(root: &Path, rel: &str) -> Result<PathBuf> {
    let normalized = normalize_relative_path(rel)?;
    let root_canon = root.canonicalize()?;
    let candidate = root_canon.join(path_from_rel(&normalized));
    ensure_no_symlink_ancestors(&root_canon, &normalized, rel)?;
    if let Some(parent) = candidate.parent() {
        let parent = if parent.exists() {
            parent.canonicalize()?
        } else {
            let mut existing = parent;
            while !existing.exists() {
                existing = existing.parent().ok_or_else(|| Error::InvalidPath {
                    path: rel.to_string(),
                    reason: "path has no existing ancestor".to_string(),
                })?;
            }
            existing.canonicalize()?
        };
        if !parent.starts_with(root_canon) {
            return Err(Error::InvalidPath {
                path: rel.to_string(),
                reason: "path escapes output root".to_string(),
            });
        }
    }
    Ok(candidate)
}

pub(crate) fn ensure_no_symlink_ancestors(root: &Path, normalized: &str, rel: &str) -> Result<()> {
    let mut current = root.to_path_buf();
    let rel_path = path_from_rel(normalized);
    let Some(parent) = rel_path.parent() else {
        return Ok(());
    };
    for component in parent.components() {
        let Component::Normal(part) = component else {
            continue;
        };
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(Error::InvalidPath {
                        path: rel.to_string(),
                        reason: "path uses a symlink ancestor".to_string(),
                    });
                }
                if !metadata.is_dir() {
                    return Err(Error::InvalidPath {
                        path: rel.to_string(),
                        reason: "path parent is not a directory".to_string(),
                    });
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => break,
            Err(err) => return Err(Error::from(err)),
        }
    }
    Ok(())
}

pub(crate) fn write_materialized_file(
    path: &Path,
    rel: &str,
    bytes: &[u8],
    executable: bool,
) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            return Err(Error::InvalidPath {
                path: rel.to_string(),
                reason: "refusing to follow symlink for write".to_string(),
            });
        }
        if !metadata.is_file() {
            return Err(Error::InvalidPath {
                path: rel.to_string(),
                reason: "path is not a regular file".to_string(),
            });
        }
    }
    let parent = path.parent().ok_or_else(|| Error::InvalidPath {
        path: rel.to_string(),
        reason: "path has no parent directory".to_string(),
    })?;
    fs::create_dir_all(parent)?;

    let (tmp, mut file) = create_materialize_temp_file(parent, path)?;
    let result = (|| -> Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        set_executable(&tmp, executable)?;
        fs::rename(&tmp, path)?;
        sync_directory(parent);
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

pub(crate) fn create_materialize_temp_file(
    parent: &Path,
    path: &Path,
) -> Result<(PathBuf, fs::File)> {
    let leaf = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "file".into());
    for _ in 0..16 {
        let tmp = parent.join(format!(".{leaf}.crabdb-tmp-{}", now_nanos()));
        match OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(file) => return Ok((tmp, file)),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(Error::from(err)),
        }
    }
    Err(Error::InvalidInput(
        "could not create materialization temp file".to_string(),
    ))
}

pub(crate) fn sync_directory(path: &Path) {
    if let Ok(dir) = OpenOptions::new().read(true).open(path) {
        let _ = dir.sync_all();
    }
}

pub(crate) fn merge_files_with_resolution(
    base: &BTreeMap<String, FileEntry>,
    target: &BTreeMap<String, FileEntry>,
    source: &BTreeMap<String, FileEntry>,
    conflict_paths: &BTreeSet<String>,
    take: ConflictTake,
) -> Result<BTreeMap<String, FileEntry>> {
    let mut merged = target.clone();
    let mut unresolved = Vec::new();
    let mut paths = BTreeSet::new();
    paths.extend(base.keys().cloned());
    paths.extend(target.keys().cloned());
    paths.extend(source.keys().cloned());
    for path in paths {
        let base_entry = base.get(&path);
        let target_entry = target.get(&path);
        let source_entry = source.get(&path);
        let target_changed = entry_hash(base_entry) != entry_hash(target_entry);
        let source_changed = entry_hash(base_entry) != entry_hash(source_entry);
        match (target_changed, source_changed) {
            (false, true) => match source_entry {
                Some(entry) => {
                    merged.insert(path.clone(), entry.clone());
                }
                None => {
                    merged.remove(&path);
                }
            },
            (true, true) => {
                if entry_hash(target_entry) != entry_hash(source_entry) {
                    if !conflict_paths.contains(&path) {
                        unresolved.push(format!("conflict path `{path}` was not recorded"));
                        continue;
                    }
                    if take == ConflictTake::Source {
                        match source_entry {
                            Some(entry) => {
                                merged.insert(path.clone(), entry.clone());
                            }
                            None => {
                                merged.remove(&path);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if !unresolved.is_empty() {
        return Err(Error::Conflict(unresolved.join("; ")));
    }
    Ok(merged)
}

pub(crate) fn entry_hash(entry: Option<&FileEntry>) -> Option<(&str, bool, &FileKind)> {
    entry.map(|entry| (entry.content_hash.as_str(), entry.executable, &entry.kind))
}

pub(crate) fn unified_patch(
    old_path: &str,
    new_path: &str,
    old_text: &str,
    new_text: &str,
) -> String {
    let diff = TextDiff::from_lines(old_text, new_text);
    let mut out = String::new();
    out.push_str(&format!("diff --crabdb a/{old_path} b/{new_path}\n"));
    out.push_str(&format!("--- a/{old_path}\n"));
    out.push_str(&format!("+++ b/{new_path}\n"));
    for group in diff.grouped_ops(3) {
        for op in group {
            for change in diff.iter_changes(&op) {
                let sign = match change.tag() {
                    ChangeTag::Delete => "-",
                    ChangeTag::Insert => "+",
                    ChangeTag::Equal => " ",
                };
                out.push_str(sign);
                out.push_str(change.value());
                if !change.value().ends_with('\n') {
                    out.push('\n');
                }
            }
        }
    }
    out
}

pub(crate) fn parse_range(spec: &str) -> Result<(&str, &str)> {
    let Some((left, right)) = spec.split_once("..") else {
        return Err(Error::InvalidInput(format!(
            "range `{spec}` must look like left..right"
        )));
    };
    if left.is_empty() || right.is_empty() {
        return Err(Error::InvalidInput(format!(
            "range `{spec}` must include both endpoints"
        )));
    }
    Ok((left, right))
}

pub(crate) fn parse_path_line(spec: &str) -> Result<(String, u64)> {
    let Some((path, line)) = spec.rsplit_once(':') else {
        return Err(Error::InvalidInput(format!(
            "`{spec}` must look like path:line"
        )));
    };
    let line_number = line
        .parse::<u64>()
        .map_err(|_| Error::InvalidInput(format!("invalid line number `{line}`")))?;
    if line_number == 0 {
        return Err(Error::InvalidInput("line numbers are 1-based".to_string()));
    }
    Ok((normalize_relative_path(path)?, line_number))
}

pub(crate) fn branch_ref(branch: &str) -> String {
    if branch.starts_with("refs/") {
        branch.to_string()
    } else {
        format!("{MAIN_REF_PREFIX}{branch}")
    }
}

pub(crate) fn agent_ref(agent: &str) -> String {
    if agent.starts_with("refs/") {
        agent.to_string()
    } else {
        format!("{AGENT_REF_PREFIX}{agent}")
    }
}

pub(crate) fn content_object_id(content: &FileContentRef) -> &ObjectId {
    match content {
        FileContentRef::Text(object_id)
        | FileContentRef::Opaque(object_id)
        | FileContentRef::Binary(object_id) => object_id,
    }
}

pub(crate) fn file_id_key(file_id: &FileId) -> String {
    format!("{}:{}", file_id.origin_change.0, file_id.local_seq)
}

pub(crate) fn line_id_key_value(line_id: &LineId) -> String {
    format!("{}:{}", line_id.origin_change.0, line_id.local_seq)
}

pub(crate) fn parse_line_id_key(value: &str) -> Result<LineId> {
    let (change_id, local_seq) = value.rsplit_once(':').ok_or_else(|| {
        Error::InvalidInput("line id must look like `ch_...:<local_seq>`".to_string())
    })?;
    if !change_id.starts_with("ch_") {
        return Err(Error::InvalidInput(format!(
            "line id change id must start with `ch_`, got `{change_id}`"
        )));
    }
    let local_seq = local_seq.parse::<u64>().map_err(|_| {
        Error::InvalidInput(format!("invalid line id local sequence `{local_seq}`"))
    })?;
    Ok(LineId::new(ChangeId(change_id.to_string()), local_seq))
}

pub(crate) trait LineChangeExt {
    fn line_id_key(&self) -> String;
}

impl LineChangeExt for LineChange {
    fn line_id_key(&self) -> String {
        line_id_key_value(&self.line_id)
    }
}

pub(crate) trait LineEntryExt {
    fn line_id_key(&self) -> String;
}

impl LineEntryExt for LineEntry {
    fn line_id_key(&self) -> String {
        line_id_key_value(&self.line_id)
    }
}
