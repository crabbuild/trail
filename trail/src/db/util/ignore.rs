use super::*;

pub(crate) fn is_ignore_policy_path(path: &str) -> bool {
    path_from_rel(path)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, ".trailignore" | ".gitignore"))
}

/// Counts construction of a configured ignore walker. The ignore crate does
/// not expose the dynamically discovered ignore files or bytes loaded by a
/// walker, so dependency-file and dependency-byte metrics remain exact only
/// for the root files explicitly probed by `WorkspaceIgnorePolicySnapshot`.
pub(crate) fn note_walkbuilder_policy_build(metrics: Option<&Arc<OperationMetricsState>>) {
    if let Some(metrics) = metrics {
        metrics.add(OperationMetricsDelta {
            policy_build_count: 1,
            ..OperationMetricsDelta::default()
        });
    }
}

pub(crate) fn write_default_trailignore(workspace_root: &Path) -> Result<()> {
    let path = workspace_root.join(".trailignore");
    if path.exists() {
        return Ok(());
    }
    fs::write(
        path,
        format!("{}\n", DEFAULT_CRABIGNORE_PATTERNS.join("\n")),
    )?;
    Ok(())
}

pub(crate) fn read_ignore_patterns(path: &Path) -> Result<Vec<IgnorePattern>> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(Error::Io(err)),
    };
    Ok(content
        .lines()
        .enumerate()
        .filter_map(|(idx, line)| {
            let pattern = line.trim();
            if pattern.is_empty() || pattern.starts_with('#') {
                None
            } else {
                Some(IgnorePattern {
                    line: idx + 1,
                    pattern: pattern.to_string(),
                })
            }
        })
        .collect())
}

pub(crate) fn normalize_ignore_pattern(pattern: &str) -> Result<String> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return Err(Error::InvalidInput(
            "ignore pattern cannot be empty".to_string(),
        ));
    }
    if pattern.starts_with('#') {
        return Err(Error::InvalidInput(
            "ignore pattern cannot be a comment".to_string(),
        ));
    }
    if pattern.contains('\0') || pattern.contains('\n') || pattern.contains('\r') {
        return Err(Error::InvalidInput(
            "ignore pattern cannot contain control separators".to_string(),
        ));
    }
    Ok(pattern.to_string())
}
