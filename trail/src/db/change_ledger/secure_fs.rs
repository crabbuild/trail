use std::fs::File;
use std::path::{Component, Path};

use crate::error::{Error, Result};

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

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn unlink_leaf(&self, name: &str) -> Result<bool> {
        use rustix::fs::{unlinkat, AtFlags};

        validate_leaf(name)?;
        match unlinkat(&self.file, Path::new(name), AtFlags::empty()) {
            Ok(()) => Ok(true),
            Err(error) if error == rustix::io::Errno::NOENT => Ok(false),
            Err(error) => Err(Error::Io(error.into())),
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn remove_empty_dir(&self, name: &str) -> Result<bool> {
        use rustix::fs::{unlinkat, AtFlags};

        validate_leaf(name)?;
        match unlinkat(&self.file, Path::new(name), AtFlags::REMOVEDIR) {
            Ok(()) => Ok(true),
            Err(error) if error == rustix::io::Errno::NOENT => Ok(false),
            Err(error) => Err(Error::Io(error.into())),
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn remove_empty_dir(&self, name: &str) -> Result<bool> {
        let _ = (self, name);
        Err(Error::InvalidInput(
            "secure descriptor-relative directory removal is unsupported".into(),
        ))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(crate) fn unlink_verified_regular(&self, name: &str, opened: &File) -> Result<bool> {
        self.verify_opened_regular(name, opened)?;
        self.unlink_leaf(name)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn unlink_verified_regular(&self, name: &str, opened: &File) -> Result<bool> {
        let _ = (self, name, opened);
        Err(Error::InvalidInput(
            "secure descriptor-relative verified unlink is unsupported".into(),
        ))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(crate) fn unlink_leaf(&self, name: &str) -> Result<bool> {
        let _ = (self, name);
        Err(Error::InvalidInput(
            "secure descriptor-relative unlink is unsupported".into(),
        ))
    }

    pub(crate) fn sync(&self) -> Result<()> {
        self.file.sync_all().map_err(Error::Io)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn verify_private(&self) -> Result<()> {
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
fn descriptor_identity(file: &File) -> Result<(u64, u64)> {
    let metadata = rustix::fs::fstat(file).map_err(|error| Error::Io(error.into()))?;
    descriptor_identity_from_stat(metadata)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn descriptor_identity_from_stat(metadata: rustix::fs::Stat) -> Result<(u64, u64)> {
    Ok((
        u64::try_from(metadata.st_dev)
            .map_err(|_| Error::Corrupt("negative filesystem device identity".into()))?,
        u64::try_from(metadata.st_ino)
            .map_err(|_| Error::Corrupt("negative filesystem inode identity".into()))?,
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
