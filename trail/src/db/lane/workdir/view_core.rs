use super::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File, OpenOptions};
#[cfg(unix)]
use std::os::unix::fs::{FileExt, MetadataExt, PermissionsExt};
#[cfg(windows)]
use std::os::windows::fs::FileExt;

#[cfg(unix)]
use libc::{EEXIST, EINVAL, EIO, EISDIR, ENOENT, ENOSPC, ENOTDIR, ENOTEMPTY, EPERM};
#[cfg(windows)]
const EPERM: i32 = 1;
#[cfg(windows)]
const ENOENT: i32 = 2;
#[cfg(windows)]
const EIO: i32 = 5;
#[cfg(windows)]
const EEXIST: i32 = 17;
#[cfg(windows)]
const ENOTDIR: i32 = 20;
#[cfg(windows)]
const EISDIR: i32 = 21;
#[cfg(windows)]
const EINVAL: i32 = 22;
#[cfg(windows)]
const ENOSPC: i32 = 28;
#[cfg(windows)]
const ENOTEMPTY: i32 = 39;

pub(crate) const VIEW_ROOT_INO: u64 = 1;

#[cfg(test)]
std::thread_local! {
    static FAIL_RENAME_AFTER_INTENT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FAIL_RENAME_BEFORE_DURABILITY_FENCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FAIL_NAMESPACE_BEFORE_DURABILITY_FENCE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn fail_rename_after_intent_for_current_thread() {
    FAIL_RENAME_AFTER_INTENT.with(|fail| fail.set(true));
}

#[cfg(test)]
fn fail_rename_after_intent_if_requested() -> std::result::Result<(), i32> {
    if FAIL_RENAME_AFTER_INTENT.with(|fail| fail.replace(false)) {
        Err(EIO)
    } else {
        Ok(())
    }
}

#[cfg(test)]
fn fail_rename_before_durability_fence_for_current_thread() {
    FAIL_RENAME_BEFORE_DURABILITY_FENCE.with(|fail| fail.set(true));
}

#[cfg(test)]
fn fail_rename_before_durability_fence_if_requested() -> std::result::Result<(), i32> {
    if FAIL_RENAME_BEFORE_DURABILITY_FENCE.with(|fail| fail.replace(false)) {
        Err(EIO)
    } else {
        Ok(())
    }
}

#[cfg(not(test))]
fn fail_rename_before_durability_fence_if_requested() -> std::result::Result<(), i32> {
    Ok(())
}

#[cfg(test)]
fn fail_namespace_before_durability_fence_for_current_thread() {
    FAIL_NAMESPACE_BEFORE_DURABILITY_FENCE.with(|fail| fail.set(true));
}

#[cfg(test)]
fn fail_namespace_before_durability_fence_if_requested() -> std::result::Result<(), i32> {
    if FAIL_NAMESPACE_BEFORE_DURABILITY_FENCE.with(|fail| fail.replace(false)) {
        Err(EIO)
    } else {
        Ok(())
    }
}

#[cfg(not(test))]
fn fail_namespace_before_durability_fence_if_requested() -> std::result::Result<(), i32> {
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ViewNodeKind {
    File,
    Directory,
    Symlink,
}

#[derive(Clone, Debug)]
pub(crate) struct ViewNodeAttr {
    pub(crate) ino: u64,
    pub(crate) kind: ViewNodeKind,
    pub(crate) mode: u32,
    pub(crate) size: u64,
    pub(crate) modified: SystemTime,
}

/// Backend-neutral overlay semantics for a Trail-root lower and filesystem
/// upper. Protocol adapters translate FUSE/NFS/Dokan requests into these
/// operations; the core owns copy-up, whiteouts, rename, inode, and visibility
/// behavior.
pub(crate) struct ViewCore {
    db: Trail,
    layout: ViewUpperLayout,
    layers: Vec<WorkspaceLayerBinding>,
    lower: ViewLower,
    whiteouts: BTreeSet<String>,
    ino_by_path: HashMap<String, u64>,
    path_by_ino: HashMap<u64, String>,
    dir_mtime: HashMap<String, SystemTime>,
    dir_epoch: SystemTime,
    next_ino: u64,
    journal: ViewMutationJournal,
    generation_lease: ViewGenerationLease,
}

const LAYER_MOUNT_RESET_INTENT_VERSION: u16 = 2;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct LayerMountResetIntent {
    version: u16,
    mount_path: String,
    upper_class: ViewPathClass,
    replacement_layer_id: String,
    #[serde(default)]
    binding_removed: bool,
    removed_whiteouts: Vec<String>,
}

/// A durable, filesystem-side half of workspace-layer activation.
///
/// The reset intent is written before the private upper is moved aside. If the
/// process dies, `ViewCore::new_with_lower` compares the intended replacement
/// with SQLite: a committed binding discards the backup, while an uncommitted
/// binding restores it. Dropping this value intentionally leaves the durable
/// intent for crash recovery.
#[must_use = "a prepared layer reset must be committed or rolled back"]
pub(crate) struct PreparedLayerMountReset {
    layout: ViewUpperLayout,
    mount_path: String,
    upper_class: ViewPathClass,
    intent_path: PathBuf,
    backup_path: PathBuf,
    removed_whiteouts: Vec<String>,
}

struct RecoveredLayerMountReset {
    mount_path: String,
    intent_path: PathBuf,
    backup_path: PathBuf,
    removed_whiteouts: Vec<String>,
}

enum ViewLower {
    #[cfg(test)]
    Eager(BTreeMap<String, FileEntry>),
    Root(ObjectId),
}

impl ViewCore {
    #[cfg(test)]
    pub(crate) fn new(
        db: Trail,
        upperdir: PathBuf,
        lower_files: BTreeMap<String, FileEntry>,
    ) -> Result<Self> {
        Self::new_with_lower(db, upperdir, ViewLower::Eager(lower_files))
    }

    pub(crate) fn new_lazy(db: Trail, upperdir: PathBuf, root_id: ObjectId) -> Result<Self> {
        Self::new_with_lower(db, upperdir, ViewLower::Root(root_id))
    }

    /// Construct a temporary mounted view with an explicit desired binding
    /// set. This is used while initializing path-sensitive private outputs at
    /// the lane's stable mountpoint before their generation is activated.
    ///
    /// Reset recovery is deliberately skipped: the supplied upper layout is
    /// ephemeral and cannot contain a durable activation intent. The real
    /// lane layout continues to use `new_lazy`, which always performs normal
    /// SQLite-correlated recovery.
    pub(crate) fn new_lazy_with_ephemeral_bindings(
        db: Trail,
        upperdir: PathBuf,
        root_id: ObjectId,
        layers: Vec<WorkspaceLayerBinding>,
    ) -> Result<Self> {
        Self::new_with_lower_and_bindings(db, upperdir, ViewLower::Root(root_id), Some(layers))
    }

    fn new_with_lower(db: Trail, upperdir: PathBuf, lower: ViewLower) -> Result<Self> {
        Self::new_with_lower_and_bindings(db, upperdir, lower, None)
    }

    fn new_with_lower_and_bindings(
        db: Trail,
        upperdir: PathBuf,
        lower: ViewLower,
        ephemeral_layers: Option<Vec<WorkspaceLayerBinding>>,
    ) -> Result<Self> {
        let dir_epoch = SystemTime::now();
        let layout = ViewUpperLayout::from_source_upper(upperdir);
        layout.ensure()?;
        ViewMutationJournal::initialize_storage(&layout.source_upper)?;
        let _barrier = ViewMutationBarrier::shared(&layout.meta_dir)?;
        let mut journal = ViewMutationJournal::open(&layout.source_upper)?;
        journal.observe_checkpoint(
            _barrier.checkpoint_sequence(),
            _barrier.checkpoint_generation(),
        )?;
        let generation_lease =
            ViewGenerationLease::acquire(&layout.source_upper, journal.generation())?;
        let ephemeral = ephemeral_layers.is_some();
        let layers = match ephemeral_layers {
            Some(layers) => layers,
            None => db.workspace_layer_bindings_for_source_upper(&layout.source_upper)?,
        };
        let recovered_resets = if ephemeral {
            Vec::new()
        } else {
            recover_layer_mount_resets(&layout, &layers)?
        };
        let mut core = Self {
            db,
            layout,
            layers,
            lower,
            whiteouts: BTreeSet::new(),
            ino_by_path: HashMap::from([(String::new(), VIEW_ROOT_INO)]),
            path_by_ino: HashMap::from([(VIEW_ROOT_INO, String::new())]),
            dir_mtime: HashMap::from([(String::new(), dir_epoch)]),
            dir_epoch,
            next_ino: VIEW_ROOT_INO + 1,
            journal,
            generation_lease,
        };
        core.whiteouts = core.journal.whiteouts().clone();
        if !recovered_resets.is_empty() {
            for recovered in &recovered_resets {
                if !recovered.removed_whiteouts.is_empty() {
                    let class = core.path_class(&recovered.mount_path);
                    let sequence = core.journal.append_classified_with_whiteouts(
                        ViewMutationKind::Metadata,
                        recovered.mount_path.clone(),
                        class,
                        None,
                        None,
                        recovered
                            .removed_whiteouts
                            .iter()
                            .cloned()
                            .map(ViewWhiteoutChange::Insert)
                            .collect(),
                    )?;
                    core.journal.commit_whiteouts(sequence)?;
                }
                core.whiteouts
                    .extend(recovered.removed_whiteouts.iter().cloned());
            }
            for recovered in recovered_resets {
                finish_layer_mount_reset_recovery(&recovered)?;
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

    #[cfg(test)]
    pub(crate) fn indexed_path_count(&self) -> usize {
        self.ino_by_path.len()
    }

    pub(crate) fn path_for_ino(&self, ino: u64) -> std::result::Result<String, i32> {
        self.path_by_ino.get(&ino).cloned().ok_or(ENOENT)
    }

    pub(crate) fn child_path(&self, parent: u64, name: &str) -> std::result::Result<String, i32> {
        let parent = self.path_for_ino(parent)?;
        if name.is_empty() || name.contains('/') || name == "." || name == ".." {
            return Err(EINVAL);
        }
        Ok(if parent.is_empty() {
            name.to_string()
        } else {
            format!("{parent}/{name}")
        })
    }

    pub(crate) fn upper_path(&self, path: &str) -> std::result::Result<PathBuf, i32> {
        self.upper_path_in_class(self.path_class(path), path)
    }

    fn path_class(&self, path: &str) -> ViewPathClass {
        let conventional = classify_view_path(path);
        if matches!(
            conventional,
            ViewPathClass::Internal | ViewPathClass::Secret
        ) {
            return conventional;
        }
        let binding = self.layers.iter().find(|binding| {
            path == binding.mount_path
                || path
                    .strip_prefix(&binding.mount_path)
                    .is_some_and(|rest| rest.starts_with('/'))
        });
        match binding.map(|binding| binding.kind.as_str()) {
            Some("dependency") => ViewPathClass::Dependency,
            Some("compiler-results" | "generated" | "build") => ViewPathClass::Generated,
            _ => conventional,
        }
    }

    fn upper_path_in_class(
        &self,
        class: ViewPathClass,
        path: &str,
    ) -> std::result::Result<PathBuf, i32> {
        self.upper_path_in_class_with_leaf(class, path, false)
    }

    fn upper_path_in_class_with_leaf(
        &self,
        class: ViewPathClass,
        path: &str,
        allow_leaf_symlink: bool,
    ) -> std::result::Result<PathBuf, i32> {
        if path.is_empty() {
            return Ok(self.layout.upper_for_class(class).to_path_buf());
        }
        let normalized = normalize_relative_path(path).map_err(|_| EINVAL)?;
        let components = Path::new(&normalized).components().collect::<Vec<_>>();
        let mut current = self.layout.upper_for_class(class).to_path_buf();
        for (index, component) in components.iter().enumerate() {
            let std::path::Component::Normal(name) = component else {
                return Err(EINVAL);
            };
            current.push(name);
            match fs::symlink_metadata(&current) {
                Ok(metadata)
                    if metadata.file_type().is_symlink()
                        && !(allow_leaf_symlink && index + 1 == components.len()) =>
                {
                    return Err(EPERM);
                }
                Ok(_) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(io_errno(err)),
            }
        }
        Ok(current)
    }

    fn upper_metadata(&self, path: &str) -> Option<fs::Metadata> {
        self.upper_path_in_class_with_leaf(self.path_class(path), path, true)
            .ok()
            .and_then(|path| fs::symlink_metadata(path).ok())
    }

    fn upper_path_with_leaf_symlink(&self, path: &str) -> std::result::Result<PathBuf, i32> {
        self.upper_path_in_class_with_leaf(self.path_class(path), path, true)
    }

    pub(crate) fn is_whiteouted(&self, path: &str) -> bool {
        self.whiteouts.iter().any(|item| {
            path == item
                || path
                    .strip_prefix(item)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
    }

    fn lower_file(&self, path: &str) -> std::result::Result<Option<FileEntry>, i32> {
        match &self.lower {
            #[cfg(test)]
            ViewLower::Eager(files) => Ok(files.get(path).cloned()),
            ViewLower::Root(root_id) => self.db.root_file_entry(root_id, path).map_err(|_| EIO),
        }
    }

    fn layer_path(&self, path: &str) -> std::result::Result<Option<PathBuf>, i32> {
        for binding in &self.layers {
            let Some(storage_path) = &binding.storage_path else {
                continue;
            };
            let suffix = if path == binding.mount_path {
                Some("")
            } else {
                path.strip_prefix(&binding.mount_path)
                    .and_then(|suffix| suffix.strip_prefix('/'))
            };
            let Some(suffix) = suffix else {
                continue;
            };
            let candidate = if suffix.is_empty() {
                storage_path.clone()
            } else {
                safe_join(storage_path, suffix).map_err(|_| EINVAL)?
            };
            let metadata = match fs::symlink_metadata(&candidate) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => return Err(io_errno(err)),
            };
            if metadata.file_type().is_symlink() {
                let canonical = candidate.canonicalize().map_err(io_errno)?;
                let root = storage_path.canonicalize().map_err(io_errno)?;
                if !canonical.starts_with(root) {
                    return Err(EPERM);
                }
            }
            return Ok(Some(candidate));
        }
        Ok(None)
    }

    fn layer_directory_exists(&self, path: &str) -> std::result::Result<bool, i32> {
        if self
            .layer_path(path)?
            .is_some_and(|path| fs::metadata(path).is_ok_and(|metadata| metadata.is_dir()))
        {
            return Ok(true);
        }
        let prefix = if path.is_empty() {
            String::new()
        } else {
            format!("{path}/")
        };
        Ok(self
            .layers
            .iter()
            .any(|binding| binding.mount_path.starts_with(&prefix)))
    }

    fn lower_directory_exists(&self, path: &str) -> std::result::Result<bool, i32> {
        match &self.lower {
            #[cfg(test)]
            ViewLower::Eager(files) => {
                if path.is_empty() {
                    return Ok(true);
                }
                let prefix = format!("{path}/");
                Ok(files.keys().any(|candidate| candidate.starts_with(&prefix)))
            }
            ViewLower::Root(root_id) => self
                .db
                .root_directory_exists(root_id, path)
                .map_err(|_| EIO),
        }
    }

    fn lower_children(&self, path: &str) -> std::result::Result<Vec<RootDirectoryChild>, i32> {
        match &self.lower {
            #[cfg(test)]
            ViewLower::Eager(files) => {
                let prefix = if path.is_empty() {
                    String::new()
                } else {
                    format!("{path}/")
                };
                let mut children = BTreeMap::<String, Option<FileEntry>>::new();
                for (candidate, entry) in files {
                    let Some(remainder) = candidate.strip_prefix(&prefix) else {
                        continue;
                    };
                    let (name, direct) = match remainder.split_once('/') {
                        Some((name, _)) => (name, false),
                        None => (remainder, true),
                    };
                    if name.is_empty() {
                        continue;
                    }
                    children
                        .entry(name.to_string())
                        .and_modify(|value| {
                            if !direct {
                                *value = None;
                            }
                        })
                        .or_insert_with(|| direct.then(|| entry.clone()));
                }
                Ok(children
                    .into_iter()
                    .map(|(name, entry)| RootDirectoryChild {
                        path: if path.is_empty() {
                            name.clone()
                        } else {
                            format!("{path}/{name}")
                        },
                        name,
                        entry,
                    })
                    .collect())
            }
            ViewLower::Root(root_id) => self
                .db
                .root_immediate_children(root_id, path)
                .map_err(|_| EIO),
        }
    }

    fn lower_selection(&self, path: &str) -> std::result::Result<BTreeMap<String, FileEntry>, i32> {
        match &self.lower {
            #[cfg(test)]
            ViewLower::Eager(files) => {
                let prefix = format!("{path}/");
                Ok(files
                    .iter()
                    .filter(|(candidate, _)| *candidate == path || candidate.starts_with(&prefix))
                    .map(|(path, entry)| (path.clone(), entry.clone()))
                    .collect())
            }
            ViewLower::Root(root_id) => self
                .db
                .load_root_files_for_selections(root_id, &[path.to_string()])
                .map_err(|_| EIO),
        }
    }

    pub(crate) fn node_kind(&self, path: &str) -> std::result::Result<Option<ViewNodeKind>, i32> {
        if path.is_empty() {
            return Ok(Some(ViewNodeKind::Directory));
        }
        if self.path_class(path) == ViewPathClass::Internal || self.is_whiteouted(path) {
            return Ok(None);
        }
        if let Some(metadata) = self.upper_metadata(path) {
            if metadata.file_type().is_symlink() {
                return Ok(Some(ViewNodeKind::Symlink));
            }
            if metadata.is_file() {
                return Ok(Some(ViewNodeKind::File));
            }
            if metadata.is_dir() {
                return Ok(Some(ViewNodeKind::Directory));
            }
            return Ok(None);
        }
        if let Some(layer_path) = self.layer_path(path)? {
            let metadata = fs::symlink_metadata(layer_path).map_err(io_errno)?;
            if metadata.file_type().is_symlink() {
                #[cfg(unix)]
                return Ok(Some(ViewNodeKind::Symlink));
                #[cfg(windows)]
                {
                    let metadata =
                        fs::metadata(self.layer_path(path)?.ok_or(ENOENT)?).map_err(io_errno)?;
                    return Ok(if metadata.is_dir() {
                        Some(ViewNodeKind::Directory)
                    } else if metadata.is_file() {
                        Some(ViewNodeKind::File)
                    } else {
                        None
                    });
                }
            }
            if metadata.is_file() {
                return Ok(Some(ViewNodeKind::File));
            }
            if metadata.is_dir() {
                return Ok(Some(ViewNodeKind::Directory));
            }
        }
        if self.layer_directory_exists(path)? {
            return Ok(Some(ViewNodeKind::Directory));
        }
        if self.lower_file(path)?.is_some() {
            Ok(Some(ViewNodeKind::File))
        } else if self.lower_directory_exists(path)? {
            Ok(Some(ViewNodeKind::Directory))
        } else {
            Ok(None)
        }
    }

    pub(crate) fn attr(&mut self, path: &str) -> std::result::Result<ViewNodeAttr, i32> {
        let kind = self.node_kind(path)?.ok_or(ENOENT)?;
        let ino = self.ensure_ino(path);
        if kind == ViewNodeKind::Directory {
            let mode = self
                .upper_metadata(path)
                .map(|metadata| metadata_mode(&metadata, true))
                .unwrap_or(0o755);
            return Ok(ViewNodeAttr {
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
            if kind == ViewNodeKind::Symlink {
                return Ok(ViewNodeAttr {
                    ino,
                    kind,
                    mode: 0o777,
                    size: metadata.len(),
                    modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                });
            }
            return Ok(ViewNodeAttr {
                ino,
                kind,
                mode: metadata_mode(&metadata, false),
                size: metadata.len(),
                modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            });
        }
        if let Some(layer_path) = self.layer_path(path)? {
            let metadata = if kind == ViewNodeKind::Symlink {
                fs::symlink_metadata(layer_path)
            } else {
                fs::metadata(layer_path)
            }
            .map_err(io_errno)?;
            return Ok(ViewNodeAttr {
                ino,
                kind,
                mode: if kind == ViewNodeKind::File {
                    // Published layers are immutable on disk, but the composed
                    // view is writable through copy-on-write. NFS clients apply
                    // mode checks before issuing WRITE/CREATE requests, so
                    // exposing the backing layer's read-only bit would prevent
                    // the request from ever reaching the COW adapter.
                    copy_up_mode(&metadata)
                } else {
                    metadata_mode(&metadata, kind == ViewNodeKind::Directory)
                },
                size: if kind != ViewNodeKind::Directory {
                    metadata.len()
                } else {
                    0
                },
                modified: SystemTime::UNIX_EPOCH,
            });
        }
        let entry = self.lower_file(path)?.ok_or(ENOENT)?;
        Ok(ViewNodeAttr {
            ino,
            kind,
            mode: if entry.executable {
                0o755
            } else {
                entry.mode & 0o777
            },
            size: entry.size_bytes,
            modified: SystemTime::UNIX_EPOCH,
        })
    }

    pub(crate) fn lookup(&mut self, parent: u64, name: &str) -> std::result::Result<u64, i32> {
        if name == "." {
            return Ok(parent);
        }
        if name == ".." {
            let path = self.path_for_ino(parent)?;
            let parent_path = Path::new(&path)
                .parent()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
            return Ok(self.ensure_ino(&parent_path));
        }
        let path = self.child_path(parent, name)?;
        self.attr(&path).map(|attr| attr.ino)
    }

    pub(crate) fn children(
        &mut self,
        dir_ino: u64,
    ) -> std::result::Result<Vec<(String, ViewNodeAttr)>, i32> {
        let path = self.path_for_ino(dir_ino)?;
        if self.node_kind(&path)? != Some(ViewNodeKind::Directory) {
            return Err(ENOTDIR);
        }
        let mut names = BTreeSet::new();
        for child in self.lower_children(&path)? {
            if !self.is_whiteouted(&child.path) {
                names.insert(child.name);
            }
        }
        let prefix = if path.is_empty() {
            String::new()
        } else {
            format!("{path}/")
        };
        for binding in &self.layers {
            if let Some(remainder) = binding.mount_path.strip_prefix(&prefix)
                && let Some(name) = remainder.split('/').next()
                && !name.is_empty()
            {
                names.insert(name.to_string());
            }
        }
        if let Some(layer_dir) = self.layer_path(&path)?
            && let Ok(dir) = fs::read_dir(layer_dir)
        {
            for entry in dir.flatten() {
                names.insert(entry.file_name().to_string_lossy().into_owned());
            }
        }
        for class in [
            ViewPathClass::Source,
            ViewPathClass::Generated,
            ViewPathClass::Scratch,
        ] {
            if let Ok(dir) = self
                .upper_path_in_class(class, &path)
                .and_then(|path| fs::read_dir(path).map_err(io_errno))
            {
                for entry in dir.flatten() {
                    names.insert(entry.file_name().to_string_lossy().into_owned());
                }
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

    pub(crate) fn read(
        &mut self,
        ino: u64,
        offset: u64,
        count: u32,
    ) -> std::result::Result<(Vec<u8>, bool), i32> {
        let path = self.path_for_ino(ino)?;
        if self.node_kind(&path)? != Some(ViewNodeKind::File) {
            return Err(EISDIR);
        }
        if let Some(metadata) = self.upper_metadata(&path) {
            if !metadata.is_file() {
                return Err(EINVAL);
            }
            let file = File::open(self.upper_path(&path)?).map_err(io_errno)?;
            let mut bytes = vec![0; count as usize];
            let read = read_file_at(&file, &mut bytes, offset).map_err(io_errno)?;
            bytes.truncate(read);
            Ok((bytes, offset.saturating_add(read as u64) >= metadata.len()))
        } else if let Some(layer_path) = self.layer_path(&path)? {
            let metadata = fs::metadata(&layer_path).map_err(io_errno)?;
            if !metadata.is_file() {
                return Err(EISDIR);
            }
            let file = File::open(layer_path).map_err(io_errno)?;
            let mut bytes = vec![0; count as usize];
            let read = read_file_at(&file, &mut bytes, offset).map_err(io_errno)?;
            bytes.truncate(read);
            Ok((bytes, offset.saturating_add(read as u64) >= metadata.len()))
        } else {
            let entry = self.lower_file(&path)?.ok_or(ENOENT)?;
            let projection = self.db.project_entry_file(&entry).map_err(|_| EIO)?;
            let file = File::open(projection).map_err(io_errno)?;
            let mut bytes = vec![0; count as usize];
            let read = read_file_at(&file, &mut bytes, offset).map_err(io_errno)?;
            bytes.truncate(read);
            Ok((
                bytes,
                offset.saturating_add(read as u64) >= entry.size_bytes,
            ))
        }
    }

    pub(crate) fn readlink(&self, ino: u64) -> std::result::Result<PathBuf, i32> {
        let path = self.path_for_ino(ino)?;
        if self.node_kind(&path)? != Some(ViewNodeKind::Symlink) {
            return Err(EINVAL);
        }
        if self.upper_metadata(&path).is_some() {
            let target =
                fs::read_link(self.upper_path_with_leaf_symlink(&path)?).map_err(io_errno)?;
            validate_view_symlink_target(&path, &target)?;
            return Ok(target);
        }
        let layer_path = self.layer_path(&path)?.ok_or(ENOENT)?;
        let target = fs::read_link(layer_path).map_err(io_errno)?;
        validate_view_symlink_target(&path, &target)?;
        Ok(target)
    }

    fn ensure_upper_parent(&self, path: &str) -> std::result::Result<(), i32> {
        let parent = parent_of(path);
        if !parent.is_empty() && self.node_kind(&parent)? != Some(ViewNodeKind::Directory) {
            return Err(ENOTDIR);
        }
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

    fn enforce_mutation_quota(
        &self,
        path: &str,
        proposed_size: Option<u64>,
        creates_file: bool,
    ) -> std::result::Result<(), i32> {
        let limits = &self.db.config().workspace_views;
        if limits.upper_logical_bytes == 0
            && limits.upper_file_count == 0
            && limits.single_file_bytes == 0
            && limits.journal_bytes == 0
        {
            return Ok(());
        }
        if limits.single_file_bytes > 0
            && proposed_size.is_some_and(|size| size > limits.single_file_bytes)
        {
            return Err(ENOSPC);
        }
        if limits.journal_bytes > 0 {
            let journal_size = fs::metadata(self.layout.journal_path())
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            // Journal records are deliberately compact. Reserving 1 KiB
            // makes the check conservative without serializing the record
            // twice before the mutation is known to succeed.
            if journal_size.saturating_add(1024) > limits.journal_bytes {
                return Err(ENOSPC);
            }
        }
        let usage = view_upper_usage(&self.layout).map_err(|_| EIO)?;
        let existing = self.upper_metadata(path);
        let previous_size = existing
            .as_ref()
            .filter(|metadata| metadata.is_file())
            .map(fs::Metadata::len)
            .unwrap_or(0);
        let adds_file = creates_file
            || (proposed_size.is_some()
                && !existing.as_ref().is_some_and(|metadata| metadata.is_file()));
        let projected_files = usage.file_count.saturating_add(u64::from(adds_file));
        let projected_bytes = usage
            .logical_bytes
            .saturating_sub(previous_size)
            .saturating_add(proposed_size.unwrap_or(previous_size));
        if (limits.upper_file_count > 0 && projected_files > limits.upper_file_count)
            || (limits.upper_logical_bytes > 0 && projected_bytes > limits.upper_logical_bytes)
        {
            return Err(ENOSPC);
        }
        Ok(())
    }

    fn begin_mutation(&mut self) -> std::result::Result<ViewMutationBarrier, i32> {
        let barrier = ViewMutationBarrier::shared(&self.layout.meta_dir).map_err(|_| EIO)?;
        self.journal
            .observe_checkpoint(
                barrier.checkpoint_sequence(),
                barrier.checkpoint_generation(),
            )
            .map_err(|_| EIO)?;
        self.generation_lease
            .advance(&self.layout.source_upper, self.journal.generation())
            .map_err(|_| EIO)?;
        if !self.journal.is_qualified() {
            return Err(EIO);
        }
        Ok(barrier)
    }

    fn begin_qualified_view_mutation(&mut self) -> std::result::Result<ViewMutationBarrier, i32> {
        self.begin_mutation()
    }

    #[allow(dead_code)]
    pub(crate) fn ensure_upper_file(
        &mut self,
        path: &str,
        truncate: bool,
    ) -> std::result::Result<File, i32> {
        // TRAIL_FS_PRODUCER: mounted_cow_copy_up CowPublication controlled
        let barrier = self.begin_qualified_view_mutation()?;
        let file = self.ensure_upper_file_under_barrier(path, truncate)?;
        barrier.validate().map_err(|_| EIO)?;
        Ok(file)
    }

    fn ensure_upper_file_under_barrier(
        &mut self,
        path: &str,
        truncate: bool,
    ) -> std::result::Result<File, i32> {
        if self.node_kind(path)? == Some(ViewNodeKind::Directory) {
            return Err(EISDIR);
        }
        let upper = self.upper_path(path)?;
        let created_upper = !upper.exists();
        let class = self.path_class(path);
        let whiteout_changes = if self.whiteouts.contains(path) {
            vec![ViewWhiteoutChange::Remove(path.to_string())]
        } else {
            Vec::new()
        };
        let has_whiteout_changes = !whiteout_changes.is_empty();
        let intent_sequence = self
            .journal
            .append_classified_with_whiteouts(
                ViewMutationKind::Write,
                path.to_string(),
                class,
                None,
                None,
                whiteout_changes,
            )
            .map_err(|_| EIO)?;
        self.ensure_upper_parent(path)?;
        if created_upper {
            let visible_size = if truncate {
                0
            } else if let Some(layer_path) = self.layer_path(path)? {
                fs::metadata(layer_path).map_err(io_errno)?.len()
            } else {
                self.lower_file(path)?
                    .map(|entry| entry.size_bytes)
                    .unwrap_or(0)
            };
            self.enforce_mutation_quota(path, Some(visible_size), true)?;
            if !truncate {
                if let Some(layer_path) = self.layer_path(path)? {
                    clone_or_copy_projected_file(&layer_path, &upper).map_err(|_| EIO)?;
                    let metadata = fs::metadata(&layer_path).map_err(io_errno)?;
                    set_file_mode(&upper, copy_up_mode(&metadata)).map_err(io_errno)?;
                } else {
                    let entry = self.lower_file(path)?;
                    if let Some(entry) = entry {
                        let projection = self.db.project_entry_file(&entry).map_err(|_| EIO)?;
                        clone_or_copy_projected_file(&projection, &upper).map_err(|_| EIO)?;
                        set_file_mode(
                            &upper,
                            if entry.executable {
                                0o755
                            } else {
                                entry.mode & 0o777
                            },
                        )
                        .map_err(io_errno)?;
                    } else {
                        File::create(&upper).map_err(io_errno)?;
                    }
                }
            } else {
                File::create(&upper).map_err(io_errno)?;
            }
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .truncate(truncate)
            .open(upper)
            .map_err(io_errno)?;
        if truncate {
            file.sync_data().map_err(io_errno)?;
        }
        if created_upper || has_whiteout_changes {
            self.sync_namespace_publication(path)?;
        }
        if has_whiteout_changes {
            self.journal
                .commit_whiteouts(intent_sequence)
                .map_err(|_| EIO)?;
        }
        self.whiteouts.remove(path);
        Ok(file)
    }

    pub(crate) fn write(
        &mut self,
        ino: u64,
        offset: u64,
        data: &[u8],
    ) -> std::result::Result<ViewNodeAttr, i32> {
        // TRAIL_FS_PRODUCER: mounted_cow_write CowPublication controlled
        let barrier = self.begin_qualified_view_mutation()?;
        let path = self.path_for_ino(ino)?;
        let visible_size = self.attr(&path)?.size;
        let proposed_size = visible_size.max(offset.saturating_add(data.len() as u64));
        self.enforce_mutation_quota(&path, Some(proposed_size), false)?;
        let file = self.ensure_upper_file_under_barrier(&path, false)?;
        write_all_file_at(&file, data, offset).map_err(io_errno)?;
        file.sync_data().map_err(io_errno)?;
        test_crash_point("checkpoint_after_source_sync");
        barrier.validate().map_err(|_| EIO)?;
        self.attr(&path)
    }

    pub(crate) fn create(
        &mut self,
        parent: u64,
        name: &str,
        mode: u32,
        exclusive: bool,
    ) -> std::result::Result<ViewNodeAttr, i32> {
        // TRAIL_FS_PRODUCER: mounted_cow_create CowPublication controlled
        let barrier = self.begin_qualified_view_mutation()?;
        let path = self.child_path(parent, name)?;
        if exclusive && self.node_kind(&path)?.is_some() {
            return Err(EEXIST);
        }
        self.enforce_mutation_quota(&path, Some(0), self.upper_metadata(&path).is_none())?;
        let class = self.path_class(&path);
        let intent_sequence = self
            .journal
            .append_classified_with_whiteouts(
                ViewMutationKind::Create,
                path.clone(),
                class,
                None,
                None,
                vec![ViewWhiteoutChange::Remove(path.clone())],
            )
            .map_err(|_| EIO)?;
        self.ensure_upper_parent(&path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(!exclusive)
            .create_new(exclusive)
            .open(self.upper_path(&path)?)
            .map_err(io_errno)?;
        set_file_mode(&self.upper_path(&path)?, mode & 0o777).map_err(io_errno)?;
        file.sync_data().map_err(io_errno)?;
        self.sync_namespace_publication(&path)?;
        self.journal
            .commit_whiteouts(intent_sequence)
            .map_err(|_| EIO)?;
        self.whiteouts.remove(&path);
        self.touch_parent(&path);
        barrier.validate().map_err(|_| EIO)?;
        self.attr(&path)
    }

    pub(crate) fn mkdir(
        &mut self,
        parent: u64,
        name: &str,
        mode: u32,
    ) -> std::result::Result<ViewNodeAttr, i32> {
        // TRAIL_FS_PRODUCER: mounted_cow_mkdir CowPublication controlled
        let barrier = self.begin_qualified_view_mutation()?;
        let path = self.child_path(parent, name)?;
        if self.node_kind(&path)?.is_some() {
            return Err(EEXIST);
        }
        self.enforce_mutation_quota(&path, None, false)?;
        let class = self.path_class(&path);
        let intent_sequence = self
            .journal
            .append_classified_with_whiteouts(
                ViewMutationKind::Mkdir,
                path.clone(),
                class,
                None,
                None,
                vec![ViewWhiteoutChange::Remove(path.clone())],
            )
            .map_err(|_| EIO)?;
        fs::create_dir_all(self.upper_path(&path)?).map_err(io_errno)?;
        set_file_mode(&self.upper_path(&path)?, mode & 0o777).map_err(io_errno)?;
        self.sync_namespace_publication(&path)?;
        self.journal
            .commit_whiteouts(intent_sequence)
            .map_err(|_| EIO)?;
        self.whiteouts.remove(&path);
        self.touch_parent(&path);
        self.touch_dir(path.clone());
        barrier.validate().map_err(|_| EIO)?;
        self.attr(&path)
    }

    #[cfg(unix)]
    pub(crate) fn symlink(
        &mut self,
        parent: u64,
        name: &str,
        target: &Path,
    ) -> std::result::Result<ViewNodeAttr, i32> {
        // TRAIL_FS_PRODUCER: mounted_cow_symlink CowPublication controlled
        let barrier = self.begin_qualified_view_mutation()?;
        let path = self.child_path(parent, name)?;
        if self.node_kind(&path)?.is_some() {
            return Err(EEXIST);
        }
        validate_view_symlink_target(&path, target)?;
        self.enforce_mutation_quota(&path, Some(target.to_string_lossy().len() as u64), true)?;
        let class = self.path_class(&path);
        let intent_sequence = self
            .journal
            .append_classified_with_whiteouts(
                ViewMutationKind::Create,
                path.clone(),
                class,
                None,
                None,
                vec![ViewWhiteoutChange::Remove(path.clone())],
            )
            .map_err(|_| EIO)?;
        self.ensure_upper_parent(&path)?;
        std::os::unix::fs::symlink(target, self.upper_path(&path)?).map_err(io_errno)?;
        self.sync_namespace_publication(&path)?;
        self.journal
            .commit_whiteouts(intent_sequence)
            .map_err(|_| EIO)?;
        self.whiteouts.remove(&path);
        self.touch_parent(&path);
        barrier.validate().map_err(|_| EIO)?;
        self.attr(&path)
    }

    pub(crate) fn setattr(
        &mut self,
        ino: u64,
        size: Option<u64>,
        mode: Option<u32>,
    ) -> std::result::Result<ViewNodeAttr, i32> {
        let path = self.path_for_ino(ino)?;
        if size.is_none() && mode.is_none() {
            return self.attr(&path);
        }
        // TRAIL_FS_PRODUCER: mounted_cow_setattr CowPublication controlled
        let barrier = self.begin_qualified_view_mutation()?;
        let class = self.path_class(&path);
        self.journal
            .append_classified(ViewMutationKind::Metadata, path.clone(), class, None, None)
            .map_err(|_| EIO)?;
        if let Some(size) = size {
            self.enforce_mutation_quota(&path, Some(size), false)?;
            let file = self.ensure_upper_file_under_barrier(&path, false)?;
            file.set_len(size).map_err(io_errno)?;
            file.sync_data().map_err(io_errno)?;
        }
        if let Some(mode) = mode {
            self.enforce_mutation_quota(&path, None, false)?;
            if self.node_kind(&path)? == Some(ViewNodeKind::File) {
                let file = self.ensure_upper_file_under_barrier(&path, false)?;
                set_file_mode(&self.upper_path(&path)?, mode & 0o777).map_err(io_errno)?;
                file.sync_all().map_err(io_errno)?;
            } else {
                set_file_mode(&self.upper_path(&path)?, mode & 0o777).map_err(io_errno)?;
            }
        }
        barrier.validate().map_err(|_| EIO)?;
        self.attr(&path)
    }

    pub(crate) fn remove(&mut self, parent: u64, name: &str) -> std::result::Result<(), i32> {
        // TRAIL_FS_PRODUCER: mounted_cow_remove CowPublication controlled
        let barrier = self.begin_qualified_view_mutation()?;
        let path = self.child_path(parent, name)?;
        self.enforce_mutation_quota(&path, None, false)?;
        let kind = self.node_kind(&path)?.ok_or(ENOENT)?;
        let ino = self.ensure_ino(&path);
        if kind == ViewNodeKind::Directory && !self.children(ino)?.is_empty() {
            return Err(ENOTEMPTY);
        }
        let hides_lower = self.layer_path(&path)?.is_some()
            || self.layer_directory_exists(&path)?
            || self.lower_file(&path)?.is_some()
            || self.lower_directory_exists(&path)?;
        let whiteout_changes = if hides_lower {
            vec![ViewWhiteoutChange::Insert(path.clone())]
        } else {
            Vec::new()
        };
        let class = self.path_class(&path);
        let intent_sequence = self
            .journal
            .append_classified_with_whiteouts(
                ViewMutationKind::Delete,
                path.clone(),
                class,
                None,
                None,
                whiteout_changes,
            )
            .map_err(|_| EIO)?;
        if let Some(metadata) = self.upper_metadata(&path) {
            if metadata.is_dir() {
                fs::remove_dir(self.upper_path(&path)?).map_err(io_errno)?;
            } else {
                fs::remove_file(self.upper_path_with_leaf_symlink(&path)?).map_err(io_errno)?;
            }
        }
        self.sync_namespace_publication(&path)?;
        if hides_lower {
            self.journal
                .commit_whiteouts(intent_sequence)
                .map_err(|_| EIO)?;
        }
        if hides_lower {
            self.whiteouts.insert(path.clone());
        }
        self.touch_parent(&path);
        barrier.validate().map_err(|_| EIO)?;
        Ok(())
    }

    /// Prepare an adapter-owned mount replacement without irreversibly
    /// deleting private generated state. The returned transaction is paired
    /// with the SQLite layer-binding savepoint by the caller.
    pub(crate) fn prepare_declared_layer_mount_path(
        &mut self,
        path: &str,
        layer_kind: &str,
        replacement_layer_id: &str,
    ) -> Result<PreparedLayerMountReset> {
        let path = normalize_relative_path(path)?;
        if matches!(
            classify_view_path(&path),
            ViewPathClass::Internal | ViewPathClass::Secret
        ) {
            return Err(Error::InvalidInput(format!(
                "workspace layer replacement path `{path}` is protected internal or secret state"
            )));
        }
        let class = match layer_kind {
            "dependency" => ViewPathClass::Dependency,
            "compiler-results" | "generated" | "build" => ViewPathClass::Generated,
            other => {
                return Err(Error::InvalidInput(format!(
                    "workspace layer kind `{other}` cannot own a writable mount path"
                )));
            }
        };
        self.prepare_validated_layer_mount_path(&path, class, replacement_layer_id, false)
    }

    /// Prepare a writable-private output replacement. Its durable binding
    /// identity is component/output/key-derived rather than a workspace layer
    /// ID, but crash recovery is otherwise identical to layer replacement.
    pub(crate) fn prepare_declared_private_mount_path(
        &mut self,
        path: &str,
        layer_kind: &str,
        binding_identity: &str,
    ) -> Result<PreparedLayerMountReset> {
        self.prepare_declared_layer_mount_path(path, layer_kind, binding_identity)
    }

    /// Preserve a compatible writable-private output and make an empty first
    /// binding visible without manufacturing an immutable lower layer.
    pub(crate) fn ensure_declared_private_mount_path(
        &mut self,
        path: &str,
        layer_kind: &str,
    ) -> Result<()> {
        let path = normalize_relative_path(path)?;
        if matches!(
            classify_view_path(&path),
            ViewPathClass::Internal | ViewPathClass::Secret
        ) {
            return Err(Error::InvalidInput(format!(
                "writable-private path `{path}` is protected internal or secret state"
            )));
        }
        let class = match layer_kind {
            "dependency" => ViewPathClass::Dependency,
            "compiler-results" | "generated" | "build" => ViewPathClass::Generated,
            other => {
                return Err(Error::InvalidInput(format!(
                    "environment kind `{other}` cannot own a writable-private path"
                )));
            }
        };
        let upper = self
            .upper_path_in_class_with_leaf(class, &path, true)
            .map_err(|errno| Error::Io(std::io::Error::from_raw_os_error(errno)))?;
        let _barrier = self
            .begin_mutation()
            .map_err(|errno| Error::Io(std::io::Error::from_raw_os_error(errno)))?;
        let intent_sequence = self.journal.append_classified_with_whiteouts(
            ViewMutationKind::Mkdir,
            path.clone(),
            class,
            None,
            None,
            vec![ViewWhiteoutChange::RemoveTree(path.clone())],
        )?;
        match fs::symlink_metadata(&upper) {
            Ok(metadata) if metadata.is_dir() => Ok(()),
            Ok(_) => Err(Error::InvalidInput(format!(
                "writable-private mount `{path}` is not a directory"
            ))),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(&upper)?;
                Ok(())
            }
            Err(err) => Err(Error::Io(err)),
        }?;
        self.sync_namespace_publication(&path)
            .map_err(|errno| Error::Io(std::io::Error::from_raw_os_error(errno)))?;
        self.journal.commit_whiteouts(intent_sequence)?;
        let prefix = format!("{path}/");
        self.whiteouts
            .retain(|whiteout| whiteout != &path && !whiteout.starts_with(&prefix));
        self.touch_parent(&path);
        Ok(())
    }

    /// Prepare removal of an adapter-owned mount with the same durable
    /// rollback semantics as replacement. Recovery commits the reset only when
    /// SQLite no longer contains a binding at this path.
    pub(crate) fn prepare_declared_layer_unmount_path(
        &mut self,
        path: &str,
        layer_kind: &str,
    ) -> Result<PreparedLayerMountReset> {
        let path = normalize_relative_path(path)?;
        if matches!(
            classify_view_path(&path),
            ViewPathClass::Internal | ViewPathClass::Secret
        ) {
            return Err(Error::InvalidInput(format!(
                "workspace layer removal path `{path}` is protected internal or secret state"
            )));
        }
        let class = match layer_kind {
            "dependency" => ViewPathClass::Dependency,
            "compiler-results" | "generated" | "build" => ViewPathClass::Generated,
            other => {
                return Err(Error::InvalidInput(format!(
                    "workspace layer kind `{other}` cannot own a writable mount path"
                )));
            }
        };
        self.prepare_validated_layer_mount_path(&path, class, "binding-removed", true)
    }

    #[cfg(test)]
    pub(crate) fn prepare_layer_mount_path(
        &mut self,
        path: &str,
        replacement_layer_id: &str,
    ) -> Result<PreparedLayerMountReset> {
        let path = normalize_relative_path(path)?;
        let class = classify_view_path(&path);
        if !matches!(class, ViewPathClass::Dependency | ViewPathClass::Generated) {
            return Err(Error::InvalidInput(format!(
                "workspace layer replacement path `{path}` is not dependency or generated state"
            )));
        }
        self.prepare_validated_layer_mount_path(&path, class, replacement_layer_id, false)
    }

    fn prepare_validated_layer_mount_path(
        &mut self,
        path: &str,
        class: ViewPathClass,
        replacement_layer_id: &str,
        binding_removed: bool,
    ) -> Result<PreparedLayerMountReset> {
        if replacement_layer_id.trim().is_empty() {
            return Err(Error::InvalidInput(
                "replacement workspace layer id cannot be empty".to_string(),
            ));
        }
        let source_path = self
            .upper_path_in_class_with_leaf(ViewPathClass::Source, path, true)
            .map_err(|errno| Error::Io(std::io::Error::from_raw_os_error(errno)))?;
        if fs::symlink_metadata(&source_path).is_ok() {
            return Err(Error::InvalidInput(format!(
                "adapter mount `{path}` overlaps pre-existing source-upper state; preserve or remove that source state before synchronizing the environment"
            )));
        }
        let _barrier = self
            .begin_mutation()
            .map_err(|errno| Error::Io(std::io::Error::from_raw_os_error(errno)))?;
        let upper = self
            .upper_path_in_class_with_leaf(class, path, true)
            .map_err(|errno| Error::Io(std::io::Error::from_raw_os_error(errno)))?;
        let prefix = format!("{path}/");
        let removed_whiteouts = self
            .whiteouts
            .iter()
            .filter(|whiteout| *whiteout == path || whiteout.starts_with(&prefix))
            .cloned()
            .collect::<Vec<_>>();
        let (intent_path, backup_path) = create_layer_mount_reset_paths(&self.layout, path)?;
        let intent = LayerMountResetIntent {
            version: LAYER_MOUNT_RESET_INTENT_VERSION,
            mount_path: path.to_string(),
            upper_class: class,
            replacement_layer_id: replacement_layer_id.to_string(),
            binding_removed,
            removed_whiteouts: removed_whiteouts.clone(),
        };
        write_file_atomic(&intent_path, &serde_json::to_vec_pretty(&intent)?, true)?;
        let intent_sequence = self.journal.append_classified_with_whiteouts(
            ViewMutationKind::Metadata,
            path.to_string(),
            class,
            None,
            None,
            vec![ViewWhiteoutChange::RemoveTree(path.to_string())],
        )?;

        let reset = (|| -> Result<()> {
            match fs::symlink_metadata(&upper) {
                Ok(_) => {
                    fs::rename(&upper, &backup_path)?;
                    if let Some(parent) = upper.parent() {
                        sync_directory(parent);
                    }
                    if let Some(parent) = backup_path.parent() {
                        sync_directory(parent);
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(Error::Io(err)),
            }
            self.whiteouts
                .retain(|whiteout| whiteout != path && !whiteout.starts_with(&prefix));
            Ok(())
        })();
        let prepared = PreparedLayerMountReset {
            layout: self.layout.clone(),
            mount_path: path.to_string(),
            upper_class: class,
            intent_path,
            backup_path,
            removed_whiteouts,
        };
        if let Err(err) = reset {
            let _ = prepared.rollback(self);
            return Err(err);
        }
        self.sync_namespace_publication(path)
            .map_err(|errno| Error::Io(std::io::Error::from_raw_os_error(errno)))?;
        self.journal.commit_whiteouts(intent_sequence)?;
        self.touch_parent(path);
        Ok(prepared)
    }

    pub(crate) fn rename(
        &mut self,
        from_parent: u64,
        from_name: &str,
        to_parent: u64,
        to_name: &str,
    ) -> std::result::Result<(), i32> {
        // TRAIL_FS_PRODUCER: mounted_cow_rename CowPublication controlled
        let barrier = self.begin_qualified_view_mutation()?;
        let old = self.child_path(from_parent, from_name)?;
        let new = self.child_path(to_parent, to_name)?;
        if old == new {
            return Ok(());
        }
        self.enforce_mutation_quota(&old, None, false)?;
        let kind = self.node_kind(&old)?.ok_or(ENOENT)?;
        if kind == ViewNodeKind::Directory && new.starts_with(&format!("{old}/")) {
            return Err(EINVAL);
        }
        if let Some(target_kind) = self.node_kind(&new)? {
            if kind == ViewNodeKind::File && target_kind == ViewNodeKind::Directory {
                return Err(EISDIR);
            }
            if kind == ViewNodeKind::Directory && target_kind == ViewNodeKind::File {
                return Err(ENOTDIR);
            }
            if target_kind == ViewNodeKind::Directory {
                let target_ino = self.ensure_ino(&new);
                if !self.children(target_ino)?.is_empty() {
                    return Err(ENOTEMPTY);
                }
            }
        }
        let old_class = self.path_class(&old);
        let new_class = self.path_class(&new);
        let intent_sequence = self
            .journal
            .append_classified_with_whiteouts(
                ViewMutationKind::Rename,
                old.clone(),
                old_class,
                Some(new.clone()),
                Some(new_class),
                vec![
                    ViewWhiteoutChange::Insert(old.clone()),
                    ViewWhiteoutChange::RemoveTree(new.clone()),
                ],
            )
            .map_err(|_| EIO)?;
        #[cfg(test)]
        fail_rename_after_intent_if_requested()?;
        self.ensure_upper_parent(&new)?;
        if self.upper_metadata(&new).is_some() {
            let metadata = self.upper_metadata(&new).unwrap();
            if metadata.is_dir() {
                fs::remove_dir(self.upper_path(&new)?).map_err(io_errno)?;
            } else {
                fs::remove_file(self.upper_path_with_leaf_symlink(&new)?).map_err(io_errno)?;
            }
        }
        if kind == ViewNodeKind::Directory {
            self.merge_lower_subtree_into_upper(&old)?;
            self.move_upper_subtree(&old, &new)?;
        } else if self.upper_metadata(&old).is_some() {
            fs::rename(
                self.upper_path_with_leaf_symlink(&old)?,
                self.upper_path(&new)?,
            )
            .map_err(io_errno)?;
        } else if kind == ViewNodeKind::File {
            self.copy_lower_file(&old, &new)?;
        } else if kind == ViewNodeKind::Symlink {
            let old_ino = self.ensure_ino(&old);
            let target = self.readlink(old_ino)?;
            validate_view_symlink_target(&new, &target)?;
            create_view_symlink(&target, &self.upper_path(&new)?).map_err(io_errno)?;
        }
        fail_rename_before_durability_fence_if_requested()?;
        self.sync_rename_publication(&old, &new)?;
        self.journal
            .commit_whiteouts(intent_sequence)
            .map_err(|_| EIO)?;
        self.whiteouts.insert(old.clone());
        let new_prefix = format!("{new}/");
        self.whiteouts
            .retain(|path| path != &new && !path.starts_with(&new_prefix));
        self.touch_parent(&old);
        self.touch_parent(&new);
        let replaced_inodes = self
            .ino_by_path
            .keys()
            .filter(|path| *path == &new || path.starts_with(&new_prefix))
            .cloned()
            .collect::<Vec<_>>();
        for path in replaced_inodes {
            if let Some(ino) = self.ino_by_path.remove(&path) {
                self.path_by_ino.remove(&ino);
            }
        }
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
        barrier.validate().map_err(|_| EIO)?;
        Ok(())
    }

    /// Returns the checkpoint candidate set without walking the composed view.
    /// The journal is the fast path; upper and whiteout scans make recovery
    /// correct after an interrupted append.
    pub(crate) fn checkpoint_candidates(&self) -> Result<ViewCheckpointCandidates> {
        match &self.lower {
            #[cfg(test)]
            ViewLower::Eager(files) => {
                recover_view_checkpoint_candidates(&self.layout.source_upper, files)
            }
            ViewLower::Root(root_id) => recover_view_checkpoint_candidates_for_root(
                &self.db,
                &self.layout.source_upper,
                root_id,
            ),
        }
    }

    fn copy_lower_file(&self, source: &str, target: &str) -> std::result::Result<(), i32> {
        if let Some(layer_path) = self.layer_path(source)? {
            self.ensure_upper_parent(target)?;
            let target_path = self.upper_path(target)?;
            clone_or_copy_projected_file(&layer_path, &target_path).map_err(|_| EIO)?;
            let metadata = fs::metadata(layer_path).map_err(io_errno)?;
            set_file_mode(&target_path, copy_up_mode(&metadata)).map_err(io_errno)?;
            self.sync_copied_file(&target_path)?;
            return Ok(());
        }
        let entry = self.lower_file(source)?.ok_or(ENOENT)?;
        self.ensure_upper_parent(target)?;
        let projection = self.db.project_entry_file(&entry).map_err(|_| EIO)?;
        let target_path = self.upper_path(target)?;
        clone_or_copy_projected_file(&projection, &target_path).map_err(|_| EIO)?;
        set_file_mode(
            &target_path,
            if entry.executable {
                0o755
            } else {
                entry.mode & 0o777
            },
        )
        .map_err(io_errno)?;
        self.sync_copied_file(&target_path)
    }

    fn sync_copied_file(&self, path: &Path) -> std::result::Result<(), i32> {
        OpenOptions::new()
            .read(true)
            .open(path)
            .and_then(|file| file.sync_all())
            .map_err(io_errno)?;
        if let Some(parent) = path.parent() {
            sync_directory_strict(parent).map_err(|_| EIO)?;
        }
        Ok(())
    }

    fn sync_namespace_publication(&self, path: &str) -> std::result::Result<(), i32> {
        fail_namespace_before_durability_fence_if_requested()?;
        let upper = self.upper_path_with_leaf_symlink(path)?;
        match fs::symlink_metadata(&upper) {
            Ok(metadata) if metadata.is_file() => {
                OpenOptions::new()
                    .read(true)
                    .open(&upper)
                    .and_then(|file| file.sync_all())
                    .map_err(io_errno)?;
            }
            Ok(metadata) if metadata.is_dir() => {
                sync_directory_strict(&upper).map_err(|_| EIO)?;
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(io_errno(err)),
        }

        let root = self.layout.upper_for_class(self.path_class(path));
        let mut cursor = upper.parent();
        while let Some(directory) = cursor {
            if directory.is_dir() {
                sync_directory_strict(directory).map_err(|_| EIO)?;
            }
            if directory == root {
                break;
            }
            cursor = directory.parent();
        }
        Ok(())
    }

    fn sync_rename_publication(&self, old: &str, new: &str) -> std::result::Result<(), i32> {
        let mut directories = BTreeSet::new();
        for root in [
            &self.layout.source_upper,
            &self.layout.generated_upper,
            &self.layout.scratch_upper,
        ] {
            let source = safe_join(root, old).map_err(|_| EINVAL)?;
            if let Some(parent) = source.parent() {
                insert_directory_ancestry(&mut directories, parent, root);
            }
            let destination = safe_join(root, new).map_err(|_| EINVAL)?;
            if let Some(parent) = destination.parent() {
                insert_directory_ancestry(&mut directories, parent, root);
            }
            let metadata = match fs::symlink_metadata(&destination) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => return Err(io_errno(err)),
            };
            if metadata.is_dir() {
                for entry in walkdir::WalkDir::new(&destination).follow_links(false) {
                    let entry = entry.map_err(|_| EIO)?;
                    if entry.file_type().is_dir() {
                        directories.insert(entry.path().to_path_buf());
                    }
                }
            } else if metadata.is_file() {
                OpenOptions::new()
                    .read(true)
                    .open(&destination)
                    .and_then(|file| file.sync_all())
                    .map_err(io_errno)?;
            }
        }
        for directory in directories {
            if directory.is_dir() {
                sync_directory_strict(&directory).map_err(|_| EIO)?;
            }
        }
        Ok(())
    }

    fn merge_lower_subtree_into_upper(&self, root: &str) -> std::result::Result<(), i32> {
        fs::create_dir_all(self.upper_path(root)?).map_err(io_errno)?;
        if let Some(layer_root) = self.layer_path(root)? {
            for entry in walkdir::WalkDir::new(&layer_root).follow_links(false) {
                let entry = entry.map_err(|_| EIO)?;
                if entry.path() == layer_root {
                    continue;
                }
                let suffix = entry.path().strip_prefix(&layer_root).map_err(|_| EIO)?;
                let logical =
                    normalize_relative_path(&Path::new(root).join(suffix).to_string_lossy())
                        .map_err(|_| EINVAL)?;
                if entry.file_type().is_dir() {
                    fs::create_dir_all(self.upper_path(&logical)?).map_err(io_errno)?;
                } else if entry.file_type().is_file()
                    || (entry.file_type().is_symlink()
                        && fs::metadata(entry.path()).is_ok_and(|metadata| metadata.is_file()))
                {
                    self.copy_lower_file(&logical, &logical)?;
                }
            }
        }
        for path in self.lower_selection(root)?.into_keys() {
            if !self.is_whiteouted(&path) && self.upper_metadata(&path).is_none() {
                self.copy_lower_file(&path, &path)?;
            }
        }
        Ok(())
    }

    fn move_upper_subtree(&self, old: &str, new: &str) -> std::result::Result<(), i32> {
        let mut directories = BTreeSet::new();
        let mut leaves = Vec::new();
        for root in [
            &self.layout.source_upper,
            &self.layout.generated_upper,
            &self.layout.scratch_upper,
        ] {
            let physical_root = safe_join(root, old).map_err(|_| EINVAL)?;
            if !physical_root.exists() {
                continue;
            }
            for entry in walkdir::WalkDir::new(&physical_root).follow_links(false) {
                let entry = entry.map_err(|_| EIO)?;
                let suffix = entry.path().strip_prefix(&physical_root).map_err(|_| EIO)?;
                let source_logical = if suffix.as_os_str().is_empty() {
                    old.to_string()
                } else {
                    normalize_relative_path(&Path::new(old).join(suffix).to_string_lossy())
                        .map_err(|_| EINVAL)?
                };
                let destination_logical = source_logical
                    .strip_prefix(old)
                    .map(|suffix| format!("{new}{suffix}"))
                    .ok_or(EINVAL)?;
                if entry.file_type().is_dir() {
                    directories.insert(destination_logical);
                } else {
                    leaves.push((entry.path().to_path_buf(), destination_logical));
                }
            }
        }
        for logical in &directories {
            fs::create_dir_all(self.upper_path(logical)?).map_err(io_errno)?;
        }
        for (source, logical) in leaves {
            let destination = self.upper_path(&logical)?;
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(io_errno)?;
            }
            if destination.exists() {
                let metadata = fs::symlink_metadata(&destination).map_err(io_errno)?;
                if metadata.is_dir() {
                    fs::remove_dir_all(&destination).map_err(io_errno)?;
                } else {
                    fs::remove_file(&destination).map_err(io_errno)?;
                }
            }
            fs::rename(source, destination).map_err(io_errno)?;
        }
        for root in [
            &self.layout.source_upper,
            &self.layout.generated_upper,
            &self.layout.scratch_upper,
        ] {
            let physical_root = safe_join(root, old).map_err(|_| EINVAL)?;
            if physical_root.exists() {
                fs::remove_dir_all(physical_root).map_err(io_errno)?;
            }
        }
        Ok(())
    }
}

fn insert_directory_ancestry(directories: &mut BTreeSet<PathBuf>, start: &Path, root: &Path) {
    let mut cursor = Some(start);
    while let Some(directory) = cursor {
        if !directory.starts_with(root) {
            break;
        }
        directories.insert(directory.to_path_buf());
        if directory == root {
            break;
        }
        cursor = directory.parent();
    }
}

#[cfg(all(debug_assertions, unix))]
pub(crate) fn run_changed_path_view_flow() -> std::result::Result<(), String> {
    let temp = tempfile::tempdir().map_err(|err| err.to_string())?;
    fs::write(temp.path().join("README.md"), b"baseline\n").map_err(|err| err.to_string())?;
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false)
        .map_err(|err| err.to_string())?;
    let db = Trail::open(temp.path()).map_err(|err| err.to_string())?;
    let root = db
        .resolve_branch_ref("main")
        .map_err(|err| err.to_string())?
        .root_id;

    let upper = temp.path().join("view/source-upper");
    let mut view = ViewCore::new_lazy(
        Trail::open(temp.path()).map_err(|err| err.to_string())?,
        upper.clone(),
        root.clone(),
    )
    .map_err(|err| err.to_string())?;
    let readme = view
        .lookup(VIEW_ROOT_INO, "README.md")
        .map_err(|code| format!("lookup failed with errno {code}"))?;
    fail_next_view_journal_sync_for_current_thread();
    if view.write(readme, 0, b"new").is_ok() {
        return Err("injected journal sync failure unexpectedly exposed a write".to_string());
    }
    if upper.join("README.md").exists() {
        return Err("copy-up became visible before its intent was durable".to_string());
    }
    if fs::read(temp.path().join("README.md")).map_err(|err| err.to_string())? != b"baseline\n" {
        return Err("journal sync failure changed the lower file".to_string());
    }

    view.write(readme, 0, b"changed")
        .map_err(|code| format!("write failed with errno {code}"))?;
    let qualified = view
        .checkpoint_candidates()
        .map_err(|err| err.to_string())?;
    if !qualified.qualified || qualified.upper_recovery_walks != 0 {
        return Err("qualified view did not retain the zero-walk fast path".to_string());
    }
    OpenOptions::new()
        .append(true)
        .open(ViewUpperLayout::from_source_upper(upper.clone()).journal_path())
        .and_then(|mut file| file.write_all(b"corrupt-complete-record\n"))
        .map_err(|err| err.to_string())?;
    let recovered = view
        .checkpoint_candidates()
        .map_err(|err| err.to_string())?;
    if recovered.qualified
        || recovered.upper_recovery_walks == 0
        || !recovered.paths.contains("README.md")
    {
        return Err("untrusted view did not reconcile its upper tree".to_string());
    }

    let whiteout_upper = temp.path().join("whiteout-view/source-upper");
    let mut whiteout_view = ViewCore::new_lazy(
        Trail::open(temp.path()).map_err(|err| err.to_string())?,
        whiteout_upper.clone(),
        root.clone(),
    )
    .map_err(|err| err.to_string())?;
    whiteout_view
        .remove(VIEW_ROOT_INO, "README.md")
        .map_err(|code| format!("remove failed with errno {code}"))?;
    drop(whiteout_view);
    let reopened = ViewCore::new_lazy(
        Trail::open(temp.path()).map_err(|err| err.to_string())?,
        whiteout_upper,
        root,
    )
    .map_err(|err| err.to_string())?;
    if !reopened.is_whiteouted("README.md") {
        return Err("incremental whiteout was not replayed".to_string());
    }
    let whiteout_candidates = reopened
        .checkpoint_candidates()
        .map_err(|err| err.to_string())?;
    if !whiteout_candidates.qualified
        || whiteout_candidates.upper_recovery_walks != 0
        || !whiteout_candidates.paths.contains("README.md")
    {
        return Err("qualified incremental whiteout lost checkpoint authority".to_string());
    }
    Ok(())
}

impl PreparedLayerMountReset {
    pub(crate) fn install_private_directory(&self, source: &Path) -> Result<()> {
        if !source.is_dir() {
            return Err(Error::InvalidInput(format!(
                "writable-private seed `{}` is not a directory",
                source.display()
            )));
        }
        let upper = safe_join(
            self.layout.upper_for_class(self.upper_class),
            &self.mount_path,
        )?;
        if fs::symlink_metadata(&upper).is_ok() {
            return Err(Error::Corrupt(format!(
                "writable-private destination `{}` was recreated during activation",
                upper.display()
            )));
        }
        copy_dir_recursive(source, &upper)?;
        if let Some(parent) = upper.parent() {
            sync_directory(parent);
        }
        Ok(())
    }

    /// SQLite already committed the replacement. Cleanup is best-effort; a
    /// retained intent is safe because the next view open observes the new
    /// binding and completes cleanup idempotently.
    pub(crate) fn commit(self) {
        let _ = remove_layer_mount_reset_path(&self.backup_path);
        if !self.backup_path.exists() {
            let _ = fs::remove_file(&self.intent_path);
            if let Some(parent) = self.intent_path.parent() {
                sync_directory(parent);
            }
        }
    }

    pub(crate) fn rollback(self, core: &mut ViewCore) -> Result<()> {
        let whiteout_sequence = if !self.removed_whiteouts.is_empty() {
            let class = core.path_class(&self.mount_path);
            let sequence = core.journal.append_classified_with_whiteouts(
                ViewMutationKind::Metadata,
                self.mount_path.clone(),
                class,
                None,
                None,
                self.removed_whiteouts
                    .iter()
                    .cloned()
                    .map(ViewWhiteoutChange::Insert)
                    .collect(),
            )?;
            Some(sequence)
        } else {
            None
        };
        let upper = safe_join(
            self.layout.upper_for_class(self.upper_class),
            &self.mount_path,
        )?;
        remove_layer_mount_reset_path(&upper)?;
        if self.backup_path.exists() {
            if let Some(parent) = upper.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&self.backup_path, &upper)?;
            if let Some(parent) = upper.parent() {
                sync_directory(parent);
            }
        }
        if let Some(sequence) = whiteout_sequence {
            core.journal.commit_whiteouts(sequence)?;
        }
        core.whiteouts.extend(self.removed_whiteouts);
        remove_layer_mount_reset_path(&self.intent_path)?;
        if let Some(parent) = self.intent_path.parent() {
            sync_directory(parent);
        }
        Ok(())
    }
}

fn layer_mount_reset_intents_dir(layout: &ViewUpperLayout) -> PathBuf {
    layout.meta_dir.join("layer-reset-intents")
}

fn layer_mount_reset_backups_dir(layout: &ViewUpperLayout) -> PathBuf {
    layout.meta_dir.join("layer-reset-backups")
}

fn create_layer_mount_reset_paths(
    layout: &ViewUpperLayout,
    mount_path: &str,
) -> Result<(PathBuf, PathBuf)> {
    let intents = layer_mount_reset_intents_dir(layout);
    let backups = layer_mount_reset_backups_dir(layout);
    fs::create_dir_all(&intents)?;
    fs::create_dir_all(&backups)?;
    for attempt in 0..32_u8 {
        let id = sha256_hex(
            format!(
                "{}:{}:{}:{}",
                std::process::id(),
                now_nanos(),
                mount_path,
                attempt
            )
            .as_bytes(),
        );
        let intent = intents.join(format!("{id}.json"));
        let backup = backups.join(&id);
        if !intent.exists() && !backup.exists() {
            return Ok((intent, backup));
        }
    }
    Err(Error::InvalidInput(
        "could not allocate a unique workspace-layer reset intent".to_string(),
    ))
}

fn recover_layer_mount_resets(
    layout: &ViewUpperLayout,
    layers: &[WorkspaceLayerBinding],
) -> Result<Vec<RecoveredLayerMountReset>> {
    let intents_dir = layer_mount_reset_intents_dir(layout);
    let entries = match fs::read_dir(&intents_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(Error::Io(err)),
    };
    let mut intent_paths = entries
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    intent_paths.sort();

    let mut recovered = Vec::new();
    for intent_path in intent_paths {
        let id = intent_path
            .file_stem()
            .and_then(|value| value.to_str())
            .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or_else(|| {
                Error::Corrupt(format!(
                    "invalid workspace-layer reset intent name `{}`",
                    intent_path.display()
                ))
            })?;
        let intent: LayerMountResetIntent = serde_json::from_slice(&fs::read(&intent_path)?)?;
        if intent.version != 1 && intent.version != LAYER_MOUNT_RESET_INTENT_VERSION {
            return Err(Error::Corrupt(format!(
                "unsupported workspace-layer reset intent version {}",
                intent.version
            )));
        }
        let mount_path = normalize_relative_path(&intent.mount_path)?;
        if !matches!(
            intent.upper_class,
            ViewPathClass::Dependency | ViewPathClass::Generated
        ) {
            return Err(Error::Corrupt(format!(
                "workspace-layer reset for `{mount_path}` targets protected upper class `{}`",
                intent.upper_class.as_str()
            )));
        }
        let backup_path = layer_mount_reset_backups_dir(layout).join(id);
        let binding_committed = if intent.binding_removed {
            layers
                .iter()
                .all(|binding| binding.mount_path != mount_path)
        } else {
            layers.iter().any(|binding| {
                binding.mount_path == mount_path
                    && binding.binding_identity == intent.replacement_layer_id
            })
        };
        if binding_committed {
            remove_layer_mount_reset_path(&backup_path)?;
            remove_layer_mount_reset_path(&intent_path)?;
            sync_directory(&intents_dir);
            continue;
        }

        let upper = safe_join(layout.upper_for_class(intent.upper_class), &mount_path)?;
        remove_layer_mount_reset_path(&upper)?;
        if backup_path.exists() {
            if let Some(parent) = upper.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&backup_path, &upper)?;
            if let Some(parent) = upper.parent() {
                sync_directory(parent);
            }
        }
        recovered.push(RecoveredLayerMountReset {
            mount_path,
            intent_path,
            backup_path,
            removed_whiteouts: intent.removed_whiteouts,
        });
    }
    Ok(recovered)
}

fn finish_layer_mount_reset_recovery(recovered: &RecoveredLayerMountReset) -> Result<()> {
    remove_layer_mount_reset_path(&recovered.backup_path)?;
    remove_layer_mount_reset_path(&recovered.intent_path)?;
    if let Some(parent) = recovered.intent_path.parent() {
        sync_directory(parent);
    }
    Ok(())
}

fn remove_layer_mount_reset_path(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            fs::remove_dir_all(path)?;
        }
        Ok(_) => fs::remove_file(path)?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(Error::Io(err)),
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default)]
struct ViewUpperUsage {
    logical_bytes: u64,
    file_count: u64,
}

fn view_upper_usage(layout: &ViewUpperLayout) -> Result<ViewUpperUsage> {
    let mut usage = ViewUpperUsage::default();
    for root in [
        &layout.source_upper,
        &layout.generated_upper,
        &layout.scratch_upper,
    ] {
        if !root.exists() {
            continue;
        }
        for entry in walkdir::WalkDir::new(root).follow_links(false) {
            let entry = entry.map_err(|err| Error::InvalidInput(err.to_string()))?;
            if entry.file_type().is_file() || entry.file_type().is_symlink() {
                let metadata = entry.metadata().map_err(|err| Error::Io(err.into()))?;
                usage.logical_bytes = usage.logical_bytes.saturating_add(metadata.len());
                usage.file_count = usage.file_count.saturating_add(1);
            }
        }
    }
    Ok(usage)
}

fn parent_of(path: &str) -> String {
    Path::new(path)
        .parent()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn validate_view_symlink_target(link_path: &str, target: &Path) -> std::result::Result<(), i32> {
    use std::path::Component;

    if target.is_absolute() {
        return Err(EPERM);
    }
    let mut resolved = parent_of(link_path)
        .split('/')
        .filter(|component| !component.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    for component in target.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(name) => resolved.push(name.to_str().ok_or(EINVAL)?.to_string()),
            Component::ParentDir => {
                if resolved.pop().is_none() {
                    return Err(EPERM);
                }
            }
            Component::RootDir | Component::Prefix(_) => return Err(EPERM),
        }
    }
    if classify_view_path(&resolved.join("/")) == ViewPathClass::Internal {
        return Err(EPERM);
    }
    Ok(())
}

#[cfg(unix)]
fn create_view_symlink(target: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, destination)
}

#[cfg(windows)]
fn create_view_symlink(_target: &Path, _destination: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "writable view symlinks are not supported on Windows",
    ))
}

#[cfg(unix)]
fn metadata_mode(metadata: &fs::Metadata, _directory: bool) -> u32 {
    metadata.mode() & 0o777
}

fn copy_up_mode(metadata: &fs::Metadata) -> u32 {
    if metadata_mode(metadata, false) & 0o111 != 0 {
        0o755
    } else {
        0o644
    }
}

#[cfg(windows)]
fn metadata_mode(metadata: &fs::Metadata, directory: bool) -> u32 {
    if directory {
        0o755
    } else if metadata.permissions().readonly() {
        0o444
    } else {
        0o644
    }
}

#[cfg(unix)]
fn set_file_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

#[cfg(windows)]
fn set_file_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_readonly(mode & 0o200 == 0);
    fs::set_permissions(path, permissions)
}

#[cfg(unix)]
fn read_file_at(file: &File, bytes: &mut [u8], offset: u64) -> std::io::Result<usize> {
    file.read_at(bytes, offset)
}

#[cfg(windows)]
fn read_file_at(file: &File, bytes: &mut [u8], offset: u64) -> std::io::Result<usize> {
    file.seek_read(bytes, offset)
}

#[cfg(unix)]
fn write_file_at(file: &File, bytes: &[u8], offset: u64) -> std::io::Result<usize> {
    file.write_at(bytes, offset)
}

#[cfg(windows)]
fn write_file_at(file: &File, bytes: &[u8], offset: u64) -> std::io::Result<usize> {
    file.seek_write(bytes, offset)
}

fn write_all_file_at(file: &File, mut bytes: &[u8], mut offset: u64) -> std::io::Result<()> {
    while !bytes.is_empty() {
        let written = write_file_at(file, bytes, offset)?;
        if written == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "failed to write the complete view buffer",
            ));
        }
        bytes = &bytes[written..];
        offset = offset.saturating_add(written as u64);
    }
    Ok(())
}

#[cfg(unix)]
fn io_errno(error: std::io::Error) -> i32 {
    error.raw_os_error().unwrap_or(EIO)
}

#[cfg(windows)]
fn io_errno(error: std::io::Error) -> i32 {
    match error.kind() {
        std::io::ErrorKind::NotFound => ENOENT,
        std::io::ErrorKind::AlreadyExists => EEXIST,
        std::io::ErrorKind::PermissionDenied => EPERM,
        std::io::ErrorKind::NotADirectory => ENOTDIR,
        std::io::ErrorKind::IsADirectory => EISDIR,
        std::io::ErrorKind::DirectoryNotEmpty => ENOTEMPTY,
        std::io::ErrorKind::InvalidInput | std::io::ErrorKind::InvalidData => EINVAL,
        _ => EIO,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Trail, ObjectId, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("src/nested")).unwrap();
        fs::write(temp.path().join("README.md"), "baseline\n").unwrap();
        fs::write(temp.path().join("src/lower.txt"), "lower\n").unwrap();
        fs::write(temp.path().join("src/nested/tool.sh"), "#!/bin/sh\n").unwrap();
        fs::set_permissions(
            temp.path().join("src/nested/tool.sh"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let root = db.resolve_branch_ref("main").unwrap().root_id;
        let upper = temp.path().join("upper");
        ViewUpperLayout::from_source_upper(upper.clone())
            .ensure()
            .unwrap();
        (temp, db, root, upper)
    }

    fn core(db: &Trail, root: &ObjectId, upper: PathBuf) -> ViewCore {
        ViewCore::new(
            Trail::open(db.workspace_root()).unwrap(),
            upper,
            db.load_root_files(root).unwrap(),
        )
        .unwrap()
    }

    fn lazy_core(db: &Trail, root: &ObjectId, upper: PathBuf) -> ViewCore {
        ViewCore::new_lazy(
            Trail::open(db.workspace_root()).unwrap(),
            upper,
            root.clone(),
        )
        .unwrap()
    }

    #[test]
    fn view_core_lazy_root_starts_without_indexing_every_path() {
        let (_temp, db, root, upper) = fixture();
        let mut view = lazy_core(&db, &root, upper);

        assert!(matches!(view.lower, ViewLower::Root(_)));
        assert_eq!(view.ino_by_path.len(), 1);
        let children = view.children(VIEW_ROOT_INO).unwrap();
        assert_eq!(
            children
                .iter()
                .map(|(name, attr)| (name.as_str(), attr.kind))
                .collect::<Vec<_>>(),
            vec![
                ("README.md", ViewNodeKind::File),
                ("src", ViewNodeKind::Directory),
            ]
        );
        assert_eq!(view.ino_by_path.len(), 3);
        let readme = view.lookup(VIEW_ROOT_INO, "README.md").unwrap();
        let (slice, eof) = view.read(readme, 2, 3).unwrap();
        assert_eq!(slice, b"sel");
        assert!(!eof);
    }

    #[test]
    fn interrupted_layer_reset_restores_private_upper_and_whiteouts_when_binding_did_not_commit() {
        let (_temp, db, root, upper) = fixture();
        let layout = ViewUpperLayout::from_source_upper(upper.clone());
        fs::create_dir_all(layout.generated_upper.join("node_modules/pkg")).unwrap();
        fs::write(
            layout.generated_upper.join("node_modules/pkg/private.js"),
            "private\n",
        )
        .unwrap();
        let mut view = lazy_core(&db, &root, upper.clone());
        view.whiteouts
            .insert("node_modules/pkg/removed.js".to_string());

        let reset = view
            .prepare_declared_layer_mount_path("node_modules", "dependency", "layer-not-committed")
            .unwrap();
        assert!(!layout.generated_upper.join("node_modules").exists());
        let seed = _temp.path().join("replacement-seed");
        fs::create_dir_all(seed.join("pkg")).unwrap();
        fs::write(seed.join("pkg/replacement.js"), "replacement\n").unwrap();
        reset.install_private_directory(&seed).unwrap();
        assert!(layout
            .generated_upper
            .join("node_modules/pkg/replacement.js")
            .is_file());
        drop(reset); // simulate process loss: recovery owns the durable intent
        drop(view);

        let reopened = lazy_core(&db, &root, upper);
        assert_eq!(
            fs::read(layout.generated_upper.join("node_modules/pkg/private.js")).unwrap(),
            b"private\n"
        );
        assert!(reopened.is_whiteouted("node_modules/pkg/removed.js"));
        assert!(fs::read_dir(layer_mount_reset_intents_dir(&layout))
            .unwrap()
            .next()
            .is_none());
    }

    #[test]
    fn interrupted_private_seed_is_kept_only_after_matching_binding_commit() {
        let (temp, db, root, upper) = fixture();
        let layout = ViewUpperLayout::from_source_upper(upper.clone());
        fs::create_dir_all(layout.generated_upper.join("build")).unwrap();
        fs::write(layout.generated_upper.join("build/old.txt"), "old\n").unwrap();
        let seed = temp.path().join("private-seed");
        fs::create_dir_all(&seed).unwrap();
        fs::write(seed.join("new.txt"), "new\n").unwrap();

        let mut view = lazy_core(&db, &root, upper);
        let reset = view
            .prepare_declared_private_mount_path("build", "generated", "private_committed_identity")
            .unwrap();
        reset.install_private_directory(&seed).unwrap();
        drop(reset);
        drop(view);

        let bindings = vec![WorkspaceLayerBinding {
            binding_identity: "private_committed_identity".to_string(),
            layer_id: None,
            mount_path: "build".to_string(),
            storage_path: None,
            kind: "generated".to_string(),
            priority: 100,
        }];
        assert!(recover_layer_mount_resets(&layout, &bindings)
            .unwrap()
            .is_empty());
        assert_eq!(
            fs::read(layout.generated_upper.join("build/new.txt")).unwrap(),
            b"new\n"
        );
        assert!(!layout.generated_upper.join("build/old.txt").exists());
        assert!(fs::read_dir(layer_mount_reset_intents_dir(&layout))
            .unwrap()
            .next()
            .is_none());
    }

    #[test]
    fn interrupted_layer_unmount_discards_private_upper_when_binding_removal_committed() {
        let (_temp, db, root, upper) = fixture();
        let layout = ViewUpperLayout::from_source_upper(upper.clone());
        fs::create_dir_all(layout.generated_upper.join("generated/old")).unwrap();
        fs::write(
            layout.generated_upper.join("generated/old/private.txt"),
            "private\n",
        )
        .unwrap();
        let mut view = lazy_core(&db, &root, upper.clone());
        view.whiteouts
            .insert("generated/old/removed.txt".to_string());

        let reset = view
            .prepare_declared_layer_unmount_path("generated/old", "generated")
            .unwrap();
        assert!(!layout.generated_upper.join("generated/old").exists());
        drop(reset); // SQLite contains no binding, so recovery commits removal.
        drop(view);

        let reopened = lazy_core(&db, &root, upper);
        assert!(!layout.generated_upper.join("generated/old").exists());
        assert!(!reopened.is_whiteouted("generated/old/removed.txt"));
        assert!(fs::read_dir(layer_mount_reset_intents_dir(&layout))
            .unwrap()
            .next()
            .is_none());
    }

    #[test]
    fn view_core_conformance_copy_up_write_delete_and_reopen() {
        let (temp, db, root, upper) = fixture();
        let mut view = core(&db, &root, upper.clone());
        assert!(view
            .prepare_layer_mount_path("README.md", "layer-test")
            .is_err());
        let readme = view.lookup(VIEW_ROOT_INO, "README.md").unwrap();
        assert!(!upper.join("README.md").exists());
        view.write(readme, 0, b"changed\n").unwrap();
        assert_eq!(
            fs::read_to_string(upper.join("README.md")).unwrap(),
            "changed\n\n"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("README.md")).unwrap(),
            "baseline\n"
        );

        view.remove(VIEW_ROOT_INO, "README.md").unwrap();
        assert!(view.lookup(VIEW_ROOT_INO, "README.md").is_err());
        let reopened = core(&db, &root, upper);
        assert!(reopened.is_whiteouted("README.md"));
    }

    #[test]
    fn crash_after_rename_intent_does_not_publish_whiteout_state() {
        let (_temp, db, root, upper) = fixture();
        let mut view = core(&db, &root, upper.clone());
        view.journal
            .append_classified_with_whiteouts(
                ViewMutationKind::Rename,
                "README.md".into(),
                ViewPathClass::Source,
                Some("renamed.md".into()),
                Some(ViewPathClass::Source),
                vec![
                    ViewWhiteoutChange::Insert("README.md".into()),
                    ViewWhiteoutChange::RemoveTree("renamed.md".into()),
                ],
            )
            .unwrap();
        drop(view); // crash before the filesystem rename and commit record

        let mut reopened = core(&db, &root, upper);
        assert!(reopened.lookup(VIEW_ROOT_INO, "README.md").is_ok());
        assert!(reopened.lookup(VIEW_ROOT_INO, "renamed.md").is_err());
        assert!(!reopened.is_whiteouted("README.md"));
    }

    #[test]
    fn rename_io_failure_after_intent_does_not_publish_whiteout_state() {
        let (_temp, db, root, upper) = fixture();
        let mut view = core(&db, &root, upper.clone());
        fail_rename_after_intent_for_current_thread();
        assert!(matches!(
            view.rename(VIEW_ROOT_INO, "README.md", VIEW_ROOT_INO, "renamed.md"),
            Err(code) if code == EIO
        ));
        drop(view);

        let mut reopened = core(&db, &root, upper);
        assert!(reopened.lookup(VIEW_ROOT_INO, "README.md").is_ok());
        assert!(reopened.lookup(VIEW_ROOT_INO, "renamed.md").is_err());
        assert!(!reopened.is_whiteouted("README.md"));
    }

    #[test]
    fn rename_durability_fence_precedes_semantic_commit() {
        let (_temp, db, root, upper) = fixture();
        let mut view = core(&db, &root, upper.clone());
        let readme = view.lookup(VIEW_ROOT_INO, "README.md").unwrap();
        view.write(readme, 0, b"changed\n").unwrap();
        fail_rename_before_durability_fence_for_current_thread();

        assert!(matches!(
            view.rename(VIEW_ROOT_INO, "README.md", VIEW_ROOT_INO, "renamed.md"),
            Err(code) if code == EIO
        ));
        assert!(upper.join("renamed.md").is_file());
        drop(view);

        let mut reopened = core(&db, &root, upper);
        assert!(reopened.lookup(VIEW_ROOT_INO, "README.md").is_ok());
        assert!(reopened.lookup(VIEW_ROOT_INO, "renamed.md").is_ok());
        assert!(!reopened.is_whiteouted("README.md"));
        let candidates = reopened.checkpoint_candidates().unwrap();
        assert!(candidates.paths.contains("README.md"));
        assert!(candidates.paths.contains("renamed.md"));
    }

    #[test]
    fn namespace_durability_fence_precedes_remove_whiteout_commit() {
        let (_temp, db, root, upper) = fixture();
        let mut view = core(&db, &root, upper.clone());
        fail_namespace_before_durability_fence_for_current_thread();

        assert!(matches!(
            view.remove(VIEW_ROOT_INO, "README.md"),
            Err(code) if code == EIO
        ));
        drop(view);

        let mut reopened = core(&db, &root, upper);
        assert!(reopened.lookup(VIEW_ROOT_INO, "README.md").is_ok());
        assert!(!reopened.is_whiteouted("README.md"));
        assert!(reopened
            .checkpoint_candidates()
            .unwrap()
            .paths
            .contains("README.md"));
    }

    #[test]
    fn first_copy_up_requires_namespace_durability_before_write() {
        let (_temp, db, root, upper) = fixture();
        let mut view = core(&db, &root, upper.clone());
        let readme = view.lookup(VIEW_ROOT_INO, "README.md").unwrap();
        fail_namespace_before_durability_fence_for_current_thread();

        assert!(matches!(
            view.write(readme, 0, b"changed\n"),
            Err(code) if code == EIO
        ));
        assert_eq!(fs::read(upper.join("README.md")).unwrap(), b"baseline\n");
        drop(view);

        let mut reopened = core(&db, &root, upper);
        let reopened_readme = reopened.lookup(VIEW_ROOT_INO, "README.md").unwrap();
        assert_eq!(
            reopened.read(reopened_readme, 0, 32).unwrap().0,
            b"baseline\n"
        );
        assert!(reopened
            .checkpoint_candidates()
            .unwrap()
            .paths
            .contains("README.md"));
    }

    #[test]
    fn semantic_upper_mutation_is_not_visible_before_intent_is_durable() {
        let (temp, db, root, upper) = fixture();
        let mut view = core(&db, &root, upper.clone());
        let readme = view.lookup(VIEW_ROOT_INO, "README.md").unwrap();
        fail_next_view_journal_sync_for_current_thread();

        assert!(matches!(view.write(readme, 0, b"new"), Err(code) if code == EIO));
        assert!(!upper.join("README.md").exists());
        assert_eq!(
            fs::read(temp.path().join("README.md")).unwrap(),
            b"baseline\n"
        );
        assert_eq!(view.read(readme, 0, 32).unwrap().0, b"baseline\n");
    }

    #[test]
    fn live_handle_reloads_after_noop_checkpoint_rotates_generation() {
        let (_temp, db, root, upper) = fixture();
        let mut view = core(&db, &root, upper.clone());
        let layout = ViewUpperLayout::from_source_upper(upper.clone());
        {
            let mut checkpoint = ViewMutationBarrier::exclusive(&layout.meta_dir).unwrap();
            ViewMutationJournal::rotate_after_checkpoint(&upper, 0, 1).unwrap();
            checkpoint.record_checkpoint_cut(0, 1).unwrap();
        }

        let readme = view.lookup(VIEW_ROOT_INO, "README.md").unwrap();
        view.write(readme, 0, b"changed\n").unwrap();

        let candidates = view.checkpoint_candidates().unwrap();
        assert!(candidates.qualified);
        assert_eq!(candidates.journal_sequence, 1);
        assert!(candidates.paths.contains("README.md"));
        let active = ViewMutationJournal::open(&upper).unwrap();
        assert_eq!(active.generation(), 1);
        assert!(active.dirty_source_paths().contains("README.md"));
    }

    #[test]
    fn untrusted_view_scans_upper_instead_of_claiming_zero_recovery_walks() {
        let (_temp, db, root, upper) = fixture();
        let mut view = core(&db, &root, upper.clone());
        let readme = view.lookup(VIEW_ROOT_INO, "README.md").unwrap();
        view.write(readme, 0, b"changed").unwrap();
        let journal_path = ViewUpperLayout::from_source_upper(upper.clone()).journal_path();
        OpenOptions::new()
            .append(true)
            .open(journal_path)
            .unwrap()
            .write_all(b"corrupt-complete-record\n")
            .unwrap();

        let candidates = view.checkpoint_candidates().unwrap();
        assert!(!candidates.qualified);
        assert!(candidates.upper_recovery_walks > 0);
        assert!(candidates.paths.contains("README.md"));
    }

    #[derive(Clone, Copy)]
    enum JournalDamage {
        Missing,
        Corrupt,
        Gapped,
        ValidPrefix,
    }

    fn damage_mutation_journal(upper: &Path, damage: JournalDamage) {
        let path = ViewUpperLayout::from_source_upper(upper.to_path_buf()).journal_path();
        match damage {
            JournalDamage::Missing => fs::remove_file(path).unwrap(),
            JournalDamage::Corrupt => OpenOptions::new()
                .append(true)
                .open(path)
                .unwrap()
                .write_all(b"corrupt-complete-record\n")
                .unwrap(),
            JournalDamage::Gapped => fs::write(
                path,
                br#"{"sequence":99,"generation":0,"class":"source","kind":"delete","path":"README.md"}
"#,
            )
            .unwrap(),
            JournalDamage::ValidPrefix => fs::write(path, b"").unwrap(),
        }
    }

    #[test]
    fn damaged_path_journal_never_loses_deleted_lower_path() {
        for damage in [
            JournalDamage::Missing,
            JournalDamage::Corrupt,
            JournalDamage::Gapped,
            JournalDamage::ValidPrefix,
        ] {
            let (_temp, db, root, upper) = fixture();
            let mut view = core(&db, &root, upper.clone());
            view.remove(VIEW_ROOT_INO, "README.md").unwrap();
            damage_mutation_journal(&upper, damage);

            let candidates = view.checkpoint_candidates().unwrap();
            assert!(!candidates.qualified);
            assert!(candidates.upper_recovery_walks > 0);
            assert!(candidates.paths.contains("README.md"));
        }
    }

    #[test]
    fn damaged_path_journal_recovers_both_rename_sides() {
        for damage in [
            JournalDamage::Missing,
            JournalDamage::Corrupt,
            JournalDamage::Gapped,
            JournalDamage::ValidPrefix,
        ] {
            let (_temp, db, root, upper) = fixture();
            let mut view = core(&db, &root, upper.clone());
            view.rename(VIEW_ROOT_INO, "README.md", VIEW_ROOT_INO, "RENAMED.md")
                .unwrap();
            damage_mutation_journal(&upper, damage);

            let candidates = view.checkpoint_candidates().unwrap();
            assert!(!candidates.qualified);
            assert!(candidates.paths.contains("README.md"));
            assert!(candidates.paths.contains("RENAMED.md"));
        }
    }

    #[test]
    fn valid_prefix_truncation_recovers_written_upper_path() {
        let (_temp, db, root, upper) = fixture();
        let mut view = core(&db, &root, upper.clone());
        let readme = view.lookup(VIEW_ROOT_INO, "README.md").unwrap();
        view.write(readme, 0, b"changed\n").unwrap();
        damage_mutation_journal(&upper, JournalDamage::ValidPrefix);

        let candidates = view.checkpoint_candidates().unwrap();
        assert!(!candidates.qualified);
        assert!(candidates.upper_recovery_walks > 0);
        assert!(candidates.paths.contains("README.md"));
    }

    #[test]
    fn recovery_fails_closed_when_both_independent_journals_are_lost() {
        let (_temp, db, root, upper) = fixture();
        let mut view = core(&db, &root, upper.clone());
        view.remove(VIEW_ROOT_INO, "README.md").unwrap();
        let layout = ViewUpperLayout::from_source_upper(upper.clone());
        fs::remove_file(layout.journal_path()).unwrap();
        fs::remove_file(layout.whiteout_journal_path()).unwrap();

        assert!(matches!(
            view.checkpoint_candidates(),
            Err(Error::ChangeLedgerReconcileRequired { .. })
        ));
    }

    #[test]
    fn qualified_view_replays_incremental_whiteouts_without_upper_walk() {
        let (_temp, db, root, upper) = fixture();
        let mut view = core(&db, &root, upper.clone());
        view.remove(VIEW_ROOT_INO, "README.md").unwrap();
        drop(view);

        let reopened = core(&db, &root, upper);
        assert!(reopened.is_whiteouted("README.md"));
        let candidates = reopened.checkpoint_candidates().unwrap();
        assert!(candidates.qualified);
        assert_eq!(candidates.upper_recovery_walks, 0);
        assert!(candidates.paths.contains("README.md"));
    }

    #[test]
    fn view_core_conformance_rename_directory_preserves_mixed_contents_and_mode() {
        let (_temp, db, root, upper) = fixture();
        fs::create_dir_all(upper.join("src")).unwrap();
        fs::write(upper.join("src/upper.txt"), "upper\n").unwrap();
        let mut view = core(&db, &root, upper.clone());

        view.rename(VIEW_ROOT_INO, "src", VIEW_ROOT_INO, "moved")
            .unwrap();

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
        assert!(view.lookup(VIEW_ROOT_INO, "src").is_err());
        assert!(view.lookup(VIEW_ROOT_INO, "moved").is_ok());
    }

    #[test]
    fn view_core_noop_setattr_does_not_create_checkpoint_candidate() {
        let (_temp, db, root, upper) = fixture();
        let mut view = lazy_core(&db, &root, upper.clone());
        let readme = view.lookup(VIEW_ROOT_INO, "README.md").unwrap();

        let attr = view.setattr(readme, None, None).unwrap();

        assert_eq!(attr.ino, readme);
        assert!(!upper.join("README.md").exists());
        let candidates = view.checkpoint_candidates().unwrap();
        assert!(!candidates.paths.contains("README.md"));
    }

    #[test]
    fn directory_rename_moves_files_from_every_classified_upper() {
        let (temp, db, root, _legacy_upper) = fixture();
        let view_dir = temp.path().join("view-rename");
        let source_upper = view_dir.join("source-upper");
        let mut view = lazy_core(&db, &root, source_upper.clone());
        let project = view.mkdir(VIEW_ROOT_INO, "project", 0o755).unwrap();
        let source = view.create(project.ino, "main.rs", 0o644, true).unwrap();
        view.write(source.ino, 0, b"source\n").unwrap();
        let target = view.mkdir(project.ino, "target", 0o755).unwrap();
        let artifact = view.create(target.ino, "artifact", 0o644, true).unwrap();
        view.write(artifact.ino, 0, b"generated\n").unwrap();

        view.rename(VIEW_ROOT_INO, "project", VIEW_ROOT_INO, "renamed")
            .unwrap();
        assert!(!source_upper.join("project").exists());
        assert!(!view_dir.join("generated-upper/project").exists());
        assert_eq!(
            fs::read(source_upper.join("renamed/main.rs")).unwrap(),
            b"source\n"
        );
        assert_eq!(
            fs::read(view_dir.join("generated-upper/renamed/target/artifact")).unwrap(),
            b"generated\n"
        );
        assert!(view.lookup(VIEW_ROOT_INO, "project").is_err());
        let renamed = view.lookup(VIEW_ROOT_INO, "renamed").unwrap();
        assert!(view.lookup(renamed, "main.rs").is_ok());
        assert!(view.lookup(renamed, "target").is_ok());
    }

    #[test]
    fn view_core_conformance_truncate_mode_and_symlink_escape() {
        let (temp, db, root, upper) = fixture();
        let mut view = core(&db, &root, upper.clone());
        let script = view
            .lookup(VIEW_ROOT_INO, "src/nested/tool.sh")
            .unwrap_err();
        assert_eq!(script, EINVAL);
        let src = view.lookup(VIEW_ROOT_INO, "src").unwrap();
        let nested = view.lookup(src, "nested").unwrap();
        let script = view.lookup(nested, "tool.sh").unwrap();
        view.setattr(script, Some(3), Some(0o755)).unwrap();
        assert_eq!(fs::read(upper.join("src/nested/tool.sh")).unwrap(), b"#!/");

        let link = view
            .symlink(src, "tool-link", Path::new("nested/tool.sh"))
            .unwrap();
        assert_eq!(link.kind, ViewNodeKind::Symlink);
        assert_eq!(
            view.readlink(link.ino).unwrap(),
            Path::new("nested/tool.sh")
        );
        assert!(fs::symlink_metadata(upper.join("src/tool-link"))
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(matches!(
            view.symlink(VIEW_ROOT_INO, "bad-link", Path::new("../../outside")),
            Err(EPERM)
        ));

        let outside = temp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, upper.join("escape")).unwrap();
        assert!(view.mkdir(VIEW_ROOT_INO, "escape", 0o755).is_err());
        let escape = view.lookup(VIEW_ROOT_INO, "escape").unwrap();
        assert_eq!(view.readlink(escape), Err(EPERM));
        assert_eq!(view.upper_path("escape/file"), Err(EPERM));
        assert!(!outside.join("file").exists());
    }

    #[test]
    fn view_core_checkpoint_candidates_recover_from_upper_and_whiteouts() {
        let (_temp, db, root, upper) = fixture();
        let mut view = core(&db, &root, upper.clone());
        let readme = view.lookup(VIEW_ROOT_INO, "README.md").unwrap();
        view.write(readme, 0, b"changed").unwrap();
        view.remove(VIEW_ROOT_INO, "src").unwrap_err();
        let src = view.lookup(VIEW_ROOT_INO, "src").unwrap();
        let lower = view.lookup(src, "lower.txt").unwrap();
        assert!(lower > VIEW_ROOT_INO);
        view.remove(src, "lower.txt").unwrap();

        let candidates = view.checkpoint_candidates().unwrap();
        assert!(candidates.journal_sequence > 0);
        assert!(candidates.paths.contains("README.md"));
        assert!(candidates.paths.contains("src/lower.txt"));

        let reopened = core(&db, &root, upper);
        assert_eq!(reopened.checkpoint_candidates().unwrap(), candidates);
    }

    #[test]
    fn view_core_splits_generated_dependency_secret_and_source_uppers() {
        let (temp, db, root, _legacy_upper) = fixture();
        let view_dir = temp.path().join("view");
        let source_upper = view_dir.join("source-upper");
        let mut view = lazy_core(&db, &root, source_upper.clone());

        let src = view.lookup(VIEW_ROOT_INO, "src").unwrap();
        view.create(src, "new.rs", 0o644, true).unwrap();
        let target = view.mkdir(VIEW_ROOT_INO, "target", 0o755).unwrap();
        view.create(target.ino, "artifact", 0o644, true).unwrap();
        let modules = view.mkdir(VIEW_ROOT_INO, "node_modules", 0o755).unwrap();
        view.create(modules.ino, "package.json", 0o644, true)
            .unwrap();
        view.create(VIEW_ROOT_INO, ".env.local", 0o600, true)
            .unwrap();

        assert!(source_upper.join("src/new.rs").is_file());
        assert!(view_dir.join("generated-upper/target/artifact").is_file());
        assert!(view_dir
            .join("generated-upper/node_modules/package.json")
            .is_file());
        assert!(view_dir.join("scratch-upper/.env.local").is_file());

        let candidates = view.checkpoint_candidates().unwrap();
        assert!(candidates.paths.contains("src/new.rs"));
        assert!(!candidates.paths.contains("target/artifact"));
        assert!(!candidates.paths.contains("node_modules/package.json"));
        assert!(!candidates.paths.contains(".env.local"));
        assert!(
            fs::read_to_string(view_dir.join("meta/mutation-journal.jsonl"))
                .unwrap()
                .contains("\"class\":\"generated\"")
        );
    }
}
