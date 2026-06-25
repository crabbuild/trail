use super::*;
use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};
use notify::{
    Config as NotifyConfig, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use rayon::prelude::*;

const DAEMON_STATUS_DIRTY_PATH_LIMIT: usize = 16_384;
const WORKTREE_INDEX_BASELINE_ROOT_KEY: &str = "worktree.index.baseline_root";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WorktreeFileStamp {
    size_bytes: u64,
    modified_ns: i64,
    changed_ns: i64,
    device_id: i64,
    inode: i64,
    executable: bool,
}

#[derive(Debug)]
struct IndexedWorktreeManifest {
    stamp: WorktreeFileStamp,
}

#[derive(Debug)]
struct WorktreeIndexReadCandidate {
    path: String,
    abs_path: PathBuf,
    stamp: WorktreeFileStamp,
}

#[derive(Debug)]
struct WorktreeIndexUpdate {
    path: String,
    stamp: WorktreeFileStamp,
    disk_manifest: DiskManifest,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct WorktreeIndexRefresh {
    pub(crate) files: u64,
    pub(crate) changed: bool,
}

impl CrabDb {
    pub fn enable_daemon_worktree_cache(&mut self) -> Result<()> {
        let warmup = self.start_daemon_worktree_cache()?;
        warmup.run()
    }

    pub fn start_daemon_worktree_cache(&mut self) -> Result<DaemonWorktreeCacheWarmup> {
        let cache = DaemonWorktreeCache::start(&self.workspace_root)?;
        let warmup = DaemonWorktreeCacheWarmup {
            workspace_root: self.workspace_root.clone(),
            db_dir: self.db_dir.clone(),
            state: Arc::clone(&cache.state),
            generation: cache.generation(),
        };
        self.daemon_worktree_cache = Some(cache);
        Ok(warmup)
    }

    fn finish_daemon_worktree_cache_baseline(
        &self,
        state: &Arc<Mutex<DaemonWorktreeCacheState>>,
        generation: u64,
    ) -> Result<()> {
        let branch = self.current_branch()?;
        let head = self.resolve_branch_ref(&branch)?;
        let changed_paths = self.status_changed_paths_uncached(&branch, &branch, &head.root_id)?;
        DaemonWorktreeCache::finish_initial_baseline(
            state,
            generation,
            &head.root_id,
            &changed_paths,
        );
        Ok(())
    }

    pub(crate) fn daemon_worktree_snapshot(&self) -> Option<DaemonWorktreeSnapshot> {
        self.daemon_worktree_cache
            .as_ref()
            .map(DaemonWorktreeCache::snapshot)
    }

    pub(crate) fn daemon_dirty_path_limit(&self) -> usize {
        DAEMON_STATUS_DIRTY_PATH_LIMIT
    }

    pub(crate) fn reconcile_daemon_status_paths(
        &self,
        root_id: &ObjectId,
        checked_paths: &[String],
        summaries: &[FileDiffSummary],
        generation: u64,
    ) {
        if let Some(cache) = &self.daemon_worktree_cache {
            cache.reconcile_selected(root_id, checked_paths, summaries, generation);
        }
    }

    pub(crate) fn reconcile_daemon_full_status(
        &self,
        root_id: &ObjectId,
        summaries: &[FileDiffSummary],
        generation: Option<u64>,
    ) {
        if let Some(cache) = &self.daemon_worktree_cache {
            cache.reconcile_full(root_id, summaries, generation);
        }
    }

    pub fn refresh_worktree_index(&self) -> Result<WorktreeIndexReport> {
        let started = Instant::now();
        let refresh = self.refresh_worktree_index_streaming_report()?;
        Ok(WorktreeIndexReport {
            files: refresh.files,
            indexed_entries: self.worktree_index_count()?,
            duration_ms: elapsed_ms(started.elapsed()),
        })
    }

    pub(crate) fn refresh_worktree_index_streaming_report(&self) -> Result<WorktreeIndexRefresh> {
        let scan_id = worktree_scan_id();
        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let result = self.refresh_worktree_index_streaming_in_transaction(scan_id);
        match result {
            Ok(count) => {
                self.conn.execute_batch("COMMIT;")?;
                Ok(count)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                Err(err)
            }
        }
    }

    fn refresh_worktree_index_streaming_in_transaction(
        &self,
        scan_id: i64,
    ) -> Result<WorktreeIndexRefresh> {
        let mut cached_entries = self.load_worktree_index_entries()?;
        let root = self.workspace_root.canonicalize()?;
        let mut builder = WalkBuilder::new(&root);
        builder
            .hidden(false)
            .git_ignore(self.config.recording.ignore_gitignored)
            .git_exclude(self.config.recording.ignore_gitignored)
            .git_global(self.config.recording.ignore_gitignored)
            .add_custom_ignore_filename(".crabignore");

        let walker = builder.build();
        let mut count = 0u64;
        let mut read_candidates = Vec::new();
        for item in walker {
            let entry = item.map_err(|err| Error::InvalidInput(err.to_string()))?;
            let path = entry.path();
            if path == root {
                continue;
            }
            let rel = path
                .strip_prefix(&root)
                .map_err(|err| Error::InvalidInput(err.to_string()))?;
            let rel = normalize_relative_path(&rel.to_string_lossy())?;
            if entry.file_type().is_some_and(|kind| kind.is_dir()) && is_default_ignored(&rel) {
                continue;
            }
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            if is_default_ignored(&rel) {
                continue;
            }

            let metadata = fs::symlink_metadata(path)?;
            let stamp = worktree_file_stamp(&metadata);
            if cached_entries
                .remove(&rel)
                .is_some_and(|cached| cached.stamp == stamp)
            {
                count += 1;
                continue;
            }

            read_candidates.push(WorktreeIndexReadCandidate {
                path: rel,
                abs_path: path.to_path_buf(),
                stamp,
            });
            count += 1;
        }

        let changed = !read_candidates.is_empty() || !cached_entries.is_empty();
        let updates = read_worktree_index_candidates(&read_candidates, &self.config.text)?;
        for update in updates {
            self.upsert_worktree_index_manifest_for_scan(
                &update.path,
                update.stamp,
                &update.disk_manifest,
                scan_id,
            )?;
        }
        for path in cached_entries.keys() {
            self.delete_worktree_index_path_row(path)?;
        }
        if changed {
            self.clear_worktree_index_baseline()?;
        }
        Ok(WorktreeIndexRefresh {
            files: count,
            changed,
        })
    }

    fn load_worktree_index_entries(&self) -> Result<HashMap<String, IndexedWorktreeManifest>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, size_bytes, modified_ns, changed_ns, device_id, inode, executable \
             FROM worktree_file_index",
        )?;
        let rows = stmt.query_map([], |row| {
            let executable = row.get::<_, i64>(6)? != 0;
            Ok((
                row.get::<_, String>(0)?,
                IndexedWorktreeManifest {
                    stamp: WorktreeFileStamp {
                        size_bytes: row.get::<_, i64>(1)?.max(0) as u64,
                        modified_ns: row.get(2)?,
                        changed_ns: row.get(3)?,
                        device_id: row.get(4)?,
                        inode: row.get(5)?,
                        executable,
                    },
                },
            ))
        })?;
        rows.collect::<std::result::Result<HashMap<_, _>, _>>()
            .map_err(Error::from)
    }

    pub(crate) fn scan_worktree_manifest_indexed(&self) -> Result<BTreeMap<String, DiskManifest>> {
        let root = self.workspace_root.canonicalize()?;
        let mut builder = WalkBuilder::new(&root);
        builder
            .hidden(false)
            .git_ignore(self.config.recording.ignore_gitignored)
            .git_exclude(self.config.recording.ignore_gitignored)
            .git_global(self.config.recording.ignore_gitignored)
            .add_custom_ignore_filename(".crabignore");

        let walker = builder.build();
        let mut manifest = BTreeMap::new();
        let mut seen = BTreeSet::new();
        for item in walker {
            let entry = item.map_err(|err| Error::InvalidInput(err.to_string()))?;
            let path = entry.path();
            if path == root {
                continue;
            }
            let rel = path
                .strip_prefix(&root)
                .map_err(|err| Error::InvalidInput(err.to_string()))?;
            let rel = normalize_relative_path(&rel.to_string_lossy())?;
            if entry.file_type().is_some_and(|kind| kind.is_dir()) && is_default_ignored(&rel) {
                continue;
            }
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            if is_default_ignored(&rel) {
                continue;
            }

            let metadata = fs::symlink_metadata(path)?;
            let stamp = worktree_file_stamp(&metadata);
            let disk_manifest = if let Some(cached) = self.cached_worktree_manifest(&rel, stamp)? {
                cached
            } else {
                let bytes = fs::read(path)?;
                let disk_manifest = DiskManifest {
                    kind: classify_file_kind(&bytes, &self.config.text),
                    executable: stamp.executable,
                    content_hash: sha256_hex(&bytes),
                };
                self.upsert_worktree_index_manifest(&rel, stamp, &disk_manifest)?;
                disk_manifest
            };
            seen.insert(rel.clone());
            manifest.insert(rel, disk_manifest);
        }
        self.prune_worktree_index(&seen)?;
        Ok(manifest)
    }

    pub(crate) fn update_worktree_index_from_disk_files(&self, files: &[DiskFile]) -> Result<()> {
        if !files.is_empty() {
            self.clear_worktree_index_baseline()?;
        }
        for file in files {
            let abs = self.workspace_root.join(path_from_rel(&file.path));
            let metadata = match fs::symlink_metadata(&abs) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    self.delete_worktree_index_path(&file.path)?;
                    continue;
                }
                Err(err) => return Err(Error::Io(err)),
            };
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                self.delete_worktree_index_path(&file.path)?;
                continue;
            }
            let stamp = worktree_file_stamp(&metadata);
            let disk_manifest = DiskManifest {
                kind: classify_file_kind(&file.bytes, &self.config.text),
                executable: stamp.executable,
                content_hash: sha256_hex(&file.bytes),
            };
            self.upsert_worktree_index_manifest(&file.path, stamp, &disk_manifest)?;
        }
        Ok(())
    }

    pub(crate) fn delete_worktree_index_path(&self, path: &str) -> Result<()> {
        self.clear_worktree_index_baseline()?;
        self.delete_worktree_index_path_row(path)
    }

    fn delete_worktree_index_path_row(&self, path: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM worktree_file_index WHERE path = ?1",
            params![path],
        )?;
        Ok(())
    }

    fn worktree_index_count(&self) -> Result<u64> {
        let count = self
            .conn
            .query_row("SELECT COUNT(*) FROM worktree_file_index", [], |row| {
                row.get::<_, i64>(0)
            })?;
        Ok(count.max(0) as u64)
    }

    fn cached_worktree_manifest(
        &self,
        path: &str,
        stamp: WorktreeFileStamp,
    ) -> Result<Option<DiskManifest>> {
        self.conn
            .query_row(
                "SELECT kind, content_hash FROM worktree_file_index \
                 WHERE path = ?1 AND size_bytes = ?2 AND modified_ns = ?3 \
                 AND changed_ns = ?4 AND device_id = ?5 AND inode = ?6 \
                 AND executable = ?7",
                params![
                    path,
                    stamp.size_bytes as i64,
                    stamp.modified_ns,
                    stamp.changed_ns,
                    stamp.device_id,
                    stamp.inode,
                    i64::from(stamp.executable)
                ],
                |row| {
                    let kind = file_kind_from_index(&row.get::<_, String>(0)?).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(err),
                        )
                    })?;
                    Ok(DiskManifest {
                        kind,
                        executable: stamp.executable,
                        content_hash: row.get(1)?,
                    })
                },
            )
            .optional()
            .map_err(Error::from)
    }

    fn upsert_worktree_index_manifest(
        &self,
        path: &str,
        stamp: WorktreeFileStamp,
        disk_manifest: &DiskManifest,
    ) -> Result<()> {
        self.upsert_worktree_index_manifest_for_scan(path, stamp, disk_manifest, worktree_scan_id())
    }

    fn upsert_worktree_index_manifest_for_scan(
        &self,
        path: &str,
        stamp: WorktreeFileStamp,
        disk_manifest: &DiskManifest,
        scan_id: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO worktree_file_index \
             (path, size_bytes, modified_ns, changed_ns, device_id, inode, executable, kind, content_hash, last_seen_scan, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                path,
                stamp.size_bytes as i64,
                stamp.modified_ns,
                stamp.changed_ns,
                stamp.device_id,
                stamp.inode,
                i64::from(stamp.executable),
                file_kind_index_label(&disk_manifest.kind),
                disk_manifest.content_hash,
                scan_id,
                now_ts()
            ],
        )?;
        Ok(())
    }

    fn prune_worktree_index(&self, seen: &BTreeSet<String>) -> Result<()> {
        let mut stmt = self.conn.prepare("SELECT path FROM worktree_file_index")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let cached_paths = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        drop(stmt);
        for path in cached_paths {
            if !seen.contains(&path) {
                self.delete_worktree_index_path(&path)?;
            }
        }
        Ok(())
    }

    pub(crate) fn prune_worktree_index_for_selections(
        &self,
        selections: &[String],
        seen: &BTreeSet<String>,
    ) -> Result<()> {
        let mut stmt = self.conn.prepare("SELECT path FROM worktree_file_index")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let cached_paths = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        drop(stmt);
        for path in cached_paths {
            if seen.contains(&path) {
                continue;
            }
            if selections
                .iter()
                .any(|selection| path_matches_selection(&path, selection))
            {
                self.delete_worktree_index_path(&path)?;
            }
        }
        Ok(())
    }

    pub(crate) fn worktree_index_baseline_root(&self) -> Result<Option<ObjectId>> {
        Ok(self
            .schema_meta_value(WORKTREE_INDEX_BASELINE_ROOT_KEY)?
            .map(ObjectId))
    }

    pub(crate) fn set_worktree_index_baseline(&self, root_id: &ObjectId) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO schema_meta (key, value, updated_at) VALUES (?1, ?2, ?3)",
            params![WORKTREE_INDEX_BASELINE_ROOT_KEY, root_id.0, now_ts()],
        )?;
        Ok(())
    }

    pub(crate) fn clear_worktree_index_baseline(&self) -> Result<()> {
        self.conn.execute(
            "DELETE FROM schema_meta WHERE key = ?1",
            params![WORKTREE_INDEX_BASELINE_ROOT_KEY],
        )?;
        Ok(())
    }
}

