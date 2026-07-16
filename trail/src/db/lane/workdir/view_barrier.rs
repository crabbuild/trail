use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Cross-process reader/writer barrier for one workspace view.
///
/// Filesystem mutations hold a shared lock for the duration of their durable
/// upper/journal transition. Checkpointing holds the exclusive lock while it
/// scans source uppers, advances the lane ref, and writes the clean marker.
/// Operating-system locks are released automatically if a mount or checkpoint
/// process exits abruptly.
pub(crate) struct ViewMutationBarrier {
    authority: File,
    file: File,
    meta_dir: PathBuf,
    path: PathBuf,
    exclusive: bool,
    checkpoint_sequence: u64,
    checkpoint_generation: u64,
}

impl ViewMutationBarrier {
    pub(crate) fn shared(meta_dir: &Path) -> io::Result<Self> {
        Self::acquire(meta_dir, false)
    }

    pub(crate) fn exclusive(meta_dir: &Path) -> io::Result<Self> {
        Self::acquire(meta_dir, true)
    }

    fn acquire(meta_dir: &Path, exclusive: bool) -> io::Result<Self> {
        fs::create_dir_all(meta_dir)?;
        if !fs::symlink_metadata(meta_dir)?.file_type().is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "workspace view metadata path is not a real directory",
            ));
        }
        let path = meta_dir.join("checkpoint-barrier.lock");
        let authority = open_barrier_authority(meta_dir)?;
        lock_file(&authority, exclusive)?;
        validate_authority_identity(meta_dir, &authority)?;
        let mut file = open_barrier_child(&authority, &path)?;
        validate_barrier_child_identity(&authority, &path, &file)?;
        validate_or_initialize_barrier_identity(meta_dir, &authority, &file)?;
        let (checkpoint_sequence, checkpoint_generation) = read_checkpoint_cut(&mut file)?;
        validate_authority_identity(meta_dir, &authority)?;
        validate_barrier_child_identity(&authority, &path, &file)?;
        Ok(Self {
            authority,
            file,
            meta_dir: meta_dir.to_path_buf(),
            path,
            exclusive,
            checkpoint_sequence,
            checkpoint_generation,
        })
    }

    pub(crate) fn checkpoint_sequence(&self) -> u64 {
        self.checkpoint_sequence
    }

    pub(crate) fn checkpoint_generation(&self) -> u64 {
        self.checkpoint_generation
    }

    pub(crate) fn validate(&self) -> io::Result<()> {
        validate_authority_identity(&self.meta_dir, &self.authority)?;
        validate_barrier_child_identity(&self.authority, &self.path, &self.file)?;
        validate_or_initialize_barrier_identity(&self.meta_dir, &self.authority, &self.file)
    }

    /// Update the epoch observed by future shared-lock holders.
    ///
    /// The caller must already hold the view's exclusive barrier lock. The
    /// file is updated in place so the inode carrying the OS lock is never
    /// replaced.
    pub(crate) fn record_checkpoint_cut(
        &mut self,
        sequence: u64,
        generation: u64,
    ) -> io::Result<()> {
        if !self.exclusive {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "checkpoint cut requires the held exclusive view barrier",
            ));
        }
        self.validate()?;
        self.file.seek(SeekFrom::Start(0))?;
        self.file.set_len(0)?;
        write!(self.file, "{sequence} {generation}\n")?;
        self.file.sync_data()?;
        self.validate()?;
        self.checkpoint_sequence = sequence;
        self.checkpoint_generation = generation;
        Ok(())
    }
}

#[cfg(unix)]
fn open_barrier_authority(meta_dir: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let authority = options.open(meta_dir)?;
    let metadata = authority.metadata()?;
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "workspace view barrier authority is not a directory",
        ));
    }
    validate_authority_identity(meta_dir, &authority)?;
    Ok(authority)
}

#[cfg(windows)]
fn open_barrier_authority(meta_dir: &Path) -> io::Result<File> {
    open_barrier_no_follow(&meta_dir.join("checkpoint-barrier.authority.lock"))
}

#[cfg(unix)]
fn validate_authority_identity(path: &Path, authority: &File) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;

    let published = fs::symlink_metadata(path)?;
    let held = authority.metadata()?;
    if !published.file_type().is_dir()
        || held.dev() != published.dev()
        || held.ino() != published.ino()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "workspace view barrier authority directory was replaced",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn validate_authority_identity(_path: &Path, _authority: &File) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn open_regular_at(
    authority: &File,
    name: &str,
    read: bool,
    write: bool,
    create: bool,
    exclusive: bool,
) -> io::Result<File> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};

    let name = CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid barrier child name"))?;
    let mut flags = libc::O_NOFOLLOW | libc::O_CLOEXEC;
    flags |= match (read, write) {
        (true, true) => libc::O_RDWR,
        (false, true) => libc::O_WRONLY,
        _ => libc::O_RDONLY,
    };
    if create {
        flags |= libc::O_CREAT;
    }
    if exclusive {
        flags |= libc::O_EXCL;
    }
    let fd = unsafe { libc::openat(authority.as_raw_fd(), name.as_ptr(), flags, 0o600) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file.metadata()?;
    use std::os::unix::fs::MetadataExt;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "workspace view barrier child is unsafe",
        ));
    }
    Ok(file)
}

