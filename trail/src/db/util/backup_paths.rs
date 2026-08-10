use super::*;

pub(crate) fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

pub(crate) fn backup_manifest_path(path: &Path) -> PathBuf {
    path.join("manifest.json")
}

pub(crate) fn backup_sqlite_path(path: &Path) -> PathBuf {
    path.join(DB_RELATIVE_PATH)
}

pub(crate) fn read_backup_manifest(path: &Path) -> Result<BackupManifest> {
    let bytes = fs::read(backup_manifest_path(path))?;
    serde_json::from_slice(&bytes).map_err(Error::from)
}

pub(crate) fn file_digest(path: &Path) -> Result<(u64, String)> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut bytes = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok((bytes, hex::encode(hasher.finalize())))
}

pub(crate) fn portable_tree_digest(path: &Path) -> Result<(u64, String)> {
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    if !path.exists() {
        return Ok((bytes, hex::encode(hasher.finalize())));
    }
    let entries = walkdir::WalkDir::new(path)
        .follow_links(false)
        .min_depth(1)
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| Error::Io(error.into()))?;
    let mut entries = entries
        .into_iter()
        .map(|entry| {
            let relative = entry.path().strip_prefix(path).map_err(|_| {
                Error::Corrupt(format!(
                    "backup private path `{}` escaped its tree",
                    entry.path().display()
                ))
            })?;
            Ok((portable_relative_path_bytes(relative)?, entry))
        })
        .collect::<Result<Vec<_>>>()?;
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    for (relative, entry) in entries {
        hasher.update((relative.len() as u64).to_be_bytes());
        hasher.update(&relative);
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.is_dir() {
            hasher.update(b"directory\0");
        } else if metadata.is_file() {
            hasher.update(b"file\0");
            hasher.update(metadata.len().to_be_bytes());
            let (file_bytes, digest) = file_digest(entry.path())?;
            if file_bytes != metadata.len() {
                return Err(Error::Conflict(format!(
                    "backup private file `{}` changed while being sealed",
                    entry.path().display()
                )));
            }
            bytes = bytes.saturating_add(file_bytes);
            hasher.update(digest.as_bytes());
        } else if metadata.file_type().is_symlink() {
            hasher.update(b"symlink\0");
            let target = fs::read_link(entry.path())?;
            let target = target.as_os_str().as_encoded_bytes();
            hasher.update((target.len() as u64).to_be_bytes());
            hasher.update(target);
        } else {
            return Err(Error::InvalidInput(format!(
                "backup private path `{}` has an unsupported file type",
                entry.path().display()
            )));
        }
    }
    Ok((bytes, hex::encode(hasher.finalize())))
}

fn portable_relative_path_bytes(path: &Path) -> Result<Vec<u8>> {
    let mut encoded = Vec::new();
    for component in path.components() {
        let Component::Normal(component) = component else {
            return Err(Error::Corrupt(format!(
                "backup private path `{}` is not relative and normalized",
                path.display()
            )));
        };
        if !encoded.is_empty() {
            encoded.push(b'/');
        }
        encoded.extend_from_slice(component.as_encoded_bytes());
    }
    if encoded.is_empty() {
        return Err(Error::Corrupt(
            "backup private tree contains an empty relative path".into(),
        ));
    }
    Ok(encoded)
}
