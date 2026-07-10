use super::*;

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex, MutexGuard, Once};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dokan::{
    init, unmount, CreateFileInfo, DiskSpaceInfo, FileInfo, FileSystemHandler, FileSystemMounter,
    FillDataError, FillDataResult, FindData, MountFlags, MountOptions, OperationInfo,
    OperationResult, VolumeInfo, IO_SECURITY_CONTEXT,
};
use dokan_sys::win32::{
    FILE_CREATE, FILE_DELETE_ON_CLOSE, FILE_DIRECTORY_FILE, FILE_MAXIMUM_DISPOSITION,
    FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OVERWRITE, FILE_OVERWRITE_IF, FILE_SUPERSEDE,
};
use widestring::{U16CStr, U16CString};
use winapi::shared::ntdef::NTSTATUS;
use winapi::shared::ntstatus::*;
use winapi::um::winnt;

static DOKAN_INIT: Once = Once::new();

const OVERLAY_META_DIR: &str = ".trail";

pub(crate) struct OverlayCowMount {
    mountpoint: PathBuf,
    mount_name: U16CString,
    worker: Option<JoinHandle<()>>,
    lease: WorkspaceMountLease,
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
    let upperdir = db
        .prepare_workspace_view_storage_for_lane_name(lane)?
        .source_upper;
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

    let head = db.get_ref(&branch.ref_name)?;
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
        head.root_id,
    )?;
    let mut lease = db.acquire_workspace_mount_lease(lane, "dokan")?;
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
        };
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
    lease.mark_mounted()?;

    Ok(OverlayCowMount {
        mountpoint,
        mount_name,
        worker: Some(worker),
        lease,
    })
}

