use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Cross-process reader/writer barrier for one workspace view.
///
/// Filesystem mutations hold a shared lock for the duration of their durable
/// upper/journal transition. Checkpointing holds the exclusive lock while it
/// scans source uppers, advances the lane ref, and writes the clean marker.
/// Operating-system locks are released automatically if a mount or checkpoint
/// process exits abruptly.
pub(crate) struct ViewMutationBarrier {
    file: File,
    checkpoint_sequence: u64,
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
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(meta_dir.join("checkpoint-barrier.lock"))?;
        lock_file(&file, exclusive)?;
        let checkpoint_sequence = read_checkpoint_sequence(&mut file)?;
        Ok(Self {
            file,
            checkpoint_sequence,
        })
    }

    pub(crate) fn checkpoint_sequence(&self) -> u64 {
        self.checkpoint_sequence
    }

    /// Update the epoch observed by future shared-lock holders.
    ///
    /// The caller must already hold the view's exclusive barrier lock. The
    /// file is updated in place so the inode carrying the OS lock is never
    /// replaced.
    pub(crate) fn record_checkpoint_sequence(meta_dir: &Path, sequence: u64) -> io::Result<()> {
        fs::create_dir_all(meta_dir)?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(meta_dir.join("checkpoint-barrier.lock"))?;
        file.seek(SeekFrom::Start(0))?;
        file.set_len(0)?;
        write!(file, "{sequence}\n")?;
        file.sync_data()
    }
}

fn read_checkpoint_sequence(file: &mut File) -> io::Result<u64> {
    file.seek(SeekFrom::Start(0))?;
    let mut value = String::new();
    file.read_to_string(&mut value)?;
    Ok(value.trim().parse::<u64>().unwrap_or(0))
}

impl Drop for ViewMutationBarrier {
    fn drop(&mut self) {
        let _ = unlock_file(&self.file);
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
}
