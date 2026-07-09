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