fn read_worktree_index_candidates(
    candidates: &[WorktreeIndexReadCandidate],
    text_config: &TextConfig,
) -> Result<Vec<WorktreeIndexUpdate>> {
    if candidates.len() <= 1 {
        return candidates
            .iter()
            .map(|candidate| read_worktree_index_candidate(candidate, text_config))
            .collect();
    }

    candidates
        .par_iter()
        .map(|candidate| read_worktree_index_candidate(candidate, text_config))
        .collect()
}

fn read_worktree_index_candidate(
    candidate: &WorktreeIndexReadCandidate,
    text_config: &TextConfig,
) -> Result<WorktreeIndexUpdate> {
    let bytes = fs::read(&candidate.abs_path)?;
    Ok(WorktreeIndexUpdate {
        path: candidate.path.clone(),
        stamp: candidate.stamp,
        disk_manifest: DiskManifest {
            kind: classify_file_kind(&bytes, text_config),
            executable: candidate.stamp.executable,
            content_hash: sha256_hex(&bytes),
        },
    })
}

fn worktree_file_stamp(metadata: &fs::Metadata) -> WorktreeFileStamp {
    WorktreeFileStamp {
        size_bytes: metadata.len(),
        modified_ns: metadata_modified_ns(metadata),
        changed_ns: metadata_changed_ns(metadata),
        device_id: metadata_device_id(metadata),
        inode: metadata_inode(metadata),
        executable: executable_from_metadata(metadata),
    }
}

