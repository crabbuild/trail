use super::*;
use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};
use notify::{
    Config as NotifyConfig, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use rayon::prelude::*;

const DAEMON_STATUS_DIRTY_PATH_LIMIT: usize = 16_384;
const WORKTREE_INDEX_BASELINE_ROOT_KEY: &str = "worktree.index.baseline_root";
const DAEMON_WORKTREE_SNAPSHOT_FILE: &str = "worktree-daemon-cache.json";
const DAEMON_WORKTREE_SNAPSHOT_VERSION: u32 = 1;

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

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PersistedDaemonWorktreeSnapshot {
    version: u32,
    pid: u32,
    workspace_root: String,
    generation: u64,
    initialized: bool,
    overflow: bool,
    baseline_root_id: Option<String>,
    dirty_paths: Vec<String>,
    updated_ns: i64,
}

impl CrabDb {
    pub fn enable_daemon_worktree_cache(&mut self) -> Result<()> {
        let warmup = self.start_daemon_worktree_cache()?;
        warmup.run()
    }

    pub fn start_daemon_worktree_cache(&mut self) -> Result<DaemonWorktreeCacheWarmup> {
        let cache = DaemonWorktreeCache::start(&self.workspace_root, &self.db_dir)?;
        let warmup = DaemonWorktreeCacheWarmup {
            workspace_root: self.workspace_root.clone(),
            db_dir: self.db_dir.clone(),
            state: Arc::clone(&cache.state),
            persist: cache.persist.clone(),
            generation: cache.generation(),
        };
        self.daemon_worktree_cache = Some(cache);
        Ok(warmup)
    }

    fn finish_daemon_worktree_cache_baseline(
        &self,
        state: &Arc<Mutex<DaemonWorktreeCacheState>>,
        persist: Option<&DaemonWorktreeCachePersist>,
        generation: u64,
    ) -> Result<()> {
        let branch = self.current_branch()?;
        let head = self.resolve_branch_ref(&branch)?;
        let changed_paths = self.status_changed_paths_uncached(&branch, &branch, &head.root_id)?;
        DaemonWorktreeCache::finish_initial_baseline(
            state,
            persist,
            generation,
            &head.root_id,
            &changed_paths,
        );
        Ok(())
    }

    pub(crate) fn daemon_worktree_snapshot(&self) -> Option<DaemonWorktreeSnapshot> {
        if let Some(snapshot) = self
            .daemon_worktree_cache
            .as_ref()
            .map(DaemonWorktreeCache::snapshot)
        {
            return Some(snapshot);
        }
        self.persisted_daemon_worktree_snapshot().ok().flatten()
    }

    pub(crate) fn daemon_dirty_path_limit(&self) -> usize {
        DAEMON_STATUS_DIRTY_PATH_LIMIT
    }

    fn persisted_daemon_worktree_snapshot(&self) -> Result<Option<DaemonWorktreeSnapshot>> {
        let path = daemon_worktree_snapshot_path(&self.db_dir);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(Error::Io(err)),
        };
        let snapshot: PersistedDaemonWorktreeSnapshot = serde_json::from_slice(&bytes)?;
        if snapshot.version != DAEMON_WORKTREE_SNAPSHOT_VERSION
            || snapshot.workspace_root != self.workspace_root.to_string_lossy()
            || !snapshot.initialized
        {
            return Ok(None);
        }
        if !daemon_snapshot_process_is_alive(snapshot.pid) {
            let _ = fs::remove_file(path);
            return Ok(None);
        }
        if snapshot.overflow {
            return Ok(Some(DaemonWorktreeSnapshot::Overflow {
                generation: snapshot.generation,
            }));
        }
        if snapshot.dirty_paths.is_empty() {
            return Ok(Some(DaemonWorktreeSnapshot::Clean {
                generation: snapshot.generation,
                root_id: snapshot.baseline_root_id.map(ObjectId),
            }));
        }
        Ok(Some(DaemonWorktreeSnapshot::Dirty {
            generation: snapshot.generation,
            paths: snapshot.dirty_paths,
        }))
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
        let indexed_entries = self.worktree_index_count()?;
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
        let mut indexed_seen = 0u64;
        let mut read_candidates = Vec::new();
        let mut cached_stmt = self.conn.prepare(
            "SELECT size_bytes, modified_ns, changed_ns, device_id, inode, executable \
             FROM worktree_file_index WHERE path = ?1",
        )?;
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
            if let Some(cached_stamp) = cached_worktree_file_stamp(&mut cached_stmt, &rel)? {
                indexed_seen += 1;
                if cached_stamp == stamp {
                    count += 1;
                    continue;
                }
            }

            read_candidates.push(WorktreeIndexReadCandidate {
                path: rel,
                abs_path: path.to_path_buf(),
                stamp,
            });
            count += 1;
        }
        drop(cached_stmt);

        let has_deleted_index_entries = indexed_seen < indexed_entries;
        let changed = !read_candidates.is_empty() || has_deleted_index_entries;
        let updates = read_worktree_index_candidates(&read_candidates, &self.config.text)?;
        for update in updates {
            self.upsert_worktree_index_manifest_for_scan(
                &update.path,
                update.stamp,
                &update.disk_manifest,
                scan_id,
            )?;
        }
        if has_deleted_index_entries {
            let seen = self.scan_visible_worktree_paths()?;
            self.prune_worktree_index(&seen)?;
        }
        if changed {
            self.clear_worktree_index_baseline()?;
        }
        Ok(WorktreeIndexRefresh {
            files: count,
            changed,
        })
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

    fn scan_visible_worktree_paths(&self) -> Result<BTreeSet<String>> {
        let root = self.workspace_root.canonicalize()?;
        let mut builder = WalkBuilder::new(&root);
        builder
            .hidden(false)
            .git_ignore(self.config.recording.ignore_gitignored)
            .git_exclude(self.config.recording.ignore_gitignored)
            .git_global(self.config.recording.ignore_gitignored)
            .add_custom_ignore_filename(".crabignore");

        let walker = builder.build();
        let mut paths = BTreeSet::new();
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
            paths.insert(rel);
        }
        Ok(paths)
    }

    pub(crate) fn update_worktree_index_from_disk_files_and_manifest(
        &self,
        files: &[DiskFile],
        manifests: &BTreeMap<String, DiskManifest>,
    ) -> Result<()> {
        let paths = files
            .iter()
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        self.update_worktree_index_from_paths_and_manifest(&paths, manifests)
    }

    pub(crate) fn update_worktree_index_from_paths_and_manifest(
        &self,
        paths: &[String],
        manifests: &BTreeMap<String, DiskManifest>,
    ) -> Result<()> {
        if !paths.is_empty() {
            self.clear_worktree_index_baseline()?;
        }
        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let result =
            self.update_worktree_index_from_paths_and_manifest_in_transaction(paths, manifests);
        if result.is_ok() {
            self.conn.execute_batch("COMMIT;")?;
        } else {
            let _ = self.conn.execute_batch("ROLLBACK;");
        }
        result
    }

    fn update_worktree_index_from_paths_and_manifest_in_transaction(
        &self,
        paths: &[String],
        manifests: &BTreeMap<String, DiskManifest>,
    ) -> Result<()> {
        let scan_id = worktree_scan_id();
        let now = now_ts();
        let mut upsert = self.conn.prepare_cached(
            "INSERT OR REPLACE INTO worktree_file_index \
             (path, size_bytes, modified_ns, changed_ns, device_id, inode, executable, kind, content_hash, last_seen_scan, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )?;
        for path in paths {
            let abs = self.workspace_root.join(path_from_rel(path));
            let metadata = match fs::symlink_metadata(&abs) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    self.delete_worktree_index_path(path)?;
                    continue;
                }
                Err(err) => return Err(Error::Io(err)),
            };
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                self.delete_worktree_index_path(path)?;
                continue;
            }
            let stamp = worktree_file_stamp(&metadata);
            let disk_manifest = manifests.get(path).ok_or_else(|| {
                Error::Corrupt(format!("missing computed disk manifest for `{}`", path))
            })?;
            upsert.execute(params![
                path.as_str(),
                stamp.size_bytes as i64,
                stamp.modified_ns,
                stamp.changed_ns,
                stamp.device_id,
                stamp.inode,
                i64::from(stamp.executable),
                file_kind_index_label(&disk_manifest.kind),
                disk_manifest.content_hash.as_str(),
                scan_id,
                now
            ])?;
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

    pub(crate) fn cached_worktree_manifest_for_metadata(
        &self,
        path: &str,
        metadata: &fs::Metadata,
    ) -> Result<Option<DiskManifest>> {
        self.cached_worktree_manifest(path, worktree_file_stamp(metadata))
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

fn cached_worktree_file_stamp(
    stmt: &mut rusqlite::Statement<'_>,
    path: &str,
) -> Result<Option<WorktreeFileStamp>> {
    stmt.query_row(params![path], |row| {
        Ok(WorktreeFileStamp {
            size_bytes: row.get::<_, i64>(0)?.max(0) as u64,
            modified_ns: row.get(1)?,
            changed_ns: row.get(2)?,
            device_id: row.get(3)?,
            inode: row.get(4)?,
            executable: row.get::<_, i64>(5)? != 0,
        })
    })
    .optional()
    .map_err(Error::from)
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
    fn start(workspace_root: &Path, db_dir: &Path) -> Result<Self> {
        let state = Arc::new(Mutex::new(DaemonWorktreeCacheState::default()));
        let root = workspace_root.to_path_buf();
        let persist = DaemonWorktreeCachePersist {
            path: daemon_worktree_snapshot_path(db_dir),
            workspace_root: workspace_root.to_path_buf(),
            pid: std::process::id(),
        };
        persist_daemon_worktree_state(&persist, &state);
        let state_for_watcher = Arc::clone(&state);
        let persist_for_watcher = persist.clone();
        let mut watcher = RecommendedWatcher::new(
            move |event| {
                handle_daemon_watch_event(
                    &root,
                    &state_for_watcher,
                    Some(&persist_for_watcher),
                    event,
                )
            },
            NotifyConfig::default(),
        )
        .map_err(notify_error)?;
        watcher
            .watch(workspace_root, RecursiveMode::Recursive)
            .map_err(notify_error)?;
        Ok(Self {
            state,
            persist: Some(persist),
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
        persist: Option<&DaemonWorktreeCachePersist>,
        generation: u64,
        root_id: &ObjectId,
        summaries: &[FileDiffSummary],
    ) {
        {
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
        if let Some(persist) = persist {
            persist_daemon_worktree_state(persist, state);
        }
    }

    fn reconcile_selected(
        &self,
        root_id: &ObjectId,
        checked_paths: &[String],
        summaries: &[FileDiffSummary],
        generation: u64,
    ) {
        {
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
        if let Some(persist) = &self.persist {
            persist_daemon_worktree_state(persist, &self.state);
        }
    }

    fn reconcile_full(
        &self,
        root_id: &ObjectId,
        summaries: &[FileDiffSummary],
        generation: Option<u64>,
    ) {
        {
            let mut state = self.state.lock().expect("daemon worktree cache poisoned");
            if generation.is_some_and(|expected| expected != state.generation) {
                for path in summary_paths(summaries) {
                    state.dirty_paths.insert(path);
                }
                state.baseline_root_id = None;
            } else {
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
        if let Some(persist) = &self.persist {
            persist_daemon_worktree_state(persist, &self.state);
        }
    }
}

fn handle_daemon_watch_event(
    root: &Path,
    state: &Arc<Mutex<DaemonWorktreeCacheState>>,
    persist: Option<&DaemonWorktreeCachePersist>,
    event: notify::Result<Event>,
) {
    let Ok(event) = event else {
        mark_daemon_cache_overflow(state, persist);
        return;
    };
    if matches!(event.kind, EventKind::Access(_)) {
        return;
    }
    if daemon_event_paths_all_default_ignored(root, &event.paths) {
        return;
    }
    if daemon_event_touches_ignore_file(root, &event.paths) {
        mark_daemon_cache_overflow(state, persist);
        return;
    }
    if matches!(
        event.kind,
        EventKind::Modify(ModifyKind::Name(RenameMode::Both))
    ) {
        handle_daemon_rename_both_event(root, state, persist, event.paths);
        return;
    }
    if event_requires_full_reconcile(&event.kind) {
        mark_daemon_cache_overflow(state, persist);
        return;
    }

    let mut paths = Vec::new();
    for path in event.paths {
        let Some(path) = daemon_event_relative_path(root, &path) else {
            continue;
        };
        if is_default_ignored(&path) {
            continue;
        }
        paths.push(path);
    }

    if paths.is_empty() {
        return;
    }

    mark_daemon_cache_dirty_paths(state, persist, paths);
}

fn daemon_event_relative_path(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    if rel.as_os_str().is_empty() {
        return None;
    }
    normalize_relative_path(&rel.to_string_lossy()).ok()
}

fn daemon_event_paths_all_default_ignored(root: &Path, paths: &[PathBuf]) -> bool {
    !paths.is_empty()
        && paths.iter().all(|path| {
            daemon_event_relative_path(root, path).is_some_and(|path| is_default_ignored(&path))
        })
}

fn daemon_event_touches_ignore_file(root: &Path, paths: &[PathBuf]) -> bool {
    paths.iter().any(|path| {
        daemon_event_relative_path(root, path)
            .is_some_and(|path| path == ".crabignore" || path == ".gitignore")
    })
}

fn handle_daemon_rename_both_event(
    root: &Path,
    state: &Arc<Mutex<DaemonWorktreeCacheState>>,
    persist: Option<&DaemonWorktreeCachePersist>,
    paths: Vec<PathBuf>,
) {
    if paths.len() != 2 {
        mark_daemon_cache_overflow(state, persist);
        return;
    }

    let mut dirty_paths = Vec::new();
    let mut has_existing_path = false;
    for path in paths {
        let Some(path) = daemon_event_relative_path(root, &path) else {
            continue;
        };
        let abs = root.join(path_from_rel(&path));
        match fs::symlink_metadata(&abs) {
            Ok(metadata) if metadata.is_dir() || metadata.is_file() => {
                has_existing_path = true;
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                mark_daemon_cache_overflow(state, persist);
                return;
            }
        }
        if !is_default_ignored(&path) {
            dirty_paths.push(path);
        }
    }

    if !has_existing_path && !dirty_paths.is_empty() {
        mark_daemon_cache_overflow(state, persist);
        return;
    }
    if dirty_paths.is_empty() {
        return;
    }
    mark_daemon_cache_dirty_paths(state, persist, dirty_paths);
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

fn mark_daemon_cache_dirty_paths(
    state: &Arc<Mutex<DaemonWorktreeCacheState>>,
    persist: Option<&DaemonWorktreeCachePersist>,
    paths: Vec<String>,
) {
    {
        let mut state = state.lock().expect("daemon worktree cache poisoned");
        for path in paths {
            state.dirty_paths.insert(path);
        }
        state.baseline_root_id = None;
        state.generation = state.generation.saturating_add(1);
    }
    if let Some(persist) = persist {
        persist_daemon_worktree_state(persist, state);
    }
}

fn mark_daemon_cache_overflow(
    state: &Arc<Mutex<DaemonWorktreeCacheState>>,
    persist: Option<&DaemonWorktreeCachePersist>,
) {
    {
        let mut state = state.lock().expect("daemon worktree cache poisoned");
        state.overflow = true;
        state.baseline_root_id = None;
        state.generation = state.generation.saturating_add(1);
    }
    if let Some(persist) = persist {
        persist_daemon_worktree_state(persist, state);
    }
}

impl DaemonWorktreeCacheWarmup {
    pub fn run(self) -> Result<()> {
        let db = CrabDb::open_with_db_dir(&self.workspace_root, &self.db_dir)?;
        db.finish_daemon_worktree_cache_baseline(
            &self.state,
            self.persist.as_ref(),
            self.generation,
        )
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

fn daemon_worktree_snapshot_path(db_dir: &Path) -> PathBuf {
    db_dir.join(DAEMON_WORKTREE_SNAPSHOT_FILE)
}

fn persist_daemon_worktree_state(
    persist: &DaemonWorktreeCachePersist,
    state: &Arc<Mutex<DaemonWorktreeCacheState>>,
) {
    let snapshot = {
        let state = state.lock().expect("daemon worktree cache poisoned");
        PersistedDaemonWorktreeSnapshot {
            version: DAEMON_WORKTREE_SNAPSHOT_VERSION,
            pid: persist.pid,
            workspace_root: persist.workspace_root.to_string_lossy().to_string(),
            generation: state.generation,
            initialized: state.initialized,
            overflow: state.overflow,
            baseline_root_id: state.baseline_root_id.as_ref().map(|id| id.0.clone()),
            dirty_paths: state.dirty_paths.iter().cloned().collect(),
            updated_ns: worktree_scan_id(),
        }
    };
    let _ = write_persisted_daemon_worktree_snapshot(&persist.path, &snapshot, persist.pid);
}

fn write_persisted_daemon_worktree_snapshot(
    path: &Path,
    snapshot: &PersistedDaemonWorktreeSnapshot,
    pid: u32,
) -> Result<()> {
    let tmp = path.with_file_name(format!("{DAEMON_WORKTREE_SNAPSHOT_FILE}.{pid}.tmp"));
    fs::write(&tmp, serde_json::to_vec(snapshot)?)?;
    fs::rename(tmp, path)?;
    Ok(())
}

#[cfg(unix)]
fn daemon_snapshot_process_is_alive(pid: u32) -> bool {
    Command::new("/bin/kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(not(unix))]
fn daemon_snapshot_process_is_alive(_pid: u32) -> bool {
    true
}

impl Drop for DaemonWorktreeCache {
    fn drop(&mut self) {
        if let Some(persist) = &self.persist {
            let _ = fs::remove_file(&persist.path);
        }
    }
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

        handle_daemon_watch_event(temp.path(), &state, None, Ok(event));

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

        handle_daemon_watch_event(temp.path(), &state, None, Ok(event));

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
            None,
            Ok(Event::new(EventKind::Create(CreateKind::Folder))
                .add_path(temp.path().join("created"))),
        );
        handle_daemon_watch_event(
            temp.path(),
            &state,
            None,
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
    fn persisted_daemon_worktree_snapshot_is_available_to_second_db_handle() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let mut daemon_db = CrabDb::open(temp.path()).unwrap();
        daemon_db.enable_daemon_worktree_cache().unwrap();
        let head = daemon_db.resolve_branch_ref("main").unwrap();
        let reader = CrabDb::open(temp.path()).unwrap();
        match reader.daemon_worktree_snapshot().unwrap() {
            DaemonWorktreeSnapshot::Clean {
                root_id: Some(root_id),
                ..
            } => assert_eq!(root_id, head.root_id),
            other => panic!("expected persisted clean snapshot, got {other:?}"),
        }

        fs::write(temp.path().join("README.md"), "hello\ndirty\n").unwrap();
        let cache = daemon_db.daemon_worktree_cache.as_ref().unwrap();
        handle_daemon_watch_event(
            temp.path(),
            &cache.state,
            cache.persist.as_ref(),
            Ok(Event::new(EventKind::Modify(ModifyKind::Data(
                notify::event::DataChange::Content,
            )))
            .add_path(temp.path().join("README.md"))),
        );

        let reader = CrabDb::open(temp.path()).unwrap();
        match reader.daemon_worktree_snapshot().unwrap() {
            DaemonWorktreeSnapshot::Dirty { paths, .. } => {
                assert_eq!(paths, vec!["README.md".to_string()]);
            }
            other => panic!("expected persisted dirty snapshot, got {other:?}"),
        }

        drop(daemon_db);
        let reader = CrabDb::open(temp.path()).unwrap();
        assert!(reader.daemon_worktree_snapshot().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn stale_persisted_daemon_worktree_snapshot_is_ignored_and_removed() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = CrabDb::open(temp.path()).unwrap();
        let head = db.resolve_branch_ref("main").unwrap();
        let path = daemon_worktree_snapshot_path(db.db_dir());

        write_persisted_daemon_worktree_snapshot(
            &path,
            &PersistedDaemonWorktreeSnapshot {
                version: DAEMON_WORKTREE_SNAPSHOT_VERSION,
                pid: u32::MAX,
                workspace_root: db.workspace_root.to_string_lossy().to_string(),
                generation: 42,
                initialized: true,
                overflow: false,
                baseline_root_id: Some(head.root_id.0),
                dirty_paths: Vec::new(),
                updated_ns: worktree_scan_id(),
            },
            u32::MAX,
        )
        .unwrap();

        let reader = CrabDb::open(temp.path()).unwrap();
        assert!(reader.daemon_worktree_snapshot().is_none());
        assert!(!path.exists());
    }

    #[test]
    fn corrupt_persisted_daemon_worktree_snapshot_is_ignored() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = CrabDb::open(temp.path()).unwrap();
        let path = daemon_worktree_snapshot_path(db.db_dir());
        fs::write(&path, b"not json").unwrap();

        let reader = CrabDb::open(temp.path()).unwrap();
        assert!(reader.daemon_worktree_snapshot().is_none());
        assert!(path.exists());
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
            persist: None,
            _watcher: watcher,
        });
    }
}
