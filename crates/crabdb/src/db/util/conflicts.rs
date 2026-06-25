use super::*;

#[derive(Debug)]
pub(crate) enum ManualConflictPayload {
    Text { content: String, executable: bool },
    Delete,
}

pub(crate) fn normalize_manual_conflict_files(
    manual: ConflictManualResolution,
    conflict_paths: &BTreeSet<String>,
) -> Result<BTreeMap<String, ConflictManualFile>> {
    if manual.files.is_empty() {
        return Err(Error::InvalidInput(
            "manual conflict resolution must include at least one file".to_string(),
        ));
    }

    let mut normalized = BTreeMap::new();
    for (path, file) in manual.files {
        let normalized_path = normalize_relative_path(&path)?;
        if normalized.insert(normalized_path.clone(), file).is_some() {
            return Err(Error::InvalidInput(format!(
                "manual conflict resolution includes duplicate path `{normalized_path}`"
            )));
        }
    }

    let provided = normalized.keys().cloned().collect::<BTreeSet<_>>();
    let missing = conflict_paths
        .difference(&provided)
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(Error::InvalidInput(format!(
            "manual conflict resolution is missing conflicted path(s): {}",
            missing.join(", ")
        )));
    }

    let extra = provided
        .difference(conflict_paths)
        .cloned()
        .collect::<Vec<_>>();
    if !extra.is_empty() {
        return Err(Error::InvalidInput(format!(
            "manual conflict resolution includes non-conflicted path(s): {}",
            extra.join(", ")
        )));
    }

    Ok(normalized)
}

pub(crate) fn manual_conflict_file_payload(
    file: ConflictManualFile,
    default_executable: bool,
) -> Result<ManualConflictPayload> {
    match file {
        ConflictManualFile::Text(content) => Ok(ManualConflictPayload::Text {
            content,
            executable: default_executable,
        }),
        ConflictManualFile::Spec(spec) if spec.delete => {
            if spec.content.is_some() {
                return Err(Error::InvalidInput(
                    "manual conflict file cannot set both `delete` and `content`".to_string(),
                ));
            }
            Ok(ManualConflictPayload::Delete)
        }
        ConflictManualFile::Spec(spec) => {
            let Some(content) = spec.content else {
                return Err(Error::InvalidInput(
                    "manual conflict file must include `content` or set `delete` to true"
                        .to_string(),
                ));
            };
            Ok(ManualConflictPayload::Text {
                content,
                executable: spec.executable.unwrap_or(default_executable),
            })
        }
    }
}

pub(crate) fn conflict_paths_from_details(details: &[String]) -> Result<BTreeSet<String>> {
    let mut paths = BTreeSet::new();
    for detail in details {
        let mut parts = detail.split('`');
        let _before = parts.next();
        if let Some(path) = parts.next() {
            paths.insert(normalize_relative_path(path)?);
        }
    }
    if paths.is_empty() {
        return Err(Error::InvalidInput(
            "conflict set does not include path details that can be resolved automatically"
                .to_string(),
        ));
    }
    Ok(paths)
}
