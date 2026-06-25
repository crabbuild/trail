use super::*;

pub(crate) fn write_default_crabignore(workspace_root: &Path) -> Result<()> {
    let path = workspace_root.join(".crabignore");
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