fn metadata_modified_ns(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(duration_ns)
        .unwrap_or(0)
}

#[cfg(unix)]
fn metadata_changed_ns(metadata: &fs::Metadata) -> i64 {
    metadata
        .ctime()
        .saturating_mul(1_000_000_000)
        .saturating_add(metadata.ctime_nsec())
}

#[cfg(not(unix))]
fn metadata_changed_ns(_metadata: &fs::Metadata) -> i64 {
    0
}

#[cfg(unix)]
fn metadata_device_id(metadata: &fs::Metadata) -> i64 {
    metadata.dev().min(i64::MAX as u64) as i64
}

#[cfg(not(unix))]
fn metadata_device_id(_metadata: &fs::Metadata) -> i64 {
    0
}

#[cfg(unix)]
fn metadata_inode(metadata: &fs::Metadata) -> i64 {
    metadata.ino().min(i64::MAX as u64) as i64
}

#[cfg(not(unix))]
fn metadata_inode(_metadata: &fs::Metadata) -> i64 {
    0
}

fn duration_ns(duration: Duration) -> i64 {
    let ns = (duration.as_secs() as u128)
        .saturating_mul(1_000_000_000)
        .saturating_add(duration.subsec_nanos() as u128);
    ns.min(i64::MAX as u128) as i64
}

