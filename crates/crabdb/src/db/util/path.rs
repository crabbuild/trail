use super::*;

pub(crate) fn normalize_relative_path(path: &str) -> Result<String> {
    if path.as_bytes().contains(&0) {
        return Err(Error::InvalidPath {
            path: path.to_string(),
            reason: "NUL bytes are not allowed".to_string(),
        });
    }
    let path = path.replace('\\', "/");
    let mut parts = Vec::new();
    for component in Path::new(&path).components() {
        match component {
            Component::Normal(part) => {
                let part = part.to_string_lossy();
                if part.is_empty() {
                    continue;
                }
                parts.push(part.to_string());
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::InvalidPath {
                    path: path.to_string(),
                    reason: "path must stay inside the workspace".to_string(),
                });
            }
        }
    }
    if parts.is_empty() {
        return Err(Error::InvalidPath {
            path: path.to_string(),
            reason: "path cannot be empty".to_string(),
        });
    }
    Ok(parts.join("/"))
}

pub(crate) fn normalize_workdir_path(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(Error::InvalidPath {
            path: String::new(),
            reason: "path cannot be empty".to_string(),
        });
    }
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push(component.as_os_str()),
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(Error::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                    reason: "agent workdir cannot contain parent directory components".to_string(),
                });
            }
        }
    }
    if out.as_os_str().is_empty() {
        return Err(Error::InvalidPath {
            path: path.to_string_lossy().to_string(),
            reason: "path cannot be empty".to_string(),
        });
    }
    Ok(out)
}

pub(crate) fn canonicalize_existing_workdir_prefix(path: &Path) -> Result<PathBuf> {
    let mut existing = path;
    let mut missing = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name() else {
            break;
        };
        missing.push(name.to_os_string());
        existing = existing.parent().ok_or_else(|| Error::InvalidPath {
            path: path.to_string_lossy().to_string(),
            reason: "agent workdir has no existing ancestor".to_string(),
        })?;
    }
    let mut out = existing.canonicalize()?;
    for name in missing.iter().rev() {
        out.push(name);
    }
    normalize_workdir_path(&out)
}

pub(crate) fn prepare_agent_workdir(path: &Path, custom_workdir: bool) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(Error::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                    reason: "agent workdir cannot be a symlink".to_string(),
                });
            }
            if !metadata.is_dir() {
                return Err(Error::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                    reason: "agent workdir path exists but is not a directory".to_string(),
                });
            }
            if custom_workdir && fs::read_dir(path)?.next().is_some() {
                return Err(Error::InvalidInput(format!(
                    "custom agent workdir `{}` must be empty or absent",
                    path.display()
                )));
            }
            fs::remove_dir_all(path)?;
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(Error::Io(err)),
    }
    fs::create_dir_all(path)?;
    Ok(())
}

pub(crate) fn prepare_checkout_workdir(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(Error::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                    reason: "checkout workdir cannot be a symlink".to_string(),
                });
            }
            if !metadata.is_dir() {
                return Err(Error::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                    reason: "checkout workdir path exists but is not a directory".to_string(),
                });
            }
            if fs::read_dir(path)?.next().is_some() {
                return Err(Error::InvalidInput(format!(
                    "checkout workdir `{}` must be empty or absent",
                    path.display()
                )));
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(Error::Io(err)),
    }
    fs::create_dir_all(path)?;
    Ok(())
}

pub(crate) fn normalize_record_paths(paths: &[String]) -> Result<Vec<String>> {
    let mut normalized = BTreeSet::new();
    for path in paths {
        normalized.insert(normalize_relative_path(path)?);
    }
    Ok(normalized.into_iter().collect())
}

pub(crate) fn path_matches_selection(path: &str, selected: &str) -> bool {
    path == selected
        || path
            .strip_prefix(selected)
            .is_some_and(|rest| rest.starts_with('/'))
}

pub(crate) fn validate_ref_segment(name: &str) -> Result<()> {
    if name.is_empty()
        || name.contains("..")
        || name.starts_with('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err(Error::InvalidInput(format!("invalid ref segment `{name}`")));
    }
    Ok(())
}

pub(crate) fn path_from_rel(path: &str) -> PathBuf {
    path.split('/').collect()
}

pub(crate) fn is_internal_path(path: &str) -> bool {
    path.split('/')
        .any(|part| part == ".crabdb" || part == ".git")
}

