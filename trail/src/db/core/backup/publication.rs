use super::*;
#[cfg(windows)]
use std::thread::sleep;
#[cfg(windows)]
use std::time::Duration;

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
            sync_file_for_publication(entry.path())?;
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
    publish_staged_tree_with_exchange(stage, target, atomic_exchange)
}

fn publish_staged_tree_with_exchange(
    stage: &Path,
    target: &Path,
    exchange: impl FnOnce(&Path, &Path) -> Result<bool>,
) -> Result<Option<PathBuf>> {
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

    if exchange(stage, target)? {
        sync_directory_strict(parent)?;
        test_crash_point("backup_restore_after_atomic_exchange");
        return Ok(Some(stage.to_path_buf()));
    }

    Err(Error::Conflict(
        "atomic directory exchange is unsupported; live tree was not moved".into(),
    ))
}

pub(super) fn remove_retained_tree(path: Option<PathBuf>, parent: &Path) -> Result<()> {
    if let Some(path) = path {
        remove_any(&path)?;
        sync_directory_strict(parent)?;
    }
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
        sync_windows_publication_operation(|| {
            OpenOptions::new()
                .read(true)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(path)
                .and_then(|directory| directory.sync_all())
        })?;
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(windows)]
fn sync_windows_publication_operation(
    mut operation: impl FnMut() -> std::io::Result<()>,
) -> Result<()> {
    let mut delay = Duration::from_millis(1);
    for attempt in 0..4 {
        match operation() {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied && attempt < 3 => {
                sleep(delay);
                delay = (delay * 2).min(Duration::from_millis(32));
            }
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(windows)]
pub(super) fn sync_file_for_publication(path: &Path) -> Result<()> {
    sync_windows_publication_operation(|| {
        OpenOptions::new()
            .read(true)
            .open(path)
            .and_then(|file| file.sync_all())
    })
}

#[cfg(not(windows))]
pub(super) fn sync_file_for_publication(path: &Path) -> Result<()> {
    OpenOptions::new().read(true).open(path)?.sync_all()?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub(super) fn atomic_exchange(left: &Path, right: &Path) -> Result<bool> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_exchange_refuses_before_moving_the_live_tree() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("live");
        let stage = root.path().join("stage");
        fs::create_dir(&target).unwrap();
        fs::create_dir(&stage).unwrap();
        fs::write(target.join("marker"), b"old").unwrap();
        fs::write(stage.join("marker"), b"new").unwrap();

        let result = publish_staged_tree_with_exchange(&stage, &target, |_, _| Ok(false));

        assert!(result.is_err());
        assert_eq!(fs::read(target.join("marker")).unwrap(), b"old");
        assert_eq!(fs::read(stage.join("marker")).unwrap(), b"new");
    }
}

#[cfg(target_os = "linux")]
pub(super) fn atomic_exchange(left: &Path, right: &Path) -> Result<bool> {
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
pub(super) fn atomic_exchange(_left: &Path, _right: &Path) -> Result<bool> {
    Ok(false)
}