fn worktree_scan_id() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(duration_ns)
        .unwrap_or(0)
}

fn file_kind_index_label(kind: &FileKind) -> &'static str {
    match kind {
        FileKind::Text => "Text",
        FileKind::OpaqueText => "OpaqueText",
        FileKind::Binary => "Binary",
    }
}

pub(crate) fn file_kind_from_index(value: &str) -> std::result::Result<FileKind, Error> {
    match value {
        "Text" => Ok(FileKind::Text),
        "OpaqueText" => Ok(FileKind::OpaqueText),
        "Binary" => Ok(FileKind::Binary),
        other => Err(Error::Corrupt(format!(
            "invalid worktree file index kind `{other}`"
        ))),
    }
}

impl DaemonWorktreeCache {
    fn start(workspace_root: &Path) -> Result<Self> {
        let state = Arc::new(Mutex::new(DaemonWorktreeCacheState::default()));
        let root = workspace_root.to_path_buf();
        let state_for_watcher = Arc::clone(&state);
        let mut watcher = RecommendedWatcher::new(
            move |event| handle_daemon_watch_event(&root, &state_for_watcher, event),
            NotifyConfig::default(),
        )
        .map_err(notify_error)?;
        watcher
            .watch(workspace_root, RecursiveMode::Recursive)
            .map_err(notify_error)?;
        Ok(Self {
            state,
            _watcher: watcher,
        })
    }

