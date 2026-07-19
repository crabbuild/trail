use super::*;

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use async_trait::async_trait;
    use nfsserve::nfs::{
        fattr3, fileid3, filename3, ftype3, nfspath3, nfsstat3, nfstime3, sattr3, set_mode3,
        set_size3, specdata3,
    };
    use nfsserve::tcp::{NFSTcp, NFSTcpListener};
    use nfsserve::vfs::{DirEntry, NFSFileSystem, ReadDirResult, VFSCapabilities};
    #[cfg(test)]
    use std::collections::BTreeSet;
    use std::ffi::{CStr, CString, OsStr};
    use std::fs::{self, OpenOptions};
    use std::os::unix::ffi::OsStrExt;
    #[cfg(test)]
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    use std::sync::Mutex;
    use std::thread::{self, JoinHandle};

    const ROOT_INO: u64 = 1;
    const OVERLAY_META_DIR: &str = ".trail";
    const NFS_MOUNT_STATE_FILE: &str = "nfs-mount.json";
    const LEGACY_NFS_MOUNT_STATE_FILE: &str = "mount.json";

    type CowCore = ViewCore;
    type NodeKind = ViewNodeKind;
    type NodeAttr = ViewNodeAttr;

    struct NfsAdapter {
        core: Mutex<CowCore>,
    }
    impl NfsAdapter {
        fn new(core: CowCore) -> Self {
            Self {
                core: Mutex::new(core),
            }
        }
    }

    #[async_trait]
    impl NFSFileSystem for NfsAdapter {
        fn capabilities(&self) -> VFSCapabilities {
            VFSCapabilities::ReadWrite
        }
        fn root_dir(&self) -> fileid3 {
            ROOT_INO
        }
        async fn lookup(
            &self,
            dirid: fileid3,
            filename: &filename3,
        ) -> std::result::Result<fileid3, nfsstat3> {
            let name = std::str::from_utf8(&filename.0).map_err(|_| nfsstat3::NFS3ERR_INVAL)?;
            self.core
                .lock()
                .unwrap()
                .lookup(dirid, name)
                .map_err(nfs_error)
        }
        async fn getattr(&self, id: fileid3) -> std::result::Result<fattr3, nfsstat3> {
            let mut core = self.core.lock().unwrap();
            let path = core.path_for_ino(id).map_err(nfs_error)?;
            core.attr(&path)
                .map(|attr| to_nfs_attr(&attr))
                .map_err(nfs_error)
        }
        async fn setattr(
            &self,
            id: fileid3,
            attr: sattr3,
        ) -> std::result::Result<fattr3, nfsstat3> {
            let size = if let set_size3::size(value) = attr.size {
                Some(value)
            } else {
                None
            };
            let mode = if let set_mode3::mode(value) = attr.mode {
                Some(value)
            } else {
                None
            };
            self.core
                .lock()
                .unwrap()
                .setattr(id, size, mode)
                .map(|attr| to_nfs_attr(&attr))
                .map_err(nfs_error)
        }
        async fn read(
            &self,
            id: fileid3,
            offset: u64,
            count: u32,
        ) -> std::result::Result<(Vec<u8>, bool), nfsstat3> {
            self.core
                .lock()
                .unwrap()
                .read(id, offset, count)
                .map_err(nfs_error)
        }
        async fn write(
            &self,
            id: fileid3,
            offset: u64,
            data: &[u8],
        ) -> std::result::Result<fattr3, nfsstat3> {
            self.core
                .lock()
                .unwrap()
                .write(id, offset, data)
                .map(|attr| to_nfs_attr(&attr))
                .map_err(nfs_error)
        }
        async fn create(
            &self,
            dirid: fileid3,
            filename: &filename3,
            attr: sattr3,
        ) -> std::result::Result<(fileid3, fattr3), nfsstat3> {
            let name = std::str::from_utf8(&filename.0).map_err(|_| nfsstat3::NFS3ERR_INVAL)?;
            let mode = if let set_mode3::mode(value) = attr.mode {
                value
            } else {
                0o644
            };
            let attr = self
                .core
                .lock()
                .unwrap()
                .create(dirid, name, mode, false)
                .map_err(nfs_error)?;
            Ok((attr.ino, to_nfs_attr(&attr)))
        }
        async fn create_exclusive(
            &self,
            dirid: fileid3,
            filename: &filename3,
        ) -> std::result::Result<fileid3, nfsstat3> {
            let name = std::str::from_utf8(&filename.0).map_err(|_| nfsstat3::NFS3ERR_INVAL)?;
            self.core
                .lock()
                .unwrap()
                .create(dirid, name, 0o644, true)
                .map(|attr| attr.ino)
                .map_err(nfs_error)
        }
        async fn mkdir(
            &self,
            dirid: fileid3,
            dirname: &filename3,
        ) -> std::result::Result<(fileid3, fattr3), nfsstat3> {
            let name = std::str::from_utf8(&dirname.0).map_err(|_| nfsstat3::NFS3ERR_INVAL)?;
            let attr = self
                .core
                .lock()
                .unwrap()
                .mkdir(dirid, name, 0o755)
                .map_err(nfs_error)?;
            Ok((attr.ino, to_nfs_attr(&attr)))
        }
        async fn remove(
            &self,
            dirid: fileid3,
            filename: &filename3,
        ) -> std::result::Result<(), nfsstat3> {
            let name = std::str::from_utf8(&filename.0).map_err(|_| nfsstat3::NFS3ERR_INVAL)?;
            self.core
                .lock()
                .unwrap()
                .remove(dirid, name)
                .map_err(nfs_error)
        }
        async fn rename(
            &self,
            from_dirid: fileid3,
            from_filename: &filename3,
            to_dirid: fileid3,
            to_filename: &filename3,
        ) -> std::result::Result<(), nfsstat3> {
            let from =
                std::str::from_utf8(&from_filename.0).map_err(|_| nfsstat3::NFS3ERR_INVAL)?;
            let to = std::str::from_utf8(&to_filename.0).map_err(|_| nfsstat3::NFS3ERR_INVAL)?;
            self.core
                .lock()
                .unwrap()
                .rename(from_dirid, from, to_dirid, to)
                .map_err(nfs_error)
        }
        async fn readdir(
            &self,
            dirid: fileid3,
            start_after: fileid3,
            max_entries: usize,
        ) -> std::result::Result<ReadDirResult, nfsstat3> {
            let mut core = self.core.lock().unwrap();
            let entries = core.children(dirid).map_err(nfs_error)?;
            let start = if start_after == 0 {
                0
            } else {
                entries
                    .iter()
                    .position(|(_, attr)| attr.ino == start_after)
                    .map(|index| index + 1)
                    .ok_or(nfsstat3::NFS3ERR_BAD_COOKIE)?
            };
            let page = entries
                .iter()
                .skip(start)
                .take(max_entries)
                .map(|(name, attr)| DirEntry {
                    fileid: attr.ino,
                    name: name.clone().into_bytes().into(),
                    attr: to_nfs_attr(attr),
                })
                .collect::<Vec<_>>();
            Ok(ReadDirResult {
                end: start + page.len() >= entries.len(),
                entries: page,
            })
        }
        async fn symlink(
            &self,
            dirid: fileid3,
            linkname: &filename3,
            symlink: &nfspath3,
            _attr: &sattr3,
        ) -> std::result::Result<(fileid3, fattr3), nfsstat3> {
            let name = std::str::from_utf8(&linkname.0).map_err(|_| nfsstat3::NFS3ERR_INVAL)?;
            let target = std::str::from_utf8(&symlink.0).map_err(|_| nfsstat3::NFS3ERR_INVAL)?;
            let attr = self
                .core
                .lock()
                .unwrap()
                .symlink(dirid, name, Path::new(target))
                .map_err(nfs_error)?;
            Ok((attr.ino, to_nfs_attr(&attr)))
        }
        async fn readlink(&self, id: fileid3) -> std::result::Result<nfspath3, nfsstat3> {
            self.core
                .lock()
                .unwrap()
                .readlink(id)
                .map(|target| target.as_os_str().as_bytes().to_vec().into())
                .map_err(nfs_error)
        }
    }

    pub(crate) struct NfsCowMount {
        mountpoint: PathBuf,
        state_path: PathBuf,
        shutdown: Option<tokio::sync::oneshot::Sender<()>>,
        worker: Option<JoinHandle<()>>,
        #[allow(dead_code)]
        lease: WorkspaceMountLease,
    }

    impl Drop for NfsCowMount {
        fn drop(&mut self) {
            let _ = unmount(&self.mountpoint);
            if let Some(shutdown) = self.shutdown.take() {
                let _ = shutdown.send(());
            }
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            let _ = fs::remove_file(&self.state_path);
        }
    }

    struct PendingNfsMount {
        mountpoint: PathBuf,
        state_path: PathBuf,
        shutdown: Option<tokio::sync::oneshot::Sender<()>>,
        worker: Option<JoinHandle<()>>,
        mounted: bool,
        committed: bool,
    }

    impl PendingNfsMount {
        fn commit(
            mut self,
        ) -> (
            PathBuf,
            PathBuf,
            tokio::sync::oneshot::Sender<()>,
            JoinHandle<()>,
        ) {
            self.committed = true;
            (
                std::mem::take(&mut self.mountpoint),
                std::mem::take(&mut self.state_path),
                self.shutdown.take().expect("pending NFS shutdown sender"),
                self.worker.take().expect("pending NFS worker"),
            )
        }
    }

    impl Drop for PendingNfsMount {
        fn drop(&mut self) {
            if self.committed {
                return;
            }
            if self.mounted {
                let _ = unmount(&self.mountpoint);
            }
            if let Some(shutdown) = self.shutdown.take() {
                let _ = shutdown.send(());
            }
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            let _ = fs::remove_file(&self.state_path);
        }
    }

    pub(crate) fn prepare_nfs_cow_workdir(
        db: &Trail,
        lane: &str,
        dir: &Path,
        custom: bool,
    ) -> Result<PathBuf> {
        prepare_lane_workdir(dir, custom)?;
        let upper = db
            .prepare_workspace_view_storage_for_lane_name(lane)?
            .source_upper;
        fs::create_dir_all(upper.join(OVERLAY_META_DIR))?;
        Ok(upper)
    }

    pub(crate) fn nfs_candidate_paths(db: &Trail, lane: &str) -> Result<ViewCheckpointCandidates> {
        let upper = nfs_upperdir(db, lane)?;
        let branch = db.lane_branch(lane)?;
        let head = db.get_ref(&branch.ref_name)?;
        let mut candidates =
            recover_view_checkpoint_candidates_for_root(db, &upper, &head.root_id)?;
        candidates.paths.retain(|path| {
            !Path::new(path)
                .file_name()
                .and_then(OsStr::to_str)
                .is_some_and(is_macos_junk)
        });
        Ok(candidates)
    }

    pub(crate) fn mount_nfs_cow_for_lane(db: &Trail, lane: &str) -> Result<NfsCowMount> {
        mount_nfs_cow_for_lane_with_view(db, lane, None)
    }

    pub(crate) fn mount_nfs_cow_for_lane_with_ephemeral_bindings(
        db: &Trail,
        lane: &str,
        source_upper: PathBuf,
        source_root: ObjectId,
        bindings: Vec<WorkspaceLayerBinding>,
    ) -> Result<NfsCowMount> {
        mount_nfs_cow_for_lane_with_view(db, lane, Some((source_upper, source_root, bindings)))
    }

    fn mount_nfs_cow_for_lane_with_view(
        db: &Trail,
        lane: &str,
        ephemeral: Option<(PathBuf, ObjectId, Vec<WorkspaceLayerBinding>)>,
    ) -> Result<NfsCowMount> {
        validate_ref_segment(lane)?;
        let branch = db.lane_branch(lane)?;
        let record = db.lane_record(&branch.lane_id)?;
        if db.lane_workdir_mode_for(&record, &branch)? != LaneWorkdirMode::NfsCow {
            return Err(Error::InvalidInput(format!(
                "lane `{lane}` does not use nfs-cow"
            )));
        }
        let mountpoint = PathBuf::from(branch.workdir.ok_or_else(|| {
            Error::InvalidInput(format!("nfs-cow lane `{lane}` has no mountpoint"))
        })?);
        if !Path::new("/sbin/mount_nfs").is_file() {
            return Err(Error::InvalidInput(
                "nfs-cow requires /sbin/mount_nfs on macOS".to_string(),
            ));
        }
        let state_path = db
            .workspace_view_paths_for_lane_name(lane)
            .meta_dir
            .join(NFS_MOUNT_STATE_FILE);
        let legacy_state_path = state_path.with_file_name(LEGACY_NFS_MOUNT_STATE_FILE);
        if fs::read(&legacy_state_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .is_some_and(|value| {
                value
                    .get("pid")
                    .and_then(serde_json::Value::as_i64)
                    .is_some()
            })
        {
            recover_stale_mount(&mountpoint, &legacy_state_path)?;
        }
        // A dead loopback server can leave the mountpoint in an unresponsive
        // NFS state. Recover it before *any* path operation on the mountpoint;
        // even `create_dir_all` or metadata can otherwise block in the kernel.
        recover_stale_mount(&mountpoint, &state_path)?;
        let mut lease = db.acquire_workspace_mount_lease(lane, "nfs")?;
        fs::create_dir_all(&mountpoint)?;
        let head = db.get_ref(&branch.ref_name)?;
        let (upper, source_root, bindings) = match ephemeral {
            Some((upper, source_root, bindings)) => (upper, source_root, Some(bindings)),
            None => (nfs_upperdir(db, lane)?, head.root_id, None),
        };
        fs::create_dir_all(upper.join(OVERLAY_META_DIR))?;
        let core_db = Trail::open_with_db_dir(db.workspace_root.clone(), db.db_dir.clone())?;
        let core = match bindings {
            Some(bindings) => CowCore::new_lazy_with_ephemeral_bindings(
                core_db,
                upper.clone(),
                source_root,
                bindings,
            )?,
            None => CowCore::new_lazy(core_db, upper.clone(), source_root)?,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| Error::InvalidInput(err.to_string()))?;
        let adapter = NfsAdapter::new(core);
        let listener = runtime
            .block_on(NFSTcpListener::bind("127.0.0.1:0", adapter))
            .map_err(Error::Io)?;
        let port = listener.get_listen_port();
        let state_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&state_path)
            .map_err(|err| {
                Error::InvalidInput(format!(
                    "nfs-cow lane `{lane}` is already being mounted: {err}"
                ))
            })?;
        let process_start_token = current_process_start_token();
        serde_json::to_writer(
            &state_file,
            &serde_json::json!({
                "pid": std::process::id(),
                "process_start_token": process_start_token,
                "port": port,
                "mountpoint": mountpoint,
            }),
        )?;
        state_file.sync_all()?;
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let worker = thread::spawn(move || {
            runtime.block_on(async move {
                tokio::select! { _ = listener.handle_forever() => {}, _ = shutdown_rx => {} }
            })
        });
        let mut pending_mount = PendingNfsMount {
            mountpoint: mountpoint.clone(),
            state_path: state_path.clone(),
            shutdown: Some(shutdown_tx),
            worker: Some(worker),
            mounted: false,
            committed: false,
        };
        // Directory mutations must be visible to the checkpoint scan that runs
        // immediately after the agent exits. Attribute caching can otherwise
        // retain a pre-create READDIR result and omit brand-new paths. macOS
        // also defaults to `nosync`, where a write syscall may return before
        // this userspace server has received the WRITE RPC. A lane can then be
        // unmounted and remounted with only the preceding truncate visible.
        // `sync` makes successful writes an actual durability boundary; the
        // server itself fsyncs every WRITE before reporting FILE_SYNC.
        let opts = format!(
            "locallocks,vers=3,tcp,sync,rsize=1048576,wsize=1048576,noac,actimeo=0,nonegnamecache,nobrowse,port={port},mountport={port}"
        );
        let status = Command::new("/sbin/mount_nfs")
            .args(["-o", &opts, "127.0.0.1:/", &mountpoint.to_string_lossy()])
            .status()?;
        if !status.success() {
            return Err(Error::InvalidInput(format!(
                "mount_nfs failed for `{}` with {status}",
                mountpoint.display()
            )));
        }
        if !is_nfs_mount(&mountpoint) {
            return Err(Error::InvalidInput(format!(
                "mount_nfs returned success, but `{}` is not an active NFS mount",
                mountpoint.display()
            )));
        }
        pending_mount.mounted = true;
        lease.mark_mounted()?;
        let (mountpoint, state_path, shutdown_tx, worker) = pending_mount.commit();
        Ok(NfsCowMount {
            mountpoint,
            state_path,
            shutdown: Some(shutdown_tx),
            worker: Some(worker),
            lease,
        })
    }

    fn nfs_upperdir(db: &Trail, lane: &str) -> Result<PathBuf> {
        validate_ref_segment(lane)?;
        Ok(db.workspace_view_paths_for_lane_name(lane).source_upper)
    }

    fn recover_stale_mount(mountpoint: &Path, state: &Path) -> Result<()> {
        let mut known_dead_owner = false;
        if let Ok(bytes) = fs::read(state)
            && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
            && let Some(pid) = value.get("pid").and_then(serde_json::Value::as_i64)
        {
            let token = value
                .get("process_start_token")
                .and_then(serde_json::Value::as_str);
            if token.is_some_and(|token| process_matches_start_token(pid as u32, token)) {
                return Err(Error::InvalidInput(format!(
                    "nfs-cow mount `{}` is already active in process {pid}",
                    mountpoint.display()
                )));
            }
            known_dead_owner = true;
        }
        if known_dead_owner {
            force_unmount_stale_without_probe(mountpoint)?;
        } else if is_nfs_mount(mountpoint) {
            unmount(mountpoint)?;
        }
        let _ = fs::remove_file(state);
        Ok(())
    }

    fn force_unmount_stale_without_probe(path: &Path) -> Result<()> {
        let cpath = CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| Error::InvalidInput("invalid NFS mount path".to_string()))?;
        if unsafe { libc::unmount(cpath.as_ptr(), libc::MNT_FORCE) } == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error
            .raw_os_error()
            .is_some_and(|code| code == libc::EINVAL || code == libc::ENOENT)
        {
            return Ok(());
        }
        let status = Command::new("/sbin/umount").arg("-f").arg(path).status()?;
        if status.success() {
            Ok(())
        } else {
            Err(Error::InvalidInput(format!(
                "failed to force-unmount stale NFS mount `{}` after owner death: {error}",
                path.display()
            )))
        }
    }

    fn unmount(path: &Path) -> Result<()> {
        if !is_nfs_mount(path) {
            return Ok(());
        }
        let cpath = CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| Error::InvalidInput("invalid NFS mount path".to_string()))?;
        if unsafe { libc::unmount(cpath.as_ptr(), 0) } == 0
            || unsafe { libc::unmount(cpath.as_ptr(), libc::MNT_FORCE) } == 0
        {
            return Ok(());
        }
        let status = Command::new("/sbin/umount").arg(path).status()?;
        if status.success() {
            Ok(())
        } else {
            Err(Error::InvalidInput(format!(
                "failed to unmount `{}`",
                path.display()
            )))
        }
    }

    fn is_nfs_mount(path: &Path) -> bool {
        let Ok(path) = CString::new(path.to_string_lossy().as_bytes()) else {
            return false;
        };
        unsafe {
            let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
            if libc::statfs(path.as_ptr(), stat.as_mut_ptr()) != 0 {
                return false;
            }
            let stat = stat.assume_init();
            CStr::from_ptr(stat.f_fstypename.as_ptr()).to_bytes() == b"nfs"
        }
    }

    fn to_nfs_attr(attr: &NodeAttr) -> fattr3 {
        let time = to_nfs_time(attr.modified);
        fattr3 {
            ftype: match attr.kind {
                NodeKind::Directory => ftype3::NF3DIR,
                NodeKind::File => ftype3::NF3REG,
                NodeKind::Symlink => ftype3::NF3LNK,
            },
            mode: attr.mode,
            nlink: if attr.kind == NodeKind::Directory {
                2
            } else {
                1
            },
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            size: attr.size,
            used: attr.size,
            rdev: specdata3::default(),
            fsid: 0,
            fileid: attr.ino,
            atime: time,
            mtime: time,
            ctime: time,
        }
    }
    fn to_nfs_time(time: SystemTime) -> nfstime3 {
        let duration = time
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        nfstime3 {
            seconds: duration.as_secs().min(u32::MAX as u64) as u32,
            nseconds: duration.subsec_nanos(),
        }
    }
    fn nfs_error(error: i32) -> nfsstat3 {
        match error {
            libc::ENOENT => nfsstat3::NFS3ERR_NOENT,
            libc::EEXIST => nfsstat3::NFS3ERR_EXIST,
            libc::ENOTDIR => nfsstat3::NFS3ERR_NOTDIR,
            libc::EISDIR => nfsstat3::NFS3ERR_ISDIR,
            libc::EINVAL => nfsstat3::NFS3ERR_INVAL,
            libc::EACCES | libc::EPERM => nfsstat3::NFS3ERR_ACCES,
            libc::ENOTEMPTY => nfsstat3::NFS3ERR_NOTEMPTY,
            libc::ENOSPC => nfsstat3::NFS3ERR_NOSPC,
            _ => nfsstat3::NFS3ERR_IO,
        }
    }
    fn is_macos_junk(name: &str) -> bool {
        name.starts_with("._")
            || matches!(
                name,
                ".DS_Store"
                    | ".Spotlight-V100"
                    | ".Trashes"
                    | ".fseventsd"
                    | ".metadata_never_index"
                    | ".metadata_never_index_unless_rootfs"
                    | ".metadata_direct_scope_only"
            )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        static COMMAND_AUTHORITY_TEST: std::sync::OnceLock<std::sync::Mutex<()>> =
            std::sync::OnceLock::new();

        #[test]
        fn cow_core_copies_up_writes_and_persists_whiteouts() {
            let temp = tempfile::tempdir().unwrap();
            fs::write(temp.path().join("README.md"), "baseline\n").unwrap();
            Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let db = Trail::open(temp.path()).unwrap();
            let head = db.resolve_branch_ref("main").unwrap();
            let lower = db.load_root_files(&head.root_id).unwrap();
            let upper = temp.path().join("upper");
            fs::create_dir_all(upper.join(OVERLAY_META_DIR)).unwrap();
            let mut core =
                CowCore::new(Trail::open(temp.path()).unwrap(), upper.clone(), lower).unwrap();

            let readme = core.lookup(ROOT_INO, "README.md").unwrap();
            assert!(!upper.join("README.md").exists());
            core.write(readme, 0, b"changed\n").unwrap();
            assert_eq!(
                fs::read_to_string(upper.join("README.md")).unwrap(),
                "changed\n\n"
            );
            assert_eq!(
                fs::read_to_string(temp.path().join("README.md")).unwrap(),
                "baseline\n"
            );

            core.remove(ROOT_INO, "README.md").unwrap();
            assert!(core.lookup(ROOT_INO, "README.md").is_err());
            let reopened = CowCore::new(
                Trail::open(temp.path()).unwrap(),
                upper,
                db.load_root_files(&head.root_id).unwrap(),
            )
            .unwrap();
            assert!(reopened.is_whiteouted("README.md"));
        }

        #[test]
        fn cow_core_renames_mixed_lower_and_upper_directory_contents() {
            let temp = tempfile::tempdir().unwrap();
            fs::create_dir_all(temp.path().join("src/nested")).unwrap();
            fs::write(temp.path().join("src/lower.txt"), "lower\n").unwrap();
            fs::write(temp.path().join("src/nested/tool.sh"), "#!/bin/sh\n").unwrap();
            fs::set_permissions(
                temp.path().join("src/nested/tool.sh"),
                fs::Permissions::from_mode(0o755),
            )
            .unwrap();
            Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let db = Trail::open(temp.path()).unwrap();
            let head = db.resolve_branch_ref("main").unwrap();
            let upper = temp.path().join("upper");
            fs::create_dir_all(upper.join("src")).unwrap();
            fs::write(upper.join("src/upper.txt"), "upper\n").unwrap();
            let mut core = CowCore::new(
                Trail::open(temp.path()).unwrap(),
                upper.clone(),
                db.load_root_files(&head.root_id).unwrap(),
            )
            .unwrap();

            core.rename(ROOT_INO, "src", ROOT_INO, "moved").unwrap();

            assert_eq!(fs::read(upper.join("moved/lower.txt")).unwrap(), b"lower\n");
            assert_eq!(fs::read(upper.join("moved/upper.txt")).unwrap(), b"upper\n");
            assert_eq!(
                fs::metadata(upper.join("moved/nested/tool.sh"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o755
            );
            assert!(core.lookup(ROOT_INO, "src").is_err());
            assert!(core.lookup(ROOT_INO, "moved").is_ok());
        }

        #[test]
        fn cow_core_truncates_changes_mode_and_rejects_symlink_escape() {
            let temp = tempfile::tempdir().unwrap();
            fs::write(temp.path().join("script.sh"), "abcdef\n").unwrap();
            Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let db = Trail::open(temp.path()).unwrap();
            let head = db.resolve_branch_ref("main").unwrap();
            let upper = temp.path().join("upper");
            fs::create_dir_all(&upper).unwrap();
            let mut core = CowCore::new(
                Trail::open(temp.path()).unwrap(),
                upper.clone(),
                db.load_root_files(&head.root_id).unwrap(),
            )
            .unwrap();

            let script = core.lookup(ROOT_INO, "script.sh").unwrap();
            core.setattr(script, Some(3), Some(0o755)).unwrap();
            assert_eq!(fs::read(upper.join("script.sh")).unwrap(), b"abc");
            assert_eq!(
                fs::metadata(upper.join("script.sh"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o755
            );

            let outside = temp.path().join("outside");
            fs::create_dir_all(&outside).unwrap();
            std::os::unix::fs::symlink(&outside, upper.join("escape")).unwrap();
            assert!(core.mkdir(ROOT_INO, "escape", 0o755).is_err());
            assert_eq!(core.upper_path("escape/file"), Err(libc::EPERM));
            assert!(!outside.join("file").exists());
        }

        #[test]
        fn stale_mount_state_is_removed_for_a_dead_process() {
            let temp = tempfile::tempdir().unwrap();
            let mountpoint = temp.path().join("mount");
            fs::create_dir_all(&mountpoint).unwrap();
            let state = temp.path().join(NFS_MOUNT_STATE_FILE);
            fs::write(&state, br#"{"pid":2147483647,"port":1}"#).unwrap();

            recover_stale_mount(&mountpoint, &state).unwrap();

            assert!(!state.exists());
        }

        #[test]
        fn stale_mount_state_uses_process_start_identity_not_pid_alone() {
            let temp = tempfile::tempdir().unwrap();
            let mountpoint = temp.path().join("mount");
            fs::create_dir_all(&mountpoint).unwrap();
            let state = temp.path().join(NFS_MOUNT_STATE_FILE);
            fs::write(
                &state,
                serde_json::to_vec(&serde_json::json!({
                    "pid": std::process::id(),
                    "process_start_token": "reused-pid",
                    "port": 1,
                }))
                .unwrap(),
            )
            .unwrap();

            recover_stale_mount(&mountpoint, &state).unwrap();
            assert!(!state.exists());

            fs::write(
                &state,
                serde_json::to_vec(&serde_json::json!({
                    "pid": std::process::id(),
                    "process_start_token": current_process_start_token(),
                    "port": 1,
                }))
                .unwrap(),
            )
            .unwrap();
            assert!(recover_stale_mount(&mountpoint, &state).is_err());
            assert!(state.exists());
        }

        #[test]
        fn pending_nfs_mount_drop_cleans_state_and_worker_before_publication() {
            let temp = tempfile::tempdir().unwrap();
            let mountpoint = temp.path().join("mount");
            fs::create_dir_all(&mountpoint).unwrap();
            let state_path = temp.path().join(NFS_MOUNT_STATE_FILE);
            fs::write(&state_path, b"pending").unwrap();
            let (shutdown, stopped) = tokio::sync::oneshot::channel();
            let worker = thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                let _ = runtime.block_on(stopped);
            });

            drop(PendingNfsMount {
                mountpoint,
                state_path: state_path.clone(),
                shutdown: Some(shutdown),
                worker: Some(worker),
                mounted: false,
                committed: false,
            });

            assert!(!state_path.exists());
        }

        #[test]
        fn nfs_adapter_validates_names_paginates_and_rejects_stale_inodes() {
            use nfsserve::nfs::nfsstring;

            let temp = tempfile::tempdir().unwrap();
            fs::write(temp.path().join("lower.txt"), "lower\n").unwrap();
            Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let db = Trail::open(temp.path()).unwrap();
            let head = db.resolve_branch_ref("main").unwrap();
            let upper = temp.path().join("upper");
            fs::create_dir_all(&upper).unwrap();
            let adapter = NfsAdapter::new(
                CowCore::new(
                    Trail::open(temp.path()).unwrap(),
                    upper,
                    db.load_root_files(&head.root_id).unwrap(),
                )
                .unwrap(),
            );
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                assert!(matches!(
                    adapter.lookup(ROOT_INO, &nfsstring(vec![0xff])).await,
                    Err(nfsstat3::NFS3ERR_INVAL)
                ));
                for name in ["a", "b", "c"] {
                    adapter
                        .create(
                            ROOT_INO,
                            &nfsstring(name.as_bytes().to_vec()),
                            sattr3::default(),
                        )
                        .await
                        .unwrap();
                }
                assert_eq!(
                    adapter
                        .core
                        .lock()
                        .unwrap()
                        .children(ROOT_INO)
                        .unwrap()
                        .len(),
                    4
                );
                let first = adapter.readdir(ROOT_INO, 0, 1).await.unwrap();
                assert_eq!(first.entries.len(), 1);
                assert!(!first.end);
                let second = adapter
                    .readdir(ROOT_INO, first.entries.last().unwrap().fileid, 10)
                    .await
                    .unwrap();
                assert!(second.end);
                assert!(!second.entries.is_empty());

                let stale = adapter
                    .lookup(ROOT_INO, &nfsstring(b"a".to_vec()))
                    .await
                    .unwrap();
                adapter
                    .remove(ROOT_INO, &nfsstring(b"a".to_vec()))
                    .await
                    .unwrap();
                assert!(matches!(
                    adapter.getattr(stale).await,
                    Err(nfsstat3::NFS3ERR_NOENT)
                ));
                let (link, attr) = adapter
                    .symlink(
                        ROOT_INO,
                        &nfsstring(b"link".to_vec()),
                        &nfsstring(b"b".to_vec()),
                        &sattr3::default(),
                    )
                    .await
                    .unwrap();
                assert!(matches!(attr.ftype, ftype3::NF3LNK));
                assert_eq!(adapter.readlink(link).await.unwrap().0, b"b");
                assert!(adapter
                    .symlink(
                        ROOT_INO,
                        &nfsstring(b"escape".to_vec()),
                        &nfsstring(b"../../outside".to_vec()),
                        &sattr3::default(),
                    )
                    .await
                    .is_err());
            });
        }

        #[test]
        fn nfs_adapter_runs_shared_mounted_view_suite() {
            if std::env::var_os("TRAIL_RUN_NFS_COW_TESTS").is_none() {
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
                "nfs-conformance",
                Some("main"),
                LaneWorkdirMode::NfsCow,
                None,
                None,
                None,
                &[],
                false,
            )
            .unwrap();
            let mount = db
                .mount_nfs_cow_workdir_for_lane("nfs-conformance")
                .unwrap();
            let workdir =
                PathBuf::from(db.lane_workdir("nfs-conformance").unwrap().workdir.unwrap());
            let expected = run_mounted_view_conformance(&workdir).unwrap();
            let record = db
                .record_lane_workdir("nfs-conformance", Some("conformance".to_string()))
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
        fn foreground_nfs_mount_stops_through_a_separate_trail_handle() {
            if std::env::var_os("TRAIL_RUN_NFS_COW_TESTS").is_none() {
                return;
            }
            let temp = tempfile::tempdir().unwrap();
            fs::write(temp.path().join("README.md"), "baseline\n").unwrap();
            Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let mut db = Trail::open(temp.path()).unwrap();
            let spawned = db
                .spawn_lane_with_workdir_mode_paths_and_neighbors(
                    "foreground-nfs",
                    Some("main"),
                    LaneWorkdirMode::NfsCow,
                    None,
                    None,
                    None,
                    &[],
                    false,
                )
                .unwrap();
            let workdir = PathBuf::from(spawned.workdir.unwrap());
            drop(db);

            let workspace = temp.path().to_path_buf();
            let owner = thread::spawn(move || {
                Trail::open(&workspace)
                    .unwrap()
                    .mount_lane_workspace_until_requested("foreground-nfs")
                    .unwrap()
            });
            let deadline = Instant::now() + Duration::from_secs(10);
            while !is_nfs_mount(&workdir) {
                assert!(
                    Instant::now() < deadline,
                    "foreground NFS mount did not start"
                );
                thread::sleep(Duration::from_millis(50));
            }
            fs::write(workdir.join("foreground.txt"), "owned\n").unwrap();
            let requester = Trail::open(temp.path()).unwrap();
            let stopped = requester
                .request_lane_workspace_unmount("foreground-nfs")
                .unwrap();
            assert!(stopped.healthy);
            let owned = owner.join().unwrap();
            assert_eq!(owned.view_id, stopped.view_id);
            assert!(!is_nfs_mount(&workdir));
            let mut reopened = Trail::open(temp.path()).unwrap();
            let checkpoint = reopened
                .checkpoint_lane_workspace("foreground-nfs", None)
                .unwrap();
            assert_eq!(checkpoint.source_paths, vec!["foreground.txt"]);
        }

        #[test]
        fn daemon_owned_nfs_mount_returns_ready_and_unmounts_asynchronously() {
            if std::env::var_os("TRAIL_RUN_NFS_COW_TESTS").is_none() {
                return;
            }
            let temp = tempfile::tempdir().unwrap();
            fs::write(temp.path().join("README.md"), "baseline\n").unwrap();
            Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let mut db = Trail::open(temp.path()).unwrap();
            let spawned = db
                .spawn_lane_with_workdir_mode_paths_and_neighbors(
                    "daemon-nfs",
                    Some("main"),
                    LaneWorkdirMode::NfsCow,
                    None,
                    None,
                    None,
                    &[],
                    false,
                )
                .unwrap();
            let workdir = PathBuf::from(spawned.workdir.unwrap());
            let mounted = db.start_lane_workspace_mount("daemon-nfs").unwrap();
            assert!(mounted.healthy);
            assert!(mounted.owner_pid.is_some());
            fs::write(workdir.join("daemon.txt"), "owned\n").unwrap();
            let stopped = db.request_lane_workspace_unmount("daemon-nfs").unwrap();
            assert_eq!(mounted.view_id, stopped.view_id);
            assert_eq!(stopped.owner_pid, None);
            assert!(!is_nfs_mount(&workdir));
            let checkpoint = db.checkpoint_lane_workspace("daemon-nfs", None).unwrap();
            assert_eq!(checkpoint.source_paths, vec!["daemon.txt"]);
        }

        #[test]
        fn command_authority_checkpoints_unmounted_nfs_view_from_qualified_journal() {
            if std::env::var_os("TRAIL_RUN_NFS_COW_TESTS").is_none() {
                return;
            }
            let _serial = COMMAND_AUTHORITY_TEST
                .get_or_init(|| std::sync::Mutex::new(()))
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            struct AuthorityReset;
            impl Drop for AuthorityReset {
                fn drop(&mut self) {
                    crate::db::change_ledger::set_command_authority_override(false);
                }
            }

            let temp = tempfile::tempdir().unwrap();
            fs::write(temp.path().join("README.md"), "baseline\n").unwrap();
            Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let mut db = Trail::open(temp.path()).unwrap();
            let spawned = db
                .spawn_lane_with_workdir_mode_paths_and_neighbors(
                    "authority-nfs",
                    Some("main"),
                    LaneWorkdirMode::NfsCow,
                    None,
                    None,
                    None,
                    &[],
                    false,
                )
                .unwrap();
            let lane_id = spawned.lane_id;
            let workdir = PathBuf::from(spawned.workdir.unwrap());
            assert!(workdir.read_dir().unwrap().next().is_none());

            crate::db::change_ledger::set_command_authority_override(true);
            let _reset = AuthorityReset;
            let empty = db
                .checkpoint_lane_workspace("authority-nfs", Some("empty".into()))
                .unwrap();
            assert!(empty.source_paths.is_empty());

            let mounted = db.start_lane_workspace_mount("authority-nfs").unwrap();
            assert!(mounted.healthy);
            fs::write(workdir.join("changed.txt"), "changed\n").unwrap();
            db.request_lane_workspace_unmount("authority-nfs").unwrap();
            assert!(workdir.read_dir().unwrap().next().is_none());
            let changed = db
                .checkpoint_lane_workspace("authority-nfs", Some("changed".into()))
                .unwrap();
            assert_eq!(changed.source_paths, vec!["changed.txt"]);

            let observer_scopes: i64 = db
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM changed_path_scopes
                     WHERE scope_kind='materialized_lane' AND owner_id=?1",
                    [&lane_id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(observer_scopes, 0);
        }

        #[test]
        fn command_authority_nfs_checkpoint_rejects_missing_journal_pair() {
            let _serial = COMMAND_AUTHORITY_TEST
                .get_or_init(|| std::sync::Mutex::new(()))
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            struct AuthorityReset;
            impl Drop for AuthorityReset {
                fn drop(&mut self) {
                    crate::db::change_ledger::set_command_authority_override(false);
                }
            }

            let temp = tempfile::tempdir().unwrap();
            fs::write(temp.path().join("README.md"), "baseline\n").unwrap();
            Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let mut db = Trail::open(temp.path()).unwrap();
            db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                "authority-nfs-corrupt",
                Some("main"),
                LaneWorkdirMode::NfsCow,
                None,
                None,
                None,
                &[],
                false,
            )
            .unwrap();
            let view = db
                .lane_workspace_view("authority-nfs-corrupt")
                .unwrap()
                .unwrap();
            let journal = ViewMutationJournal::open(Path::new(&view.source_upper)).unwrap();
            let (mutation_path, whiteout_path) = journal.active_paths();
            fs::remove_file(mutation_path).unwrap();
            fs::remove_file(whiteout_path).unwrap();

            crate::db::change_ledger::set_command_authority_override(true);
            let _reset = AuthorityReset;
            let error = db
                .checkpoint_lane_workspace("authority-nfs-corrupt", None)
                .unwrap_err();
            assert!(matches!(
                error,
                Error::ChangeLedgerReconcileRequired { state, .. }
                    if state == "unqualified_view_journal"
            ));
        }

        #[test]
        fn real_nfs_mount_records_new_modified_and_renamed_files() {
            if std::env::var_os("TRAIL_RUN_NFS_COW_TESTS").is_none() {
                return;
            }
            let temp = tempfile::tempdir().unwrap();
            fs::create_dir_all(temp.path().join("src")).unwrap();
            fs::write(temp.path().join("README.md"), "baseline\n").unwrap();
            fs::write(temp.path().join("src/old.txt"), "old\n").unwrap();
            Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let mut db = Trail::open(temp.path()).unwrap();
            let spawned = db
                .spawn_lane_with_workdir_mode_paths_and_neighbors(
                    "nfs-test",
                    Some("main"),
                    LaneWorkdirMode::NfsCow,
                    Some("test".to_string()),
                    None,
                    None,
                    &[],
                    false,
                )
                .unwrap();
            let workdir = PathBuf::from(spawned.workdir.unwrap());
            let mount = db.mount_nfs_cow_workdir_for_lane("nfs-test").unwrap();
            fs::write(workdir.join("README.md"), "changed\n").unwrap();
            fs::create_dir_all(workdir.join("docs")).unwrap();
            fs::write(workdir.join("docs/new.txt"), "new\n").unwrap();
            assert_eq!(fs::read(workdir.join("docs/new.txt")).unwrap(), b"new\n");
            let root_names = fs::read_dir(&workdir)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<BTreeSet<_>>();
            assert!(root_names.contains(OsStr::new("docs")));
            fs::rename(workdir.join("src/old.txt"), workdir.join("src/renamed.txt")).unwrap();
            assert!(db.mount_nfs_cow_workdir_for_lane("nfs-test").is_err());
            let report = db
                .record_lane_workdir("nfs-test", Some("NFS checkpoint".to_string()))
                .unwrap();
            let paths = report
                .changed_paths
                .iter()
                .map(|item| item.path.as_str())
                .collect::<BTreeSet<_>>();
            assert_eq!(
                paths,
                BTreeSet::from(["README.md", "docs/new.txt", "src/renamed.txt"])
            );
            drop(mount);
            assert!(!is_nfs_mount(&workdir));
            let sync = db.sync_lane_workdir("nfs-test", true).unwrap();
            assert!(sync.changed_paths.is_empty());
            let remount = db.mount_nfs_cow_workdir_for_lane("nfs-test").unwrap();
            assert_eq!(fs::read(workdir.join("README.md")).unwrap(), b"changed\n");
            assert_eq!(fs::read(workdir.join("docs/new.txt")).unwrap(), b"new\n");
            drop(remount);
            assert!(!is_nfs_mount(&workdir));
        }

        #[test]
        fn nfs_exec_checkpoint_and_recovery_are_fully_local() {
            if std::env::var_os("TRAIL_RUN_NFS_COW_TESTS").is_none() {
                return;
            }
            let temp = tempfile::tempdir().unwrap();
            fs::write(temp.path().join("README.md"), "baseline\n").unwrap();
            Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let mut db = Trail::open(temp.path()).unwrap();
            db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                "local-exec",
                Some("main"),
                LaneWorkdirMode::NfsCow,
                None,
                None,
                None,
                &[],
                false,
            )
            .unwrap();
            let exec = db
                .exec_lane_workspace(
                    "local-exec",
                    &[
                        "sh".to_string(),
                        "-c".to_string(),
                        "printf 'local\\n' > local.txt".to_string(),
                    ],
                )
                .unwrap();
            assert_eq!(exec.exit_code, 0);
            let workdir = PathBuf::from(db.lane_workdir("local-exec").unwrap().workdir.unwrap());
            assert!(!is_nfs_mount(&workdir));
            let checkpoint = db
                .checkpoint_lane_workspace("local-exec", Some("local".to_string()))
                .unwrap();
            assert_eq!(checkpoint.source_paths, vec!["local.txt"]);
            let built = tempfile::tempdir().unwrap();
            fs::write(built.path().join("tool"), "shared\n").unwrap();
            let key = WorkspaceLayerKeyV1 {
                kind: "tool".to_string(),
                adapter: "manual".to_string(),
                adapter_version: 1,
                inputs: BTreeMap::from([("fixture".to_string(), "v1".to_string())]),
                tool_versions: BTreeMap::new(),
                platform: std::env::consts::OS.to_string(),
                architecture: std::env::consts::ARCH.to_string(),
                portability_scope: "test".to_string(),
                strategy: "fixture".to_string(),
            };
            let layer = db
                .publish_workspace_layer_from_directory(&key, built.path())
                .unwrap();
            db.attach_workspace_layer(
                "local-exec",
                &layer.layer_id,
                "vendor-cache",
                "manual",
                &layer.cache_key,
            )
            .unwrap();
            let gate = db
                .run_lane_test(
                    "local-exec",
                    vec![
                        "sh".to_string(),
                        "-c".to_string(),
                        "test -f vendor-cache/tool".to_string(),
                    ],
                    None,
                    10,
                )
                .unwrap();
            assert!(gate.success);
            assert_eq!(gate.source_root, checkpoint.root_id);
            assert_eq!(gate.environment_keys, vec![layer.cache_key.clone()]);
            assert_eq!(gate.layer_ids, vec![layer.layer_id]);

            db.exec_lane_workspace(
                "local-exec",
                &[
                    "sh".to_string(),
                    "-c".to_string(),
                    "printf 'newer\\n' > local.txt".to_string(),
                ],
            )
            .unwrap();
            let newer = db.checkpoint_lane_workspace("local-exec", None).unwrap();
            assert_ne!(newer.root_id, gate.source_root);
            let readiness = db.lane_readiness("local-exec").unwrap();
            assert!(readiness
                .blockers
                .iter()
                .any(|issue| issue.code == "test_gate_stale_source_root"));
            drop(db);
            let reopened = Trail::open(temp.path()).unwrap();
            reopened.recover_workspace_views().unwrap();
            let entry = reopened
                .root_file_entry(&checkpoint.root_id, "local.txt")
                .unwrap()
                .unwrap();
            assert_eq!(
                reopened.materialize_entry_bytes(&entry).unwrap(),
                b"local\n"
            );
        }

        #[test]
        fn nfs_real_node_layer_bulk_replacement_is_isolated() {
            if std::env::var_os("TRAIL_RUN_NFS_COW_TESTS").is_none() {
                return;
            }
            for tool in ["node", "npm"] {
                assert!(
                    Command::new(tool)
                        .arg("--version")
                        .output()
                        .is_ok_and(|output| output.status.success()),
                    "{tool} is required for the real NFS Node layer acceptance test"
                );
            }

            let temp = tempfile::tempdir().unwrap();
            fs::write(
                temp.path().join("package.json"),
                r#"{"name":"trail-nfs-node","version":"1.0.0","private":true,"dependencies":{"lodash":"4.17.21","prettier":"3.3.3"}}"#,
            )
            .unwrap();
            fs::write(temp.path().join(".gitignore"), "node_modules/\ntarget/\n").unwrap();
            let lock = Command::new("npm")
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

            Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let mut db = Trail::open(temp.path()).unwrap();
            for lane in ["node-nfs-a", "node-nfs-b"] {
                db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                    lane,
                    Some("main"),
                    LaneWorkdirMode::NfsCow,
                    None,
                    None,
                    None,
                    &[],
                    false,
                )
                .unwrap();
            }

            let first = db.sync_node_dependencies("node-nfs-a", None).unwrap();
            let second = db.sync_node_dependencies("node-nfs-b", None).unwrap();
            assert_eq!(first.layer_id, second.layer_id);
            assert_eq!(first.cache_key, second.cache_key);
            assert!(first.entry_count > 500);
            let layer_file = Path::new(&first.storage_path).join("lodash/lodash.js");
            let layer_bin = Path::new(&first.storage_path).join(".bin/prettier");
            let immutable_hash = sha256_hex(&fs::read(&layer_file).unwrap());
            assert!(fs::metadata(&layer_file).unwrap().permissions().readonly());
            assert!(fs::symlink_metadata(&layer_bin)
                .unwrap()
                .file_type()
                .is_symlink());

            let mount_started = std::time::Instant::now();
            let mount_a = db.mount_nfs_cow_workdir_for_lane("node-nfs-a").unwrap();
            let mount_b = db.mount_nfs_cow_workdir_for_lane("node-nfs-b").unwrap();
            let mount_ms = mount_started.elapsed().as_millis();
            let workdir_a = PathBuf::from(db.lane_workdir("node-nfs-a").unwrap().workdir.unwrap());
            let workdir_b = PathBuf::from(db.lane_workdir("node-nfs-b").unwrap().workdir.unwrap());

            for workdir in [&workdir_a, &workdir_b] {
                let node = Command::new("node")
                    .args([
                        "-e",
                        "const _ = require('lodash'); if (_.chunk([1,2,3], 2).length !== 2) process.exit(2)",
                    ])
                    .current_dir(workdir)
                    .output()
                    .unwrap();
                assert!(
                    node.status.success(),
                    "Node could not consume the NFS-mounted layer: {}",
                    String::from_utf8_lossy(&node.stderr)
                );
                let prettier = Command::new("node_modules/.bin/prettier")
                    .arg("--version")
                    .current_dir(workdir)
                    .output()
                    .unwrap();
                assert!(
                    prettier.status.success(),
                    "NFS-mounted npm bin symlink did not execute: {}",
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

            drop(mount_a);
            let replace_started = std::time::Instant::now();
            let replaced = db.sync_node_dependencies("node-nfs-a", None).unwrap();
            let replace_ms = replace_started.elapsed().as_millis();
            assert_eq!(replaced.layer_id, first.layer_id);
            assert!(
                replace_ms < 30_000,
                "bulk dependency replacement took {replace_ms} ms"
            );
            let remount_a = db.mount_nfs_cow_workdir_for_lane("node-nfs-a").unwrap();
            assert_eq!(
                sha256_hex(&fs::read(workdir_a.join("node_modules/lodash/lodash.js")).unwrap()),
                immutable_hash
            );
            assert_eq!(
                sha256_hex(&fs::read(workdir_b.join("node_modules/lodash/lodash.js")).unwrap()),
                immutable_hash
            );
            assert_eq!(sha256_hex(&fs::read(&layer_file).unwrap()), immutable_hash);
            let restored_prettier = Command::new("node_modules/.bin/prettier")
                .arg("--version")
                .current_dir(&workdir_a)
                .output()
                .unwrap();
            assert!(
                restored_prettier.status.success(),
                "bulk-replaced npm bin symlink did not execute through NFS: {}",
                String::from_utf8_lossy(&restored_prettier.stderr)
            );

            let lane_head_before = db.lane_details("node-nfs-a").unwrap().branch.head_change;
            let checkpoint = db.checkpoint_lane_workspace("node-nfs-a", None).unwrap();
            assert!(checkpoint.source_paths.is_empty());
            assert_eq!(checkpoint.generated_dirty_paths, 0);
            assert_eq!(
                db.lane_details("node-nfs-a").unwrap().branch.head_change,
                lane_head_before
            );
            let view_a = db.lane_workspace_view("node-nfs-a").unwrap().unwrap();
            let view_b = db.lane_workspace_view("node-nfs-b").unwrap().unwrap();
            assert!(!Path::new(&view_a.generated_upper)
                .join("node_modules")
                .exists());
            assert!(!Path::new(&view_b.generated_upper)
                .join("node_modules/lodash/lodash.js")
                .exists());
            db.verify_workspace_layer(&first.layer_id).unwrap();
            eprintln!(
                "macos-nfs-node-layer layer_entries={} mount_two_ms={} bulk_replace_ms={} shared_layer={} generated_a_bytes={} generated_b_bytes={}",
                first.entry_count,
                mount_ms,
                replace_ms,
                first.layer_id,
                db.lane_workspace_space("node-nfs-a")
                    .unwrap()
                    .generated_upper_bytes,
                db.lane_workspace_space("node-nfs-b")
                    .unwrap()
                    .generated_upper_bytes,
            );
            drop(mount_b);
            drop(remount_a);
        }

        struct NfsFrameworkFixture<'a> {
            name: &'static str,
            package_json: &'static str,
            files: &'a [(&'static str, &'static str)],
            package_probe: &'static str,
            bin: &'static str,
            build_args: &'static [&'static str],
            build_mode: &'static str,
            build_timeout_secs: u64,
            require_build_success: bool,
            build_output: &'static str,
            min_layer_entries: u64,
        }

        #[test]
        fn nfs_large_nextjs_and_vite_layers_build_and_bulk_replace() {
            if std::env::var_os("TRAIL_RUN_NFS_FRAMEWORK_BENCH").is_none() {
                return;
            }
            for tool in ["node", "npm"] {
                assert!(
                    Command::new(tool)
                        .arg("--version")
                        .output()
                        .is_ok_and(|output| output.status.success()),
                    "{tool} is required for the NFS framework benchmark"
                );
            }

            let next_files = [
                (
                    "app/layout.jsx",
                    "export const metadata = { title: 'Trail Next benchmark' };\nexport default function Layout({ children }) { return <html><body>{children}</body></html>; }\n",
                ),
                (
                    "app/page.jsx",
                    "export default function Page() { return <main>Trail Next benchmark</main>; }\n",
                ),
                (
                    "next.config.mjs",
                    "export default { turbopack: { root: process.cwd() }, outputFileTracingRoot: process.cwd() };\n",
                ),
            ];
            let vite_files = [
                (
                    "index.html",
                    "<div id=\"root\"></div><script type=\"module\" src=\"/src/main.jsx\"></script>\n",
                ),
                (
                    "src/main.jsx",
                    "import React from 'react';\nimport { createRoot } from 'react-dom/client';\ncreateRoot(document.getElementById('root')).render(<main>Trail Vite benchmark</main>);\n",
                ),
                (
                    "vite.config.mjs",
                    "import { defineConfig } from 'vite';\nimport react from '@vitejs/plugin-react';\nexport default defineConfig({ plugins: [react()] });\n",
                ),
            ];
            let fixtures = [
                NfsFrameworkFixture {
                    name: "next",
                    package_json: r#"{"name":"trail-next-nfs-bench","version":"1.0.0","private":true,"scripts":{"build":"next build"},"dependencies":{"next":"16.2.10","react":"19.2.7","react-dom":"19.2.7"}}"#,
                    files: &next_files,
                    package_probe: "next/package.json",
                    bin: "next",
                    build_args: &["build"],
                    build_mode: "turbopack",
                    build_timeout_secs: 120,
                    require_build_success: false,
                    build_output: ".next/BUILD_ID",
                    min_layer_entries: 2_000,
                },
                NfsFrameworkFixture {
                    name: "vite",
                    package_json: r#"{"name":"trail-vite-nfs-bench","version":"1.0.0","private":true,"scripts":{"build":"vite build"},"dependencies":{"react":"19.2.7","react-dom":"19.2.7"},"devDependencies":{"@vitejs/plugin-react":"6.0.3","vite":"8.1.4"}}"#,
                    files: &vite_files,
                    package_probe: "vite/package.json",
                    bin: "vite",
                    build_args: &["build"],
                    build_mode: "rolldown",
                    build_timeout_secs: 120,
                    require_build_success: true,
                    build_output: "dist/index.html",
                    min_layer_entries: 300,
                },
            ];

            let filter = std::env::var("TRAIL_NFS_FRAMEWORK_FILTER").ok();
            let mut ran = 0_u32;
            for fixture in fixtures {
                if filter.as_deref().is_some_and(|name| name != fixture.name) {
                    continue;
                }
                run_nfs_framework_benchmark(fixture);
                ran += 1;
            }
            assert!(ran > 0, "framework benchmark filter matched no fixture");
        }

        fn run_nfs_framework_benchmark(fixture: NfsFrameworkFixture<'_>) {
            let temp = tempfile::tempdir().unwrap();
            fs::write(temp.path().join("package.json"), fixture.package_json).unwrap();
            fs::write(
                temp.path().join(".gitignore"),
                "node_modules/\n.next/\ndist/\ntarget/\n",
            )
            .unwrap();
            for (path, contents) in fixture.files {
                let path = temp.path().join(path);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, contents).unwrap();
            }
            let lock_started = std::time::Instant::now();
            let lock = Command::new("npm")
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
            let lock_ms = lock_started.elapsed().as_millis();
            assert!(
                lock.status.success(),
                "{} lock generation failed: {}",
                fixture.name,
                String::from_utf8_lossy(&lock.stderr)
            );

            Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let mut db = Trail::open(temp.path()).unwrap();
            let lane_a = format!("{}-nfs-a", fixture.name);
            let lane_b = format!("{}-nfs-b", fixture.name);
            for lane in [&lane_a, &lane_b] {
                db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                    lane,
                    Some("main"),
                    LaneWorkdirMode::NfsCow,
                    None,
                    None,
                    None,
                    &[],
                    false,
                )
                .unwrap();
            }

            let cold_started = std::time::Instant::now();
            let first = db.sync_node_dependencies(&lane_a, None).unwrap();
            let cold_sync_ms = cold_started.elapsed().as_millis();
            let hit_started = std::time::Instant::now();
            let second = db.sync_node_dependencies(&lane_b, None).unwrap();
            let cache_hit_ms = hit_started.elapsed().as_millis();
            assert_eq!(first.layer_id, second.layer_id);
            assert_eq!(first.cache_key, second.cache_key);
            assert!(
                first.entry_count >= fixture.min_layer_entries,
                "{} layer had only {} entries",
                fixture.name,
                first.entry_count
            );
            assert!(
                first.logical_bytes >= 10 * 1024 * 1024,
                "{} layer had only {} logical bytes",
                fixture.name,
                first.logical_bytes
            );
            assert!(cache_hit_ms < 30_000);

            let layer_probe = Path::new(&first.storage_path).join(fixture.package_probe);
            let immutable_hash = sha256_hex(&fs::read(&layer_probe).unwrap());
            let layer_bin = Path::new(&first.storage_path)
                .join(".bin")
                .join(fixture.bin);
            assert!(fs::symlink_metadata(&layer_bin)
                .unwrap()
                .file_type()
                .is_symlink());
            db.verify_workspace_layer(&first.layer_id).unwrap();

            let mount_started = std::time::Instant::now();
            let mount_a = db.mount_nfs_cow_workdir_for_lane(&lane_a).unwrap();
            let mount_b = db.mount_nfs_cow_workdir_for_lane(&lane_b).unwrap();
            let mount_two_ms = mount_started.elapsed().as_millis();
            let workdir_a = PathBuf::from(db.lane_workdir(&lane_a).unwrap().workdir.unwrap());
            let workdir_b = PathBuf::from(db.lane_workdir(&lane_b).unwrap().workdir.unwrap());

            let bin_started = std::time::Instant::now();
            let version = Command::new(format!("node_modules/.bin/{}", fixture.bin))
                .arg("--version")
                .current_dir(&workdir_a)
                .output()
                .unwrap();
            let bin_probe_ms = bin_started.elapsed().as_millis();
            assert!(
                version.status.success(),
                "{} bin failed through NFS: {}",
                fixture.name,
                String::from_utf8_lossy(&version.stderr)
            );

            let mut build_command = vec![format!("node_modules/.bin/{}", fixture.bin)];
            build_command.extend(fixture.build_args.iter().map(|arg| (*arg).to_string()));
            let build = run_command_with_timeout_env(
                &build_command,
                &workdir_a,
                Duration::from_secs(fixture.build_timeout_secs),
                &[
                    ("CI".to_string(), "1".to_string()),
                    ("NEXT_TELEMETRY_DISABLED".to_string(), "1".to_string()),
                ],
            )
            .unwrap();
            let build_ms = build.duration_ms;
            if fixture.require_build_success {
                assert!(
                    build.success,
                    "{} build failed through NFS\nstdout:\n{}\nstderr:\n{}",
                    fixture.name,
                    String::from_utf8_lossy(&build.stdout),
                    String::from_utf8_lossy(&build.stderr)
                );
            } else {
                assert!(
                    build.success || build.timed_out,
                    "{} build failed before its benchmark budget\nstdout:\n{}\nstderr:\n{}",
                    fixture.name,
                    String::from_utf8_lossy(&build.stdout),
                    String::from_utf8_lossy(&build.stderr)
                );
            }
            if build.success {
                assert!(workdir_a.join(fixture.build_output).is_file());
            }
            assert!(!workdir_b.join(fixture.build_output).exists());

            let probe_a = workdir_a.join("node_modules").join(fixture.package_probe);
            let probe_b = workdir_b.join("node_modules").join(fixture.package_probe);
            fs::write(&probe_a, "lane-a-private\n").unwrap();
            assert_eq!(fs::read_to_string(&probe_a).unwrap(), "lane-a-private\n");
            assert_eq!(sha256_hex(&fs::read(&probe_b).unwrap()), immutable_hash);
            assert_eq!(sha256_hex(&fs::read(&layer_probe).unwrap()), immutable_hash);

            drop(mount_a);
            let view_a = db.lane_workspace_view(&lane_a).unwrap().unwrap();
            let private_root = Path::new(&view_a.generated_upper).join("node_modules");
            let private_started = std::time::Instant::now();
            let private_entries =
                materialize_private_dependency_tree(Path::new(&first.storage_path), &private_root);
            let private_materialize_ms = private_started.elapsed().as_millis();
            assert_eq!(private_entries, first.entry_count);
            assert!(private_root.join(fixture.package_probe).is_file());

            let replace_started = std::time::Instant::now();
            let replaced = db.sync_node_dependencies(&lane_a, None).unwrap();
            let bulk_replace_ms = replace_started.elapsed().as_millis();
            assert_eq!(replaced.layer_id, first.layer_id);
            assert!(
                bulk_replace_ms < 60_000,
                "{} bulk replacement took {} ms",
                fixture.name,
                bulk_replace_ms
            );
            assert!(!private_root.exists());

            let remount_a = db.mount_nfs_cow_workdir_for_lane(&lane_a).unwrap();
            assert_eq!(sha256_hex(&fs::read(&probe_a).unwrap()), immutable_hash);
            assert_eq!(sha256_hex(&fs::read(&probe_b).unwrap()), immutable_hash);
            if build.success {
                assert!(workdir_a.join(fixture.build_output).is_file());
            }
            let restored_bin = Command::new(format!("node_modules/.bin/{}", fixture.bin))
                .arg("--version")
                .current_dir(&workdir_a)
                .output()
                .unwrap();
            assert!(restored_bin.status.success());

            let checkpoint = db.checkpoint_lane_workspace(&lane_a, None).unwrap();
            assert!(checkpoint.source_paths.is_empty());
            if build.success {
                assert!(checkpoint.generated_dirty_paths > 0);
            }
            let space_a = db.lane_workspace_space(&lane_a).unwrap();
            let space_b = db.lane_workspace_space(&lane_b).unwrap();
            assert_eq!(space_b.generated_upper_bytes, 0);
            eprintln!(
                "macos-nfs-framework name={} build_mode={} layer_entries={} logical_bytes={} physical_bytes={} lock_ms={} cold_sync_ms={} cache_hit_ms={} mount_two_ms={} bin_probe_ms={} build_ms={} build_success={} build_timed_out={} private_materialize_ms={} private_entries={} bulk_replace_ms={} generated_a_bytes={} generated_b_bytes={}",
                fixture.name,
                fixture.build_mode,
                first.entry_count,
                first.logical_bytes,
                first.physical_bytes.unwrap_or(0),
                lock_ms,
                cold_sync_ms,
                cache_hit_ms,
                mount_two_ms,
                bin_probe_ms,
                build_ms,
                build.success,
                build.timed_out,
                private_materialize_ms,
                private_entries,
                bulk_replace_ms,
                space_a.generated_upper_bytes,
                space_b.generated_upper_bytes,
            );
            drop(mount_b);
            drop(remount_a);
        }

        fn materialize_private_dependency_tree(source: &Path, destination: &Path) -> u64 {
            if destination.exists() {
                fs::remove_dir_all(destination).unwrap();
            }
            fs::create_dir_all(destination).unwrap();
            let mut entries = 0_u64;
            for entry in walkdir::WalkDir::new(source).follow_links(false) {
                let entry = entry.unwrap();
                if entry.path() == source {
                    continue;
                }
                entries += 1;
                let relative = entry.path().strip_prefix(source).unwrap();
                let target = destination.join(relative);
                if entry.file_type().is_dir() {
                    fs::create_dir_all(&target).unwrap();
                } else if entry.file_type().is_symlink() {
                    fs::create_dir_all(target.parent().unwrap()).unwrap();
                    std::os::unix::fs::symlink(fs::read_link(entry.path()).unwrap(), target)
                        .unwrap();
                } else {
                    fs::create_dir_all(target.parent().unwrap()).unwrap();
                    clone_or_copy_projected_file(entry.path(), &target).unwrap();
                    let metadata = fs::metadata(entry.path()).unwrap();
                    fs::set_permissions(
                        &target,
                        fs::Permissions::from_mode(metadata.permissions().mode() | 0o200),
                    )
                    .unwrap();
                }
            }
            entries
        }

        #[test]
        fn nfs_cargo_target_seed_is_shared_and_writable_targets_are_isolated() {
            if std::env::var_os("TRAIL_RUN_NFS_COW_TESTS").is_none() {
                return;
            }
            assert!(
                Command::new("cargo")
                    .arg("--version")
                    .output()
                    .is_ok_and(|output| output.status.success()),
                "cargo is required for the real NFS target-layer acceptance test"
            );

            let temp = tempfile::tempdir().unwrap();
            fs::create_dir_all(temp.path().join("src")).unwrap();
            fs::create_dir_all(temp.path().join("shared-dep/src")).unwrap();
            fs::write(
                temp.path().join("Cargo.toml"),
                "[package]\nname = \"nfs-cache-probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nshared-dep = { path = \"shared-dep\" }\n",
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
            let lock = Command::new("cargo")
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
            for lane in ["rust-nfs-a", "rust-nfs-b"] {
                db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                    lane,
                    Some("main"),
                    LaneWorkdirMode::NfsCow,
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
                    "rust-nfs-a",
                    &[
                        "cargo".to_string(),
                        "build".to_string(),
                        "--offline".to_string(),
                    ],
                )
                .unwrap();
            assert_eq!(first.exit_code, 0);
            let lane_a_head = db.lane_details("rust-nfs-a").unwrap().branch.head_change;
            let checkpoint = db.checkpoint_lane_workspace("rust-nfs-a", None).unwrap();
            assert!(checkpoint.source_paths.is_empty());
            assert!(checkpoint.generated_dirty_paths > 0);
            assert_eq!(
                db.lane_details("rust-nfs-a").unwrap().branch.head_change,
                lane_a_head
            );
            let view_a = db.lane_workspace_view("rust-nfs-a").unwrap().unwrap();
            let target_a = PathBuf::from(&view_a.generated_upper).join("target");
            assert!(tree_has_name_fragment(&target_a, "libshared_dep"));

            let layer = db
                .sync_workspace_environment("rust-nfs-b", "cargo", None)
                .unwrap();
            assert_eq!(layer.adapter, "cargo-target-seed");

            let second = db
                .exec_lane_workspace(
                    "rust-nfs-b",
                    &[
                        "cargo".to_string(),
                        "build".to_string(),
                        "--offline".to_string(),
                    ],
                )
                .unwrap();
            assert_eq!(second.exit_code, 0);
            let view_b = db.lane_workspace_view("rust-nfs-b").unwrap().unwrap();
            let target_b = PathBuf::from(&view_b.generated_upper).join("target");
            assert!(
                !tree_has_name_fragment(&target_b, "libshared_dep"),
                "the second NFS lane rebuilt a dependency present in its immutable target seed"
            );
            assert!(tree_has_name_fragment(
                Path::new(&layer.storage_path),
                "libshared_dep"
            ));

            let clean = db
                .exec_lane_workspace("rust-nfs-b", &["cargo".to_string(), "clean".to_string()])
                .unwrap();
            assert_eq!(clean.exit_code, 0);
            assert!(tree_has_name_fragment(&target_a, "libshared_dep"));
            assert!(tree_has_name_fragment(
                Path::new(&layer.storage_path),
                "libshared_dep"
            ));
            db.verify_workspace_layer(&layer.layer_id).unwrap();
            eprintln!(
                "macos-nfs-cargo-layer shared_layer={} producer_bytes={} consumer_bytes={} checkpoint_source_paths={}",
                layer.layer_id,
                db.lane_workspace_space("rust-nfs-a")
                    .unwrap()
                    .generated_upper_bytes,
                db.lane_workspace_space("rust-nfs-b")
                    .unwrap()
                    .generated_upper_bytes,
                checkpoint.source_paths.len(),
            );
        }

        fn tree_has_name_fragment(root: &Path, fragment: &str) -> bool {
            ignore::WalkBuilder::new(root)
                .hidden(false)
                .build()
                .filter_map(std::result::Result::ok)
                .any(|entry| entry.file_name().to_string_lossy().contains(fragment))
        }

        #[test]
        fn nfs_git_checkout_reset_and_clean_are_lane_local() {
            if std::env::var_os("TRAIL_RUN_NFS_COW_TESTS").is_none() {
                return;
            }
            let temp = tempfile::tempdir().unwrap();
            for args in [
                vec!["init", "--quiet"],
                vec!["config", "user.name", "Trail Test"],
                vec!["config", "user.email", "trail@example.invalid"],
            ] {
                assert!(Command::new("git")
                    .arg("-C")
                    .arg(temp.path())
                    .args(args)
                    .status()
                    .unwrap()
                    .success());
            }
            fs::write(temp.path().join("README.md"), "baseline\n").unwrap();
            assert!(Command::new("git")
                .arg("-C")
                .arg(temp.path())
                .args(["add", "README.md"])
                .status()
                .unwrap()
                .success());
            assert!(Command::new("git")
                .arg("-C")
                .arg(temp.path())
                .args(["commit", "--quiet", "-m", "base"])
                .status()
                .unwrap()
                .success());
            let real_head = Command::new("git")
                .arg("-C")
                .arg(temp.path())
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout;
            let real_index = fs::read(temp.path().join(".git/index")).unwrap();

            Trail::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();
            let mut db = Trail::open(temp.path()).unwrap();
            for lane in ["git-a", "git-b"] {
                db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                    lane,
                    Some("main"),
                    LaneWorkdirMode::NfsCow,
                    None,
                    None,
                    None,
                    &[],
                    false,
                )
                .unwrap();
            }
            let run = db
                .exec_lane_workspace(
                    "git-a",
                    &[
                        "sh".to_string(),
                        "-c".to_string(),
                        r#"
set -eu
test "$(cat README.md)" = baseline
printf 'staged\n' > README.md
git add README.md
git reset --hard HEAD
test "$(cat README.md)" = baseline
git checkout -b agent-local
printf 'dirty\n' > README.md
git checkout -- README.md
test "$(cat README.md)" = baseline
mkdir -p node_modules/pkg target/debug
printf 'dependency\n' > node_modules/pkg/index.js
printf 'artifact\n' > target/debug/artifact
git clean -fdx
test ! -e node_modules
test ! -e target
"#
                        .to_string(),
                    ],
                )
                .unwrap();
            assert_eq!(run.exit_code, 0);
            let other = db
                .exec_lane_workspace(
                    "git-b",
                    &[
                        "sh".to_string(),
                        "-c".to_string(),
                        "test \"$(cat README.md)\" = baseline; test ! -e node_modules; test ! -e target/debug/artifact"
                            .to_string(),
                    ],
                )
                .unwrap();
            assert_eq!(other.exit_code, 0);

            let real_after = Command::new("git")
                .arg("-C")
                .arg(temp.path())
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout;
            assert_eq!(real_after, real_head);
            assert_eq!(
                fs::read(temp.path().join(".git/index")).unwrap(),
                real_index
            );
            assert_eq!(
                fs::read_to_string(temp.path().join("README.md")).unwrap(),
                "baseline\n"
            );
            assert!(!temp.path().join(".git/refs/heads/agent-local").exists());
            let view = db.lane_workspace_view("git-a").unwrap().unwrap();
            let shadow = db.workspace_git_shadow(&view).unwrap().unwrap();
            let refs = Command::new("git")
                .env("GIT_DIR", &shadow.git_dir)
                .args(["show-ref", "--verify", "refs/heads/agent-local"])
                .status()
                .unwrap();
            assert!(refs.success());
            assert!(db.lane_readiness("git-a").unwrap().ready);
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) use macos::*;

