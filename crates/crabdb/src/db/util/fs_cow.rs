use super::*;

pub(crate) fn materialize_from_workspace_cow(
    workspace_root: &Path,
    output_root: &Path,
    target: &BTreeMap<String, FileEntry>,
) -> Result<bool> {
    reject_case_insensitive_collisions(output_root, target)?;
    for (path, entry) in target {
        let source = safe_join(workspace_root, path)?;
        let destination = safe_join(output_root, path)?;
        let source_metadata = match fs::symlink_metadata(&source) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(err) => return Err(Error::Io(err)),
        };
        if source_metadata.file_type().is_symlink() || !source_metadata.is_file() {
            return Ok(false);
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        match cow_clone_file(&source, &destination) {
            Ok(true) => {
                set_executable(&destination, entry.executable)?;
                if !clear_cloned_xattrs(&destination)? {
                    return Ok(false);
                }
            }
            Ok(false) => return Ok(false),
            Err(err) => return Err(Error::Io(err)),
        }
    }
    Ok(true)
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn cow_clone_file(source: &Path, destination: &Path) -> std::io::Result<bool> {
    use rustix::fs::{fclonefileat, CloneFlags};

    let source_file = fs::File::open(source)?;
    let parent = destination.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "CoW clone destination has no parent",
        )
    })?;
    let file_name = destination.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "CoW clone destination has no file name",
        )
    })?;
    let parent_dir = fs::File::open(parent)?;
    match fclonefileat(
        &source_file,
        &parent_dir,
        file_name,
        CloneFlags::NOFOLLOW | CloneFlags::NOOWNERCOPY,
    ) {
        Ok(()) => Ok(true),
        Err(err) if cow_clone_unavailable(err) => Ok(false),
        Err(err) => Err(err.into()),
    }
}

#[cfg(all(
    target_os = "linux",
    not(any(target_arch = "sparc", target_arch = "sparc64"))
))]
fn cow_clone_file(source: &Path, destination: &Path) -> std::io::Result<bool> {
    use rustix::fs::ioctl_ficlone;

    let source_file = fs::File::open(source)?;
    let destination_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    match ioctl_ficlone(&destination_file, &source_file) {
        Ok(()) => Ok(true),
        Err(err) if cow_clone_unavailable(err) => Ok(false),
        Err(err) => Err(err.into()),
    }
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    all(
        target_os = "linux",
        not(any(target_arch = "sparc", target_arch = "sparc64"))
    )
)))]
fn cow_clone_file(_source: &Path, _destination: &Path) -> std::io::Result<bool> {
    Ok(false)
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    all(
        target_os = "linux",
        not(any(target_arch = "sparc", target_arch = "sparc64"))
    )
))]
fn clear_cloned_xattrs(path: &Path) -> std::io::Result<bool> {
    use std::ffi::CString;

    use rustix::fs::{listxattr, removexattr};

    let mut empty: [u8; 0] = [];
    let size = match listxattr(path, &mut empty) {
        Ok(size) => size,
        Err(err) if cow_clone_unavailable(err) => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    if size == 0 {
        return Ok(true);
    }

    let mut names = vec![0; size];
    let size = match listxattr(path, &mut names) {
        Ok(size) => size,
        Err(err) if cow_clone_unavailable(err) => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    names.truncate(size);
    for name in names
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
    {
        let name = CString::new(name).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid xattr name: {err}"),
            )
        })?;
        match removexattr(path, name.as_c_str()) {
            Ok(()) => {}
            Err(err) if cow_clone_unavailable(err) => return Ok(false),
            Err(err) => return Err(err.into()),
        }
    }
    Ok(true)
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    all(
        target_os = "linux",
        not(any(target_arch = "sparc", target_arch = "sparc64"))
    )
)))]
fn clear_cloned_xattrs(_path: &Path) -> std::io::Result<bool> {
    Ok(true)
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    all(
        target_os = "linux",
        not(any(target_arch = "sparc", target_arch = "sparc64"))
    )
))]
fn cow_clone_unavailable(err: rustix::io::Errno) -> bool {
    matches!(
        err,
        rustix::io::Errno::NOTSUP
            | rustix::io::Errno::OPNOTSUPP
            | rustix::io::Errno::NOSYS
            | rustix::io::Errno::XDEV
            | rustix::io::Errno::INVAL
            | rustix::io::Errno::PERM
            | rustix::io::Errno::ACCESS
    )
}
