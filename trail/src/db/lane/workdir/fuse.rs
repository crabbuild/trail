use super::*;

#[cfg(any(target_os = "linux", all(target_os = "macos", feature = "macfuse")))]
mod fuse_overlay {
    use super::*;
    use fuser::{
        FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyCreate, ReplyData,
        ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyWrite, Request,
    };
    use libc::{EINVAL, ENOENT, ENOTDIR, O_ACCMODE, O_RDWR, O_TRUNC, O_WRONLY};
    use std::collections::HashMap;
    use std::ffi::OsStr;
    use std::fs::{self, File, OpenOptions};
    use std::os::unix::fs::FileExt;
    use std::path::{Path, PathBuf};
    #[cfg(target_os = "macos")]
    use std::process::{Command, Stdio};
    #[cfg(target_os = "macos")]
    use std::time::Instant;
    use std::time::{Duration, SystemTime};

    const TTL: Duration = Duration::from_secs(1);
    const OVERLAY_META_DIR: &str = ".trail";

    pub(crate) struct FuseCowMount {
        #[allow(dead_code)]
        session: fuser::BackgroundSession,
        #[allow(dead_code)]
        mountpoint: PathBuf,
        #[allow(dead_code)]
        lease: WorkspaceMountLease,
    }

    impl FuseCowMount {
        #[allow(dead_code)]
        pub(crate) fn mountpoint(&self) -> &Path {
            &self.mountpoint
        }
    }

    impl Drop for FuseCowMount {
        fn drop(&mut self) {}
    }

    pub(crate) fn prepare_fuse_cow_workdir(
        db: &Trail,
        lane: &str,
        dir: &Path,
        custom_workdir: bool,
    ) -> Result<PathBuf> {
        prepare_lane_workdir(dir, custom_workdir)?;
        let upperdir = db
            .prepare_workspace_view_storage_for_lane_name(lane)?
            .source_upper;
        fs::create_dir_all(upperdir.join(OVERLAY_META_DIR))?;
        Ok(upperdir)
    }

    pub(crate) fn mount_fuse_cow_for_lane(db: &Trail, lane: &str) -> Result<FuseCowMount> {
        mount_fuse_cow_for_lane_with_view(db, lane, None)
    }

    pub(crate) fn mount_fuse_cow_for_lane_with_ephemeral_bindings(
        db: &Trail,
        lane: &str,
        source_upper: PathBuf,
        source_root: ObjectId,
        bindings: Vec<WorkspaceLayerBinding>,
    ) -> Result<FuseCowMount> {
        mount_fuse_cow_for_lane_with_view(db, lane, Some((source_upper, source_root, bindings)))
    }

    fn mount_fuse_cow_for_lane_with_view(
        db: &Trail,
        lane: &str,
        ephemeral: Option<(PathBuf, ObjectId, Vec<WorkspaceLayerBinding>)>,
    ) -> Result<FuseCowMount> {
        validate_ref_segment(lane)?;
        let branch = db.lane_branch(lane)?;
        let record = db.lane_record(&branch.lane_id)?;
        let mode = db.lane_workdir_mode_for(&record, &branch)?;
        if mode != LaneWorkdirMode::FuseCow {
            return Err(Error::InvalidInput(format!(
                "lane `{lane}` uses workdir mode `{}`; expected fuse-cow",
                mode.as_str()
            )));
        }
        let Some(workdir) = branch.workdir.clone() else {
            return Err(Error::InvalidInput(format!(
                "fuse-cow lane `{lane}` has no mountpoint"
            )));
        };
        let mountpoint = PathBuf::from(workdir);
        prepare_overlay_mountpoint(&mountpoint, false)?;
        let head = db.get_ref(&branch.ref_name)?;
        let (upperdir, source_root, bindings) = match ephemeral {
            Some((upperdir, source_root, bindings)) => (upperdir, source_root, Some(bindings)),
            None => (overlay_upperdir(db, lane)?, head.root_id, None),
        };
        fs::create_dir_all(&upperdir)?;
        fs::create_dir_all(upperdir.join(OVERLAY_META_DIR))?;
        let fs = SharedOverlayFs::new(
            db.workspace_root.clone(),
            db.db_dir.clone(),
            upperdir,
            source_root,
            bindings,
        )?;
        #[cfg(target_os = "linux")]
        let mut options = vec![MountOption::FSName(format!("trail-fuse-cow-{lane}"))];
        #[cfg(target_os = "macos")]
        let options = vec![MountOption::FSName(format!("trail-fuse-cow-{lane}"))];
        #[cfg(target_os = "linux")]
        {
            options.push(MountOption::Subtype("trail-fuse-cow".to_string()));
            options.push(MountOption::RW);
            options.push(MountOption::NoAtime);
        }
        ensure_platform_fuse_ready()?;
        let mut lease = db.acquire_workspace_mount_lease(lane, "fuse")?;
        let session = fuser::spawn_mount2(fs, &mountpoint, &options)
            .map_err(|err| overlay_mount_error(&mountpoint, err))?;
        lease.mark_mounted()?;

        Ok(FuseCowMount {
            session,
            mountpoint,
            lease,
        })
    }