#[cfg(unix)]
fn open_barrier_child(authority: &File, _path: &Path) -> io::Result<File> {
    open_regular_at(
        authority,
        "checkpoint-barrier.lock",
        true,
        true,
        true,
        false,
    )
}

#[cfg(windows)]
fn open_barrier_child(_authority: &File, path: &Path) -> io::Result<File> {
    open_barrier_no_follow(path)
}

#[cfg(unix)]
fn validate_barrier_child_identity(authority: &File, _path: &Path, held: &File) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;

    let published = open_regular_at(
        authority,
        "checkpoint-barrier.lock",
        true,
        false,
        false,
        false,
    )?;
    let published = published.metadata()?;
    let held = held.metadata()?;
    if published.dev() != held.dev() || published.ino() != held.ino() || held.nlink() != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "workspace view barrier pathname changed under its pinned authority",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn validate_barrier_child_identity(_authority: &File, path: &Path, held: &File) -> io::Result<()> {
    validate_barrier_identity(path, held)
}

#[cfg(unix)]
fn validate_or_initialize_barrier_identity(
    _meta_dir: &Path,
    authority: &File,
    file: &File,
) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata()?;
    let expected = format!("{} {}\n", metadata.dev(), metadata.ino());
    let init_lock = open_regular_at(
        authority,
        "checkpoint-barrier.identity-init.lock",
        true,
        true,
        true,
        false,
    )?;
    lock_file(&init_lock, true)?;
    let result = match open_regular_at(
        authority,
        "checkpoint-barrier.identity",
        false,
        true,
        true,
        true,
    ) {
        Ok(mut identity) => {
            identity.write_all(expected.as_bytes())?;
            identity.sync_all()?;
            authority.sync_all()?;
            Ok(())
        }
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            let mut identity = open_regular_at(
                authority,
                "checkpoint-barrier.identity",
                true,
                false,
                false,
                false,
            )?;
            let identity_metadata = identity.metadata()?;
            if !identity_metadata.is_file() || identity_metadata.nlink() != 1 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "workspace view barrier identity is unsafe",
                ));
            }
            let mut observed = String::new();
            identity.read_to_string(&mut observed)?;
            if observed != expected {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "workspace view barrier identity does not match its authority",
                ));
            }
            Ok(())
        }
        Err(err) => Err(err),
    };
    let _ = unlock_file(&init_lock);
    result
}

#[cfg(windows)]
fn validate_or_initialize_barrier_identity(
    _meta_dir: &Path,
    _authority: &File,
    _file: &File,
) -> io::Result<()> {
    Ok(())
}

#[cfg(windows)]
fn open_barrier_no_follow(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        options.mode(0o600);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "workspace view barrier is not a regular file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "workspace view barrier has an unsafe hard-link count",
            ));
        }
    }
    Ok(file)
}

#[cfg(windows)]
fn validate_barrier_identity(path: &Path, file: &File) -> io::Result<()> {
    let path_metadata = fs::symlink_metadata(path)?;
    if !path_metadata.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "workspace view barrier pathname was replaced",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let held = file.metadata()?;
        if held.dev() != path_metadata.dev() || held.ino() != path_metadata.ino() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "workspace view barrier pathname changed while acquiring its lock",
            ));
        }
    }
    Ok(())
}

fn read_checkpoint_cut(file: &mut File) -> io::Result<(u64, u64)> {
    file.seek(SeekFrom::Start(0))?;
    let mut value = String::new();
    file.read_to_string(&mut value)?;
    if value.trim().is_empty() {
        return Ok((0, 0));
    }
    let mut fields = value.split_whitespace();
    let sequence = fields
        .next()
        .and_then(|field| field.parse::<u64>().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid checkpoint sequence"))?;
    let generation = fields
        .next()
        .and_then(|field| field.parse::<u64>().ok())
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "invalid checkpoint generation")
        })?;
    if fields.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "checkpoint cut has trailing fields",
        ));
    }
    Ok((sequence, generation))
}

impl Drop for ViewMutationBarrier {
    fn drop(&mut self) {
        let _ = unlock_file(&self.authority);
    }
}

#[cfg(unix)]
fn lock_file(file: &File, exclusive: bool) -> io::Result<()> {
    use rustix::fs::{flock, FlockOperation};

    flock(
        file,
        if exclusive {
            FlockOperation::LockExclusive
        } else {
            FlockOperation::LockShared
        },
    )
    .map_err(io::Error::from)
}

#[cfg(unix)]
fn unlock_file(file: &File) -> io::Result<()> {
    rustix::fs::flock(file, rustix::fs::FlockOperation::Unlock).map_err(io::Error::from)
}

