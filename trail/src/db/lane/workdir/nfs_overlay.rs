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
    #[cfg(test)]
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    use std::sync::Mutex;
    use std::thread::{self, JoinHandle};

    const ROOT_INO: u64 = 1;
    const OVERLAY_META_DIR: &str = ".trail";
    const MOUNT_STATE_FILE: &str = "mount.json";

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
            _dirid: fileid3,
            _linkname: &filename3,
            _symlink: &nfspath3,
            _attr: &sattr3,
        ) -> std::result::Result<(fileid3, fattr3), nfsstat3> {
            Err(nfsstat3::NFS3ERR_NOTSUPP)
        }
        async fn readlink(&self, _id: fileid3) -> std::result::Result<nfspath3, nfsstat3> {
            Err(nfsstat3::NFS3ERR_NOTSUPP)
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

    pub(crate) fn nfs_clean_manifest_path(db: &Trail, lane: &str) -> Result<PathBuf> {
        Ok(db
            .workspace_view_paths_for_lane_name(lane)
            .meta_dir
            .join("workdir-manifest.json"))
    }

    pub(crate) fn nfs_candidate_paths(db: &Trail, lane: &str) -> Result<Vec<String>> {
        let upper = nfs_upperdir(db, lane)?;
        let branch = db.lane_branch(lane)?;
        let head = db.get_ref(&branch.ref_name)?;
        Ok(
            recover_view_checkpoint_candidates_for_root(db, &upper, &head.root_id)?
                .paths
                .into_iter()
                .filter(|path| {
                    !Path::new(path)
                        .file_name()
                        .and_then(OsStr::to_str)
                        .is_some_and(is_macos_junk)
                })
                .collect(),
        )
    }

    pub(crate) fn mount_nfs_cow_for_lane(db: &Trail, lane: &str) -> Result<NfsCowMount> {
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
        let mut lease = db.acquire_workspace_mount_lease(lane, "nfs")?;
        fs::create_dir_all(&mountpoint)?;
        let upper = nfs_upperdir(db, lane)?;
        fs::create_dir_all(upper.join(OVERLAY_META_DIR))?;
        let state_path = db
            .workspace_view_paths_for_lane_name(lane)
            .meta_dir
            .join(MOUNT_STATE_FILE);
        let head = db.get_ref(&branch.ref_name)?;
        let core = CowCore::new_lazy(
            Trail::open_with_db_dir(db.workspace_root.clone(), db.db_dir.clone())?,
            upper.clone(),
            head.root_id,
        )?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| Error::InvalidInput(err.to_string()))?;
        let adapter = NfsAdapter::new(core);
        let listener = runtime
            .block_on(NFSTcpListener::bind("127.0.0.1:0", adapter))
            .map_err(Error::Io)?;
        let port = listener.get_listen_port();
        recover_stale_mount(&mountpoint, &state_path)?;
        let state_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&state_path)
            .map_err(|err| {
                Error::InvalidInput(format!(
                    "nfs-cow lane `{lane}` is already being mounted: {err}"
                ))
            })?;
        serde_json::to_writer(
            &state_file,
            &serde_json::json!({"pid": std::process::id(), "port": port, "mountpoint": mountpoint}),
        )?;
        state_file.sync_all()?;
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let worker = thread::spawn(move || {
            runtime.block_on(async move {
                tokio::select! { _ = listener.handle_forever() => {}, _ = shutdown_rx => {} }
            })
        });
        // Directory mutations must be visible to the checkpoint scan that runs
        // immediately after the agent exits. Attribute caching can otherwise
        // retain a pre-create READDIR result and omit brand-new paths.
        let opts = format!("locallocks,vers=3,tcp,rsize=1048576,wsize=1048576,noac,actimeo=0,nonegnamecache,nobrowse,port={port},mountport={port}");
        let status = Command::new("/sbin/mount_nfs")
            .args(["-o", &opts, "127.0.0.1:/", &mountpoint.to_string_lossy()])
            .status()?;
        if !status.success() {
            let _ = shutdown_tx.send(());
            let _ = worker.join();
            let _ = fs::remove_file(&state_path);
            return Err(Error::InvalidInput(format!(
                "mount_nfs failed for `{}` with {status}",
                mountpoint.display()
            )));
        }
        if !is_nfs_mount(&mountpoint) {
            let _ = shutdown_tx.send(());
            let _ = worker.join();
            let _ = fs::remove_file(&state_path);
            return Err(Error::InvalidInput(format!(
                "mount_nfs returned success, but `{}` is not an active NFS mount",
                mountpoint.display()
            )));
        }
        lease.mark_mounted()?;
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
        if let Ok(bytes) = fs::read(state) {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if let Some(pid) = value.get("pid").and_then(serde_json::Value::as_i64) {
                    if unsafe { libc::kill(pid as i32, 0) } == 0 {
                        return Err(Error::InvalidInput(format!(
                            "nfs-cow mount `{}` is already active in process {pid}",
                            mountpoint.display()
                        )));
                    }
                }
            }
        }
        if is_nfs_mount(mountpoint) {
            unmount(mountpoint)?;
        }
        let _ = fs::remove_file(state);
        Ok(())
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
            ftype: if attr.kind == NodeKind::Directory {
                ftype3::NF3DIR
            } else {
                ftype3::NF3REG
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
            assert!(matches!(
                core.mkdir(ROOT_INO, "escape", 0o755),
                Err(libc::EPERM)
            ));
            assert_eq!(core.upper_path("escape/file"), Err(libc::EPERM));
            assert!(!outside.join("file").exists());
        }

        #[test]
        fn stale_mount_state_is_removed_for_a_dead_process() {
            let temp = tempfile::tempdir().unwrap();
            let mountpoint = temp.path().join("mount");
            fs::create_dir_all(&mountpoint).unwrap();
            let state = temp.path().join(MOUNT_STATE_FILE);
            fs::write(&state, br#"{"pid":2147483647,"port":1}"#).unwrap();

            recover_stale_mount(&mountpoint, &state).unwrap();

            assert!(!state.exists());
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
                assert!(matches!(
                    adapter
                        .symlink(
                            ROOT_INO,
                            &nfsstring(b"link".to_vec()),
                            &nfsstring(b"target".to_vec()),
                            &sattr3::default(),
                        )
                        .await,
                    Err(nfsstat3::NFS3ERR_NOTSUPP)
                ));
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
pub(crate) fn nfs_clean_manifest_path(_db: &Trail, _lane: &str) -> Result<PathBuf> {
    Err(Error::InvalidInput(
        "nfs-cow workdirs are currently supported only on macOS".to_string(),
    ))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn nfs_candidate_paths(_db: &Trail, _lane: &str) -> Result<Vec<String>> {
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

    pub fn mount_nfs_cow_workdir_for_lane(&self, lane: &str) -> Result<impl Drop> {
        mount_nfs_cow_for_lane(self, lane)
    }

    pub(crate) fn nfs_clean_workdir_manifest_path_for_lane(&self, lane: &str) -> Result<PathBuf> {
        nfs_clean_manifest_path(self, lane)
    }

    pub(crate) fn nfs_cow_candidate_paths_for_lane(&self, lane: &str) -> Result<Vec<String>> {
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