    pub(crate) fn fuse_candidate_paths(db: &Trail, lane: &str) -> Result<Vec<String>> {
        let upper = overlay_upperdir(db, lane)?;
        let branch = db.lane_branch(lane)?;
        let head = db.get_ref(&branch.ref_name)?;
        Ok(
            recover_view_checkpoint_candidates_for_root(db, &upper, &head.root_id)?
                .paths
                .into_iter()
                .collect(),
        )
    }

    fn overlay_mount_error(mountpoint: &Path, err: std::io::Error) -> Error {
        Error::InvalidInput(format!(
            "failed to mount fuse-cow workdir at `{}`: {err}. On macOS install macFUSE; on Linux ensure /dev/fuse is available and your user can mount FUSE filesystems.",
            mountpoint.display()
        ))
    }

    #[cfg(target_os = "linux")]
    fn ensure_platform_fuse_ready() -> Result<()> {
        if Path::new("/dev/fuse").exists() {
            return Ok(());
        }
        Err(Error::InvalidInput(
            "fuse-cow workdirs require `/dev/fuse`; enable FUSE for this Linux environment"
                .to_string(),
        ))
    }

    #[cfg(target_os = "macos")]
    fn ensure_platform_fuse_ready() -> Result<()> {
        if macos_fuse_device_present()? {
            return Ok(());
        }
        let loader = Path::new("/Library/Filesystems/macfuse.fs/Contents/Resources/load_macfuse");
        if !loader.exists() {
            return Err(Error::InvalidInput(
                "fuse-cow workdirs require macFUSE; install macFUSE and approve its system extension".to_string(),
            ));
        }
        run_macos_fuse_loader(loader, Duration::from_secs(5))?;
        if macos_fuse_device_present()? {
            return Ok(());
        }
        Err(Error::InvalidInput(
            "macFUSE is installed but its device is not available; approve or enable the macFUSE system extension in macOS System Settings, then retry".to_string(),
        ))
    }

    #[cfg(target_os = "macos")]
    fn macos_fuse_device_present() -> Result<bool> {
        let entries = match fs::read_dir("/dev") {
            Ok(entries) => entries,
            Err(err) => return Err(Error::Io(err)),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "fuse" || name.starts_with("macfuse") || name.starts_with("osxfuse") {
                return Ok(true);
            }
        }
        Ok(false)
    }