    fn snapshot(&self) -> DaemonWorktreeSnapshot {
        let state = self.state.lock().expect("daemon worktree cache poisoned");
        if !state.initialized || state.overflow {
            DaemonWorktreeSnapshot::Overflow {
                generation: state.generation,
            }
        } else if state.dirty_paths.is_empty() {
            DaemonWorktreeSnapshot::Clean {
                generation: state.generation,
                root_id: state.baseline_root_id.clone(),
            }
        } else {
            DaemonWorktreeSnapshot::Dirty {
                generation: state.generation,
                paths: state.dirty_paths.iter().cloned().collect(),
            }
        }
    }

    fn generation(&self) -> u64 {
        self.state
            .lock()
            .expect("daemon worktree cache poisoned")
            .generation
    }

    fn finish_initial_baseline(
        state: &Arc<Mutex<DaemonWorktreeCacheState>>,
        generation: u64,
        root_id: &ObjectId,
        summaries: &[FileDiffSummary],
    ) {
        let mut state = state.lock().expect("daemon worktree cache poisoned");
        if state.initialized {
            return;
        }
        for path in summary_paths(summaries) {
            state.dirty_paths.insert(path);
        }
        state.initialized = true;
        if state.generation == generation && !state.overflow && state.dirty_paths.is_empty() {
            state.baseline_root_id = Some(root_id.clone());
        } else {
            state.baseline_root_id = None;
        }
        state.generation = state.generation.saturating_add(1);
    }

    fn reconcile_selected(
        &self,
        root_id: &ObjectId,
        checked_paths: &[String],
        summaries: &[FileDiffSummary],
        generation: u64,
    ) {
        let mut state = self.state.lock().expect("daemon worktree cache poisoned");
        if state.generation != generation || state.overflow {
            return;
        }
        state.initialized = true;
        for path in checked_paths {
            state.dirty_paths.remove(path);
        }
        for path in summary_paths(summaries) {
            state.dirty_paths.insert(path);
        }
        if state.dirty_paths.is_empty() {
            state.baseline_root_id = Some(root_id.clone());
        } else {
            state.baseline_root_id = None;
        }
        state.generation = state.generation.saturating_add(1);
    }

