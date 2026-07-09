use super::*;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex, Once};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dokan::{
    init, unmount, CreateFileInfo, DiskSpaceInfo, FileInfo, FileSystemHandler, FileSystemMounter,
    FillDataError, FillDataResult, FindData, MountFlags, MountOptions, OperationInfo,
    OperationResult, VolumeInfo, IO_SECURITY_CONTEXT,
};
use dokan_sys::win32::{
    FILE_CREATE, FILE_DELETE_ON_CLOSE, FILE_DIRECTORY_FILE, FILE_MAXIMUM_DISPOSITION,
    FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_IF, FILE_OVERWRITE, FILE_OVERWRITE_IF,
    FILE_SUPERSEDE,
};
use widestring::{U16CStr, U16CString};
use winapi::shared::ntdef::NTSTATUS;
use winapi::shared::ntstatus::*;
use winapi::um::winnt;

static DOKAN_INIT: Once = Once::new();

const OVERLAY_META_DIR: &str = ".trail";
const WHITEOUTS_FILE: &str = "overlay-whiteouts.json";

pub(crate) struct OverlayCowMount {
    mountpoint: PathBuf,
    mount_name: U16CString,
    worker: Option<JoinHandle<()>>,
}

impl OverlayCowMount {
    #[allow(dead_code)]
    pub(crate) fn mountpoint(&self) -> &Path {
        &self.mountpoint
    }
}

impl Drop for OverlayCowMount {
    fn drop(&mut self) {
        let _ = unmount(&self.mount_name);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub(crate) fn prepare_overlay_cow_workdir(
    db: &Trail,
    lane: &str,
    dir: &Path,
    custom_workdir: bool,
) -> Result<PathBuf> {
    prepare_lane_workdir(dir, custom_workdir)?;
    let upperdir = overlay_upperdir(db, lane)?;
    if upperdir.exists() {
        fs::remove_dir_all(&upperdir)?;
    }
    fs::create_dir_all(upperdir.join(OVERLAY_META_DIR))?;
    Ok(upperdir)
}

pub(crate) fn mount_overlay_cow_for_lane(db: &Trail, lane: &str) -> Result<OverlayCowMount> {
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
    fs::create_dir_all(upperdir.join(OVERLAY_META_DIR))?;
    let manifest_path = overlay_clean_manifest_path(&upperdir);
    let write_initial_manifest = !manifest_path.is_file() && !upperdir_has_user_content(&upperdir)?;

    let head = db.get_ref(&branch.ref_name)?;
    let lower_files = db.load_root_files(&head.root_id)?;
    let mount_name = U16CString::from_str(mountpoint.to_string_lossy().as_ref()).map_err(|_| {
        Error::InvalidInput(format!(
            "overlay-cow mountpoint `{}` is not a valid Windows mount string",
            mountpoint.display()
        ))
    })?;

    let handler = DokanOverlayFs::new(
        db.workspace_root.clone(),
        db.db_dir.clone(),
        upperdir,
        lower_files.clone(),
    )?;
    let thread_mount_name = mount_name.clone();
    let (tx, rx) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        DOKAN_INIT.call_once(init);
        let options = MountOptions {
            flags: MountFlags::CURRENT_SESSION | MountFlags::CASE_SENSITIVE,
            timeout: Duration::from_secs(60),
            ..Default::default()
        };
        let mut mounter = FileSystemMounter::new(&handler, &thread_mount_name, &options);
        match mounter.mount() {
            Ok(file_system) => {
                let _ = tx.send(Ok(()));
                drop(file_system);
            }
            Err(err) => {
                let _ = tx.send(Err(err.to_string()));
            }
        }
    });

    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => {}
        Ok(Err(err)) => {
            let _ = worker.join();
            return Err(overlay_mount_error(&mountpoint, &err));
        }
        Err(err) => {
            let _ = unmount(&mount_name);
            let _ = worker.join();
            return Err(overlay_mount_error(&mountpoint, &err.to_string()));
        }
    }

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
        mountpoint,
        mount_name,
        worker: Some(worker),
    })
}

