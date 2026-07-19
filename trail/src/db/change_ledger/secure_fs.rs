use std::fs::File;
use std::io::{Read, Write};
use std::path::{Component, Path};

use crate::error::{Error, Result};

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
thread_local! {
    static VERIFIED_UNLINK_INNER_WINDOW_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
thread_local! {
    static PRIVATE_DIR_CREATE_OPEN_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) fn install_private_dir_create_open_hook(hook: impl FnOnce() + 'static) {
    PRIVATE_DIR_CREATE_OPEN_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
pub(crate) fn clear_private_dir_create_open_hook() {
    PRIVATE_DIR_CREATE_OPEN_HOOK.with(|slot| slot.borrow_mut().take());
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
fn run_private_dir_create_open_hook() {
    PRIVATE_DIR_CREATE_OPEN_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
fn install_verified_unlink_inner_window_hook(hook: impl FnOnce() + 'static) {
    VERIFIED_UNLINK_INNER_WINDOW_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
fn run_verified_unlink_inner_window_hook() {
    VERIFIED_UNLINK_INNER_WINDOW_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[derive(Debug)]
pub(crate) struct SecureDirectory {
    file: File,
}

impl SecureDirectory {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn open_absolute(path: &Path) -> Result<Self> {
        use rustix::fs::{openat, Mode, OFlags, CWD};

        if !path.is_absolute() {
            return Err(Error::InvalidInput(format!(
                "secure directory `{}` is not absolute",
                path.display()
            )));
        }
        let mut file = File::from(
            openat(
                CWD,
                Path::new("/"),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| Error::Io(error.into()))?,
        );
        for component in path
            .strip_prefix(Path::new("/"))
            .map_err(|error| Error::InvalidInput(error.to_string()))?
            .components()
        {
            let Component::Normal(name) = component else {
                return Err(Error::InvalidInput(format!(
                    "secure directory `{}` is not normalized",
                    path.display()
                )));
            };
            file = File::from(
                openat(
                    &file,
                    Path::new(name),
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(|error| Error::Io(error.into()))?,
            );
        }
        Ok(Self { file })
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn open_absolute(path: &Path) -> Result<Self> {
        Err(Error::InvalidInput(format!(
            "secure descriptor-relative filesystem authority is unsupported for `{}`",
            path.display()
        )))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn open_dir(&self, name: &str) -> Result<Self> {
        use rustix::fs::{openat, Mode, OFlags};

        validate_leaf(name)?;
        let file = File::from(
            openat(
                &self.file,
                Path::new(name),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| Error::Io(error.into()))?,
        );
        verify_entry_identity(&self.file, name, &file, true)?;
        Ok(Self { file })
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn create_private_dir(&self, name: &str) -> Result<Self> {
        use rustix::fs::{mkdirat, Mode};

        validate_leaf(name)?;
        mkdirat(&self.file, Path::new(name), Mode::from_raw_mode(0o700))
            .map_err(|error| Error::Io(error.into()))?;
        #[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
        run_private_dir_create_open_hook();
        let directory = self.open_dir(name)?;
        directory.verify_private()?;
        Ok(directory)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn create_private_dir(&self, name: &str) -> Result<Self> {
        let _ = (self, name);
        Err(Error::InvalidInput(
            "secure private directory creation is unsupported".into(),
        ))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn open_private_dir(&self, name: &str) -> Result<Self> {
        let directory = self.open_dir(name)?;
        directory.verify_private()?;
        Ok(directory)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn open_private_dir(&self, name: &str) -> Result<Self> {
        let _ = (self, name);
        Err(Error::InvalidInput(
            "secure private directory open is unsupported".into(),
        ))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn open_dir(&self, name: &str) -> Result<Self> {
        let _ = (self, name);
        Err(Error::InvalidInput(
            "secure descriptor-relative directory traversal is unsupported".into(),
        ))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn open_regular(&self, name: &str) -> Result<File> {
        use rustix::fs::{openat, Mode, OFlags};

        validate_leaf(name)?;
        let file = File::from(
            openat(
                &self.file,
                Path::new(name),
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| Error::Io(error.into()))?,
        );
        verify_entry_identity(&self.file, name, &file, false)?;
        Ok(file)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn read_regular_optional_bounded(
        &self,
        name: &str,
        max_bytes: u64,
    ) -> Result<Option<Vec<u8>>> {
        let mut file = match self.open_regular(name) {
            Ok(file) => file,
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        if file.metadata()?.len() > max_bytes {
            return Err(Error::InvalidInput(format!(
                "secure marker `{name}` exceeds {max_bytes} bytes"
            )));
        }
        let mut bytes = Vec::new();
        std::io::Read::by_ref(&mut file)
            .take(max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_bytes {
            return Err(Error::InvalidInput(format!(
                "secure marker `{name}` grew beyond {max_bytes} bytes"
            )));
        }
        self.verify_opened_regular(name, &file)?;
        Ok(Some(bytes))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn read_regular_optional_bounded(
        &self,
        name: &str,
        max_bytes: u64,
    ) -> Result<Option<Vec<u8>>> {
        let _ = (self, name, max_bytes);
        Err(Error::InvalidInput(
            "secure descriptor-relative marker read is unsupported".into(),
        ))
    }

    /// Atomically replace a regular leaf without following either the
    /// temporary or destination pathname. The pinned directory descriptor is
    /// the sole rename authority.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn write_atomic_regular(&self, name: &str, bytes: &[u8]) -> Result<()> {
        use rustix::fs::{openat, renameat, Mode, OFlags};

        validate_leaf(name)?;
        let mut random = [0_u8; 12];
        getrandom::getrandom(&mut random)
            .map_err(|error| Error::Io(std::io::Error::other(error.to_string())))?;
        let temporary = format!(".marker-{}.tmp", hex::encode(random));
        let mut file = File::from(
            openat(
                &self.file,
                Path::new(&temporary),
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )
            .map_err(|error| Error::Io(error.into()))?,
        );
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            renameat(
                &self.file,
                Path::new(&temporary),
                &self.file,
                Path::new(name),
            )
            .map_err(|error| Error::Io(error.into()))?;
            self.sync()
        })();
        if result.is_err() {
            let _ = rustix::fs::unlinkat(
                &self.file,
                Path::new(&temporary),
                rustix::fs::AtFlags::empty(),
            );
        }
        result
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn write_atomic_regular(&self, name: &str, bytes: &[u8]) -> Result<()> {
        let _ = (self, name, bytes);
        Err(Error::InvalidInput(
            "secure descriptor-relative marker write is unsupported".into(),
        ))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn remove_leaf(&self, name: &str) -> Result<()> {
        validate_leaf(name)?;
        match rustix::fs::unlinkat(&self.file, Path::new(name), rustix::fs::AtFlags::empty()) {
            Ok(()) => self.sync(),
            Err(error) if error == rustix::io::Errno::NOENT => Ok(()),
            Err(error) => Err(Error::Io(error.into())),
        }
    }

    /// Remove one directory tree using only descriptor-relative traversal.
    /// The caller must first authenticate this directory and the child inode
    /// against persisted ownership authority.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn remove_owned_tree_leaf(
        &self,
        name: &str,
        expected_identity: (u64, u64),
    ) -> Result<()> {
        validate_leaf(name)?;
        let child = self.open_dir(name)?;
        child.verify_identity(expected_identity)?;
        child.remove_owned_tree_contents()?;
        // Reopen through the pinned parent immediately before rmdir so a
        // pathname substitution cannot redirect deletion to another inode.
        self.open_dir(name)?.verify_identity(expected_identity)?;
        rustix::fs::unlinkat(&self.file, Path::new(name), rustix::fs::AtFlags::REMOVEDIR)
            .map_err(|error| Error::Io(error.into()))?;
        self.sync()
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn remove_owned_tree_contents(&self) -> Result<()> {
        for name in self.entry_names()? {
            let name = name.to_str().ok_or_else(|| {
                Error::InvalidInput("owned materialization tree has a non-UTF8 leaf".into())
            })?;
            match self.open_dir(name) {
                Ok(directory) => {
                    let identity = directory.identity()?;
                    directory.remove_owned_tree_contents()?;
                    self.open_dir(name)?.verify_identity(identity)?;
                    rustix::fs::unlinkat(
                        &self.file,
                        Path::new(name),
                        rustix::fs::AtFlags::REMOVEDIR,
                    )
                    .map_err(|error| Error::Io(error.into()))?;
                }
                Err(Error::Io(error)) if error.raw_os_error() == Some(libc::ENOTDIR) => {
                    let opened = self.open_regular(name)?;
                    self.verify_opened_regular(name, &opened)?;
                    rustix::fs::unlinkat(&self.file, Path::new(name), rustix::fs::AtFlags::empty())
                        .map_err(|error| Error::Io(error.into()))?;
                }
                Err(error) => return Err(error),
            }
        }
        self.sync()
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn remove_owned_tree_leaf(
        &self,
        name: &str,
        expected_identity: (u64, u64),
    ) -> Result<()> {
        let _ = (self, name, expected_identity);
        Err(Error::InvalidInput(
            "secure owned-tree removal is unsupported".into(),
        ))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn remove_leaf(&self, name: &str) -> Result<()> {
        let _ = (self, name);
        Err(Error::InvalidInput(
            "secure descriptor-relative marker removal is unsupported".into(),
        ))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn open_regular(&self, name: &str) -> Result<File> {
        let _ = (self, name);
        Err(Error::InvalidInput(
            "secure descriptor-relative file open is unsupported".into(),
        ))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn entry_names(&self) -> Result<Vec<std::ffi::OsString>> {
        use std::os::unix::ffi::OsStrExt;

        let mut directory =
            rustix::fs::Dir::read_from(&self.file).map_err(|error| Error::Io(error.into()))?;
        let mut names = Vec::new();
        while let Some(entry) = directory.read() {
            let entry = entry.map_err(|error| Error::Io(error.into()))?;
            let bytes = entry.file_name().to_bytes();
            if matches!(bytes, b"." | b"..") {
                continue;
            }
            names.push(std::ffi::OsStr::from_bytes(bytes).to_os_string());
        }
        Ok(names)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn entry_names(&self) -> Result<Vec<std::ffi::OsString>> {
        let _ = self;
        Err(Error::InvalidInput(
            "secure descriptor-relative directory enumeration is unsupported".into(),
        ))
    }

    pub(crate) fn file(&self) -> &File {
        &self.file
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn identity(&self) -> Result<(u64, u64)> {
        descriptor_identity(&self.file)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn identity(&self) -> Result<(u64, u64)> {
        let _ = self;
        Err(Error::InvalidInput(
            "secure directory identity is unsupported".into(),
        ))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn verify_identity(&self, expected: (u64, u64)) -> Result<()> {
        if self.identity()? != expected {
            return Err(Error::InvalidInput(
                "secure directory identity does not match persisted authority".into(),
            ));
        }
        Ok(())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn verify_identity(&self, expected: (u64, u64)) -> Result<()> {
        let _ = (self, expected);
        Err(Error::InvalidInput(
            "secure directory identity validation is unsupported".into(),
        ))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn verify_opened_regular(&self, name: &str, opened: &File) -> Result<()> {
        validate_leaf(name)?;
        verify_entry_identity(&self.file, name, opened, false)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn verify_same_opened_regular(
        &self,
        expected: &File,
        observed: &File,
    ) -> Result<()> {
        use rustix::fs::{fstat, FileType};

        let expected = fstat(expected).map_err(|error| Error::Io(error.into()))?;
        let observed = fstat(observed).map_err(|error| Error::Io(error.into()))?;
        if FileType::from_raw_mode(expected.st_mode) != FileType::RegularFile
            || FileType::from_raw_mode(observed.st_mode) != FileType::RegularFile
            || expected.st_dev != observed.st_dev
            || expected.st_ino != observed.st_ino
        {
            return Err(Error::InvalidInput(
                "secure filesystem quarantine inode does not match deletion authority".into(),
            ));
        }
        Ok(())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn verify_same_opened_regular(
        &self,
        expected: &File,
        observed: &File,
    ) -> Result<()> {
        let _ = (self, expected, observed);
        Err(Error::InvalidInput(
            "secure descriptor-relative inode comparison is unsupported".into(),
        ))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn rename_leaf_noreplace(&self, source: &str, destination: &str) -> Result<()> {
        use rustix::fs::{renameat_with, RenameFlags};

        validate_leaf(source)?;
        validate_leaf(destination)?;
        renameat_with(
            &self.file,
            Path::new(source),
            &self.file,
            Path::new(destination),
            RenameFlags::NOREPLACE,
        )
        .map_err(|error| Error::Io(error.into()))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn rename_leaf_to_noreplace(
        &self,
        source: &str,
        destination_directory: &Self,
        destination: &str,
    ) -> Result<()> {
        use rustix::fs::{renameat_with, RenameFlags};

        validate_leaf(source)?;
        validate_leaf(destination)?;
        renameat_with(
            &self.file,
            Path::new(source),
            &destination_directory.file,
            Path::new(destination),
            RenameFlags::NOREPLACE,
        )
        .map_err(|error| Error::Io(error.into()))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn rename_leaf_to_noreplace(
        &self,
        source: &str,
        destination_directory: &Self,
        destination: &str,
    ) -> Result<()> {
        let _ = (self, source, destination_directory, destination);
        Err(Error::InvalidInput(
            "secure descriptor-relative cross-directory rename is unsupported".into(),
        ))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn rename_leaf_noreplace(&self, source: &str, destination: &str) -> Result<()> {
        let _ = (self, source, destination);
        Err(Error::InvalidInput(
            "secure descriptor-relative quarantine rename is unsupported".into(),
        ))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn verify_opened_regular(&self, name: &str, opened: &File) -> Result<()> {
        let _ = (self, name, opened);
        Err(Error::InvalidInput(
            "secure descriptor-relative validation is unsupported".into(),
        ))
    }

    #[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn unlink_verified_regular(&self, name: &str, opened: &File) -> Result<bool> {
        self.verify_opened_regular(name, opened)?;
        run_verified_unlink_inner_window_hook();
        Err(Error::InvalidInput(
            "verified pathname unlink has no inode-bound POSIX authority".into(),
        ))
    }

    pub(crate) fn sync(&self) -> Result<()> {
        self.file.sync_all().map_err(Error::Io)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn restrict_private(&self) -> Result<()> {
        rustix::fs::fchmod(&self.file, rustix::fs::Mode::from_raw_mode(0o700))
            .map_err(|error| Error::Io(error.into()))?;
        self.verify_private()
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn restrict_private(&self) -> Result<()> {
        let _ = self;
        Err(Error::InvalidInput(
            "secure private marker directory is unsupported".into(),
        ))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn verify_private(&self) -> Result<()> {
        use rustix::fs::{fstat, FileType};

        let metadata = fstat(&self.file).map_err(|error| Error::Io(error.into()))?;
        if FileType::from_raw_mode(metadata.st_mode) != FileType::Directory
            || metadata.st_mode & 0o777 != 0o700
            || metadata.st_uid != rustix::process::geteuid().as_raw()
        {
            return Err(Error::InvalidInput(
                "secure quarantine directory is not private".into(),
            ));
        }
        Ok(())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn verify_private(&self) -> Result<()> {
        let _ = self;
        Err(Error::InvalidInput(
            "secure private marker directory is unsupported".into(),
        ))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn file_identity(file: &File) -> Result<(u64, u64)> {
    use rustix::fs::{fstat, FileType};

    let metadata = fstat(file).map_err(|error| Error::Io(error.into()))?;
    if FileType::from_raw_mode(metadata.st_mode) != FileType::RegularFile {
        return Err(Error::InvalidInput(
            "secure filesystem deletion authority is not a regular file".into(),
        ));
    }
    descriptor_identity_from_stat(metadata)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn lock_observer_writer(file: &File) -> Result<()> {
    rustix::fs::flock(file, rustix::fs::FlockOperation::NonBlockingLockExclusive)
        .map_err(|error| Error::Io(error.into()))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) fn lock_observer_writer(file: &File) -> Result<()> {
    let _ = file;
    Err(Error::InvalidInput(
        "observer writer descriptor locking is unsupported".into(),
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn try_lock_observer_quiescence(file: &File) -> Result<()> {
    rustix::fs::flock(file, rustix::fs::FlockOperation::NonBlockingLockExclusive)
        .map_err(|error| Error::Io(error.into()))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) fn try_lock_observer_quiescence(file: &File) -> Result<()> {
    let _ = file;
    Err(Error::InvalidInput(
        "observer quiescence descriptor locking is unsupported".into(),
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn descriptor_identity(file: &File) -> Result<(u64, u64)> {
    let metadata = rustix::fs::fstat(file).map_err(|error| Error::Io(error.into()))?;
    descriptor_identity_from_stat(metadata)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn descriptor_identity_from_stat(metadata: rustix::fs::Stat) -> Result<(u64, u64)> {
    Ok((
        u64::try_from(metadata.st_dev)
            .map_err(|_| Error::Corrupt("negative filesystem device identity".into()))?,
        metadata.st_ino,
    ))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) fn file_identity(file: &File) -> Result<(u64, u64)> {
    let _ = file;
    Err(Error::InvalidInput(
        "secure filesystem file identity is unsupported".into(),
    ))
}

fn validate_leaf(name: &str) -> Result<()> {
    let mut components = Path::new(name).components();
    if !matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    ) || name.contains(['/', '\0'])
    {
        return Err(Error::InvalidInput(format!(
            "secure filesystem leaf is not confined: `{name}`"
        )));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn verify_entry_identity(parent: &File, name: &str, opened: &File, directory: bool) -> Result<()> {
    use rustix::fs::{fstat, statat, AtFlags, FileType};

    let path_stat = statat(parent, Path::new(name), AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| Error::Io(error.into()))?;
    let opened_stat = fstat(opened).map_err(|error| Error::Io(error.into()))?;
    let expected_type = if directory {
        FileType::Directory
    } else {
        FileType::RegularFile
    };
    if FileType::from_raw_mode(path_stat.st_mode) != expected_type
        || FileType::from_raw_mode(opened_stat.st_mode) != expected_type
        || path_stat.st_dev != opened_stat.st_dev
        || path_stat.st_ino != opened_stat.st_ino
    {
        return Err(Error::InvalidInput(format!(
            "secure filesystem entry changed while opening `{name}`"
        )));
    }
    Ok(())
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::{install_verified_unlink_inner_window_hook, SecureDirectory};

    #[test]
    fn verified_unlink_never_deletes_an_inner_window_replacement() {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let original = root_path.join("segment.cplq");
        let retained = root_path.join("authorized.retained.cplq");
        std::fs::write(&original, b"authenticated original\n").unwrap();
        let directory = SecureDirectory::open_absolute(&root_path).unwrap();
        let opened = directory.open_regular("segment.cplq").unwrap();
        let hook_original = original.clone();
        let hook_retained = retained.clone();
        install_verified_unlink_inner_window_hook(move || {
            std::fs::rename(&hook_original, &hook_retained).unwrap();
            std::fs::write(&hook_original, b"hostile replacement\n").unwrap();
        });

        let result = directory.unlink_verified_regular("segment.cplq", &opened);

        assert!(
            retained.exists(),
            "the authenticated inode was not retained"
        );
        assert!(
            original.exists(),
            "pathname unlink deleted the inner-window replacement"
        );
        assert!(
            result.is_err(),
            "verified pathname unlink must fail closed when no inode-bound primitive exists"
        );
    }
}