    fn reconcile_full(
        &self,
        root_id: &ObjectId,
        summaries: &[FileDiffSummary],
        generation: Option<u64>,
    ) {
        let mut state = self.state.lock().expect("daemon worktree cache poisoned");
        if generation.is_some_and(|expected| expected != state.generation) {
            for path in summary_paths(summaries) {
                state.dirty_paths.insert(path);
            }
            state.baseline_root_id = None;
            return;
        }

        state.overflow = false;
        state.initialized = true;
        state.dirty_paths = summary_paths(summaries).into_iter().collect();
        if state.dirty_paths.is_empty() {
            state.baseline_root_id = Some(root_id.clone());
        } else {
            state.baseline_root_id = None;
        }
        state.generation = state.generation.saturating_add(1);
    }
}

fn handle_daemon_watch_event(
    root: &Path,
    state: &Arc<Mutex<DaemonWorktreeCacheState>>,
    event: notify::Result<Event>,
) {
    let Ok(event) = event else {
        mark_daemon_cache_overflow(state);
        return;
    };
    if matches!(event.kind, EventKind::Access(_)) {
        return;
    }
    if matches!(
        event.kind,
        EventKind::Modify(ModifyKind::Name(RenameMode::Both))
    ) {
        handle_daemon_rename_both_event(root, state, event.paths);
        return;
    }
    if event_requires_full_reconcile(&event.kind) {
        mark_daemon_cache_overflow(state);
        return;
    }

    let mut paths = Vec::new();
    for path in event.paths {
        let Some(path) = daemon_event_relative_path(root, &path) else {
            continue;
        };
        if path == ".crabignore" || path == ".gitignore" {
            mark_daemon_cache_overflow(state);
            return;
        }
        if is_default_ignored(&path) {
            continue;
        }
        paths.push(path);
    }

    if paths.is_empty() {
        return;
    }

    mark_daemon_cache_dirty_paths(state, paths);
}

fn daemon_event_relative_path(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    if rel.as_os_str().is_empty() {
        return None;
    }
    normalize_relative_path(&rel.to_string_lossy()).ok()
}

fn handle_daemon_rename_both_event(
    root: &Path,
    state: &Arc<Mutex<DaemonWorktreeCacheState>>,
    paths: Vec<PathBuf>,
) {
    if paths.len() != 2 {
        mark_daemon_cache_overflow(state);
        return;
    }

    let mut dirty_paths = Vec::new();
    let mut has_existing_path = false;
    for path in paths {
        let Some(path) = daemon_event_relative_path(root, &path) else {
            continue;
        };
        if path == ".crabignore" || path == ".gitignore" {
            mark_daemon_cache_overflow(state);
            return;
        }
        let abs = root.join(path_from_rel(&path));
        match fs::symlink_metadata(&abs) {
            Ok(metadata) if metadata.is_dir() || metadata.is_file() => {
                has_existing_path = true;
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                mark_daemon_cache_overflow(state);
                return;
            }
        }
        if !is_default_ignored(&path) {
            dirty_paths.push(path);
        }
    }

    if !has_existing_path && !dirty_paths.is_empty() {
        mark_daemon_cache_overflow(state);
        return;
    }
    if dirty_paths.is_empty() {
        return;
    }
    mark_daemon_cache_dirty_paths(state, dirty_paths);
}

fn event_requires_full_reconcile(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Any
            | EventKind::Other
            | EventKind::Create(CreateKind::Any | CreateKind::Other)
            | EventKind::Remove(RemoveKind::Any | RemoveKind::Other)
            | EventKind::Modify(ModifyKind::Any | ModifyKind::Name(_) | ModifyKind::Other)
    )
}

fn mark_daemon_cache_dirty_paths(state: &Arc<Mutex<DaemonWorktreeCacheState>>, paths: Vec<String>) {
    let mut state = state.lock().expect("daemon worktree cache poisoned");
    for path in paths {
        state.dirty_paths.insert(path);
    }
    state.baseline_root_id = None;
    state.generation = state.generation.saturating_add(1);
}