    #[cfg(target_os = "macos")]
    fn run_macos_fuse_loader(loader: &Path, timeout: Duration) -> Result<()> {
        let mut child = Command::new(loader)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(Error::Io)?;
        let deadline = Instant::now() + timeout;
        loop {
            if child.try_wait().map_err(Error::Io)?.is_some() {
                let output = child.wait_with_output().map_err(Error::Io)?;
                if output.status.success() {
                    return Ok(());
                }
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let detail = if stderr.is_empty() { stdout } else { stderr };
                return Err(Error::InvalidInput(format!(
                    "macFUSE loader failed{}; approve or reinstall macFUSE",
                    if detail.is_empty() {
                        String::new()
                    } else {
                        format!(": {detail}")
                    }
                )));
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait_with_output();
                return Err(Error::InvalidInput(
                    "macFUSE loader did not finish within 5 seconds; approve or enable the macFUSE system extension in macOS System Settings".to_string(),
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn overlay_upperdir(db: &Trail, lane: &str) -> Result<PathBuf> {
        validate_ref_segment(lane)?;
        Ok(db.workspace_view_paths_for_lane_name(lane).source_upper)
    }

    fn prepare_overlay_mountpoint(path: &Path, custom_workdir: bool) -> Result<()> {
        if let Ok(metadata) = fs::symlink_metadata(path) {
            if metadata.file_type().is_symlink() {
                return Err(Error::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                    reason: "fuse-cow mountpoint cannot be a symlink".to_string(),
                });
            }
            if !metadata.is_dir() {
                return Err(Error::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                    reason: "fuse-cow mountpoint must be a directory".to_string(),
                });
            }
            if fs::read_dir(path)?.next().transpose()?.is_some() && custom_workdir {
                return Err(Error::InvalidInput(format!(
                    "custom fuse-cow workdir `{}` must be empty",
                    path.display()
                )));
            }
        } else {
            fs::create_dir_all(path)?;
        }
        Ok(())
    }

    struct SharedOverlayFs {
        core: ViewCore,
        handles: HashMap<u64, File>,
        directory_handles: HashMap<u64, Vec<(u64, ViewNodeKind, String)>>,
        next_fh: u64,
    }

    impl SharedOverlayFs {
        fn new(
            workspace_root: PathBuf,
            db_dir: PathBuf,
            upperdir: PathBuf,
            root_id: ObjectId,
            ephemeral_bindings: Option<Vec<WorkspaceLayerBinding>>,
        ) -> Result<Self> {
            let db = Trail::open_with_db_dir(workspace_root, db_dir)?;
            let core = match ephemeral_bindings {
                Some(bindings) => {
                    ViewCore::new_lazy_with_ephemeral_bindings(db, upperdir, root_id, bindings)?
                }
                None => ViewCore::new_lazy(db, upperdir, root_id)?,
            };
            Ok(Self {
                core,
                handles: HashMap::new(),
                directory_handles: HashMap::new(),
                next_fh: 1,
            })
        }

        fn insert_handle(&mut self, file: File) -> u64 {
            let fh = self.next_fh;
            self.next_fh += 1;
            self.handles.insert(fh, file);
            fh
        }

        fn directory_entries(
            &mut self,
            ino: u64,
        ) -> std::result::Result<Vec<(u64, ViewNodeKind, String)>, i32> {
            let parent_ino = self.core.lookup(ino, "..").unwrap_or(VIEW_ROOT_INO);
            let mut entries = vec![
                (ino, ViewNodeKind::Directory, ".".to_string()),
                (parent_ino, ViewNodeKind::Directory, "..".to_string()),
            ];
            entries.extend(
                self.core
                    .children(ino)?
                    .into_iter()
                    .map(|(name, attr)| (attr.ino, attr.kind, name)),
            );
            Ok(entries)
        }

        fn path_for_ino(&self, ino: u64) -> std::result::Result<String, i32> {
            self.core.path_for_ino(ino)
        }

        fn attr_for_ino(&mut self, ino: u64) -> std::result::Result<FileAttr, i32> {
            let path = self.core.path_for_ino(ino)?;
            self.core.attr(&path).map(|attr| view_file_attr(&attr))
        }

        fn upper_file(&self, path: &str) -> Option<PathBuf> {
            self.core
                .upper_path(path)
                .ok()
                .filter(|path| path.is_file())
        }
    }

    impl Filesystem for SharedOverlayFs {
        fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
            let Some(name) = name.to_str() else {
                reply.error(EINVAL);
                return;
            };
            match self.core.lookup(parent, name) {
                Ok(ino) => match self.attr_for_ino(ino) {
                    Ok(attr) => reply.entry(&TTL, &attr, 0),
                    Err(err) => reply.error(err),
                },
                Err(err) => reply.error(err),
            }
        }

        fn getattr(&mut self, _req: &Request<'_>, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
            match self.attr_for_ino(ino) {
                Ok(attr) => reply.attr(&TTL, &attr),
                Err(err) => reply.error(err),
            }
        }

        fn readlink(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyData) {
            match self.core.readlink(ino) {
                Ok(target) => reply.data(target.as_os_str().as_encoded_bytes()),
                Err(err) => reply.error(err),
            }
        }

        fn setattr(
            &mut self,
            _req: &Request<'_>,
            ino: u64,
            mode: Option<u32>,
            _uid: Option<u32>,
            _gid: Option<u32>,
            size: Option<u64>,
            _atime: Option<fuser::TimeOrNow>,
            _mtime: Option<fuser::TimeOrNow>,
            _ctime: Option<SystemTime>,
            _fh: Option<u64>,
            _crtime: Option<SystemTime>,
            _chgtime: Option<SystemTime>,
            _bkuptime: Option<SystemTime>,
            _flags: Option<u32>,
            reply: ReplyAttr,
        ) {
            match self.core.setattr(ino, size, mode) {
                Ok(attr) => reply.attr(&TTL, &view_file_attr(&attr)),
                Err(err) => reply.error(err),
            }
        }

        fn mkdir(
            &mut self,
            _req: &Request<'_>,
            parent: u64,
            name: &OsStr,
            mode: u32,
            umask: u32,
            reply: ReplyEntry,
        ) {
            let Some(name) = name.to_str() else {
                reply.error(EINVAL);
                return;
            };
            match self.core.mkdir(parent, name, (mode & !umask) & 0o777) {
                Ok(attr) => reply.entry(&TTL, &view_file_attr(&attr), 0),
                Err(err) => reply.error(err),
            }
        }

        fn symlink(
            &mut self,
            _req: &Request<'_>,
            parent: u64,
            link_name: &OsStr,
            target: &Path,
            reply: ReplyEntry,
        ) {
            let Some(link_name) = link_name.to_str() else {
                reply.error(EINVAL);
                return;
            };
            match self.core.symlink(parent, link_name, target) {
                Ok(attr) => reply.entry(&TTL, &view_file_attr(&attr), 0),
                Err(err) => reply.error(err),
            }
        }

        fn mknod(
            &mut self,
            _req: &Request<'_>,
            parent: u64,
            name: &OsStr,
            mode: u32,
            umask: u32,
            _rdev: u32,
            reply: ReplyEntry,
        ) {
            let Some(name) = name.to_str() else {
                reply.error(EINVAL);
                return;
            };
            match self
                .core
                .create(parent, name, (mode & !umask) & 0o777, true)
            {
                Ok(attr) => reply.entry(&TTL, &view_file_attr(&attr), 0),
                Err(err) => reply.error(err),
            }
        }

        fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
            let Some(name) = name.to_str() else {
                reply.error(EINVAL);
                return;
            };
            match self.core.remove(parent, name) {
                Ok(()) => reply.ok(),
                Err(err) => reply.error(err),
            }
        }

        fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
            self.unlink(_req, parent, name, reply);
        }

        fn rename(
            &mut self,
            _req: &Request<'_>,
            parent: u64,
            name: &OsStr,
            newparent: u64,
            newname: &OsStr,
            flags: u32,
            reply: ReplyEmpty,
        ) {
            if flags != 0 {
                reply.error(EINVAL);
                return;
            }
            let (Some(name), Some(newname)) = (name.to_str(), newname.to_str()) else {
                reply.error(EINVAL);
                return;
            };
            match self.core.rename(parent, name, newparent, newname) {
                Ok(()) => reply.ok(),
                Err(err) => reply.error(err),
            }
        }

        fn open(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
            let path = match self.path_for_ino(ino) {
                Ok(path) => path,
                Err(err) => {
                    reply.error(err);
                    return;
                }
            };
            if wants_write(flags) {
                match self.core.ensure_upper_file(&path, flags & O_TRUNC != 0) {
                    Ok(file) => reply.opened(self.insert_handle(file), 0),
                    Err(err) => reply.error(err),
                }
            } else if let Some(path) = self.upper_file(&path) {
                match OpenOptions::new().read(true).open(path) {
                    Ok(file) => reply.opened(self.insert_handle(file), 0),
                    Err(err) => reply.error(io_err(err)),
                }
            } else {
                match self.core.node_kind(&path) {
                    Ok(Some(ViewNodeKind::File)) => reply.opened(0, 0),
                    Ok(_) => reply.error(ENOENT),
                    Err(err) => reply.error(err),
                }
            }
        }

        fn create(
            &mut self,
            _req: &Request<'_>,
            parent: u64,
            name: &OsStr,
            mode: u32,
            umask: u32,
            flags: i32,
            reply: ReplyCreate,
        ) {
            let Some(name) = name.to_str() else {
                reply.error(EINVAL);
                return;
            };
            let path = match self.core.child_path(parent, name) {
                Ok(path) => path,
                Err(err) => {
                    reply.error(err);
                    return;
                }
            };
            let attr = if matches!(self.core.node_kind(&path), Ok(Some(_))) {
                match self.core.ensure_upper_file(&path, flags & O_TRUNC != 0) {
                    Ok(_) => self.core.attr(&path),
                    Err(err) => Err(err),
                }
            } else {
                self.core
                    .create(parent, name, (mode & !umask) & 0o777, true)
            };
            let attr = match attr {
                Ok(attr) => attr,
                Err(err) => {
                    reply.error(err);
                    return;
                }
            };
            match self.core.ensure_upper_file(&path, false) {
                Ok(file) => {
                    let fh = self.insert_handle(file);
                    reply.created(&TTL, &view_file_attr(&attr), 0, fh, 0);
                }
                Err(err) => reply.error(err),
            }
        }

        fn read(
            &mut self,
            _req: &Request<'_>,
            ino: u64,
            fh: u64,
            offset: i64,
            size: u32,
            _flags: i32,
            _lock_owner: Option<u64>,
            reply: ReplyData,
        ) {
            if let Some(file) = self.handles.get(&fh) {
                let mut buffer = vec![0; size as usize];
                match file.read_at(&mut buffer, offset.max(0) as u64) {
                    Ok(read) => {
                        buffer.truncate(read);
                        reply.data(&buffer);
                    }
                    Err(err) => reply.error(io_err(err)),
                }
                return;
            }
            match self.core.read(ino, offset.max(0) as u64, size) {
                Ok((bytes, _)) => reply.data(&bytes),
                Err(err) => reply.error(err),
            }
        }

        fn write(
            &mut self,
            _req: &Request<'_>,
            ino: u64,
            _fh: u64,
            offset: i64,
            data: &[u8],
            _write_flags: u32,
            _flags: i32,
            _lock_owner: Option<u64>,
            reply: ReplyWrite,
        ) {
            match self.core.write(ino, offset.max(0) as u64, data) {
                Ok(_) => reply.written(data.len() as u32),
                Err(err) => reply.error(err),
            }
        }

        fn flush(
            &mut self,
            _req: &Request<'_>,
            _ino: u64,
            fh: u64,
            _lock_owner: u64,
            reply: ReplyEmpty,
        ) {
            if let Some(file) = self.handles.get(&fh) {
                if let Err(err) = file.sync_data() {
                    reply.error(io_err(err));
                    return;
                }
            }
            reply.ok();
        }

        fn release(
            &mut self,
            _req: &Request<'_>,
            _ino: u64,
            fh: u64,
            _flags: i32,
            _lock_owner: Option<u64>,
            _flush: bool,
            reply: ReplyEmpty,
        ) {
            self.handles.remove(&fh);
            reply.ok();
        }

        fn fsync(
            &mut self,
            _req: &Request<'_>,
            _ino: u64,
            fh: u64,
            _datasync: bool,
            reply: ReplyEmpty,
        ) {
            if let Some(file) = self.handles.get(&fh) {
                if let Err(err) = file.sync_all() {
                    reply.error(io_err(err));
                    return;
                }
            }
            reply.ok();
        }

        fn opendir(&mut self, _req: &Request<'_>, ino: u64, _flags: i32, reply: ReplyOpen) {
            match self.path_for_ino(ino) {
                Ok(path) => match self.core.node_kind(&path) {
                    Ok(Some(ViewNodeKind::Directory)) => match self.directory_entries(ino) {
                        Ok(entries) => {
                            let fh = self.next_fh;
                            self.next_fh += 1;
                            self.directory_handles.insert(fh, entries);
                            reply.opened(fh, 0);
                        }
                        Err(err) => reply.error(err),
                    },
                    Ok(_) => reply.error(ENOTDIR),
                    Err(err) => reply.error(err),
                },
                Err(err) => reply.error(err),
            }
        }

        fn readdir(
            &mut self,
            _req: &Request<'_>,
            _ino: u64,
            fh: u64,
            offset: i64,
            mut reply: ReplyDirectory,
        ) {
            let Some(entries) = self.directory_handles.get(&fh) else {
                reply.error(ENOENT);
                return;
            };
            for (index, (entry_ino, kind, name)) in
                entries.iter().enumerate().skip(offset.max(0) as usize)
            {
                let kind = match kind {
                    ViewNodeKind::File => FileType::RegularFile,
                    ViewNodeKind::Directory => FileType::Directory,
                    ViewNodeKind::Symlink => FileType::Symlink,
                };
                if reply.add(*entry_ino, (index + 1) as i64, kind, name) {
                    break;
                }
            }
            reply.ok();
        }

        fn releasedir(
            &mut self,
            _req: &Request<'_>,
            _ino: u64,
            fh: u64,
            _flags: i32,
            reply: ReplyEmpty,
        ) {
            self.directory_handles.remove(&fh);
            reply.ok();
        }

        fn statfs(&mut self, _req: &Request<'_>, _ino: u64, reply: fuser::ReplyStatfs) {
            reply.statfs(0, 0, 0, 0, 0, 512, 255, 0);
        }

        fn access(&mut self, _req: &Request<'_>, ino: u64, _mask: i32, reply: ReplyEmpty) {
            match self.attr_for_ino(ino) {
                Ok(_) => reply.ok(),
                Err(err) => reply.error(err),
            }
        }
    }

