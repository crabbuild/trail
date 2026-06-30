use super::*;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod fuse_overlay {
    use super::*;
    use fuser::{
        FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyCreate, ReplyData,
        ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyWrite, Request,
    };
    use libc::{
        EEXIST, EINVAL, EISDIR, ENOENT, ENOTDIR, EPERM, O_ACCMODE, O_APPEND, O_RDWR, O_TRUNC,
        O_WRONLY,
    };
    use std::collections::{BTreeMap, BTreeSet, HashMap};
    use std::ffi::{OsStr, OsString};
    use std::fs::{self, File, OpenOptions};
    use std::io::Read;
    use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::{Path, PathBuf};
    #[cfg(target_os = "macos")]
    use std::process::{Command, Stdio};
    #[cfg(target_os = "macos")]
    use std::time::Instant;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    const ROOT_INO: u64 = fuser::FUSE_ROOT_ID;
    const TTL: Duration = Duration::from_secs(1);
    const OVERLAY_META_DIR: &str = ".crabdb";
    const WHITEOUTS_FILE: &str = "overlay-whiteouts.json";

    pub(crate) struct OverlayCowMount {
        #[allow(dead_code)]
        session: fuser::BackgroundSession,
        #[allow(dead_code)]
        mountpoint: PathBuf,
    }

    impl OverlayCowMount {
        #[allow(dead_code)]
        pub(crate) fn mountpoint(&self) -> &Path {
            &self.mountpoint
        }
    }

    impl Drop for OverlayCowMount {
        fn drop(&mut self) {}
    }

    pub(crate) fn prepare_overlay_cow_workdir(
        db: &CrabDb,
        lane: &str,
        dir: &Path,
        custom_workdir: bool,
    ) -> Result<PathBuf> {
        prepare_lane_workdir(dir, custom_workdir)?;
        let upperdir = overlay_upperdir(db, lane)?;
        if upperdir.exists() {
            fs::remove_dir_all(&upperdir)?;
        }
        fs::create_dir_all(&upperdir)?;
        fs::create_dir_all(upperdir.join(OVERLAY_META_DIR))?;
        Ok(upperdir)
    }

    pub(crate) fn mount_overlay_cow_for_lane(db: &CrabDb, lane: &str) -> Result<OverlayCowMount> {
        validate_ref_segment(lane)?;
        let branch = db.lane_branch(lane)?;
        let record = db.lane_record(&branch.lane_id)?;
        let mode = db.lane_workdir_mode_for(&record, &branch)?;
        if mode != LaneWorkdirMode::OverlayCow {
            return Err(Error::InvalidInput(format!(
                "lane `{lane}` uses workdir mode `{}`; expected overlay-cow",
                mode.as_str()
            )));
        }
        let Some(workdir) = branch.workdir.clone() else {
            return Err(Error::InvalidInput(format!(
                "overlay-cow lane `{lane}` has no mountpoint"
            )));
        };
        let mountpoint = PathBuf::from(workdir);
        prepare_overlay_mountpoint(&mountpoint, false)?;
        let upperdir = overlay_upperdir(db, lane)?;
        fs::create_dir_all(&upperdir)?;
        fs::create_dir_all(upperdir.join(OVERLAY_META_DIR))?;
        let manifest_path = overlay_clean_manifest_path(&upperdir);
        let write_initial_manifest =
            !manifest_path.is_file() && !upperdir_has_user_content(&upperdir)?;

        let head = db.get_ref(&branch.ref_name)?;
        let lower_files = db.load_root_files(&head.root_id)?;
        let fs = CrabOverlayFs::new(
            db.workspace_root.clone(),
            db.db_dir.clone(),
            upperdir,
            lower_files.clone(),
        )?;
        #[cfg(target_os = "linux")]
        let mut options = vec![MountOption::FSName(format!("crabdb-overlay-cow-{lane}"))];
        #[cfg(target_os = "macos")]
        let options = vec![MountOption::FSName(format!("crabdb-overlay-cow-{lane}"))];
        #[cfg(target_os = "linux")]
        {
            options.push(MountOption::Subtype("crabdb-overlay-cow".to_string()));
            options.push(MountOption::RW);
            options.push(MountOption::NoAtime);
        }
        ensure_platform_fuse_ready()?;
        let session = fuser::spawn_mount2(fs, &mountpoint, &options)
            .map_err(|err| overlay_mount_error(&mountpoint, err))?;

        if write_initial_manifest {
            db.write_clean_workdir_manifest_to_path(
                &mountpoint,
                &manifest_path,
                &head.root_id,
                &lower_files,
                lower_files.keys(),
            )?;
        }

        Ok(OverlayCowMount {
            session,
            mountpoint,
        })
    }

    fn overlay_mount_error(mountpoint: &Path, err: std::io::Error) -> Error {
        Error::InvalidInput(format!(
            "failed to mount overlay-cow workdir at `{}`: {err}. On macOS install macFUSE; on Linux ensure /dev/fuse is available and your user can mount FUSE filesystems.",
            mountpoint.display()
        ))
    }

    #[cfg(target_os = "linux")]
    fn ensure_platform_fuse_ready() -> Result<()> {
        if Path::new("/dev/fuse").exists() {
            return Ok(());
        }
        Err(Error::InvalidInput(
            "overlay-cow workdirs require `/dev/fuse`; enable FUSE for this Linux environment"
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
                "overlay-cow workdirs require macFUSE; install macFUSE and approve its system extension".to_string(),
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

    fn overlay_upperdir(db: &CrabDb, lane: &str) -> Result<PathBuf> {
        let lane = normalize_relative_path(lane)?;
        Ok(db
            .db_dir
            .join("overlay-cow")
            .join(path_from_rel(&lane))
            .join("upper"))
    }

    fn overlay_clean_manifest_path(upperdir: &Path) -> PathBuf {
        upperdir
            .join(OVERLAY_META_DIR)
            .join("workdir-manifest.json")
    }

    fn upperdir_has_user_content(upperdir: &Path) -> Result<bool> {
        let entries = match fs::read_dir(upperdir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(err) => return Err(Error::Io(err)),
        };
        for entry in entries {
            let entry = entry.map_err(Error::Io)?;
            if entry.file_name() != OsStr::new(OVERLAY_META_DIR) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn prepare_overlay_mountpoint(path: &Path, custom_workdir: bool) -> Result<()> {
        if let Ok(metadata) = fs::symlink_metadata(path) {
            if metadata.file_type().is_symlink() {
                return Err(Error::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                    reason: "overlay-cow mountpoint cannot be a symlink".to_string(),
                });
            }
            if !metadata.is_dir() {
                return Err(Error::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                    reason: "overlay-cow mountpoint must be a directory".to_string(),
                });
            }
            if fs::read_dir(path)?.next().transpose()?.is_some() && custom_workdir {
                return Err(Error::InvalidInput(format!(
                    "custom overlay-cow workdir `{}` must be empty",
                    path.display()
                )));
            }
        } else {
            fs::create_dir_all(path)?;
        }
        Ok(())
    }

    struct CrabOverlayFs {
        db: CrabDb,
        upperdir: PathBuf,
        lower_files: BTreeMap<String, FileEntry>,
        lower_dirs: BTreeSet<String>,
        whiteouts: BTreeSet<String>,
        ino_by_path: HashMap<String, u64>,
        path_by_ino: HashMap<u64, String>,
        next_ino: u64,
        handles: HashMap<u64, File>,
        next_fh: u64,
    }

    impl CrabOverlayFs {
        fn new(
            workspace_root: PathBuf,
            db_dir: PathBuf,
            upperdir: PathBuf,
            lower_files: BTreeMap<String, FileEntry>,
        ) -> Result<Self> {
            let db = CrabDb::open_with_db_dir(workspace_root, db_dir)?;
            let mut fs = Self {
                db,
                upperdir,
                lower_files,
                lower_dirs: BTreeSet::new(),
                whiteouts: BTreeSet::new(),
                ino_by_path: HashMap::new(),
                path_by_ino: HashMap::new(),
                next_ino: ROOT_INO + 1,
                handles: HashMap::new(),
                next_fh: 1,
            };
            fs.ino_by_path.insert(String::new(), ROOT_INO);
            fs.path_by_ino.insert(ROOT_INO, String::new());
            fs.rebuild_lower_dirs();
            fs.whiteouts = fs.load_whiteouts()?;
            fs.index_existing_paths()?;
            Ok(fs)
        }

        fn rebuild_lower_dirs(&mut self) {
            self.lower_dirs.insert(String::new());
            for path in self.lower_files.keys() {
                let mut current = Path::new(path).parent();
                while let Some(parent) = current {
                    let rel = parent.to_string_lossy();
                    if rel.is_empty() {
                        break;
                    }
                    self.lower_dirs.insert(rel.to_string());
                    current = parent.parent();
                }
            }
        }

        fn index_existing_paths(&mut self) -> Result<()> {
            let mut paths = BTreeSet::new();
            paths.insert(String::new());
            paths.extend(self.lower_dirs.iter().cloned());
            paths.extend(self.lower_files.keys().cloned());
            for path in self.upper_paths()? {
                paths.insert(path);
            }
            for path in paths {
                self.ensure_ino(&path);
            }
            Ok(())
        }

        fn ensure_ino(&mut self, path: &str) -> u64 {
            if let Some(ino) = self.ino_by_path.get(path) {
                return *ino;
            }
            let ino = self.next_ino;
            self.next_ino += 1;
            self.ino_by_path.insert(path.to_string(), ino);
            self.path_by_ino.insert(ino, path.to_string());
            ino
        }

        fn path_for_ino(&self, ino: u64) -> Option<String> {
            self.path_by_ino.get(&ino).cloned()
        }

        fn load_whiteouts(&self) -> Result<BTreeSet<String>> {
            let path = self.whiteouts_path();
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(BTreeSet::new())
                }
                Err(err) => return Err(Error::Io(err)),
            };
            let paths: Vec<String> = serde_json::from_slice(&bytes)?;
            paths
                .into_iter()
                .map(|path| normalize_relative_path(&path))
                .collect()
        }

        fn save_whiteouts(&self) -> Result<()> {
            let path = self.whiteouts_path();
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let paths = self.whiteouts.iter().cloned().collect::<Vec<_>>();
            write_file_atomic(&path, &serde_json::to_vec(&paths)?, false)
        }

        fn whiteouts_path(&self) -> PathBuf {
            self.upperdir.join(OVERLAY_META_DIR).join(WHITEOUTS_FILE)
        }

        fn is_whiteouted(&self, path: &str) -> bool {
            self.whiteouts.iter().any(|whiteout| {
                path == whiteout
                    || path
                        .strip_prefix(whiteout)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            })
        }

        fn upper_path(&self, path: &str) -> PathBuf {
            if path.is_empty() {
                return self.upperdir.clone();
            }
            self.upperdir.join(path_from_rel(path))
        }

        fn upper_paths(&self) -> Result<Vec<String>> {
            let mut paths = Vec::new();
            if !self.upperdir.exists() {
                return Ok(paths);
            }
            let walker = walkdir::WalkDir::new(&self.upperdir).into_iter();
            for entry in walker {
                let entry = entry.map_err(|err| Error::InvalidInput(err.to_string()))?;
                if entry.path() == self.upperdir {
                    continue;
                }
                let rel = entry
                    .path()
                    .strip_prefix(&self.upperdir)
                    .map_err(|err| Error::InvalidInput(err.to_string()))?;
                let rel = normalize_relative_path(&rel.to_string_lossy())?;
                if rel == OVERLAY_META_DIR || rel.starts_with(".crabdb/") {
                    continue;
                }
                paths.push(rel);
            }
            Ok(paths)
        }

        fn parent_path(parent: &str, name: &OsStr) -> std::result::Result<String, i32> {
            let Some(name) = name.to_str() else {
                return Err(EINVAL);
            };
            if name.is_empty() || name.contains('/') {
                return Err(EINVAL);
            }
            if parent.is_empty() {
                Ok(name.to_string())
            } else {
                Ok(format!("{parent}/{name}"))
            }
        }

        fn parent_of(path: &str) -> String {
            Path::new(path)
                .parent()
                .map(|parent| parent.to_string_lossy().to_string())
                .unwrap_or_default()
        }

        fn file_name(path: &str) -> OsString {
            Path::new(path)
                .file_name()
                .map(OsString::from)
                .unwrap_or_else(|| OsString::from(""))
        }

        fn lower_dir_exists(&self, path: &str) -> bool {
            self.lower_dirs.contains(path) && !self.is_whiteouted(path)
        }

        fn lower_file_exists(&self, path: &str) -> bool {
            self.lower_files.contains_key(path) && !self.is_whiteouted(path)
        }

        fn upper_metadata(&self, path: &str) -> Option<fs::Metadata> {
            fs::symlink_metadata(self.upper_path(path)).ok()
        }

        fn node_kind(&self, path: &str) -> Option<FileType> {
            if path.is_empty() {
                return Some(FileType::Directory);
            }
            if path == OVERLAY_META_DIR || path.starts_with(".crabdb/") {
                return None;
            }
            if let Some(metadata) = self.upper_metadata(path) {
                if metadata.is_dir() {
                    return Some(FileType::Directory);
                }
                if metadata.is_file() {
                    return Some(FileType::RegularFile);
                }
                return None;
            }
            if self.lower_dir_exists(path) {
                return Some(FileType::Directory);
            }
            if self.lower_file_exists(path) {
                return Some(FileType::RegularFile);
            }
            None
        }

        fn attr_for_path(&mut self, path: &str) -> Option<FileAttr> {
            let ino = self.ensure_ino(path);
            if path.is_empty() {
                return Some(dir_attr(ino));
            }
            if path == OVERLAY_META_DIR || path.starts_with(".crabdb/") {
                return None;
            }
            if let Some(metadata) = self.upper_metadata(path) {
                return Some(attr_from_metadata(ino, &metadata));
            }
            if self.lower_dir_exists(path) {
                return Some(dir_attr(ino));
            }
            self.lower_files
                .get(path)
                .filter(|_| !self.is_whiteouted(path))
                .map(|entry| lower_file_attr(ino, entry))
        }

        fn load_lower_bytes(&self, path: &str) -> Result<Vec<u8>> {
            let entry = self
                .lower_files
                .get(path)
                .ok_or_else(|| Error::InvalidPath {
                    path: path.to_string(),
                    reason: "lower file is missing".to_string(),
                })?;
            self.db.materialize_entry_bytes(entry)
        }

        fn ensure_upper_parent(&self, path: &str) -> std::io::Result<()> {
            if let Some(parent) = self.upper_path(path).parent() {
                fs::create_dir_all(parent)?;
            }
            Ok(())
        }

        fn ensure_upper_file(
            &mut self,
            path: &str,
            truncate: bool,
        ) -> std::result::Result<File, i32> {
            if self
                .node_kind(path)
                .is_some_and(|kind| kind == FileType::Directory)
            {
                return Err(EISDIR);
            }
            let upper = self.upper_path(path);
            self.ensure_upper_parent(path).map_err(io_err)?;
            if !upper.exists() {
                if !truncate && self.lower_file_exists(path) {
                    let bytes = self.load_lower_bytes(path).map_err(|_| EINVAL)?;
                    fs::write(&upper, bytes).map_err(io_err)?;
                    let perm = self.lower_files.get(path).map(file_perm).unwrap_or(0o644);
                    let _ = fs::set_permissions(&upper, fs::Permissions::from_mode(perm as u32));
                } else {
                    File::create(&upper).map_err(io_err)?;
                }
            }
            if truncate {
                OpenOptions::new()
                    .write(true)
                    .open(&upper)
                    .and_then(|file| file.set_len(0))
                    .map_err(io_err)?;
            }
            self.whiteouts.remove(path);
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .mode(0o666)
                .open(&upper)
                .map_err(io_err)?;
            self.ensure_ino(path);
            Ok(file)
        }

        fn insert_handle(&mut self, file: File) -> u64 {
            let fh = self.next_fh;
            self.next_fh += 1;
            self.handles.insert(fh, file);
            fh
        }

        fn children(&mut self, path: &str) -> Vec<(u64, FileType, OsString)> {
            let mut names = BTreeMap::<String, FileType>::new();
            for dir in self.lower_dirs.clone() {
                if dir.is_empty() || self.is_whiteouted(&dir) {
                    continue;
                }
                if Self::parent_of(&dir) == path {
                    names.insert(
                        Self::file_name(&dir).to_string_lossy().to_string(),
                        FileType::Directory,
                    );
                }
            }
            for file in self.lower_files.keys() {
                if self.is_whiteouted(file) {
                    continue;
                }
                if Self::parent_of(file) == path {
                    names.insert(
                        Self::file_name(file).to_string_lossy().to_string(),
                        FileType::RegularFile,
                    );
                }
            }
            if let Ok(read_dir) = fs::read_dir(self.upper_path(path)) {
                for entry in read_dir.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if path.is_empty() && name == OVERLAY_META_DIR {
                        continue;
                    }
                    let child = if path.is_empty() {
                        name.clone()
                    } else {
                        format!("{path}/{name}")
                    };
                    if let Some(kind) = self.node_kind(&child) {
                        names.insert(name, kind);
                    }
                }
            }
            names
                .into_iter()
                .map(|(name, kind)| {
                    let child = if path.is_empty() {
                        name.clone()
                    } else {
                        format!("{path}/{name}")
                    };
                    (self.ensure_ino(&child), kind, OsString::from(name))
                })
                .collect()
        }
    }

    impl Filesystem for CrabOverlayFs {
        fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
            let Some(parent_path) = self.path_for_ino(parent) else {
                reply.error(ENOENT);
                return;
            };
            let path = match Self::parent_path(&parent_path, name) {
                Ok(path) => path,
                Err(err) => {
                    reply.error(err);
                    return;
                }
            };
            match self.attr_for_path(&path) {
                Some(attr) => reply.entry(&TTL, &attr, 0),
                None => reply.error(ENOENT),
            }
        }

        fn getattr(&mut self, _req: &Request<'_>, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
            let Some(path) = self.path_for_ino(ino) else {
                reply.error(ENOENT);
                return;
            };
            match self.attr_for_path(&path) {
                Some(attr) => reply.attr(&TTL, &attr),
                None => reply.error(ENOENT),
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
            let Some(path) = self.path_for_ino(ino) else {
                reply.error(ENOENT);
                return;
            };
            if let Some(size) = size {
                match self.ensure_upper_file(&path, false) {
                    Ok(file) => {
                        if let Err(err) = file.set_len(size) {
                            reply.error(io_err(err));
                            return;
                        }
                    }
                    Err(err) => {
                        reply.error(err);
                        return;
                    }
                }
            }
            if let Some(mode) = mode {
                let upper = self.upper_path(&path);
                if upper.exists() {
                    let _ = fs::set_permissions(upper, fs::Permissions::from_mode(mode & 0o777));
                }
            }
            match self.attr_for_path(&path) {
                Some(attr) => reply.attr(&TTL, &attr),
                None => reply.error(ENOENT),
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
            let Some(parent_path) = self.path_for_ino(parent) else {
                reply.error(ENOENT);
                return;
            };
            let path = match Self::parent_path(&parent_path, name) {
                Ok(path) => path,
                Err(err) => {
                    reply.error(err);
                    return;
                }
            };
            if self.node_kind(&path).is_some() {
                reply.error(EEXIST);
                return;
            }
            let upper = self.upper_path(&path);
            if let Err(err) = fs::create_dir_all(&upper) {
                reply.error(io_err(err));
                return;
            }
            let perm = (mode & !umask) & 0o777;
            let _ = fs::set_permissions(&upper, fs::Permissions::from_mode(perm));
            self.whiteouts.remove(&path);
            match self.attr_for_path(&path) {
                Some(attr) => reply.entry(&TTL, &attr, 0),
                None => reply.error(ENOENT),
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
            let Some(parent_path) = self.path_for_ino(parent) else {
                reply.error(ENOENT);
                return;
            };
            let path = match Self::parent_path(&parent_path, name) {
                Ok(path) => path,
                Err(err) => {
                    reply.error(err);
                    return;
                }
            };
            if self.node_kind(&path).is_some() {
                reply.error(EEXIST);
                return;
            }
            let upper = self.upper_path(&path);
            if let Some(parent) = upper.parent() {
                if let Err(err) = fs::create_dir_all(parent) {
                    reply.error(io_err(err));
                    return;
                }
            }
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode((mode & !umask) & 0o777)
                .open(&upper)
            {
                Ok(_) => {
                    self.whiteouts.remove(&path);
                    match self.attr_for_path(&path) {
                        Some(attr) => reply.entry(&TTL, &attr, 0),
                        None => reply.error(ENOENT),
                    }
                }
                Err(err) => reply.error(io_err(err)),
            }
        }

        fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
            let Some(parent_path) = self.path_for_ino(parent) else {
                reply.error(ENOENT);
                return;
            };
            let path = match Self::parent_path(&parent_path, name) {
                Ok(path) => path,
                Err(err) => {
                    reply.error(err);
                    return;
                }
            };
            if self.upper_metadata(&path).is_some() {
                if let Err(err) = fs::remove_file(self.upper_path(&path)) {
                    reply.error(io_err(err));
                    return;
                }
            }
            if self.lower_file_exists(&path) {
                self.whiteouts.insert(path);
                if let Err(err) = self.save_whiteouts() {
                    reply.error(error_errno(err));
                    return;
                }
            }
            reply.ok();
        }

        fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
            let Some(parent_path) = self.path_for_ino(parent) else {
                reply.error(ENOENT);
                return;
            };
            let path = match Self::parent_path(&parent_path, name) {
                Ok(path) => path,
                Err(err) => {
                    reply.error(err);
                    return;
                }
            };
            if self.upper_metadata(&path).is_some() {
                if let Err(err) = fs::remove_dir(self.upper_path(&path)) {
                    reply.error(io_err(err));
                    return;
                }
            }
            if self.lower_dir_exists(&path) {
                self.whiteouts.insert(path);
                if let Err(err) = self.save_whiteouts() {
                    reply.error(error_errno(err));
                    return;
                }
            }
            reply.ok();
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
            let Some(parent_path) = self.path_for_ino(parent) else {
                reply.error(ENOENT);
                return;
            };
            let Some(newparent_path) = self.path_for_ino(newparent) else {
                reply.error(ENOENT);
                return;
            };
            let old_path = match Self::parent_path(&parent_path, name) {
                Ok(path) => path,
                Err(err) => {
                    reply.error(err);
                    return;
                }
            };
            let new_path = match Self::parent_path(&newparent_path, newname) {
                Ok(path) => path,
                Err(err) => {
                    reply.error(err);
                    return;
                }
            };
            if let Some(parent) = self.upper_path(&new_path).parent() {
                if let Err(err) = fs::create_dir_all(parent) {
                    reply.error(io_err(err));
                    return;
                }
            }
            if self.upper_metadata(&old_path).is_some() {
                if let Err(err) = fs::rename(self.upper_path(&old_path), self.upper_path(&new_path))
                {
                    reply.error(io_err(err));
                    return;
                }
            } else if self.lower_file_exists(&old_path) {
                let bytes = match self.load_lower_bytes(&old_path) {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        reply.error(error_errno(err));
                        return;
                    }
                };
                if let Err(err) = fs::write(self.upper_path(&new_path), bytes) {
                    reply.error(io_err(err));
                    return;
                }
                self.whiteouts.insert(old_path);
            } else if self.lower_dir_exists(&old_path) {
                reply.error(EPERM);
                return;
            } else {
                reply.error(ENOENT);
                return;
            }
            self.whiteouts.remove(&new_path);
            if let Err(err) = self.save_whiteouts() {
                reply.error(error_errno(err));
                return;
            }
            self.ensure_ino(&new_path);
            reply.ok();
        }

        fn open(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
            let Some(path) = self.path_for_ino(ino) else {
                reply.error(ENOENT);
                return;
            };
            let write = wants_write(flags);
            let truncate = flags & O_TRUNC != 0;
            if write || truncate {
                match self.ensure_upper_file(&path, truncate) {
                    Ok(file) => {
                        let fh = self.insert_handle(file);
                        reply.opened(fh, 0);
                    }
                    Err(err) => reply.error(err),
                }
                return;
            }
            if self.upper_metadata(&path).is_some() {
                match OpenOptions::new().read(true).open(self.upper_path(&path)) {
                    Ok(file) => {
                        let fh = self.insert_handle(file);
                        reply.opened(fh, 0);
                    }
                    Err(err) => reply.error(io_err(err)),
                }
            } else if self.lower_file_exists(&path) {
                reply.opened(0, 0);
            } else {
                reply.error(ENOENT);
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
            let Some(parent_path) = self.path_for_ino(parent) else {
                reply.error(ENOENT);
                return;
            };
            let path = match Self::parent_path(&parent_path, name) {
                Ok(path) => path,
                Err(err) => {
                    reply.error(err);
                    return;
                }
            };
            if let Some(parent) = self.upper_path(&path).parent() {
                if let Err(err) = fs::create_dir_all(parent) {
                    reply.error(io_err(err));
                    return;
                }
            }
            let file = match OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(flags & O_TRUNC != 0)
                .mode((mode & !umask) & 0o777)
                .open(self.upper_path(&path))
            {
                Ok(file) => file,
                Err(err) => {
                    reply.error(io_err(err));
                    return;
                }
            };
            self.whiteouts.remove(&path);
            let fh = self.insert_handle(file);
            match self.attr_for_path(&path) {
                Some(attr) => reply.created(&TTL, &attr, 0, fh, 0),
                None => reply.error(ENOENT),
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
            let Some(path) = self.path_for_ino(ino) else {
                reply.error(ENOENT);
                return;
            };
            let offset = offset.max(0) as usize;
            let size = size as usize;
            if let Some(file) = self.handles.get(&fh) {
                let mut buffer = vec![0; size];
                match file.read_at(&mut buffer, offset as u64) {
                    Ok(read) => {
                        buffer.truncate(read);
                        reply.data(&buffer);
                    }
                    Err(err) => reply.error(io_err(err)),
                }
                return;
            }
            if self.upper_metadata(&path).is_some() {
                match File::open(self.upper_path(&path)) {
                    Ok(mut file) => {
                        let mut bytes = Vec::new();
                        if let Err(err) = file.read_to_end(&mut bytes) {
                            reply.error(io_err(err));
                            return;
                        }
                        reply.data(slice_bytes(&bytes, offset, size));
                    }
                    Err(err) => reply.error(io_err(err)),
                }
            } else if self.lower_file_exists(&path) {
                match self.load_lower_bytes(&path) {
                    Ok(bytes) => reply.data(slice_bytes(&bytes, offset, size)),
                    Err(err) => reply.error(error_errno(err)),
                }
            } else {
                reply.error(ENOENT);
            }
        }

        fn write(
            &mut self,
            _req: &Request<'_>,
            ino: u64,
            fh: u64,
            offset: i64,
            data: &[u8],
            _write_flags: u32,
            flags: i32,
            _lock_owner: Option<u64>,
            reply: ReplyWrite,
        ) {
            let Some(path) = self.path_for_ino(ino) else {
                reply.error(ENOENT);
                return;
            };
            if !self.handles.contains_key(&fh) {
                match self.ensure_upper_file(&path, false) {
                    Ok(file) => {
                        self.handles.insert(fh, file);
                    }
                    Err(err) => {
                        reply.error(err);
                        return;
                    }
                }
            }
            let Some(file) = self.handles.get(&fh) else {
                reply.error(ENOENT);
                return;
            };
            let offset = if flags & O_APPEND != 0 {
                match file.metadata() {
                    Ok(metadata) => metadata.len(),
                    Err(err) => {
                        reply.error(io_err(err));
                        return;
                    }
                }
            } else {
                offset.max(0) as u64
            };
            match file.write_at(data, offset) {
                Ok(written) => reply.written(written as u32),
                Err(err) => reply.error(io_err(err)),
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
            let Some(path) = self.path_for_ino(ino) else {
                reply.error(ENOENT);
                return;
            };
            if self.node_kind(&path) == Some(FileType::Directory) {
                reply.opened(0, 0);
            } else {
                reply.error(ENOTDIR);
            }
        }

        fn readdir(
            &mut self,
            _req: &Request<'_>,
            ino: u64,
            _fh: u64,
            offset: i64,
            mut reply: ReplyDirectory,
        ) {
            let Some(path) = self.path_for_ino(ino) else {
                reply.error(ENOENT);
                return;
            };
            if self.node_kind(&path) != Some(FileType::Directory) {
                reply.error(ENOTDIR);
                return;
            }
            let parent_ino = if path.is_empty() {
                ROOT_INO
            } else {
                let parent = Self::parent_of(&path);
                self.ensure_ino(&parent)
            };
            let mut entries = vec![
                (ino, FileType::Directory, OsString::from(".")),
                (parent_ino, FileType::Directory, OsString::from("..")),
            ];
            entries.extend(self.children(&path));
            for (idx, (entry_ino, kind, name)) in
                entries.into_iter().enumerate().skip(offset as usize)
            {
                if reply.add(entry_ino, (idx + 1) as i64, kind, name) {
                    break;
                }
            }
            reply.ok();
        }

        fn statfs(&mut self, _req: &Request<'_>, _ino: u64, reply: fuser::ReplyStatfs) {
            reply.statfs(0, 0, 0, 0, 0, 512, 255, 0);
        }

        fn access(&mut self, _req: &Request<'_>, ino: u64, _mask: i32, reply: ReplyEmpty) {
            if self.path_for_ino(ino).is_some() {
                reply.ok();
            } else {
                reply.error(ENOENT);
            }
        }
    }

    fn wants_write(flags: i32) -> bool {
        matches!(flags & O_ACCMODE, O_WRONLY | O_RDWR) || flags & O_TRUNC != 0
    }

    fn slice_bytes(bytes: &[u8], offset: usize, size: usize) -> &[u8] {
        if offset >= bytes.len() {
            return &[];
        }
        let end = offset.saturating_add(size).min(bytes.len());
        &bytes[offset..end]
    }

    fn file_perm(entry: &FileEntry) -> u16 {
        let mode = (entry.mode & 0o777) as u16;
        if mode != 0 {
            mode
        } else if entry.executable {
            0o755
        } else {
            0o644
        }
    }

    fn lower_file_attr(ino: u64, entry: &FileEntry) -> FileAttr {
        let perm = file_perm(entry);
        FileAttr {
            ino,
            size: entry.size_bytes,
            blocks: entry.size_bytes.saturating_add(511) / 512,
            atime: stable_time(),
            mtime: stable_time(),
            ctime: stable_time(),
            crtime: stable_time(),
            kind: FileType::RegularFile,
            perm,
            nlink: 1,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    fn dir_attr(ino: u64) -> FileAttr {
        FileAttr {
            ino,
            size: 0,
            blocks: 0,
            atime: stable_time(),
            mtime: stable_time(),
            ctime: stable_time(),
            crtime: stable_time(),
            kind: FileType::Directory,
            perm: 0o755,
            nlink: 2,
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    fn attr_from_metadata(ino: u64, metadata: &fs::Metadata) -> FileAttr {
        let kind = if metadata.is_dir() {
            FileType::Directory
        } else {
            FileType::RegularFile
        };
        FileAttr {
            ino,
            size: metadata.len(),
            blocks: metadata.blocks(),
            atime: system_time(metadata.atime(), metadata.atime_nsec()),
            mtime: system_time(metadata.mtime(), metadata.mtime_nsec()),
            ctime: system_time(metadata.ctime(), metadata.ctime_nsec()),
            crtime: system_time(metadata.ctime(), metadata.ctime_nsec()),
            kind,
            perm: (metadata.mode() & 0o777) as u16,
            nlink: metadata.nlink() as u32,
            uid: metadata.uid(),
            gid: metadata.gid(),
            rdev: metadata.rdev() as u32,
            blksize: metadata.blksize() as u32,
            flags: 0,
        }
    }

    fn stable_time() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1)
    }

    fn system_time(sec: i64, nsec: i64) -> SystemTime {
        if sec >= 0 {
            UNIX_EPOCH + Duration::new(sec as u64, nsec.max(0) as u32)
        } else {
            UNIX_EPOCH
        }
    }

    fn io_err(err: std::io::Error) -> i32 {
        err.raw_os_error().unwrap_or(EINVAL)
    }

    fn error_errno(err: Error) -> i32 {
        match err {
            Error::Io(err) => io_err(err),
            Error::InvalidPath { .. } => EINVAL,
            Error::InvalidInput(_) => EINVAL,
            Error::WorkspaceNotFound(_) => ENOENT,
            _ => EINVAL,
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) use fuse_overlay::*;

#[cfg(target_os = "windows")]
mod dokan_overlay;

#[cfg(target_os = "windows")]
pub(crate) use dokan_overlay::*;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) struct OverlayCowMount {
    #[allow(dead_code)]
    mountpoint: PathBuf,
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
impl OverlayCowMount {
    #[allow(dead_code)]
    pub(crate) fn mountpoint(&self) -> &Path {
        &self.mountpoint
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
impl Drop for OverlayCowMount {
    fn drop(&mut self) {}
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) fn prepare_overlay_cow_workdir(
    _db: &CrabDb,
    _lane: &str,
    _dir: &Path,
    _custom_workdir: bool,
) -> Result<PathBuf> {
    Err(Error::InvalidInput(
        "overlay-cow workdirs require Linux/macOS FUSE support or Windows Dokan support"
            .to_string(),
    ))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) fn mount_overlay_cow_for_lane(_db: &CrabDb, lane: &str) -> Result<OverlayCowMount> {
    Err(Error::InvalidInput(format!(
        "overlay-cow lane `{lane}` cannot be mounted on this platform"
    )))
}

impl CrabDb {
    pub(crate) fn overlay_clean_workdir_manifest_path_for_lane(
        &self,
        lane: &str,
    ) -> Result<PathBuf> {
        let lane = normalize_relative_path(lane)?;
        Ok(self
            .db_dir
            .join("overlay-cow")
            .join(path_from_rel(&lane))
            .join("upper")
            .join(".crabdb")
            .join("workdir-manifest.json"))
    }

    pub(crate) fn prepare_overlay_cow_lane_workdir(
        &self,
        lane: &str,
        dir: &Path,
        custom_workdir: bool,
    ) -> Result<PathBuf> {
        prepare_overlay_cow_workdir(self, lane, dir, custom_workdir)
    }

    pub fn mount_overlay_cow_workdir_for_lane(&self, lane: &str) -> Result<impl Drop> {
        mount_overlay_cow_for_lane(self, lane)
    }

    pub(crate) fn maybe_mount_overlay_cow_workdir_for_lane(
        &self,
        lane: &str,
    ) -> Result<Option<OverlayCowMount>> {
        validate_ref_segment(lane)?;
        let branch = self.lane_branch(lane)?;
        let record = self.lane_record(&branch.lane_id)?;
        if self.lane_workdir_mode_for(&record, &branch)? == LaneWorkdirMode::OverlayCow {
            Ok(Some(mount_overlay_cow_for_lane(self, lane)?))
        } else {
            Ok(None)
        }
    }
}
