use super::*;
use sha2::{Digest, Sha256};

pub(crate) enum WorkspaceCowMaterializeStatus {
    Cloned,
    Skipped,
    Unavailable,
}

pub(crate) fn materialize_from_workspace_cow(
    workspace_root: &Path,
    output_root: &Path,
    target: &BTreeMap<String, FileEntry>,
    source_stamps: &BTreeMap<String, WorktreeFileStamp>,
    durable: bool,
) -> Result<bool> {
    reject_case_insensitive_collisions(output_root, target)?;
    for (path, entry) in target {
        let Some(source_stamp) = source_stamps.get(path) else {
            return Ok(false);
        };
        match materialize_workspace_file_cow_status_if_stamp_matches(
            workspace_root,
            output_root,
            path,
            entry,
            *source_stamp,
            durable,
        )? {
            WorkspaceCowMaterializeStatus::Cloned => {}
            WorkspaceCowMaterializeStatus::Skipped | WorkspaceCowMaterializeStatus::Unavailable => {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

pub(crate) fn materialize_workspace_file_cow_status_if_matching(
    workspace_root: &Path,
    output_root: &Path,
    path: &str,
    entry: &FileEntry,
) -> Result<WorkspaceCowMaterializeStatus> {
    let source = safe_join(workspace_root, path)?;
    let destination = safe_join(output_root, path)?;
    match fs::symlink_metadata(&destination) {
        Ok(_) => return Ok(WorkspaceCowMaterializeStatus::Skipped),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(Error::Io(err)),
    }
    if !workspace_file_matches_entry(&source, entry)? {
        return Ok(WorkspaceCowMaterializeStatus::Skipped);
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    clone_file_cow_clean(&source, &destination, entry.executable, false)
}

fn materialize_workspace_file_cow_status_if_stamp_matches(
    workspace_root: &Path,
    output_root: &Path,
    path: &str,
    entry: &FileEntry,
    source_stamp: WorktreeFileStamp,
    durable: bool,
) -> Result<WorkspaceCowMaterializeStatus> {
    let source = safe_join(workspace_root, path)?;
    let destination = safe_join(output_root, path)?;
    match fs::symlink_metadata(&destination) {
        Ok(_) => return Ok(WorkspaceCowMaterializeStatus::Skipped),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(Error::Io(err)),
    }
    if !workspace_file_matches_stamp_and_entry(&source, source_stamp, entry)? {
        return Ok(WorkspaceCowMaterializeStatus::Skipped);
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    clone_file_cow_clean(&source, &destination, entry.executable, durable)
}

fn workspace_file_matches_entry(source: &Path, entry: &FileEntry) -> Result<bool> {
    let source_metadata = match fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(Error::Io(err)),
    };
    if source_metadata.file_type().is_symlink() || !source_metadata.is_file() {
        return Ok(false);
    }
    if source_metadata.len() != entry.size_bytes {
        return Ok(false);
    }
    Ok(sha256_file_hex(source)? == entry.content_hash)
}

fn workspace_file_matches_stamp_and_entry(
    source: &Path,
    source_stamp: WorktreeFileStamp,
    entry: &FileEntry,
) -> Result<bool> {
    if source_stamp.size_bytes != entry.size_bytes || source_stamp.executable != entry.executable {
        return Ok(false);
    }
    let source_metadata = match fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(Error::Io(err)),
    };
    if source_metadata.file_type().is_symlink() || !source_metadata.is_file() {
        return Ok(false);
    }
    Ok(WorktreeFileStamp::from_metadata(&source_metadata) == source_stamp)
}

fn sync_cloned_file(path: &Path) -> Result<()> {
    OpenOptions::new().read(true).open(path)?.sync_all()?;
    Ok(())
}

fn clone_file_cow_clean(
    source: &Path,
    destination: &Path,
    executable: bool,
    durable: bool,
) -> Result<WorkspaceCowMaterializeStatus> {
    match cow_clone_file(source, destination) {
        Ok(true) => {
            let result = (|| -> Result<bool> {
                set_executable(destination, executable)?;
                let clean = clear_cloned_xattrs(destination)?;
                if clean && durable {
                    sync_cloned_file(destination)?;
                    if let Some(parent) = destination.parent() {
                        sync_directory(parent);
                    }
                }
                Ok(clean)
            })();
            if !matches!(result, Ok(true)) {
                let _ = fs::remove_file(destination);
            }
            if result? {
                Ok(WorkspaceCowMaterializeStatus::Cloned)
            } else {
                Ok(WorkspaceCowMaterializeStatus::Unavailable)
            }
        }
        Ok(false) => {
            let _ = fs::remove_file(destination);
            Ok(WorkspaceCowMaterializeStatus::Unavailable)
        }
        Err(err) => {
            let _ = fs::remove_file(destination);
            Err(Error::Io(err))
        }
    }
}

fn sha256_file_hex(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
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