fn mark_daemon_cache_overflow(state: &Arc<Mutex<DaemonWorktreeCacheState>>) {
    let mut state = state.lock().expect("daemon worktree cache poisoned");
    state.overflow = true;
    state.baseline_root_id = None;
    state.generation = state.generation.saturating_add(1);
}

impl DaemonWorktreeCacheWarmup {
    pub fn run(self) -> Result<()> {
        let db = CrabDb::open_with_db_dir(&self.workspace_root, &self.db_dir)?;
        db.finish_daemon_worktree_cache_baseline(&self.state, self.generation)
    }
}

fn summary_paths(summaries: &[FileDiffSummary]) -> Vec<String> {
    let mut paths = BTreeSet::new();
    for summary in summaries {
        paths.insert(summary.path.clone());
        if let Some(old_path) = &summary.old_path {
            paths.insert(old_path.clone());
        }
    }
    paths.into_iter().collect()
}

fn notify_error(err: notify::Error) -> Error {
    Error::InvalidInput(format!("daemon worktree watcher failed: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_rename_both_tracks_file_paths_without_overflow() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("renamed.txt"), "renamed\n").unwrap();
        let state = Arc::new(Mutex::new(DaemonWorktreeCacheState {
            initialized: true,
            baseline_root_id: Some(ObjectId("root".to_string())),
            ..DaemonWorktreeCacheState::default()
        }));
        let event = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
            .add_path(temp.path().join("old.txt"))
            .add_path(temp.path().join("renamed.txt"));

        handle_daemon_watch_event(temp.path(), &state, Ok(event));

        let state = state.lock().unwrap();
        assert!(!state.overflow);
        assert_eq!(state.baseline_root_id, None);
        assert!(state.dirty_paths.contains("old.txt"));
        assert!(state.dirty_paths.contains("renamed.txt"));
    }

    #[test]
    fn daemon_rename_both_tracks_directory_prefixes_without_overflow() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("renamed")).unwrap();
        let state = Arc::new(Mutex::new(DaemonWorktreeCacheState {
            initialized: true,
            baseline_root_id: Some(ObjectId("root".to_string())),
            ..DaemonWorktreeCacheState::default()
        }));
        let event = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
            .add_path(temp.path().join("old"))
            .add_path(temp.path().join("renamed"));

        handle_daemon_watch_event(temp.path(), &state, Ok(event));

        let state = state.lock().unwrap();
        assert!(!state.overflow);
        assert_eq!(state.baseline_root_id, None);
        assert!(state.dirty_paths.contains("old"));
        assert!(state.dirty_paths.contains("renamed"));
    }

    #[test]
    fn daemon_folder_events_mark_prefixes_without_overflow() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("created")).unwrap();
        let state = Arc::new(Mutex::new(DaemonWorktreeCacheState {
            initialized: true,
            baseline_root_id: Some(ObjectId("root".to_string())),
            ..DaemonWorktreeCacheState::default()
        }));

        handle_daemon_watch_event(
            temp.path(),
            &state,
            Ok(Event::new(EventKind::Create(CreateKind::Folder))
                .add_path(temp.path().join("created"))),
        );
        handle_daemon_watch_event(
            temp.path(),
            &state,
            Ok(Event::new(EventKind::Remove(RemoveKind::Folder))
                .add_path(temp.path().join("removed"))),
        );

        let state = state.lock().unwrap();
        assert!(!state.overflow);
        assert_eq!(state.baseline_root_id, None);
        assert!(state.dirty_paths.contains("created"));
        assert!(state.dirty_paths.contains("removed"));
    }

    #[test]
    fn selected_worktree_snapshot_supports_directory_prefixes() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("src")).unwrap();
        fs::write(temp.path().join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
        CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = CrabDb::open(temp.path()).unwrap();
        let head = db.resolve_branch_ref("main").unwrap();
        fs::remove_dir_all(temp.path().join("src")).unwrap();
        let removed = db
            .selected_worktree_snapshot_for_root(&head.root_id, &["src".to_string()])
            .unwrap();
        assert_eq!(removed.summaries.len(), 1);
        assert_eq!(removed.summaries[0].path, "src/lib.rs");
        assert_eq!(removed.summaries[0].kind, FileChangeKind::Deleted);

        fs::create_dir(temp.path().join("generated")).unwrap();
        fs::write(temp.path().join("generated/new.rs"), "pub fn new() {}\n").unwrap();
        let added = db
            .selected_worktree_snapshot_for_root(&head.root_id, &["generated".to_string()])
            .unwrap();
        assert_eq!(added.summaries.len(), 1);
        assert_eq!(added.summaries[0].path, "generated/new.rs");
        assert_eq!(added.summaries[0].kind, FileChangeKind::Added);
    }

    #[test]
    fn read_worktree_index_candidates_hashes_changed_files_in_batch() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("a.txt"), "a1\n").unwrap();
        fs::write(temp.path().join("b.txt"), "b1\n").unwrap();
        let candidates = ["a.txt", "b.txt"]
            .into_iter()
            .map(|path| {
                let abs_path = temp.path().join(path);
                let stamp = worktree_file_stamp(&fs::symlink_metadata(&abs_path).unwrap());
                WorktreeIndexReadCandidate {
                    path: path.to_string(),
                    abs_path,
                    stamp,
                }
            })
            .collect::<Vec<_>>();
        let text_config = TextConfig {
            small_text_max_bytes: 64 * 1024,
            tree_text_min_bytes: 64 * 1024,
            opaque_text_max_bytes: 1024 * 1024,
            max_line_bytes: 1024,
            preserve_similarity: 0.0,
        };

        let updates = read_worktree_index_candidates(&candidates, &text_config).unwrap();

        let updates = updates
            .into_iter()
            .map(|update| (update.path.clone(), update))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            updates["a.txt"].disk_manifest.content_hash,
            sha256_hex(b"a1\n")
        );
        assert_eq!(updates["a.txt"].disk_manifest.kind, FileKind::Text);
        assert_eq!(
            updates["b.txt"].disk_manifest.content_hash,
            sha256_hex(b"b1\n")
        );
        assert_eq!(updates["b.txt"].disk_manifest.kind, FileKind::Text);
    }

    #[test]
    fn daemon_diff_dirty_handles_deleted_directory_prefix() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("src")).unwrap();
        fs::write(temp.path().join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
        CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let mut db = CrabDb::open(temp.path()).unwrap();
        install_dirty_daemon_cache(&mut db, &["src"]);
        fs::remove_dir_all(temp.path().join("src")).unwrap();

        let diff = db.diff_dirty(false, false).unwrap();
        assert_eq!(diff.files.len(), 1);
        assert_eq!(diff.files[0].path, "src/lib.rs");
        assert_eq!(diff.files[0].kind, FileChangeKind::Deleted);
    }

    #[test]
    fn daemon_record_handles_deleted_directory_prefix() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("src")).unwrap();
        fs::write(temp.path().join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
        CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let mut db = CrabDb::open(temp.path()).unwrap();
        install_dirty_daemon_cache(&mut db, &["src"]);
        fs::remove_dir_all(temp.path().join("src")).unwrap();

        let recorded = db
            .record(
                Some("main"),
                Some("record deleted directory".to_string()),
                Actor::human(),
                false,
            )
            .unwrap();
        assert!(recorded.operation.is_some());
        assert_eq!(recorded.changed_paths.len(), 1);
        assert_eq!(recorded.changed_paths[0].path, "src/lib.rs");
        assert_eq!(recorded.changed_paths[0].kind, FileChangeKind::Deleted);
    }

    fn install_dirty_daemon_cache(db: &mut CrabDb, dirty_paths: &[&str]) {
        let dirty_paths = dirty_paths
            .iter()
            .map(|path| path.to_string())
            .collect::<BTreeSet<_>>();
        let watcher = RecommendedWatcher::new(|_event| {}, NotifyConfig::default()).unwrap();
        db.daemon_worktree_cache = Some(DaemonWorktreeCache {
            state: Arc::new(Mutex::new(DaemonWorktreeCacheState {
                dirty_paths,
                overflow: false,
                initialized: true,
                baseline_root_id: None,
                generation: 1,
            })),
            _watcher: watcher,
        });
    }
}