pub(crate) fn is_default_ignored(path: &str) -> bool {
    let components = path.split('/').collect::<Vec<_>>();
    if components.iter().any(|part| {
        matches!(
            *part,
            ".crabdb" | ".git" | "node_modules" | "target" | "dist" | "build" | "coverage"
        )
    }) {
        return true;
    }
    let file_name = components.last().copied().unwrap_or_default();
    file_name == ".crabignore"
        || file_name == ".env"
        || file_name.starts_with(".env.")
        || file_name.ends_with(".pem")
        || file_name.ends_with(".key")
        || file_name.ends_with(".p12")
        || file_name.ends_with(".pfx")
        || file_name == "id_rsa"
        || file_name == "id_ed25519"
}

pub(crate) fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|byte| *byte == 0)
}

pub(crate) fn classify_file_kind(bytes: &[u8], text_config: &TextConfig) -> FileKind {
    if looks_binary(bytes) {
        FileKind::Binary
    } else if std::str::from_utf8(bytes).is_err()
        || bytes.len() as u64 > text_config.opaque_text_max_bytes
        || max_line_len(bytes) as u64 > text_config.max_line_bytes
    {
        FileKind::OpaqueText
    } else {
        FileKind::Text
    }
}

pub(crate) fn max_line_len(bytes: &[u8]) -> usize {
    bytes
        .split(|byte| *byte == b'\n')
        .map(|line| line.len())
        .max()
        .unwrap_or(0)
}

#[derive(Clone)]
pub(crate) struct SplitLine {
    pub(crate) text: Vec<u8>,
    pub(crate) newline: NewlineKind,
}

pub(crate) fn split_lines(bytes: &[u8]) -> Vec<SplitLine> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = 0;
    for (idx, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            if idx > start && bytes[idx - 1] == b'\r' {
                out.push(SplitLine {
                    text: bytes[start..idx - 1].to_vec(),
                    newline: NewlineKind::Crlf,
                });
            } else {
                out.push(SplitLine {
                    text: bytes[start..idx].to_vec(),
                    newline: NewlineKind::Lf,
                });
            }
            start = idx + 1;
        }
    }
    if start < bytes.len() {
        out.push(SplitLine {
            text: bytes[start..].to_vec(),
            newline: NewlineKind::None,
        });
    }
    out
}

pub(crate) fn materialize_lines(lines: &[LineEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    for line in lines {
        out.extend_from_slice(&line.text);
        match line.newline {
            NewlineKind::None => {}
            NewlineKind::Lf => out.push(b'\n'),
            NewlineKind::Crlf => out.extend_from_slice(b"\r\n"),
        }
    }
    out
}

pub(crate) fn line_map_by_id(lines: &[LineEntry]) -> HashMap<String, &LineEntry> {
    lines
        .iter()
        .map(|line| (line.line_id_key(), line))
        .collect()
}

pub(crate) fn line_content_equal(left: Option<&LineEntry>, right: Option<&LineEntry>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            left.text_hash == right.text_hash
                && left.newline == right.newline
                && left.text == right.text
        }
        (None, None) => true,
        _ => false,
    }
}

pub(crate) fn preserves_base_line_order(base_order: &[String], lines: &[LineEntry]) -> bool {
    let positions = base_order
        .iter()
        .enumerate()
        .map(|(idx, line_id)| (line_id.as_str(), idx))
        .collect::<HashMap<_, _>>();
    let mut last = None;
    for line in lines {
        let line_id = line.line_id_key();
        let Some(position) = positions.get(line_id.as_str()).copied() else {
            continue;
        };
        if last.is_some_and(|last| position < last) {
            return false;
        }
        last = Some(position);
    }
    true
}

pub(crate) fn inserted_line_gaps(
    lines: &[LineEntry],
    base_keys: &HashSet<String>,
) -> BTreeSet<LineGap> {
    inserted_line_groups(lines, base_keys)
        .into_iter()
        .map(|(gap, _)| gap)
        .collect()
}

pub(crate) fn inserted_line_groups(
    lines: &[LineEntry],
    base_keys: &HashSet<String>,
) -> Vec<(LineGap, Vec<LineEntry>)> {
    let mut groups: Vec<(LineGap, Vec<LineEntry>)> = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let line_id = line.line_id_key();
        if base_keys.contains(&line_id) {
            continue;
        }
        let gap = line_gap_at(lines, idx, base_keys);
        if let Some((last_gap, last_lines)) = groups.last_mut() {
            if *last_gap == gap {
                last_lines.push(line.clone());
                continue;
            }
        }
        groups.push((gap, vec![line.clone()]));
    }
    groups
}

