use super::*;
use unicode_normalization::UnicodeNormalization;

pub(crate) fn normalize_relative_path(path: &str) -> Result<String> {
    if path.as_bytes().contains(&0) {
        return Err(Error::InvalidPath {
            path: path.to_string(),
            reason: "NUL bytes are not allowed".to_string(),
        });
    }
    if path.chars().any(is_slash_lookalike) {
        return Err(Error::InvalidPath {
            path: path.to_string(),
            reason: "path contains a slash lookalike; use `/` as the separator".to_string(),
        });
    }
    #[cfg(not(windows))]
    if path.contains('\\') {
        return Err(Error::InvalidPath {
            path: path.to_string(),
            reason: "path contains a backslash separator; use `/` as the separator".to_string(),
        });
    }
    if path.nfc().ne(path.chars()) {
        return Err(Error::InvalidPath {
            path: path.to_string(),
            reason: "path must use Unicode NFC normalization".to_string(),
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
                validate_relative_path_component(&path, &part)?;
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

fn is_slash_lookalike(ch: char) -> bool {
    matches!(
        ch,
        '\u{2044}' // fraction slash
            | '\u{2215}' // division slash
            | '\u{2216}' // set minus
            | '\u{29F8}' // big solidus
            | '\u{29F9}' // big reverse solidus
            | '\u{FE68}' // small reverse solidus
            | '\u{FF0F}' // fullwidth solidus
            | '\u{FF3C}' // fullwidth reverse solidus
    )
}

fn validate_relative_path_component(path: &str, part: &str) -> Result<()> {
    if part.chars().any(char::is_control) {
        return Err(Error::InvalidPath {
            path: path.to_string(),
            reason: "path components cannot contain control characters".to_string(),
        });
    }
    if part.chars().any(is_invisible_path_format_control) {
        return Err(Error::InvalidPath {
            path: path.to_string(),
            reason: "path components cannot contain invisible Unicode format controls".to_string(),
        });
    }
    if part.contains(':') {
        return Err(Error::InvalidPath {
            path: path.to_string(),
            reason: "path components cannot contain `:`".to_string(),
        });
    }
    if part.ends_with([' ', '.']) {
        return Err(Error::InvalidPath {
            path: path.to_string(),
            reason: "path components cannot end with a space or dot".to_string(),
        });
    }
    let stem = windows_reserved_stem(part);
    if matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "CONIN$"
            | "CONOUT$"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        return Err(Error::InvalidPath {
            path: path.to_string(),
            reason: format!("path component `{part}` is reserved on Windows"),
        });
    }
    Ok(())
}

fn windows_reserved_stem(part: &str) -> String {
    part.split('.')
        .next()
        .unwrap_or(part)
        .chars()
        .map(|ch| match ch {
            '\u{00B9}' => '1',
            '\u{00B2}' => '2',
            '\u{00B3}' => '3',
            other => other.to_ascii_uppercase(),
        })
        .collect()
}

fn is_invisible_path_format_control(ch: char) -> bool {
    matches!(
        ch,
        '\u{200B}' // zero width space
            | '\u{200C}' // zero width non-joiner
            | '\u{200D}' // zero width joiner
            | '\u{200E}' // left-to-right mark
            | '\u{200F}' // right-to-left mark
            | '\u{202A}' // left-to-right embedding
            | '\u{202B}' // right-to-left embedding
            | '\u{202C}' // pop directional formatting
            | '\u{202D}' // left-to-right override
            | '\u{202E}' // right-to-left override
            | '\u{2060}' // word joiner
            | '\u{2066}' // left-to-right isolate
            | '\u{2067}' // right-to-left isolate
            | '\u{2068}' // first strong isolate
            | '\u{2069}' // pop directional isolate
            | '\u{FEFF}' // zero width no-break space / BOM
    )
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
                    reason: "lane workdir cannot contain parent directory components".to_string(),
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
            reason: "lane workdir has no existing ancestor".to_string(),
        })?;
    }
    let mut out = existing.canonicalize()?;
    for name in missing.iter().rev() {
        out.push(name);
    }
    normalize_workdir_path(&out)
}

pub(crate) fn prepare_lane_workdir(path: &Path, custom_workdir: bool) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(Error::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                    reason: "lane workdir cannot be a symlink".to_string(),
                });
            }
            if !metadata.is_dir() {
                return Err(Error::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                    reason: "lane workdir path exists but is not a directory".to_string(),
                });
            }
            if custom_workdir && fs::read_dir(path)?.next().is_some() {
                return Err(Error::InvalidInput(format!(
                    "custom lane workdir `{}` must be empty or absent",
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