fn overlay_mount_error(mountpoint: &Path, err: &str) -> Error {
    Error::InvalidInput(format!(
        "failed to mount overlay-cow workdir at `{}` with Dokan: {err}. Install Dokan 2.x and ensure the Dokan driver service is running.",
        mountpoint.display()
    ))
}

fn overlay_upperdir(db: &Trail, lane: &str) -> Result<PathBuf> {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeKind {
    Directory,
    File,
}

#[derive(Debug)]
struct DokanHandle {
    path: String,
    is_dir: bool,
    delete_on_cleanup: AtomicBool,
}

impl DokanHandle {
    fn new(path: String, is_dir: bool, delete_on_cleanup: bool) -> Self {
        Self {
            path,
            is_dir,
            delete_on_cleanup: AtomicBool::new(delete_on_cleanup),
        }
    }
}

struct DokanOverlayFs {
    workspace_root: PathBuf,
    db_dir: PathBuf,
    upperdir: PathBuf,
    lower_files: BTreeMap<String, FileEntry>,
    lower_dirs: BTreeSet<String>,
    whiteouts: Mutex<BTreeSet<String>>,
}

impl DokanOverlayFs {
    fn new(
        workspace_root: PathBuf,
        db_dir: PathBuf,
        upperdir: PathBuf,
        lower_files: BTreeMap<String, FileEntry>,
    ) -> Result<Self> {
        let mut fs = Self {
            workspace_root,
            db_dir,
            upperdir,
            lower_files,
            lower_dirs: BTreeSet::new(),
            whiteouts: Mutex::new(BTreeSet::new()),
        };
        fs.rebuild_lower_dirs();
        *fs.whiteouts.lock().unwrap() = fs.load_whiteouts()?;
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

    fn load_whiteouts(&self) -> Result<BTreeSet<String>> {
        let path = self.whiteouts_path();
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
            Err(err) => return Err(Error::Io(err)),
        };
        let paths: Vec<String> = serde_json::from_slice(&bytes)?;
        paths
            .into_iter()
            .map(|path| normalize_relative_path(&path))
            .collect()
    }

    fn save_whiteouts(&self) -> OperationResult<()> {
        let path = self.whiteouts_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_status)?;
        }
        let paths = self
            .whiteouts
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        write_file_atomic(
            &path,
            &serde_json::to_vec(&paths).map_err(error_status)?,
            false,
        )
        .map_err(error_status)
    }

    fn whiteouts_path(&self) -> PathBuf {
        self.upperdir.join(OVERLAY_META_DIR).join(WHITEOUTS_FILE)
    }

    fn is_whiteouted(&self, path: &str) -> bool {
        self.whiteouts.lock().unwrap().iter().any(|whiteout| {
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

    fn upper_metadata(&self, path: &str) -> Option<fs::Metadata> {
        fs::symlink_metadata(self.upper_path(path)).ok()
    }

    fn node_kind(&self, path: &str) -> Option<NodeKind> {
        if path.is_empty() {
            return Some(NodeKind::Directory);
        }
        if path == OVERLAY_META_DIR || path.starts_with(".trail/") {
            return None;
        }
        if let Some(metadata) = self.upper_metadata(path) {
            if metadata.is_dir() {
                return Some(NodeKind::Directory);
            }
            if metadata.is_file() {
                return Some(NodeKind::File);
            }
            return None;
        }
        if self.lower_dirs.contains(path) && !self.is_whiteouted(path) {
            return Some(NodeKind::Directory);
        }
        if self.lower_files.contains_key(path) && !self.is_whiteouted(path) {
            return Some(NodeKind::File);
        }
        None
    }

    fn load_lower_bytes(&self, path: &str) -> OperationResult<Vec<u8>> {
        let entry = self
            .lower_files
            .get(path)
            .ok_or(STATUS_OBJECT_NAME_NOT_FOUND)?;
        let db =
            Trail::open_with_db_dir(&self.workspace_root, &self.db_dir).map_err(error_status)?;
        db.materialize_entry_bytes(entry).map_err(error_status)
    }

    fn ensure_upper_parent(&self, path: &str) -> OperationResult<()> {
        let upper = self.upper_path(path);
        if let Some(parent) = upper.parent() {
            fs::create_dir_all(parent).map_err(io_status)?;
        }
        Ok(())
    }

    fn ensure_parent_visible(&self, path: &str) -> OperationResult<()> {
        let parent = parent_of(path);
        if parent.is_empty() || self.node_kind(&parent) == Some(NodeKind::Directory) {
            Ok(())
        } else {
            Err(STATUS_OBJECT_PATH_NOT_FOUND)
        }
    }

    fn ensure_upper_file(&self, path: &str, truncate: bool) -> OperationResult<File> {
        if self.node_kind(path) == Some(NodeKind::Directory) {
            return Err(STATUS_FILE_IS_A_DIRECTORY);
        }
        self.ensure_parent_visible(path)?;
        self.ensure_upper_parent(path)?;
        let upper = self.upper_path(path);
        if !upper.exists() {
            if !truncate && self.lower_files.contains_key(path) && !self.is_whiteouted(path) {
                let bytes = self.load_lower_bytes(path)?;
                fs::write(&upper, bytes).map_err(io_status)?;
            } else {
                File::create(&upper).map_err(io_status)?;
            }
        }
        if truncate {
            OpenOptions::new()
                .write(true)
                .open(&upper)
                .and_then(|file| file.set_len(0))
                .map_err(io_status)?;
        }
        self.whiteouts.lock().unwrap().remove(path);
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&upper)
            .map_err(io_status)
    }

    fn create_upper_dir(&self, path: &str) -> OperationResult<()> {
        self.ensure_parent_visible(path)?;
        fs::create_dir_all(self.upper_path(path)).map_err(io_status)?;
        self.whiteouts.lock().unwrap().remove(path);
        Ok(())
    }

    fn delete_path(&self, path: &str, is_dir: bool) -> OperationResult<()> {
        if path.is_empty() {
            return Err(STATUS_ACCESS_DENIED);
        }
        if self.upper_metadata(path).is_some() {
            let upper = self.upper_path(path);
            if is_dir {
                fs::remove_dir(&upper).map_err(io_status)?;
            } else {
                fs::remove_file(&upper).map_err(io_status)?;
            }
        }
        if self.lower_dirs.contains(path) || self.lower_files.contains_key(path) {
            self.whiteouts.lock().unwrap().insert(path.to_string());
            self.save_whiteouts()?;
        }
        Ok(())
    }

    fn remove_existing_for_replace(&self, path: &str) -> OperationResult<()> {
        let Some(kind) = self.node_kind(path) else {
            return Ok(());
        };
        match kind {
            NodeKind::Directory => {
                if !self.children(path).is_empty() {
                    return Err(STATUS_DIRECTORY_NOT_EMPTY);
                }
                if self.upper_metadata(path).is_some() {
                    fs::remove_dir(self.upper_path(path)).map_err(io_status)?;
                }
                if self.lower_dirs.contains(path) {
                    self.whiteouts.lock().unwrap().insert(path.to_string());
                    self.save_whiteouts()?;
                }
            }
            NodeKind::File => {
                if self.upper_metadata(path).is_some() {
                    fs::remove_file(self.upper_path(path)).map_err(io_status)?;
                }
                if self.lower_files.contains_key(path) {
                    self.whiteouts.lock().unwrap().insert(path.to_string());
                    self.save_whiteouts()?;
                }
            }
        }
        Ok(())
    }

    fn children(&self, path: &str) -> Vec<(String, NodeKind)> {
        let mut names = BTreeMap::<String, NodeKind>::new();
        for dir in &self.lower_dirs {
            if dir.is_empty() || self.is_whiteouted(dir) {
                continue;
            }
            if parent_of(dir) == path {
                names.insert(file_name(dir), NodeKind::Directory);
            }
        }
        for file in self.lower_files.keys() {
            if self.is_whiteouted(file) {
                continue;
            }
            if parent_of(file) == path {
                names.insert(file_name(file), NodeKind::File);
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
        names.into_iter().collect()
    }

    fn path_info(&self, path: &str) -> OperationResult<FileInfo> {
        let Some(kind) = self.node_kind(path) else {
            return Err(STATUS_OBJECT_NAME_NOT_FOUND);
        };
        if let Some(metadata) = self.upper_metadata(path) {
            return Ok(file_info_from_metadata(path, &metadata, kind));
        }
        match kind {
            NodeKind::Directory => Ok(FileInfo {
                attributes: winnt::FILE_ATTRIBUTE_DIRECTORY,
                creation_time: stable_time(),
                last_access_time: stable_time(),
                last_write_time: stable_time(),
                file_size: 0,
                number_of_links: 1,
                file_index: stable_file_index(path),
            }),
            NodeKind::File => {
                let entry = self
                    .lower_files
                    .get(path)
                    .ok_or(STATUS_OBJECT_NAME_NOT_FOUND)?;
                Ok(FileInfo {
                    attributes: winnt::FILE_ATTRIBUTE_ARCHIVE,
                    creation_time: stable_time(),
                    last_access_time: stable_time(),
                    last_write_time: stable_time(),
                    file_size: entry.size_bytes,
                    number_of_links: 1,
                    file_index: stable_file_index(path),
                })
            }
        }
    }

    fn find_data(&self, parent: &str, name: String, kind: NodeKind) -> OperationResult<FindData> {
        let path = if parent.is_empty() {
            name.clone()
        } else {
            format!("{parent}/{name}")
        };
        let info = self.path_info(&path)?;
        let attributes = match kind {
            NodeKind::Directory => info.attributes | winnt::FILE_ATTRIBUTE_DIRECTORY,
            NodeKind::File => info.attributes,
        };
        Ok(FindData {
            attributes,
            creation_time: info.creation_time,
            last_access_time: info.last_access_time,
            last_write_time: info.last_write_time,
            file_size: info.file_size,
            file_name: U16CString::from_str(&name).map_err(|_| STATUS_OBJECT_NAME_INVALID)?,
        })
    }

    fn copy_lower_subtree_to_upper(&self, old_path: &str, new_path: &str) -> OperationResult<()> {
        if self.lower_files.contains_key(old_path) {
            let bytes = self.load_lower_bytes(old_path)?;
            self.ensure_upper_parent(new_path)?;
            fs::write(self.upper_path(new_path), bytes).map_err(io_status)?;
            return Ok(());
        }
        if !self.lower_dirs.contains(old_path) {
            return Err(STATUS_OBJECT_NAME_NOT_FOUND);
        }
        fs::create_dir_all(self.upper_path(new_path)).map_err(io_status)?;
        let prefix = if old_path.is_empty() {
            String::new()
        } else {
            format!("{old_path}/")
        };
        for file in self.lower_files.keys() {
            if self.is_whiteouted(file) {
                continue;
            }
            let Some(suffix) = file.strip_prefix(&prefix) else {
                continue;
            };
            let target = if new_path.is_empty() {
                suffix.to_string()
            } else {
                format!("{new_path}/{suffix}")
            };
            let bytes = self.load_lower_bytes(file)?;
            self.ensure_upper_parent(&target)?;
            fs::write(self.upper_path(&target), bytes).map_err(io_status)?;
        }
        Ok(())
    }
}

impl<'c, 'h: 'c> FileSystemHandler<'c, 'h> for DokanOverlayFs {
    type Context = DokanHandle;

    #[allow(clippy::too_many_arguments)]
    fn create_file(
        &'h self,
        file_name: &U16CStr,
        _security_context: &IO_SECURITY_CONTEXT,
        _desired_access: winnt::ACCESS_MASK,
        file_attributes: u32,
        _share_access: u32,
        create_disposition: u32,
        create_options: u32,
        _info: &mut OperationInfo<'c, 'h, Self>,
    ) -> OperationResult<CreateFileInfo<Self::Context>> {
        if create_disposition > FILE_MAXIMUM_DISPOSITION {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let path = dokan_path_to_rel(file_name)?;
        let wants_dir = create_options & FILE_DIRECTORY_FILE != 0;
        let wants_file = create_options & FILE_NON_DIRECTORY_FILE != 0;
        let delete_on_cleanup = create_options & FILE_DELETE_ON_CLOSE != 0;
        let existing = self.node_kind(&path);

        if let Some(kind) = existing {
            if wants_dir && kind != NodeKind::Directory {
                return Err(STATUS_NOT_A_DIRECTORY);
            }
            if wants_file && kind == NodeKind::Directory {
                return Err(STATUS_FILE_IS_A_DIRECTORY);
            }
            if create_disposition == FILE_CREATE {
                return Err(STATUS_OBJECT_NAME_COLLISION);
            }
            if matches!(
                create_disposition,
                FILE_OVERWRITE | FILE_OVERWRITE_IF | FILE_SUPERSEDE
            ) {
                if kind == NodeKind::Directory {
                    return Err(STATUS_FILE_IS_A_DIRECTORY);
                }
                self.ensure_upper_file(&path, true)?;
            }
            return Ok(CreateFileInfo {
                context: DokanHandle::new(path, kind == NodeKind::Directory, delete_on_cleanup),
                is_dir: kind == NodeKind::Directory,
                new_file_created: false,
            });
        }

        if matches!(create_disposition, FILE_OPEN | FILE_OVERWRITE) {
            return Err(STATUS_OBJECT_NAME_NOT_FOUND);
        }

        if wants_dir || file_attributes & winnt::FILE_ATTRIBUTE_DIRECTORY != 0 {
            self.create_upper_dir(&path)?;
            return Ok(CreateFileInfo {
                context: DokanHandle::new(path, true, delete_on_cleanup),
                is_dir: true,
                new_file_created: true,
            });
        }

        self.ensure_upper_file(&path, true)?;
        Ok(CreateFileInfo {
            context: DokanHandle::new(path, false, delete_on_cleanup),
            is_dir: false,
            new_file_created: true,
        })
    }

    fn cleanup(
        &'h self,
        _file_name: &U16CStr,
        info: &OperationInfo<'c, 'h, Self>,
        context: &'c Self::Context,
    ) {
        if info.delete_on_close() || context.delete_on_cleanup.load(Ordering::Relaxed) {
            let _ = self.delete_path(&context.path, context.is_dir);
        }
    }

    fn read_file(
        &'h self,
        _file_name: &U16CStr,
        offset: i64,
        buffer: &mut [u8],
        _info: &OperationInfo<'c, 'h, Self>,
        context: &'c Self::Context,
    ) -> OperationResult<u32> {
        if context.is_dir {
            return Err(STATUS_FILE_IS_A_DIRECTORY);
        }
        let bytes = if self.upper_metadata(&context.path).is_some() {
            let mut bytes = Vec::new();
            File::open(self.upper_path(&context.path))
                .and_then(|mut file| file.read_to_end(&mut bytes).map(|_| ()))
                .map_err(io_status)?;
            bytes
        } else {
            self.load_lower_bytes(&context.path)?
        };
        let start = offset.max(0) as usize;
        if start >= bytes.len() {
            return Ok(0);
        }
        let end = start.saturating_add(buffer.len()).min(bytes.len());
        let slice = &bytes[start..end];
        buffer[..slice.len()].copy_from_slice(slice);
        Ok(slice.len() as u32)
    }

    fn write_file(
        &'h self,
        _file_name: &U16CStr,
        offset: i64,
        buffer: &[u8],
        info: &OperationInfo<'c, 'h, Self>,
        context: &'c Self::Context,
    ) -> OperationResult<u32> {
        if context.is_dir {
            return Err(STATUS_FILE_IS_A_DIRECTORY);
        }
        let mut file = self.ensure_upper_file(&context.path, false)?;
        let write_offset = if info.write_to_eof() {
            file.metadata().map_err(io_status)?.len()
        } else {
            offset.max(0) as u64
        };
        file.seek(SeekFrom::Start(write_offset))
            .map_err(io_status)?;
        file.write_all(buffer).map_err(io_status)?;
        Ok(buffer.len() as u32)
    }

    fn flush_file_buffers(
        &'h self,
        _file_name: &U16CStr,
        _info: &OperationInfo<'c, 'h, Self>,
        context: &'c Self::Context,
    ) -> OperationResult<()> {
        if !context.is_dir && self.upper_metadata(&context.path).is_some() {
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(self.upper_path(&context.path))
                .and_then(|file| file.sync_all())
                .map_err(io_status)?;
        }
        Ok(())
    }

    fn get_file_information(
        &'h self,
        _file_name: &U16CStr,
        _info: &OperationInfo<'c, 'h, Self>,
        context: &'c Self::Context,
    ) -> OperationResult<FileInfo> {
        self.path_info(&context.path)
    }

    fn find_files(
        &'h self,
        _file_name: &U16CStr,
        mut fill_find_data: impl FnMut(&FindData) -> FillDataResult,
        _info: &OperationInfo<'c, 'h, Self>,
        context: &'c Self::Context,
    ) -> OperationResult<()> {
        if !context.is_dir {
            return Err(STATUS_NOT_A_DIRECTORY);
        }
        for (name, kind) in self.children(&context.path) {
            let data = self.find_data(&context.path, name, kind)?;
            fill_find_data(&data).map_err(fill_status)?;
        }
        Ok(())
    }

    fn delete_file(
        &'h self,
        _file_name: &U16CStr,
        info: &OperationInfo<'c, 'h, Self>,
        context: &'c Self::Context,
    ) -> OperationResult<()> {
        if info.delete_on_close() {
            if context.is_dir {
                return Err(STATUS_FILE_IS_A_DIRECTORY);
            }
            if self.node_kind(&context.path) != Some(NodeKind::File) {
                return Err(STATUS_OBJECT_NAME_NOT_FOUND);
            }
            context.delete_on_cleanup.store(true, Ordering::Relaxed);
        } else {
            context.delete_on_cleanup.store(false, Ordering::Relaxed);
        }
        Ok(())
    }

    fn delete_directory(
        &'h self,
        _file_name: &U16CStr,
        info: &OperationInfo<'c, 'h, Self>,
        context: &'c Self::Context,
    ) -> OperationResult<()> {
        if info.delete_on_close() {
            if !context.is_dir {
                return Err(STATUS_NOT_A_DIRECTORY);
            }
            if !self.children(&context.path).is_empty() {
                return Err(STATUS_DIRECTORY_NOT_EMPTY);
            }
            context.delete_on_cleanup.store(true, Ordering::Relaxed);
        } else {
            context.delete_on_cleanup.store(false, Ordering::Relaxed);
        }
        Ok(())
    }

    fn move_file(
        &'h self,
        _file_name: &U16CStr,
        new_file_name: &U16CStr,
        replace_if_existing: bool,
        _info: &OperationInfo<'c, 'h, Self>,
        context: &'c Self::Context,
    ) -> OperationResult<()> {
        let new_path = dokan_path_to_rel(new_file_name)?;
        if self.node_kind(&new_path).is_some() && !replace_if_existing {
            return Err(STATUS_OBJECT_NAME_COLLISION);
        }
        self.ensure_parent_visible(&new_path)?;
        if replace_if_existing {
            self.remove_existing_for_replace(&new_path)?;
        }
        if self.upper_metadata(&context.path).is_some() {
            self.ensure_upper_parent(&new_path)?;
            fs::rename(self.upper_path(&context.path), self.upper_path(&new_path))
                .map_err(io_status)?;
        } else {
            self.copy_lower_subtree_to_upper(&context.path, &new_path)?;
        }
        if self.lower_dirs.contains(&context.path) || self.lower_files.contains_key(&context.path) {
            self.whiteouts.lock().unwrap().insert(context.path.clone());
            self.save_whiteouts()?;
        }
        Ok(())
    }

    fn set_end_of_file(
        &'h self,
        _file_name: &U16CStr,
        offset: i64,
        _info: &OperationInfo<'c, 'h, Self>,
        context: &'c Self::Context,
    ) -> OperationResult<()> {
        if context.is_dir {
            return Err(STATUS_FILE_IS_A_DIRECTORY);
        }
        self.ensure_upper_file(&context.path, false)?
            .set_len(offset.max(0) as u64)
            .map_err(io_status)
    }

    fn set_allocation_size(
        &'h self,
        file_name: &U16CStr,
        alloc_size: i64,
        info: &OperationInfo<'c, 'h, Self>,
        context: &'c Self::Context,
    ) -> OperationResult<()> {
        self.set_end_of_file(file_name, alloc_size, info, context)
    }

    fn get_disk_free_space(
        &'h self,
        _info: &OperationInfo<'c, 'h, Self>,
    ) -> OperationResult<DiskSpaceInfo> {
        Ok(DiskSpaceInfo {
            byte_count: 1 << 40,
            free_byte_count: 1 << 39,
            available_byte_count: 1 << 39,
        })
    }

    fn get_volume_information(
        &'h self,
        _info: &OperationInfo<'c, 'h, Self>,
    ) -> OperationResult<VolumeInfo> {
        Ok(VolumeInfo {
            name: U16CString::from_str("Trail Overlay").unwrap(),
            serial_number: 0xC0DB,
            max_component_length: 255,
            fs_flags: winnt::FILE_CASE_SENSITIVE_SEARCH | winnt::FILE_CASE_PRESERVED_NAMES,
            fs_name: U16CString::from_str("NTFS").unwrap(),
        })
    }

    fn mounted(
        &'h self,
        _mount_point: &U16CStr,
        _info: &OperationInfo<'c, 'h, Self>,
    ) -> OperationResult<()> {
        Ok(())
    }

    fn unmounted(&'h self, _info: &OperationInfo<'c, 'h, Self>) -> OperationResult<()> {
        Ok(())
    }
}

fn dokan_path_to_rel(path: &U16CStr) -> OperationResult<String> {
    let raw = path.to_string_lossy();
    let trimmed = raw.trim_matches('\\').replace('\\', "/");
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed.split('/').any(|part| part.contains(':')) {
        return Err(STATUS_OBJECT_NAME_INVALID);
    }
    normalize_relative_path(&trimmed).map_err(error_status)
}

fn parent_of(path: &str) -> String {
    Path::new(path)
        .parent()
        .map(|parent| parent.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn stable_time() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1)
}

fn stable_file_index(path: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

fn file_info_from_metadata(path: &str, metadata: &fs::Metadata, kind: NodeKind) -> FileInfo {
    let attributes = match kind {
        NodeKind::Directory => winnt::FILE_ATTRIBUTE_DIRECTORY,
        NodeKind::File => winnt::FILE_ATTRIBUTE_ARCHIVE,
    };
    FileInfo {
        attributes,
        creation_time: metadata.created().unwrap_or_else(|_| stable_time()),
        last_access_time: metadata.accessed().unwrap_or_else(|_| stable_time()),
        last_write_time: metadata.modified().unwrap_or_else(|_| stable_time()),
        file_size: metadata.len(),
        number_of_links: 1,
        file_index: stable_file_index(path),
    }
}

fn fill_status(err: FillDataError) -> NTSTATUS {
    match err {
        FillDataError::BufferFull => STATUS_BUFFER_OVERFLOW,
        FillDataError::NameTooLong => STATUS_OBJECT_NAME_INVALID,
    }
}

fn error_status(err: Error) -> NTSTATUS {
    match err {
        Error::Io(err) => io_status(err),
        Error::InvalidPath { .. } => STATUS_OBJECT_NAME_INVALID,
        Error::InvalidInput(_) => STATUS_INVALID_PARAMETER,
        Error::WorkspaceNotFound(_) => STATUS_OBJECT_PATH_NOT_FOUND,
        _ => STATUS_UNSUCCESSFUL,
    }
}

fn io_status(err: std::io::Error) -> NTSTATUS {
    match err.kind() {
        std::io::ErrorKind::NotFound => STATUS_OBJECT_NAME_NOT_FOUND,
        std::io::ErrorKind::AlreadyExists => STATUS_OBJECT_NAME_COLLISION,
        std::io::ErrorKind::PermissionDenied => STATUS_ACCESS_DENIED,
        std::io::ErrorKind::InvalidInput => STATUS_INVALID_PARAMETER,
        _ => STATUS_UNSUCCESSFUL,
    }
}