#[cfg(not(target_os = "macos"))]
pub(crate) struct NfsCowMount;

#[cfg(not(target_os = "macos"))]
impl Drop for NfsCowMount {
    fn drop(&mut self) {}
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn prepare_nfs_cow_workdir(
    _db: &Trail,
    _lane: &str,
    _dir: &Path,
    _custom: bool,
) -> Result<PathBuf> {
    Err(Error::InvalidInput(
        "nfs-cow workdirs are currently supported only on macOS".to_string(),
    ))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn mount_nfs_cow_for_lane(_db: &Trail, _lane: &str) -> Result<NfsCowMount> {
    Err(Error::InvalidInput(
        "nfs-cow workdirs are currently supported only on macOS".to_string(),
    ))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn mount_nfs_cow_for_lane_with_ephemeral_bindings(
    _db: &Trail,
    _lane: &str,
    _source_upper: PathBuf,
    _source_root: ObjectId,
    _bindings: Vec<WorkspaceLayerBinding>,
) -> Result<NfsCowMount> {
    Err(Error::InvalidInput(
        "nfs-cow workdirs are currently supported only on macOS".to_string(),
    ))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn nfs_candidate_paths(_db: &Trail, _lane: &str) -> Result<ViewCheckpointCandidates> {
    Err(Error::InvalidInput(
        "nfs-cow workdirs are currently supported only on macOS".to_string(),
    ))
}

impl Trail {
    pub(crate) fn prepare_nfs_cow_lane_workdir(
        &self,
        lane: &str,
        dir: &Path,
        custom: bool,
    ) -> Result<PathBuf> {
        prepare_nfs_cow_workdir(self, lane, dir, custom)
    }

    pub fn mount_nfs_cow_workdir_for_lane(&self, lane: &str) -> Result<impl Drop + use<>> {
        mount_nfs_cow_for_lane(self, lane)
    }

    pub(crate) fn mount_nfs_cow_workdir_for_lane_with_ephemeral_bindings(
        &self,
        lane: &str,
        source_upper: PathBuf,
        source_root: ObjectId,
        bindings: Vec<WorkspaceLayerBinding>,
    ) -> Result<impl Drop + use<>> {
        mount_nfs_cow_for_lane_with_ephemeral_bindings(
            self,
            lane,
            source_upper,
            source_root,
            bindings,
        )
    }

    pub(crate) fn nfs_cow_candidate_paths_for_lane(
        &self,
        lane: &str,
    ) -> Result<ViewCheckpointCandidates> {
        nfs_candidate_paths(self, lane)
    }

    pub(crate) fn maybe_mount_nfs_cow_workdir_for_lane(
        &self,
        lane: &str,
    ) -> Result<Option<NfsCowMount>> {
        validate_ref_segment(lane)?;
        let branch = self.lane_branch(lane)?;
        let record = self.lane_record(&branch.lane_id)?;
        if self.lane_workdir_mode_for(&record, &branch)? == LaneWorkdirMode::NfsCow {
            Ok(Some(mount_nfs_cow_for_lane(self, lane)?))
        } else {
            Ok(None)
        }
    }
}