#[cfg(windows)]
fn lock_file(file: &File, exclusive: bool) -> io::Result<()> {
    use std::mem::zeroed;
    use std::os::windows::io::AsRawHandle;
    use winapi::shared::minwindef::{DWORD, FALSE};
    use winapi::um::fileapi::LockFileEx;
    use winapi::um::minwinbase::{LOCKFILE_EXCLUSIVE_LOCK, OVERLAPPED};

    let mut overlapped: OVERLAPPED = unsafe { zeroed() };
    let flags: DWORD = if exclusive {
        LOCKFILE_EXCLUSIVE_LOCK
    } else {
        0
    };
    let locked = unsafe {
        LockFileEx(
            file.as_raw_handle().cast(),
            flags,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    };
    if locked == FALSE {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn unlock_file(file: &File) -> io::Result<()> {
    use std::mem::zeroed;
    use std::os::windows::io::AsRawHandle;
    use winapi::shared::minwindef::FALSE;
    use winapi::um::fileapi::UnlockFileEx;
    use winapi::um::minwinbase::OVERLAPPED;

    let mut overlapped: OVERLAPPED = unsafe { zeroed() };
    let unlocked = unsafe {
        UnlockFileEx(
            file.as_raw_handle().cast(),
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    };
    if unlocked == FALSE {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn exclusive_checkpoint_waits_for_mutation_and_blocks_new_mutations() {
        let temp = tempfile::tempdir().unwrap();
        let shared = ViewMutationBarrier::shared(temp.path()).unwrap();
        let path = temp.path().to_path_buf();
        let (exclusive_started, exclusive_waiting) = mpsc::channel();
        let (exclusive_acquired, acquired) = mpsc::channel();
        let checkpoint = thread::spawn(move || {
            exclusive_started.send(()).unwrap();
            let guard = ViewMutationBarrier::exclusive(&path).unwrap();
            exclusive_acquired.send(()).unwrap();
            thread::sleep(Duration::from_millis(100));
            drop(guard);
        });
        exclusive_waiting.recv().unwrap();
        assert!(acquired.recv_timeout(Duration::from_millis(100)).is_err());
        drop(shared);
        acquired.recv_timeout(Duration::from_secs(2)).unwrap();

        let path = temp.path().to_path_buf();
        let exclusive = ViewMutationBarrier::exclusive(temp.path()).unwrap();
        let (shared_acquired, acquired) = mpsc::channel();
        let mutation = thread::spawn(move || {
            let guard = ViewMutationBarrier::shared(&path).unwrap();
            shared_acquired.send(()).unwrap();
            drop(guard);
        });
        assert!(acquired.recv_timeout(Duration::from_millis(100)).is_err());
        drop(exclusive);
        acquired.recv_timeout(Duration::from_secs(2)).unwrap();
        mutation.join().unwrap();
        checkpoint.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn checkpoint_cut_rejects_path_aba_and_never_follows_replacement_symlink() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let mut barrier = ViewMutationBarrier::exclusive(temp.path()).unwrap();
        let path = temp.path().join("checkpoint-barrier.lock");
        let held = temp.path().join("checkpoint-barrier.held");
        let victim = temp.path().join("victim.txt");
        fs::write(&victim, b"preserve me").unwrap();
        fs::rename(&path, &held).unwrap();
        symlink(&victim, &path).unwrap();

        assert!(barrier.record_checkpoint_cut(7, 3).is_err());
        assert_eq!(fs::read(victim).unwrap(), b"preserve me");
        drop(barrier);
        assert!(ViewMutationBarrier::shared(temp.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn checkpoint_revalidates_pinned_meta_directory_after_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let meta_dir = temp.path().join("meta");
        let held_dir = temp.path().join("meta-held");
        let mut barrier = ViewMutationBarrier::exclusive(&meta_dir).unwrap();
        fs::rename(&meta_dir, &held_dir).unwrap();
        fs::create_dir(&meta_dir).unwrap();

        assert!(barrier.record_checkpoint_cut(9, 4).is_err());
        assert_eq!(
            fs::read(held_dir.join("checkpoint-barrier.lock")).unwrap(),
            b""
        );
        assert!(!meta_dir.join("checkpoint-barrier.lock").exists());
    }

    #[cfg(unix)]
    #[test]
    fn replacement_exclusive_cannot_split_authority_from_old_shared_holder() {
        let temp = tempfile::tempdir().unwrap();
        let shared = ViewMutationBarrier::shared(temp.path()).unwrap();
        let path = temp.path().join("checkpoint-barrier.lock");
        let held = temp.path().join("checkpoint-barrier.held");
        fs::rename(&path, &held).unwrap();
        fs::write(&path, b"0 0\n").unwrap();

        let meta_dir = temp.path().to_path_buf();
        let (attempted_tx, attempted_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let checkpoint = thread::spawn(move || {
            attempted_tx.send(()).unwrap();
            finished_tx
                .send(ViewMutationBarrier::exclusive(&meta_dir).is_ok())
                .unwrap();
        });
        attempted_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(finished_rx
            .recv_timeout(Duration::from_millis(150))
            .is_err());

        drop(shared);
        assert!(!finished_rx.recv_timeout(Duration::from_secs(2)).unwrap());
        checkpoint.join().unwrap();
    }
}
