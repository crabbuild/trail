use super::*;

pub(super) fn sibling_stage(target: &Path, label: &str) -> Result<PathBuf> {
    let parent = target
        .parent()
        .ok_or_else(|| Error::InvalidInput("publication target has no parent".into()))?;
    let leaf = target
        .file_name()
        .ok_or_else(|| Error::InvalidInput("publication target has no file name".into()))?
        .to_string_lossy();
    for _ in 0..32 {
        let candidate = parent.join(format!(".{leaf}.{label}-{}", now_nanos()));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(Error::Conflict(
        "could not allocate sibling publication stage".into(),
    ))
}

pub(super) fn sync_tree_bottom_up(root: &Path) -> Result<()> {
    let mut directories = Vec::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| Error::Io(error.into()))?;
        let file_type = entry.file_type();
        if file_type.is_file() {
            OpenOptions::new()
                .read(true)
                .open(entry.path())?
                .sync_all()?;
        } else if file_type.is_dir() {
            directories.push(entry.path().to_path_buf());
        }
    }
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in directories {
        sync_directory_strict(&directory)?;
    }
    Ok(())
}

pub(super) fn publish_staged_tree(stage: &Path, target: &Path) -> Result<Option<PathBuf>> {
    let parent = target
        .parent()
        .ok_or_else(|| Error::InvalidInput("publication target has no parent".into()))?;
    sync_tree_bottom_up(stage)?;
    sync_directory_strict(parent)?;
    test_crash_point("backup_restore_after_staging_sync");

    if !target.exists() {
        fs::rename(stage, target)?;
        sync_directory_strict(parent)?;
        test_crash_point("backup_restore_after_atomic_publish");
        return Ok(None);
    }

    if atomic_exchange(stage, target)? {
        sync_directory_strict(parent)?;
        test_crash_point("backup_restore_after_atomic_exchange");
        return Ok(Some(stage.to_path_buf()));
    }

    let previous = sibling_path(target, "previous")?;
    fs::rename(target, &previous)?;
    sync_directory_strict(parent)?;
    if let Err(error) = fs::rename(stage, target) {
        let _ = fs::rename(&previous, target);
        let _ = sync_directory_strict(parent);
        return Err(error.into());
    }
    sync_directory_strict(parent)?;
    test_crash_point("backup_restore_after_fallback_publish");
    Ok(Some(previous))
}

pub(super) fn remove_retained_tree(path: Option<PathBuf>, parent: &Path) -> Result<()> {
    if let Some(path) = path {
        remove_any(&path)?;
        sync_directory_strict(parent)?;
    }
    Ok(())
}

pub(super) fn rollback_published_tree(target: &Path, retained: Option<PathBuf>) -> Result<()> {
    let Some(retained) = retained else {
        remove_any(target)?;
        if let Some(parent) = target.parent() {
            sync_directory_strict(parent)?;
        }
        return Ok(());
    };
    let parent = target
        .parent()
        .ok_or_else(|| Error::InvalidInput("publication target has no parent".into()))?;
    if atomic_exchange(&retained, target)? {
        sync_directory_strict(parent)?;
        remove_any(&retained)?;
        sync_directory_strict(parent)?;
        return Ok(());
    }
    let failed = sibling_path(target, "failed")?;
    fs::rename(target, &failed)?;
    sync_directory_strict(parent)?;
    if let Err(error) = fs::rename(&retained, target) {
        let _ = fs::rename(&failed, target);
        let _ = sync_directory_strict(parent);
        return Err(error.into());
    }
    sync_directory_strict(parent)?;
    remove_any(&failed)?;
    sync_directory_strict(parent)?;
    Ok(())
}

pub(super) fn remove_any(path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub(super) fn sync_directory_strict(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        OpenOptions::new().read(true).open(path)?.sync_all()?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)?
            .sync_all()?;
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
    }
    Ok(())
}

fn sibling_path(target: &Path, label: &str) -> Result<PathBuf> {
    let parent = target
        .parent()
        .ok_or_else(|| Error::InvalidInput("publication target has no parent".into()))?;
    let leaf = target
        .file_name()
        .ok_or_else(|| Error::InvalidInput("publication target has no file name".into()))?
        .to_string_lossy();
    for _ in 0..32 {
        let candidate = parent.join(format!(".{leaf}.{label}-{}", now_nanos()));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(Error::Conflict(
        "could not allocate retained publication path".into(),
    ))
}

#[cfg(target_os = "macos")]
fn atomic_exchange(left: &Path, right: &Path) -> Result<bool> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let left = CString::new(left.as_os_str().as_bytes())
        .map_err(|_| Error::InvalidInput("publication path contains NUL".into()))?;
    let right = CString::new(right.as_os_str().as_bytes())
        .map_err(|_| Error::InvalidInput("publication path contains NUL".into()))?;
    let result = unsafe { libc::renamex_np(left.as_ptr(), right.as_ptr(), libc::RENAME_SWAP) };
    if result == 0 {
        Ok(true)
    } else {
        let error = std::io::Error::last_os_error();
        if matches!(error.raw_os_error(), Some(code) if code == libc::ENOTSUP || code == libc::EINVAL || code == libc::EXDEV)
        {
            Ok(false)
        } else {
            Err(error.into())
        }
    }
}

#[cfg(target_os = "linux")]
fn atomic_exchange(left: &Path, right: &Path) -> Result<bool> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let left = CString::new(left.as_os_str().as_bytes())
        .map_err(|_| Error::InvalidInput("publication path contains NUL".into()))?;
    let right = CString::new(right.as_os_str().as_bytes())
        .map_err(|_| Error::InvalidInput("publication path contains NUL".into()))?;
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            left.as_ptr(),
            libc::AT_FDCWD,
            right.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    if result == 0 {
        Ok(true)
    } else {
        let error = std::io::Error::last_os_error();
        if matches!(error.raw_os_error(), Some(code) if code == libc::ENOSYS || code == libc::ENOTSUP || code == libc::EINVAL || code == libc::EXDEV)
        {
            Ok(false)
        } else {
            Err(error.into())
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn atomic_exchange(_left: &Path, _right: &Path) -> Result<bool> {
    Ok(false)
}
