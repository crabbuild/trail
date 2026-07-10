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
    use std::collections::{BTreeMap, BTreeSet, HashMap};
    use std::ffi::{CStr, CString, OsStr};
    use std::fs::{self, File, OpenOptions};
    use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::process::Command;
    use std::sync::Mutex;
    use std::thread::{self, JoinHandle};

    const ROOT_INO: u64 = 1;
    const OVERLAY_META_DIR: &str = ".trail";
    const WHITEOUTS_FILE: &str = "overlay-whiteouts.json";
    const MOUNT_STATE_FILE: &str = "mount.json";

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum NodeKind {
        File,
        Directory,
    }

    #[derive(Clone, Debug)]
    struct NodeAttr {
        ino: u64,
        kind: NodeKind,
        mode: u32,
        size: u64,
        modified: SystemTime,
    }

    struct CowCore {
        db: Trail,
        upperdir: PathBuf,
        lower_files: BTreeMap<String, FileEntry>,
        lower_dirs: BTreeSet<String>,
        whiteouts: BTreeSet<String>,
        ino_by_path: HashMap<String, u64>,
        path_by_ino: HashMap<u64, String>,
        dir_mtime: HashMap<String, SystemTime>,
        dir_epoch: SystemTime,
        next_ino: u64,
    }

    impl CowCore {
        fn new(
            db: Trail,
            upperdir: PathBuf,
            lower_files: BTreeMap<String, FileEntry>,
        ) -> Result<Self> {
            let dir_epoch = SystemTime::now();
            let mut core = Self {
                db,
                upperdir,
                lower_files,
                lower_dirs: BTreeSet::from([String::new()]),
                whiteouts: BTreeSet::new(),
                ino_by_path: HashMap::from([(String::new(), ROOT_INO)]),
                path_by_ino: HashMap::from([(ROOT_INO, String::new())]),
                dir_mtime: HashMap::from([(String::new(), dir_epoch)]),
                dir_epoch,
                next_ino: ROOT_INO + 1,
            };
            for path in core.lower_files.keys().cloned().collect::<Vec<_>>() {
                let mut parent = Path::new(&path).parent();
                while let Some(dir) = parent {
                    let text = dir.to_string_lossy();
                    if text.is_empty() {
                        break;
                    }
                    core.lower_dirs.insert(text.into_owned());
                    parent = dir.parent();
                }
            }
            core.whiteouts = core.load_whiteouts()?;
            let mut paths = BTreeSet::from([String::new()]);
            paths.extend(core.lower_dirs.iter().cloned());
            paths.extend(core.lower_files.keys().cloned());
            paths.extend(core.upper_paths()?);
            for path in paths {
                core.ensure_ino(&path);
                if core.node_kind(&path) == Some(NodeKind::Directory) {
                    core.dir_mtime.entry(path).or_insert(core.dir_epoch);
                }
            }
            Ok(core)
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

        fn path_for_ino(&self, ino: u64) -> std::result::Result<String, i32> {
            self.path_by_ino.get(&ino).cloned().ok_or(libc::ENOENT)
        }

        fn child_path(&self, parent: u64, name: &str) -> std::result::Result<String, i32> {
            let parent = self.path_for_ino(parent)?;
            if name.is_empty() || name.contains('/') || name == "." || name == ".." {
                return Err(libc::EINVAL);
            }
            if is_macos_junk(name) {
                return Err(libc::ENOENT);
            }
            Ok(if parent.is_empty() {
                name.to_string()
            } else {
                format!("{parent}/{name}")
            })
        }

        fn upper_path(&self, path: &str) -> std::result::Result<PathBuf, i32> {
            if path.is_empty() {
                return Ok(self.upperdir.clone());
            }
            let normalized = normalize_relative_path(path).map_err(|_| libc::EINVAL)?;
            let mut current = self.upperdir.clone();
            for component in Path::new(&normalized).components() {
                let std::path::Component::Normal(name) = component else {
                    return Err(libc::EINVAL);
                };
                current.push(name);
                match fs::symlink_metadata(&current) {
                    Ok(metadata) if metadata.file_type().is_symlink() => return Err(libc::EPERM),
                    Ok(_) => {}
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(err) => return Err(io_errno(err)),
                }
            }
            Ok(current)
        }

        fn upper_metadata(&self, path: &str) -> Option<fs::Metadata> {
            self.upper_path(path)
                .ok()
                .and_then(|path| fs::symlink_metadata(path).ok())
        }

        fn is_whiteouted(&self, path: &str) -> bool {
            self.whiteouts.iter().any(|item| {
                path == item
                    || path
                        .strip_prefix(item)
                        .is_some_and(|rest| rest.starts_with('/'))
            })
        }

        fn node_kind(&self, path: &str) -> Option<NodeKind> {
            if path.is_empty() {
                return Some(NodeKind::Directory);
            }
            if path == OVERLAY_META_DIR || path.starts_with(".trail/") || self.is_whiteouted(path) {
                return None;
            }
            if let Some(metadata) = self.upper_metadata(path) {
                if metadata.is_file() {
                    return Some(NodeKind::File);
                }
                if metadata.is_dir() {
                    return Some(NodeKind::Directory);
                }
                return None;
            }
            if self.lower_files.contains_key(path) {
                Some(NodeKind::File)
            } else if self.lower_dirs.contains(path) {
                Some(NodeKind::Directory)
            } else {
                None
            }
        }

        fn attr(&mut self, path: &str) -> std::result::Result<NodeAttr, i32> {
            let kind = self.node_kind(path).ok_or(libc::ENOENT)?;
            let ino = self.ensure_ino(path);
            if kind == NodeKind::Directory {
                let mode = self
                    .upper_metadata(path)
                    .map(|metadata| metadata.mode() as u32 & 0o777)
                    .unwrap_or(0o755);
                return Ok(NodeAttr {
                    ino,
                    kind,
                    mode,
                    size: 0,
                    modified: self
                        .dir_mtime
                        .get(path)
                        .copied()
                        .unwrap_or(SystemTime::UNIX_EPOCH),
                });
            }
            if let Some(metadata) = self.upper_metadata(path) {
                return Ok(NodeAttr {
                    ino,
                    kind,
                    mode: metadata.mode() as u32 & 0o777,
                    size: metadata.len(),
                    modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                });
            }
            if let Some(entry) = self.lower_files.get(path) {
                return Ok(NodeAttr {
                    ino,
                    kind,
                    mode: if entry.executable {
                        0o755
                    } else {
                        entry.mode & 0o777
                    },
                    size: entry.size_bytes,
                    modified: SystemTime::UNIX_EPOCH,
                });
            }
            Err(libc::ENOENT)
        }

        fn lookup(&mut self, parent: u64, name: &str) -> std::result::Result<u64, i32> {
            if name == "." {
                return Ok(parent);
            }
            if name == ".." {
                let path = self.path_for_ino(parent)?;
                let parent_path = Path::new(&path)
                    .parent()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                return Ok(self.ensure_ino(&parent_path));
            }
            let path = self.child_path(parent, name)?;
            self.attr(&path).map(|attr| attr.ino)
        }

        fn children(&mut self, dir_ino: u64) -> std::result::Result<Vec<(String, NodeAttr)>, i32> {
            let path = self.path_for_ino(dir_ino)?;
            if self.node_kind(&path) != Some(NodeKind::Directory) {
                return Err(libc::ENOTDIR);
            }
            let mut names = BTreeSet::new();
            for candidate in self.lower_dirs.iter().chain(self.lower_files.keys()) {
                if !self.is_whiteouted(candidate) && parent_of(candidate) == path {
                    if let Some(name) = Path::new(candidate).file_name().and_then(OsStr::to_str) {
                        names.insert(name.to_string());
                    }
                }
            }
            if let Ok(dir) = self
                .upper_path(&path)
                .and_then(|path| fs::read_dir(path).map_err(io_errno))
            {
                for entry in dir.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if path.is_empty() && name == OVERLAY_META_DIR {
                        continue;
                    }
                    names.insert(name);
                }
            }
            let mut result = Vec::new();
            for name in names {
                let child = if path.is_empty() {
                    name.clone()
                } else {
                    format!("{path}/{name}")
                };
                if let Ok(attr) = self.attr(&child) {
                    result.push((name, attr));
                }
            }
            Ok(result)
        }

        fn read(
            &mut self,
            ino: u64,
            offset: u64,
            count: u32,
        ) -> std::result::Result<(Vec<u8>, bool), i32> {
            let path = self.path_for_ino(ino)?;
            if self.node_kind(&path) != Some(NodeKind::File) {
                return Err(libc::EISDIR);
            }
            let bytes = if let Some(metadata) = self.upper_metadata(&path) {
                if !metadata.is_file() {
                    return Err(libc::EINVAL);
                }
                fs::read(self.upper_path(&path)?).map_err(io_errno)?
            } else {
                let entry = self.lower_files.get(&path).ok_or(libc::ENOENT)?;
                self.db
                    .materialize_entry_bytes(entry)
                    .map_err(|_| libc::EIO)?
            };
            let start = (offset as usize).min(bytes.len());
            let end = start.saturating_add(count as usize).min(bytes.len());
            Ok((bytes[start..end].to_vec(), end == bytes.len()))
        }

        fn ensure_upper_parent(&self, path: &str) -> std::result::Result<(), i32> {
            let upper = self.upper_path(path)?;
            if let Some(parent) = upper.parent() {
                fs::create_dir_all(parent).map_err(io_errno)?;
            }
            Ok(())
        }

        fn touch_dir(&mut self, path: String) {
            let previous = self.dir_mtime.get(&path).copied().unwrap_or(self.dir_epoch);
            let now = SystemTime::now();
            self.dir_mtime.insert(
                path,
                if now > previous {
                    now
                } else {
                    previous + Duration::from_nanos(1)
                },
            );
        }

        fn touch_parent(&mut self, path: &str) {
            self.touch_dir(parent_of(path));
        }

        fn ensure_upper_file(
            &mut self,
            path: &str,
            truncate: bool,
        ) -> std::result::Result<File, i32> {
            if self.node_kind(path) == Some(NodeKind::Directory) {
                return Err(libc::EISDIR);
            }
            self.ensure_upper_parent(path)?;
            let upper = self.upper_path(path)?;
            if !upper.exists() {
                if !truncate && self.lower_files.contains_key(path) {
                    let entry = self.lower_files.get(path).ok_or(libc::ENOENT)?;
                    let bytes = self
                        .db
                        .materialize_entry_bytes(entry)
                        .map_err(|_| libc::EIO)?;
                    fs::write(&upper, bytes).map_err(io_errno)?;
                    fs::set_permissions(
                        &upper,
                        fs::Permissions::from_mode(if entry.executable {
                            0o755
                        } else {
                            entry.mode & 0o777
                        }),
                    )
                    .map_err(io_errno)?;
                } else {
                    File::create(&upper).map_err(io_errno)?;
                }
            }
            self.whiteouts.remove(path);
            self.save_whiteouts().map_err(|_| libc::EIO)?;
            OpenOptions::new()
                .read(true)
                .write(true)
                .truncate(truncate)
                .open(upper)
                .map_err(io_errno)
        }

        fn copy_lower_file(&self, source: &str, target: &str) -> std::result::Result<(), i32> {
            let entry = self.lower_files.get(source).ok_or(libc::ENOENT)?;
            self.ensure_upper_parent(target)?;
            let bytes = self
                .db
                .materialize_entry_bytes(entry)
                .map_err(|_| libc::EIO)?;
            let target_path = self.upper_path(target)?;
            fs::write(&target_path, bytes).map_err(io_errno)?;
            fs::set_permissions(
                target_path,
                fs::Permissions::from_mode(if entry.executable {
                    0o755
                } else {
                    entry.mode & 0o777
                }),
            )
            .map_err(io_errno)
        }

        fn merge_lower_subtree_into_upper(&self, root: &str) -> std::result::Result<(), i32> {
            fs::create_dir_all(self.upper_path(root)?).map_err(io_errno)?;
            let prefix = format!("{root}/");
            for path in self
                .lower_dirs
                .iter()
                .filter(|path| path.starts_with(&prefix) && !self.is_whiteouted(path))
            {
                fs::create_dir_all(self.upper_path(path)?).map_err(io_errno)?;
            }
            for path in self
                .lower_files
                .keys()
                .filter(|path| path.starts_with(&prefix) && !self.is_whiteouted(path))
            {
                if self.upper_metadata(path).is_none() {
                    self.copy_lower_file(path, path)?;
                }
            }
            Ok(())
        }

        fn write(
            &mut self,
            ino: u64,
            offset: u64,
            data: &[u8],
        ) -> std::result::Result<NodeAttr, i32> {
            let path = self.path_for_ino(ino)?;
            let file = self.ensure_upper_file(&path, false)?;
            file.write_at(data, offset).map_err(io_errno)?;
            file.sync_data().map_err(io_errno)?;
            self.attr(&path)
        }

        fn create(
            &mut self,
            parent: u64,
            name: &str,
            mode: u32,
            exclusive: bool,
        ) -> std::result::Result<NodeAttr, i32> {
            let path = self.child_path(parent, name)?;
            if exclusive && self.node_kind(&path).is_some() {
                return Err(libc::EEXIST);
            }
            self.ensure_upper_parent(&path)?;
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(!exclusive)
                .create_new(exclusive)
                .mode(mode & 0o777)
                .open(self.upper_path(&path)?)
                .map_err(io_errno)?;
            file.sync_data().map_err(io_errno)?;
            self.whiteouts.remove(&path);
            self.save_whiteouts().map_err(|_| libc::EIO)?;
            self.touch_parent(&path);
            self.attr(&path)
        }

        fn mkdir(
            &mut self,
            parent: u64,
            name: &str,
            mode: u32,
        ) -> std::result::Result<NodeAttr, i32> {
            let path = self.child_path(parent, name)?;
            if self.node_kind(&path).is_some() {
                return Err(libc::EEXIST);
            }
            fs::create_dir_all(self.upper_path(&path)?).map_err(io_errno)?;
            fs::set_permissions(
                self.upper_path(&path)?,
                fs::Permissions::from_mode(mode & 0o777),
            )
            .map_err(io_errno)?;
            self.whiteouts.remove(&path);
            self.save_whiteouts().map_err(|_| libc::EIO)?;
            self.touch_parent(&path);
            self.touch_dir(path.clone());
            self.attr(&path)
        }

        fn setattr(
            &mut self,
            ino: u64,
            size: Option<u64>,
            mode: Option<u32>,
        ) -> std::result::Result<NodeAttr, i32> {
            let path = self.path_for_ino(ino)?;
            if let Some(size) = size {
                self.ensure_upper_file(&path, false)?
                    .set_len(size)
                    .map_err(io_errno)?;
            }
            if let Some(mode) = mode {
                if self.node_kind(&path) == Some(NodeKind::File) {
                    let _ = self.ensure_upper_file(&path, false)?;
                }
                fs::set_permissions(
                    self.upper_path(&path)?,
                    fs::Permissions::from_mode(mode & 0o777),
                )
                .map_err(io_errno)?;
            }
            self.attr(&path)
        }

        fn remove(&mut self, parent: u64, name: &str) -> std::result::Result<(), i32> {
            let path = self.child_path(parent, name)?;
            let kind = self.node_kind(&path).ok_or(libc::ENOENT)?;
            let ino = self.ensure_ino(&path);
            if kind == NodeKind::Directory && !self.children(ino)?.is_empty() {
                return Err(libc::ENOTEMPTY);
            }
            if let Some(metadata) = self.upper_metadata(&path) {
                if metadata.is_dir() {
                    fs::remove_dir(self.upper_path(&path)?).map_err(io_errno)?;
                } else {
                    fs::remove_file(self.upper_path(&path)?).map_err(io_errno)?;
                }
            }
            if self.lower_files.contains_key(&path) || self.lower_dirs.contains(&path) {
                self.whiteouts.insert(path.clone());
            }
            self.save_whiteouts().map_err(|_| libc::EIO)?;
            self.touch_parent(&path);
            Ok(())
        }

        fn rename(
            &mut self,
            from_parent: u64,
            from_name: &str,
            to_parent: u64,
            to_name: &str,
        ) -> std::result::Result<(), i32> {
            let old = self.child_path(from_parent, from_name)?;
            let new = self.child_path(to_parent, to_name)?;
            let kind = self.node_kind(&old).ok_or(libc::ENOENT)?;
            self.ensure_upper_parent(&new)?;
            if self.upper_metadata(&new).is_some() {
                let metadata = self.upper_metadata(&new).unwrap();
                if metadata.is_dir() {
                    fs::remove_dir_all(self.upper_path(&new)?).map_err(io_errno)?;
                } else {
                    fs::remove_file(self.upper_path(&new)?).map_err(io_errno)?;
                }
            }
            if kind == NodeKind::Directory {
                self.merge_lower_subtree_into_upper(&old)?;
            }
            if self.upper_metadata(&old).is_some() {
                fs::rename(self.upper_path(&old)?, self.upper_path(&new)?).map_err(io_errno)?;
            } else if kind == NodeKind::File {
                self.copy_lower_file(&old, &new)?;
            } else {
                unreachable!("directory sources are copied up before rename")
            }
            self.whiteouts.insert(old.clone());
            self.whiteouts.remove(&new);
            self.save_whiteouts().map_err(|_| libc::EIO)?;
            self.touch_parent(&old);
            self.touch_parent(&new);
            let old_prefix = format!("{old}/");
            let moved_inodes = self
                .ino_by_path
                .iter()
                .filter(|(path, _)| *path == &old || path.starts_with(&old_prefix))
                .map(|(path, ino)| (path.clone(), *ino))
                .collect::<Vec<_>>();
            for (path, ino) in moved_inodes {
                self.ino_by_path.remove(&path);
                let suffix = path.strip_prefix(&old).unwrap();
                let moved = format!("{new}{suffix}");
                self.ino_by_path.insert(moved.clone(), ino);
                self.path_by_ino.insert(ino, moved);
            }
            Ok(())
        }

        fn load_whiteouts(&self) -> Result<BTreeSet<String>> {
            match fs::read(self.upperdir.join(OVERLAY_META_DIR).join(WHITEOUTS_FILE)) {
                Ok(bytes) => Ok(serde_json::from_slice::<Vec<String>>(&bytes)?
                    .into_iter()
                    .collect()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(BTreeSet::new()),
                Err(err) => Err(Error::Io(err)),
            }
        }

        fn save_whiteouts(&self) -> Result<()> {
            let path = self.upperdir.join(OVERLAY_META_DIR).join(WHITEOUTS_FILE);
            fs::create_dir_all(path.parent().unwrap())?;
            write_file_atomic(
                &path,
                &serde_json::to_vec(&self.whiteouts.iter().collect::<Vec<_>>())?,
                false,
            )
        }

        fn upper_paths(&self) -> Result<Vec<String>> {
            let mut paths = Vec::new();
            for entry in walkdir::WalkDir::new(&self.upperdir) {
                let entry = entry.map_err(|err| Error::InvalidInput(err.to_string()))?;
                if entry.path() == self.upperdir {
                    continue;
                }
                let path = normalize_relative_path(
                    &entry
                        .path()
                        .strip_prefix(&self.upperdir)
                        .map_err(|err| Error::InvalidInput(err.to_string()))?
                        .to_string_lossy(),
                )?;
                if path == OVERLAY_META_DIR || path.starts_with(".trail/") {
                    continue;
                }
                paths.push(path);
            }
            Ok(paths)
        }
    }

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
        let upper = nfs_upperdir(db, lane)?;
        if upper.exists() {
            fs::remove_dir_all(&upper)?;
        }
        fs::create_dir_all(upper.join(OVERLAY_META_DIR))?;
        Ok(upper)
    }

    pub(crate) fn nfs_clean_manifest_path(db: &Trail, lane: &str) -> Result<PathBuf> {
        Ok(nfs_upperdir(db, lane)?
            .join(OVERLAY_META_DIR)
            .join("workdir-manifest.json"))
    }

    pub(crate) fn nfs_candidate_paths(db: &Trail, lane: &str) -> Result<Vec<String>> {
        let upper = nfs_upperdir(db, lane)?;
        let mut paths = BTreeSet::new();
        for entry in walkdir::WalkDir::new(&upper) {
            let entry = entry.map_err(|err| Error::InvalidInput(err.to_string()))?;
            if !entry.file_type().is_file() {
                continue;
            }
            let path = normalize_relative_path(
                &entry
                    .path()
                    .strip_prefix(&upper)
                    .map_err(|err| Error::InvalidInput(err.to_string()))?
                    .to_string_lossy(),
            )?;
            if path == OVERLAY_META_DIR
                || path.starts_with(".trail/")
                || Path::new(&path)
                    .file_name()
                    .and_then(OsStr::to_str)
                    .is_some_and(is_macos_junk)
            {
                continue;
            }
            paths.insert(path);
        }
        let whiteouts_path = upper.join(OVERLAY_META_DIR).join(WHITEOUTS_FILE);
        let whiteouts = match fs::read(whiteouts_path) {
            Ok(bytes) => serde_json::from_slice::<Vec<String>>(&bytes)?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(err) => return Err(Error::Io(err)),
        };
        let branch = db.lane_branch(lane)?;
        let head = db.get_ref(&branch.ref_name)?;
        let lower = db.load_root_files(&head.root_id)?;
        for whiteout in whiteouts {
            if lower.contains_key(&whiteout) {
                paths.insert(whiteout.clone());
            }
            let prefix = format!("{whiteout}/");
            paths.extend(
                lower
                    .keys()
                    .filter(|path| path.starts_with(&prefix))
                    .cloned(),
            );
        }
        Ok(paths.into_iter().collect())
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
        fs::create_dir_all(&mountpoint)?;
        let upper = nfs_upperdir(db, lane)?;
        fs::create_dir_all(upper.join(OVERLAY_META_DIR))?;
        let state_path = upper.parent().unwrap().join(MOUNT_STATE_FILE);
        let head = db.get_ref(&branch.ref_name)?;
        let lower = db.load_root_files(&head.root_id)?;
        let core = CowCore::new(
            Trail::open_with_db_dir(db.workspace_root.clone(), db.db_dir.clone())?,
            upper.clone(),
            lower.clone(),
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
        Ok(NfsCowMount {
            mountpoint,
            state_path,
            shutdown: Some(shutdown_tx),
            worker: Some(worker),
        })
    }

    fn nfs_upperdir(db: &Trail, lane: &str) -> Result<PathBuf> {
        Ok(db
            .db_dir
            .join("nfs-cow")
            .join(path_from_rel(&normalize_relative_path(lane)?))
            .join("upper"))
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
    fn io_errno(error: std::io::Error) -> i32 {
        error.raw_os_error().unwrap_or(libc::EIO)
    }
    fn parent_of(path: &str) -> String {
        Path::new(path)
            .parent()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default()
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
    }
}

#[cfg(target_os = "macos")]
pub(crate) use macos::*;

#[cfg(not(target_os = "macos"))]
pub(crate) struct NfsCowMount;

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