pub(crate) fn line_gap_at(lines: &[LineEntry], idx: usize, base_keys: &HashSet<String>) -> LineGap {
    let previous = lines[..idx]
        .iter()
        .rev()
        .map(LineEntryExt::line_id_key)
        .find(|line_id| base_keys.contains(line_id));
    let next = lines[idx + 1..]
        .iter()
        .map(LineEntryExt::line_id_key)
        .find(|line_id| base_keys.contains(line_id));
    LineGap { previous, next }
}

pub(crate) fn replace_or_insert_line(
    lines: &mut Vec<LineEntry>,
    line_id: &str,
    replacement: LineEntry,
) {
    if let Some(line) = lines
        .iter_mut()
        .find(|line| line.line_id_key().as_str() == line_id)
    {
        *line = replacement;
    } else {
        lines.push(replacement);
    }
}

pub(crate) fn remove_line(lines: &mut Vec<LineEntry>, line_id: &str) {
    if let Some(idx) = lines
        .iter()
        .position(|line| line.line_id_key().as_str() == line_id)
    {
        lines.remove(idx);
    }
}

pub(crate) fn insert_lines_at_gap(
    lines: &mut Vec<LineEntry>,
    gap: &LineGap,
    inserted: Vec<LineEntry>,
) {
    let mut idx = if let Some(next) = &gap.next {
        lines
            .iter()
            .position(|line| line.line_id_key() == *next)
            .unwrap_or(lines.len())
    } else if let Some(previous) = &gap.previous {
        lines
            .iter()
            .position(|line| line.line_id_key() == *previous)
            .map(|idx| idx + 1)
            .unwrap_or(lines.len())
    } else {
        lines.len()
    };
    for line in inserted {
        lines.insert(idx, line);
        idx += 1;
    }
}

pub(crate) fn order_key(line_number: u64) -> Vec<u8> {
    (line_number * ORDER_KEY_STEP).to_be_bytes().to_vec()
}

pub(crate) fn line_similarity(left: &[u8], right: &[u8]) -> f32 {
    if left == right {
        return 1.0;
    }
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let max = left.len().max(right.len()) as f32;
    let common = left
        .iter()
        .zip(right)
        .filter(|(left, right)| left == right)
        .count() as f32;
    common / max
}

pub(crate) fn count_line_delta(changes: &[LineChange]) -> (u64, u64) {
    let mut additions = 0;
    let mut deletions = 0;
    for change in changes {
        match change.kind {
            LineChangeKind::Added => additions += 1,
            LineChangeKind::Deleted => deletions += 1,
            LineChangeKind::Modified => {
                additions += 1;
                deletions += 1;
            }
            LineChangeKind::Moved => {}
        }
    }
    (additions, deletions)
}

pub(crate) fn summarize_file_changes(changes: &[FileChange]) -> Vec<FileDiffSummary> {
    changes
        .iter()
        .map(|change| {
            let (additions, deletions) = count_line_delta(&change.line_changes);
            FileDiffSummary {
                path: change.path.clone(),
                old_path: change.old_path.clone(),
                kind: change.kind.clone(),
                before_hash: change.before_hash.clone(),
                after_hash: change.after_hash.clone(),
                additions,
                deletions,
                line_changes: Vec::new(),
                patch: None,
            }
        })
        .collect()
}

pub(crate) fn attach_line_changes(changes: &[FileChange], summaries: &mut [FileDiffSummary]) {
    for summary in summaries {
        summary.line_changes = changes
            .iter()
            .find(|change| {
                change.path == summary.path
                    && change.old_path == summary.old_path
                    && change.kind == summary.kind
            })
            .map(|change| change.line_changes.clone())
            .unwrap_or_default();
    }
}

pub(crate) fn worktree_state_from_changes(changed_paths: &[FileDiffSummary]) -> WorktreeState {
    if changed_paths.is_empty() {
        WorktreeState::Clean
    } else if changed_paths
        .iter()
        .any(|summary| summary.kind == FileChangeKind::Added)
    {
        WorktreeState::DirtyUntracked
    } else {
        WorktreeState::DirtyTracked
    }
}