    fn view_file_attr(attr: &ViewNodeAttr) -> FileAttr {
        FileAttr {
            ino: attr.ino,
            size: attr.size,
            blocks: attr.size.saturating_add(511) / 512,
            atime: attr.modified,
            mtime: attr.modified,
            ctime: attr.modified,
            crtime: attr.modified,
            kind: match attr.kind {
                ViewNodeKind::File => FileType::RegularFile,
                ViewNodeKind::Directory => FileType::Directory,
                ViewNodeKind::Symlink => FileType::Symlink,
            },
            perm: attr.mode as u16,
            nlink: if attr.kind == ViewNodeKind::Directory {
                2
            } else {
                1
            },
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    fn wants_write(flags: i32) -> bool {
        matches!(flags & O_ACCMODE, O_WRONLY | O_RDWR) || flags & O_TRUNC != 0
    }

    fn io_err(err: std::io::Error) -> i32 {
        err.raw_os_error().unwrap_or(EINVAL)
    }

    #[cfg(test)]
    mod mounted_conformance {
        use super::*;

        #[test]
        fn fuse_adapter_runs_shared_mounted_view_suite() {
            if std::env::var_os("TRAIL_RUN_FUSE_COW_TESTS").is_none() {
                return;
            }
            let temp = tempfile::tempdir().unwrap();
            fs::create_dir_all(temp.path().join("src")).unwrap();
            for (path, bytes) in [
                ("README.md", b"baseline\n".as_slice()),
                ("src/lower.txt", b"lower\n".as_slice()),
                ("script.sh", b"abcdef\n".as_slice()),
                ("delete.txt", b"delete\n".as_slice()),
            ] {
                fs::write(temp.path().join(path), bytes).unwrap();
            }
            Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let mut db = Trail::open(temp.path()).unwrap();
            db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                "fuse-conformance",
                Some("main"),
                LaneWorkdirMode::FuseCow,
                None,
                None,
                None,
                &[],
                false,
            )
            .unwrap();
            let mount = db
                .mount_fuse_cow_workdir_for_lane("fuse-conformance")
                .unwrap();
            let workdir = PathBuf::from(
                db.lane_workdir("fuse-conformance")
                    .unwrap()
                    .workdir
                    .unwrap(),
            );
            let expected = run_mounted_view_conformance(&workdir).unwrap();
            let record = db
                .record_lane_workdir("fuse-conformance", Some("conformance".to_string()))
                .unwrap();
            let actual = record
                .changed_paths
                .into_iter()
                .flat_map(|path| path.old_path.into_iter().chain(std::iter::once(path.path)))
                .collect::<BTreeSet<_>>();
            assert_eq!(actual, expected.changed_paths);
            assert_eq!(
                fs::read(temp.path().join("README.md")).unwrap(),
                b"baseline\n"
            );
            drop(mount);
        }

        #[test]
        fn cargo_target_seed_reuses_compiler_results_with_private_writable_targets() {
            if std::env::var_os("TRAIL_RUN_FUSE_COW_TESTS").is_none() {
                return;
            }
            if !std::process::Command::new("cargo")
                .arg("--version")
                .output()
                .is_ok_and(|output| output.status.success())
            {
                return;
            }

            let temp = tempfile::tempdir().unwrap();
            fs::create_dir_all(temp.path().join("src")).unwrap();
            fs::create_dir_all(temp.path().join("shared-dep/src")).unwrap();
            fs::write(
                temp.path().join("Cargo.toml"),
                "[package]\nname = \"cache-probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nshared-dep = { path = \"shared-dep\" }\n",
            )
            .unwrap();
            fs::write(
                temp.path().join("src/lib.rs"),
                "pub fn answer() -> u64 { shared_dep::answer() }\n",
            )
            .unwrap();
            fs::write(
                temp.path().join("shared-dep/Cargo.toml"),
                "[package]\nname = \"shared-dep\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            )
            .unwrap();
            fs::write(
                temp.path().join("shared-dep/src/lib.rs"),
                "pub fn answer() -> u64 { 42 }\n",
            )
            .unwrap();
            let lock = std::process::Command::new("cargo")
                .args(["generate-lockfile", "--offline"])
                .current_dir(temp.path())
                .output()
                .unwrap();
            assert!(
                lock.status.success(),
                "cargo generate-lockfile failed: {}",
                String::from_utf8_lossy(&lock.stderr)
            );

            Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let mut db = Trail::open(temp.path()).unwrap();
            for lane in ["rust-seed-a", "rust-seed-b"] {
                db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                    lane,
                    Some("main"),
                    LaneWorkdirMode::FuseCow,
                    None,
                    None,
                    None,
                    &[],
                    false,
                )
                .unwrap();
            }

            let first = db
                .exec_lane_workspace(
                    "rust-seed-a",
                    &[
                        "cargo".to_string(),
                        "build".to_string(),
                        "--offline".to_string(),
                    ],
                )
                .unwrap();
            assert_eq!(first.exit_code, 0);
            let lane_a_head = db.lane_details("rust-seed-a").unwrap().branch.head_change;
            let checkpoint = db.checkpoint_lane_workspace("rust-seed-a", None).unwrap();
            assert!(checkpoint.source_paths.is_empty());
            assert!(checkpoint.generated_dirty_paths > 0);
            assert_eq!(
                db.lane_details("rust-seed-a").unwrap().branch.head_change,
                lane_a_head
            );
            let view_a = db.lane_workspace_view("rust-seed-a").unwrap().unwrap();
            let target_a = PathBuf::from(&view_a.generated_upper).join("target");
            assert!(tree_has_name_fragment(&target_a, "libshared_dep"));

            let layer = db
                .sync_workspace_environment("rust-seed-b", "cargo", None)
                .unwrap();
            assert_eq!(layer.adapter, "cargo-target-seed");

            let second = db
                .exec_lane_workspace(
                    "rust-seed-b",
                    &[
                        "cargo".to_string(),
                        "build".to_string(),
                        "--offline".to_string(),
                    ],
                )
                .unwrap();
            assert_eq!(second.exit_code, 0);
            let view_b = db.lane_workspace_view("rust-seed-b").unwrap().unwrap();
            let target_b = PathBuf::from(&view_b.generated_upper).join("target");
            assert!(
                !tree_has_name_fragment(&target_b, "libshared_dep"),
                "the second lane rebuilt a dependency that was available in its immutable target seed"
            );
            assert!(tree_has_name_fragment(
                Path::new(&layer.storage_path),
                "libshared_dep"
            ));

            let clean = db
                .exec_lane_workspace("rust-seed-b", &["cargo".to_string(), "clean".to_string()])
                .unwrap();
            assert_eq!(clean.exit_code, 0);
            assert!(tree_has_name_fragment(&target_a, "libshared_dep"));
            assert!(tree_has_name_fragment(
                Path::new(&layer.storage_path),
                "libshared_dep"
            ));
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn fuse_large_root_shares_real_node_layer_but_isolates_install_and_clean() {
            if std::env::var_os("TRAIL_RUN_FUSE_NODE_LAYER_TESTS").is_none() {
                return;
            }
            for tool in ["node", "npm"] {
                assert!(
                    std::process::Command::new(tool)
                        .arg("--version")
                        .output()
                        .is_ok_and(|output| output.status.success()),
                    "{tool} is required for the real Node layer acceptance test"
                );
            }

            let temp = tempfile::tempdir().unwrap();
            fs::write(
                temp.path().join("package.json"),
                r#"{"name":"trail-fuse-node","version":"1.0.0","private":true,"dependencies":{"lodash":"4.17.21","prettier":"3.3.3"}}"#,
            )
            .unwrap();
            fs::write(temp.path().join(".gitignore"), "node_modules/\ntarget/\n").unwrap();
            for shard in 0..100 {
                let directory = temp
                    .path()
                    .join("large-source")
                    .join(format!("d{shard:03}"));
                fs::create_dir_all(&directory).unwrap();
                for file in 0..500 {
                    fs::write(
                        directory.join(format!("f{file:03}.txt")),
                        format!("source-{shard:03}-{file:03}\n"),
                    )
                    .unwrap();
                }
            }
            let lock = std::process::Command::new("npm")
                .args([
                    "install",
                    "--package-lock-only",
                    "--ignore-scripts",
                    "--no-audit",
                    "--no-fund",
                ])
                .current_dir(temp.path())
                .output()
                .unwrap();
            assert!(
                lock.status.success(),
                "npm lock generation failed: {}",
                String::from_utf8_lossy(&lock.stderr)
            );

            let init_started = std::time::Instant::now();
            Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let init_ms = init_started.elapsed().as_millis();
            let mut db = Trail::open(temp.path()).unwrap();
            for lane in ["node-fuse-a", "node-fuse-b"] {
                db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                    lane,
                    Some("main"),
                    LaneWorkdirMode::FuseCow,
                    None,
                    None,
                    None,
                    &[],
                    false,
                )
                .unwrap();
            }

            let first = db.sync_node_dependencies("node-fuse-a", None).unwrap();
            let second = db.sync_node_dependencies("node-fuse-b", None).unwrap();
            assert_eq!(first.layer_id, second.layer_id);
            assert_eq!(first.cache_key, second.cache_key);
            assert!(
                first.entry_count > 500,
                "expected a non-trivial real npm tree"
            );
            let layer_file = Path::new(&first.storage_path).join("lodash/lodash.js");
            let layer_bin = Path::new(&first.storage_path).join(".bin/prettier");
            let immutable_bytes = fs::read(&layer_file).unwrap();
            let immutable_hash = sha256_hex(&immutable_bytes);
            assert!(fs::metadata(&layer_file).unwrap().permissions().readonly());
            assert!(fs::symlink_metadata(&layer_bin)
                .unwrap()
                .file_type()
                .is_symlink());
            let binding_count = db
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM workspace_view_layers WHERE layer_id = ?1",
                    params![first.layer_id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap();
            assert_eq!(binding_count, 2);

            let mount_started = std::time::Instant::now();
            let mount_a = db.mount_fuse_cow_workdir_for_lane("node-fuse-a").unwrap();
            let mount_b = db.mount_fuse_cow_workdir_for_lane("node-fuse-b").unwrap();
            let mount_ms = mount_started.elapsed().as_millis();
            let workdir_a = PathBuf::from(db.lane_workdir("node-fuse-a").unwrap().workdir.unwrap());
            let workdir_b = PathBuf::from(db.lane_workdir("node-fuse-b").unwrap().workdir.unwrap());
            assert_eq!(
                fs::read_to_string(workdir_b.join("large-source/d099/f499.txt")).unwrap(),
                "source-099-499\n"
            );
            for workdir in [&workdir_a, &workdir_b] {
                let node = std::process::Command::new("node")
                    .args([
                        "-e",
                        "const _ = require('lodash'); if (_.chunk([1,2,3], 2).length !== 2) process.exit(2)",
                    ])
                    .current_dir(workdir)
                    .output()
                    .unwrap();
                assert!(
                    node.status.success(),
                    "Node could not consume mounted layer: {}",
                    String::from_utf8_lossy(&node.stderr)
                );
                let prettier = std::process::Command::new("node_modules/.bin/prettier")
                    .arg("--version")
                    .current_dir(workdir)
                    .output()
                    .unwrap();
                assert!(
                    prettier.status.success(),
                    "mounted npm bin did not execute: {}",
                    String::from_utf8_lossy(&prettier.stderr)
                );
            }

            fs::write(
                workdir_a.join("node_modules/lodash/lodash.js"),
                "lane-a-private\n",
            )
            .unwrap();
            assert_eq!(
                fs::read_to_string(workdir_a.join("node_modules/lodash/lodash.js")).unwrap(),
                "lane-a-private\n"
            );
            assert_eq!(
                sha256_hex(&fs::read(workdir_b.join("node_modules/lodash/lodash.js")).unwrap()),
                immutable_hash
            );
            assert_eq!(sha256_hex(&fs::read(&layer_file).unwrap()), immutable_hash);
            let reinstalled_prettier = std::process::Command::new("node_modules/.bin/prettier")
                .arg("--version")
                .current_dir(&workdir_a)
                .output()
                .unwrap();
            assert!(
                reinstalled_prettier.status.success(),
                "npm-created bin symlink did not execute after reinstall: {}",
                String::from_utf8_lossy(&reinstalled_prettier.stderr)
            );
            let prettier = std::process::Command::new("node_modules/.bin/prettier")
                .arg("--version")
                .current_dir(&workdir_a)
                .output()
                .unwrap();
            assert!(prettier.status.success());

            let clean = std::process::Command::new("rm")
                .args(["-rf", "node_modules"])
                .current_dir(&workdir_a)
                .output()
                .unwrap();
            assert!(clean.status.success());
            assert!(!workdir_a.join("node_modules").exists());
            assert!(workdir_b.join("node_modules/lodash/lodash.js").is_file());
            assert_eq!(sha256_hex(&fs::read(&layer_file).unwrap()), immutable_hash);

            let install = std::process::Command::new("npm")
                .args(["ci", "--ignore-scripts", "--no-audit", "--no-fund"])
                .env("npm_config_cache", temp.path().join("npm-test-cache"))
                .current_dir(&workdir_a)
                .output()
                .unwrap();
            assert!(
                install.status.success(),
                "npm ci through FUSE failed: {}",
                String::from_utf8_lossy(&install.stderr)
            );
            assert_eq!(
                sha256_hex(&fs::read(workdir_a.join("node_modules/lodash/lodash.js")).unwrap()),
                immutable_hash
            );
            assert_eq!(
                sha256_hex(&fs::read(workdir_b.join("node_modules/lodash/lodash.js")).unwrap()),
                immutable_hash
            );
            assert_eq!(sha256_hex(&fs::read(&layer_file).unwrap()), immutable_hash);

            let lane_head_before = db.lane_details("node-fuse-a").unwrap().branch.head_change;
            let checkpoint = db.checkpoint_lane_workspace("node-fuse-a", None).unwrap();
            assert!(checkpoint.source_paths.is_empty());
            assert!(checkpoint.generated_dirty_paths > 500);
            assert_eq!(
                db.lane_details("node-fuse-a").unwrap().branch.head_change,
                lane_head_before
            );
            let view_a = db.lane_workspace_view("node-fuse-a").unwrap().unwrap();
            let view_b = db.lane_workspace_view("node-fuse-b").unwrap().unwrap();
            let source_upper_entries = Path::new(&view_a.source_upper)
                .read_dir()
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<BTreeSet<_>>();
            assert_eq!(
                source_upper_entries,
                BTreeSet::from([std::ffi::OsString::from(".trail")])
            );
            assert!(Path::new(&view_a.generated_upper)
                .join("node_modules/lodash/lodash.js")
                .is_file());
            assert!(!Path::new(&view_b.generated_upper)
                .join("node_modules/lodash/lodash.js")
                .exists());
            db.verify_workspace_layer(&first.layer_id).unwrap();
            eprintln!(
                "linux-fuse-node-layer files=50000 layer_entries={} init_ms={} mount_two_ms={} shared_layer={} generated_a_bytes={} generated_b_bytes={}",
                first.entry_count,
                init_ms,
                mount_ms,
                first.layer_id,
                db.lane_workspace_space("node-fuse-a")
                    .unwrap()
                    .generated_upper_bytes,
                db.lane_workspace_space("node-fuse-b")
                    .unwrap()
                    .generated_upper_bytes,
            );
            drop(mount_b);
            drop(mount_a);
        }

        fn tree_has_name_fragment(root: &Path, fragment: &str) -> bool {
            WalkBuilder::new(root)
                .hidden(false)
                .build()
                .filter_map(std::result::Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().contains(fragment))
        }
    }
}

#[cfg(any(target_os = "linux", all(target_os = "macos", feature = "macfuse")))]
pub(crate) use fuse_overlay::*;

#[cfg(not(any(target_os = "linux", all(target_os = "macos", feature = "macfuse"))))]
pub(crate) struct FuseCowMount {
    #[allow(dead_code)]
    mountpoint: PathBuf,
}

#[cfg(not(any(target_os = "linux", all(target_os = "macos", feature = "macfuse"))))]
impl FuseCowMount {
    #[allow(dead_code)]
    pub(crate) fn mountpoint(&self) -> &Path {
        &self.mountpoint
    }
}

#[cfg(not(any(target_os = "linux", all(target_os = "macos", feature = "macfuse"))))]
impl Drop for FuseCowMount {
    fn drop(&mut self) {}
}

#[cfg(not(any(target_os = "linux", all(target_os = "macos", feature = "macfuse"))))]
pub(crate) fn prepare_fuse_cow_workdir(
    _db: &Trail,
    _lane: &str,
    _dir: &Path,
    _custom_workdir: bool,
) -> Result<PathBuf> {
    Err(Error::InvalidInput(
        "fuse-cow workdirs require Linux FUSE or a macOS build with --features macfuse".to_string(),
    ))
}

#[cfg(not(any(target_os = "linux", all(target_os = "macos", feature = "macfuse"))))]
pub(crate) fn mount_fuse_cow_for_lane(_db: &Trail, lane: &str) -> Result<FuseCowMount> {
    Err(Error::InvalidInput(format!(
        "fuse-cow lane `{lane}` cannot be mounted on this platform"
    )))
}

#[cfg(not(any(target_os = "linux", all(target_os = "macos", feature = "macfuse"))))]
pub(crate) fn mount_fuse_cow_for_lane_with_ephemeral_bindings(
    _db: &Trail,
    lane: &str,
    _source_upper: PathBuf,
    _source_root: ObjectId,
    _bindings: Vec<WorkspaceLayerBinding>,
) -> Result<FuseCowMount> {
    Err(Error::InvalidInput(format!(
        "fuse-cow lane `{lane}` cannot be mounted on this platform"
    )))
}

#[cfg(not(any(target_os = "linux", all(target_os = "macos", feature = "macfuse"))))]
pub(crate) fn fuse_candidate_paths(_db: &Trail, lane: &str) -> Result<Vec<String>> {
    Err(Error::InvalidInput(format!(
        "fuse-cow lane `{lane}` cannot be inspected on this platform"
    )))
}

impl Trail {
    pub(crate) fn prepare_fuse_cow_lane_workdir(
        &self,
        lane: &str,
        dir: &Path,
        custom_workdir: bool,
    ) -> Result<PathBuf> {
        prepare_fuse_cow_workdir(self, lane, dir, custom_workdir)
    }

    pub fn mount_fuse_cow_workdir_for_lane(&self, lane: &str) -> Result<impl Drop + use<>> {
        mount_fuse_cow_for_lane(self, lane)
    }

    pub(crate) fn mount_fuse_cow_workdir_for_lane_with_ephemeral_bindings(
        &self,
        lane: &str,
        source_upper: PathBuf,
        source_root: ObjectId,
        bindings: Vec<WorkspaceLayerBinding>,
    ) -> Result<impl Drop + use<>> {
        mount_fuse_cow_for_lane_with_ephemeral_bindings(
            self,
            lane,
            source_upper,
            source_root,
            bindings,
        )
    }

    pub(crate) fn fuse_cow_candidate_paths_for_lane(&self, lane: &str) -> Result<Vec<String>> {
        fuse_candidate_paths(self, lane)
    }

    pub(crate) fn maybe_mount_fuse_cow_workdir_for_lane(
        &self,
        lane: &str,
    ) -> Result<Option<FuseCowMount>> {
        validate_ref_segment(lane)?;
        let branch = self.lane_branch(lane)?;
        let record = self.lane_record(&branch.lane_id)?;
        if self.lane_workdir_mode_for(&record, &branch)? == LaneWorkdirMode::FuseCow {
            Ok(Some(mount_fuse_cow_for_lane(self, lane)?))
        } else {
            Ok(None)
        }
    }
}