pub(crate) fn overlay_candidate_paths(db: &Trail, lane: &str) -> Result<Vec<String>> {
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

fn overlay_mount_error(mountpoint: &Path, err: &str) -> Error {
    Error::InvalidInput(format!(
        "failed to mount overlay-cow workdir at `{}` with Dokan: {err}. Install Dokan 2.x and ensure the Dokan driver service is running.",
        mountpoint.display()
    ))
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

#[derive(Debug)]
struct DokanHandle {
    path: Mutex<String>,
    is_dir: bool,
    delete_on_cleanup: AtomicBool,
}

impl DokanHandle {
    fn new(path: String, is_dir: bool, delete_on_cleanup: bool) -> Self {
        Self {
            path: Mutex::new(path),
            is_dir,
            delete_on_cleanup: AtomicBool::new(delete_on_cleanup),
        }
    }

    fn path(&self) -> OperationResult<String> {
        self.path
            .lock()
            .map(|path| path.clone())
            .map_err(|_| STATUS_INTERNAL_ERROR)
    }

    fn set_path(&self, path: String) -> OperationResult<()> {
        *self.path.lock().map_err(|_| STATUS_INTERNAL_ERROR)? = path;
        Ok(())
    }
}

struct DokanOverlayFs {
    core: Mutex<ViewCore>,
}

impl DokanOverlayFs {
    fn new(
        workspace_root: PathBuf,
        db_dir: PathBuf,
        upperdir: PathBuf,
        root_id: ObjectId,
    ) -> Result<Self> {
        Ok(Self {
            core: Mutex::new(ViewCore::new_lazy(
                Trail::open_with_db_dir(workspace_root, db_dir)?,
                upperdir,
                root_id,
            )?),
        })
    }

    fn core(&self) -> OperationResult<MutexGuard<'_, ViewCore>> {
        self.core.lock().map_err(|_| STATUS_INTERNAL_ERROR)
    }

    fn delete_path(&self, path: &str, expected_dir: bool) -> OperationResult<()> {
        if path.is_empty() {
            return Err(STATUS_ACCESS_DENIED);
        }
        let mut core = self.core()?;
        let kind = core
            .node_kind(path)
            .map_err(view_status)?
            .ok_or(STATUS_OBJECT_NAME_NOT_FOUND)?;
        if expected_dir != (kind == ViewNodeKind::Directory) {
            return Err(if expected_dir {
                STATUS_NOT_A_DIRECTORY
            } else {
                STATUS_FILE_IS_A_DIRECTORY
            });
        }
        let (parent, name) = parent_and_name(&mut core, path)?;
        core.remove(parent, &name).map_err(view_status)
    }

    fn path_info(&self, path: &str) -> OperationResult<FileInfo> {
        let mut core = self.core()?;
        core.attr(path)
            .map(|attr| file_info_from_view(&attr))
            .map_err(view_status)
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
        let mut core = self.core()?;
        let existing = core.node_kind(&path).map_err(view_status)?;

        if let Some(kind) = existing {
            if wants_dir && kind != ViewNodeKind::Directory {
                return Err(STATUS_NOT_A_DIRECTORY);
            }
            if wants_file && kind == ViewNodeKind::Directory {
                return Err(STATUS_FILE_IS_A_DIRECTORY);
            }
            if create_disposition == FILE_CREATE {
                return Err(STATUS_OBJECT_NAME_COLLISION);
            }
            if matches!(
                create_disposition,
                FILE_OVERWRITE | FILE_OVERWRITE_IF | FILE_SUPERSEDE
            ) {
                if kind == ViewNodeKind::Directory {
                    return Err(STATUS_FILE_IS_A_DIRECTORY);
                }
                core.ensure_upper_file(&path, true).map_err(view_status)?;
            }
            return Ok(CreateFileInfo {
                context: DokanHandle::new(path, kind == ViewNodeKind::Directory, delete_on_cleanup),
                is_dir: kind == ViewNodeKind::Directory,
                new_file_created: false,
            });
        }

        if matches!(create_disposition, FILE_OPEN | FILE_OVERWRITE) {
            return Err(STATUS_OBJECT_NAME_NOT_FOUND);
        }

        if wants_dir || file_attributes & winnt::FILE_ATTRIBUTE_DIRECTORY != 0 {
            let (parent, name) = parent_and_name(&mut core, &path)?;
            core.mkdir(parent, &name, 0o755).map_err(view_status)?;
            return Ok(CreateFileInfo {
                context: DokanHandle::new(path, true, delete_on_cleanup),
                is_dir: true,
                new_file_created: true,
            });
        }

        let (parent, name) = parent_and_name(&mut core, &path)?;
        core.create(parent, &name, 0o644, true)
            .map_err(view_status)?;
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
            if let Ok(path) = context.path() {
                let _ = self.delete_path(&path, context.is_dir);
            }
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
        let path = context.path()?;
        let mut core = self.core()?;
        let ino = core.attr(&path).map_err(view_status)?.ino;
        let (bytes, _) = core
            .read(ino, offset.max(0) as u64, buffer.len() as u32)
            .map_err(view_status)?;
        buffer[..bytes.len()].copy_from_slice(&bytes);
        Ok(bytes.len() as u32)
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
        let path = context.path()?;
        let mut core = self.core()?;
        let attr = core.attr(&path).map_err(view_status)?;
        let write_offset = if info.write_to_eof() {
            attr.size
        } else {
            offset.max(0) as u64
        };
        core.write(attr.ino, write_offset, buffer)
            .map_err(view_status)?;
        Ok(buffer.len() as u32)
    }

    fn flush_file_buffers(
        &'h self,
        _file_name: &U16CStr,
        _info: &OperationInfo<'c, 'h, Self>,
        context: &'c Self::Context,
    ) -> OperationResult<()> {
        if !context.is_dir {
            let path = context.path()?;
            let core = self.core()?;
            let upper = core.upper_path(&path).map_err(view_status)?;
            if upper.is_file() {
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(upper)
                    .and_then(|file| file.sync_all())
                    .map_err(io_status)?;
            }
        }
        Ok(())
    }

    fn get_file_information(
        &'h self,
        _file_name: &U16CStr,
        _info: &OperationInfo<'c, 'h, Self>,
        context: &'c Self::Context,
    ) -> OperationResult<FileInfo> {
        self.path_info(&context.path()?)
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
        let path = context.path()?;
        let data = {
            let mut core = self.core()?;
            let ino = core.attr(&path).map_err(view_status)?.ino;
            core.children(ino)
                .map_err(view_status)?
                .into_iter()
                .map(|(name, attr)| find_data_from_view(name, &attr))
                .collect::<OperationResult<Vec<_>>>()?
        };
        for data in data {
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
            let path = context.path()?;
            if self.core()?.node_kind(&path).map_err(view_status)? != Some(ViewNodeKind::File) {
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
            let path = context.path()?;
            let mut core = self.core()?;
            let ino = core.attr(&path).map_err(view_status)?.ino;
            if !core.children(ino).map_err(view_status)?.is_empty() {
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
        let old_path = context.path()?;
        let new_path = dokan_path_to_rel(new_file_name)?;
        let mut core = self.core()?;
        if core.node_kind(&new_path).map_err(view_status)?.is_some() && !replace_if_existing {
            return Err(STATUS_OBJECT_NAME_COLLISION);
        }
        let (old_parent, old_name) = parent_and_name(&mut core, &old_path)?;
        let (new_parent, new_name) = parent_and_name(&mut core, &new_path)?;
        core.rename(old_parent, &old_name, new_parent, &new_name)
            .map_err(view_status)?;
        drop(core);
        context.set_path(new_path)?;
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
        let path = context.path()?;
        let mut core = self.core()?;
        let ino = core.attr(&path).map_err(view_status)?.ino;
        core.setattr(ino, Some(offset.max(0) as u64), None)
            .map(|_| ())
            .map_err(view_status)
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

fn parent_and_name(core: &mut ViewCore, path: &str) -> OperationResult<(u64, String)> {
    if path.is_empty() {
        return Err(STATUS_ACCESS_DENIED);
    }
    let path = Path::new(path);
    let parent = path
        .parent()
        .map(|parent| parent.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(STATUS_OBJECT_NAME_INVALID)?
        .to_string();
    let parent = core.attr(&parent).map_err(view_status)?;
    if parent.kind != ViewNodeKind::Directory {
        return Err(STATUS_OBJECT_PATH_NOT_FOUND);
    }
    Ok((parent.ino, name))
}

fn stable_time() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1)
}

fn file_info_from_view(attr: &ViewNodeAttr) -> FileInfo {
    let attributes = match attr.kind {
        ViewNodeKind::Directory => winnt::FILE_ATTRIBUTE_DIRECTORY,
        ViewNodeKind::File => winnt::FILE_ATTRIBUTE_ARCHIVE,
        // Windows package managers normally emit command shims rather than
        // Unix symlinks. Preserve compatibility for imported layer links as a
        // non-directory entry until Dokan reparse-point support is added.
        ViewNodeKind::Symlink => winnt::FILE_ATTRIBUTE_ARCHIVE,
    };
    FileInfo {
        attributes,
        creation_time: stable_time(),
        last_access_time: attr.modified,
        last_write_time: attr.modified,
        file_size: attr.size,
        number_of_links: 1,
        file_index: attr.ino,
    }
}

fn find_data_from_view(name: String, attr: &ViewNodeAttr) -> OperationResult<FindData> {
    let info = file_info_from_view(attr);
    Ok(FindData {
        attributes: info.attributes,
        creation_time: info.creation_time,
        last_access_time: info.last_access_time,
        last_write_time: info.last_write_time,
        file_size: info.file_size,
        file_name: U16CString::from_str(&name).map_err(|_| STATUS_OBJECT_NAME_INVALID)?,
    })
}

fn view_status(err: i32) -> NTSTATUS {
    match err {
        1 => STATUS_ACCESS_DENIED,
        2 => STATUS_OBJECT_NAME_NOT_FOUND,
        17 => STATUS_OBJECT_NAME_COLLISION,
        20 => STATUS_NOT_A_DIRECTORY,
        21 => STATUS_FILE_IS_A_DIRECTORY,
        22 => STATUS_INVALID_PARAMETER,
        39 => STATUS_DIRECTORY_NOT_EMPTY,
        _ => STATUS_UNSUCCESSFUL,
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

#[cfg(test)]
mod mounted_conformance {
    use super::*;

    #[test]
    fn dokan_adapter_runs_shared_mounted_view_suite() {
        if std::env::var_os("TRAIL_RUN_DOKAN_COW_TESTS").is_none() {
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
            "dokan-conformance",
            Some("main"),
            LaneWorkdirMode::OverlayCow,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let mount = db
            .mount_overlay_cow_workdir_for_lane("dokan-conformance")
            .unwrap();
        let workdir = PathBuf::from(
            db.lane_workdir("dokan-conformance")
                .unwrap()
                .workdir
                .unwrap(),
        );
        let expected = run_mounted_view_conformance(&workdir).unwrap();
        let record = db
            .record_lane_workdir("dokan-conformance", Some("conformance".to_string()))
            .unwrap();
        let actual = record
            .changed_paths
            .into_iter()
            .flat_map(|path| path.old_path.into_iter().chain(std::iter::once(path.path)))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(actual, expected.changed_paths);
        assert_eq!(
            fs::read(temp.path().join("README.md")).unwrap(),
            b"baseline\n"
        );
        drop(mount);
    }

    #[test]
    fn foreground_dokan_mount_stops_through_a_separate_trail_handle() {
        if std::env::var_os("TRAIL_RUN_DOKAN_COW_TESTS").is_none() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "baseline\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let spawned = db
            .spawn_lane_with_workdir_mode_paths_and_neighbors(
                "foreground-dokan",
                Some("main"),
                LaneWorkdirMode::OverlayCow,
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
                .mount_lane_workspace_until_requested("foreground-dokan")
                .unwrap()
        });
        let deadline = Instant::now() + Duration::from_secs(15);
        while !workdir.join("README.md").is_file() {
            assert!(
                Instant::now() < deadline,
                "foreground Dokan mount did not start"
            );
            thread::sleep(Duration::from_millis(50));
        }
        fs::write(workdir.join("foreground.txt"), "owned\n").unwrap();
        let requester = Trail::open(temp.path()).unwrap();
        let stopped = requester
            .request_lane_workspace_unmount("foreground-dokan")
            .unwrap();
        assert!(stopped.healthy);
        let owned = owner.join().unwrap();
        assert_eq!(owned.view_id, stopped.view_id);
        let mut reopened = Trail::open(temp.path()).unwrap();
        let checkpoint = reopened
            .checkpoint_lane_workspace("foreground-dokan", None)
            .unwrap();
        assert_eq!(checkpoint.source_paths, vec!["foreground.txt"]);
    }

    #[test]
    fn daemon_owned_dokan_mount_returns_ready_and_unmounts_asynchronously() {
        if std::env::var_os("TRAIL_RUN_DOKAN_COW_TESTS").is_none() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "baseline\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let spawned = db
            .spawn_lane_with_workdir_mode_paths_and_neighbors(
                "daemon-dokan",
                Some("main"),
                LaneWorkdirMode::OverlayCow,
                None,
                None,
                None,
                &[],
                false,
            )
            .unwrap();
        let workdir = PathBuf::from(spawned.workdir.unwrap());
        let mounted = db.start_lane_workspace_mount("daemon-dokan").unwrap();
        assert!(mounted.healthy);
        assert!(mounted.owner_pid.is_some());
        fs::write(workdir.join("daemon.txt"), "owned\n").unwrap();
        let stopped = db.request_lane_workspace_unmount("daemon-dokan").unwrap();
        assert_eq!(mounted.view_id, stopped.view_id);
        assert_eq!(stopped.owner_pid, None);
        let checkpoint = db.checkpoint_lane_workspace("daemon-dokan", None).unwrap();
        assert_eq!(checkpoint.source_paths, vec!["daemon.txt"]);
    }
}
