use super::*;
use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};
use notify::{
    Config as NotifyConfig, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use rayon::prelude::*;
use rusqlite::{Params, Statement, StatementStatus};

const DAEMON_STATUS_DIRTY_PATH_LIMIT: usize = 16_384;
const WORKTREE_INDEX_STAMP_LOOKUP_CHUNK: usize = 512;
const WORKTREE_INDEX_BASELINE_ROOT_KEY: &str = "worktree.index.baseline_root";
const DAEMON_WORKTREE_SNAPSHOT_FILE: &str = "worktree-daemon-cache.json";
const DAEMON_WORKTREE_SNAPSHOT_VERSION: u32 = 1;
const SELECT_WORKTREE_INDEX_EXACT_SQL: &str =
    "SELECT path FROM worktree_file_index WHERE path COLLATE BINARY = ?1";
const SELECT_WORKTREE_INDEX_DESCENDANTS_SQL: &str = "SELECT path FROM worktree_file_index \
     WHERE path COLLATE BINARY >= ?1 AND path COLLATE BINARY < ?2 \
     ORDER BY path COLLATE BINARY";
const DELETE_WORKTREE_INDEX_PATH_SQL: &str =
    "DELETE FROM worktree_file_index WHERE path COLLATE BINARY = ?1";
const UPSERT_WORKTREE_INDEX_PATH_SQL: &str =
    "INSERT OR REPLACE INTO worktree_file_index \
     (path, size_bytes, modified_ns, changed_ns, device_id, inode, executable, kind, content_hash, last_seen_scan, updated_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)";

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

impl Trail {
    /// Returns true only when a clean baseline identifies the same immutable
    /// visible file state as `target_root_id`. Path-invariant indexes and
    /// creator metadata are deliberately excluded because they do not change
    /// materialized bytes, paths, modes, or file identities.
    pub(crate) fn clean_baseline_matches_visible_root(
        &self,
        baseline_root_id: Option<&ObjectId>,
        target_root_id: &ObjectId,
    ) -> bool {
        let Some(baseline_root_id) = baseline_root_id else {
            return false;
        };
        if baseline_root_id == target_root_id {
            return true;
        }
        let Ok(baseline) = self.get_object::<WorktreeRoot>(WORKTREE_ROOT_KIND, baseline_root_id)
        else {
            return false;
        };
        let Ok(target) = self.get_object::<WorktreeRoot>(WORKTREE_ROOT_KIND, target_root_id) else {
            return false;
        };
        baseline.path_map_root == target.path_map_root
            && baseline.file_index_map_root == target.file_index_map_root
            && baseline.file_count == target.file_count
            && baseline.total_text_bytes == target.total_text_bytes
    }

    pub fn enable_daemon_worktree_cache(&mut self) -> Result<()> {
        let warmup = self.start_daemon_worktree_cache()?;
        warmup.run()
    }

    pub fn start_daemon_worktree_cache(&mut self) -> Result<DaemonWorktreeCacheWarmup> {
        let cache = DaemonWorktreeCache::start(
            &self.workspace_root,
            &self.db_dir,
            self.operation_metrics.clone(),
        )?;
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
        let snapshot = if let Some(snapshot) = self
            .daemon_worktree_cache
            .as_ref()
            .map(DaemonWorktreeCache::snapshot)
        {
            Some(snapshot)
        } else {
            self.persisted_daemon_worktree_snapshot().ok().flatten()
        };
        if let Some(snapshot) = &snapshot {
            let path_count = match snapshot {
                DaemonWorktreeSnapshot::Dirty { paths, .. } => {
                    saturating_u64_from_usize(paths.len())
                }
                DaemonWorktreeSnapshot::Clean { .. } | DaemonWorktreeSnapshot::Overflow { .. } => 0,
            };
            self.note_operation_metrics(OperationMetricsDelta {
                input_path_count: path_count,
                canonical_path_count: path_count,
                daemon_snapshot_path_count: path_count,
                ..OperationMetricsDelta::default()
            });
        }
        snapshot
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
        self.note_operation_metrics(OperationMetricsDelta {
            daemon_snapshot_bytes: saturating_u64_from_usize(bytes.len()),
            ..OperationMetricsDelta::default()
        });
        let snapshot: PersistedDaemonWorktreeSnapshot = serde_json::from_slice(&bytes)?;
        if snapshot.version != DAEMON_WORKTREE_SNAPSHOT_VERSION
            || snapshot.workspace_root != self.workspace_root.to_string_lossy()
            || !snapshot.initialized
        {
            return Ok(None);
        }
        if !process_is_alive(snapshot.pid) {
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
        let mut filesystem_metrics = OperationMetricsAccumulator::new(
            self.operation_metrics.as_ref(),
            OperationMetricsDelta {
                full_filesystem_walk_count: 1,
                ..OperationMetricsDelta::default()
            },
        );
        let indexed_entries = self.worktree_index_count()?;
        let root = self.workspace_root.canonicalize()?;
        let mut builder = WalkBuilder::new(&root);
        builder
            .hidden(false)
            .git_ignore(self.config.recording.ignore_gitignored)
            .git_exclude(self.config.recording.ignore_gitignored)
            .git_global(self.config.recording.ignore_gitignored)
            .add_custom_ignore_filename(".trailignore");

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
            filesystem_metrics.delta.filesystem_entry_count = filesystem_metrics
                .delta
                .filesystem_entry_count
                .saturating_add(1);
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

            filesystem_metrics.delta.filesystem_stat_count = filesystem_metrics
                .delta
                .filesystem_stat_count
                .saturating_add(1);
            let metadata = fs::symlink_metadata(path)?;
            let stamp = WorktreeFileStamp::from_metadata(&metadata);
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
        let updates = read_worktree_index_candidates(
            &read_candidates,
            &self.config.text,
            self.operation_metrics.as_ref(),
        )?;
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

    pub(crate) fn scan_worktree_manifest_indexed_with_stamps(
        &self,
    ) -> Result<BTreeMap<String, IndexedDiskManifest>> {
        let mut filesystem_metrics = OperationMetricsAccumulator::new(
            self.operation_metrics.as_ref(),
            OperationMetricsDelta {
                full_filesystem_walk_count: 1,
                ..OperationMetricsDelta::default()
            },
        );
        let root = self.workspace_root.canonicalize()?;
        let mut builder = WalkBuilder::new(&root);
        builder
            .hidden(false)
            .git_ignore(self.config.recording.ignore_gitignored)
            .git_exclude(self.config.recording.ignore_gitignored)
            .git_global(self.config.recording.ignore_gitignored)
            .add_custom_ignore_filename(".trailignore");

        let walker = builder.build();
        let mut manifest = BTreeMap::new();
        let mut seen = BTreeSet::new();
        for item in walker {
            let entry = item.map_err(|err| Error::InvalidInput(err.to_string()))?;
            let path = entry.path();
            if path == root {
                continue;
            }
            filesystem_metrics.delta.filesystem_entry_count = filesystem_metrics
                .delta
                .filesystem_entry_count
                .saturating_add(1);
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

            filesystem_metrics.delta.filesystem_stat_count = filesystem_metrics
                .delta
                .filesystem_stat_count
                .saturating_add(1);
            let metadata = fs::symlink_metadata(path)?;
            let stamp = WorktreeFileStamp::from_metadata(&metadata);
            let disk_manifest = if let Some(cached) = self.cached_worktree_manifest(&rel, stamp)? {
                cached
            } else {
                filesystem_metrics.delta.filesystem_read_count = filesystem_metrics
                    .delta
                    .filesystem_read_count
                    .saturating_add(1);
                let bytes = fs::read(path)?;
                let bytes_len = saturating_u64_from_usize(bytes.len());
                filesystem_metrics.delta.filesystem_read_bytes = filesystem_metrics
                    .delta
                    .filesystem_read_bytes
                    .saturating_add(bytes_len);
                filesystem_metrics.delta.filesystem_hash_count = filesystem_metrics
                    .delta
                    .filesystem_hash_count
                    .saturating_add(1);
                filesystem_metrics.delta.filesystem_hash_bytes = filesystem_metrics
                    .delta
                    .filesystem_hash_bytes
                    .saturating_add(bytes_len);
                let disk_manifest = DiskManifest {
                    kind: classify_file_kind(&bytes, &self.config.text),
                    executable: stamp.executable,
                    content_hash: sha256_hex(&bytes),
                };
                self.upsert_worktree_index_manifest(&rel, stamp, &disk_manifest)?;
                disk_manifest
            };
            seen.insert(rel.clone());
            manifest.insert(
                rel,
                IndexedDiskManifest {
                    manifest: disk_manifest,
                    stamp,
                },
            );
        }
        self.prune_worktree_index(&seen)?;
        Ok(manifest)
    }

    pub(crate) fn workspace_file_stamps_if_entries_match(
        &self,
        files: &BTreeMap<String, FileEntry>,
    ) -> Result<Option<BTreeMap<String, WorktreeFileStamp>>> {
        let indexed_manifest = self.scan_worktree_manifest_indexed_with_stamps()?;
        let manifest = indexed_manifest
            .iter()
            .map(|(path, indexed)| (path.clone(), indexed.manifest.clone()))
            .collect::<BTreeMap<_, _>>();
        if !self.diff_file_maps_to_manifest(files, &manifest).is_empty() {
            return Ok(None);
        }
        Ok(Some(
            indexed_manifest
                .into_iter()
                .map(|(path, indexed)| (path, indexed.stamp))
                .collect(),
        ))
    }

    pub(crate) fn workspace_file_stamps_if_clean_index_matches(
        &self,
        root_id: &ObjectId,
        files: &BTreeMap<String, FileEntry>,
    ) -> Result<Option<BTreeMap<String, WorktreeFileStamp>>> {
        let baseline = self.worktree_index_baseline_root()?;
        if !self.clean_baseline_matches_visible_root(baseline.as_ref(), root_id) {
            return Ok(None);
        }
        if files.is_empty() {
            return Ok(Some(BTreeMap::new()));
        }

        let paths = files.keys().cloned().collect::<Vec<_>>();
        let indexed = self.cached_worktree_index_entries_for_paths(&paths)?;
        if indexed.len() != files.len() {
            return Ok(None);
        }

        let mut stamps = BTreeMap::new();
        for (path, entry) in files {
            let Some(indexed) = indexed.get(path) else {
                return Ok(None);
            };
            if indexed.manifest.kind != entry.kind
                || indexed.manifest.executable != entry.executable
                || indexed.manifest.content_hash != entry.content_hash
                || indexed.stamp.size_bytes != entry.size_bytes
                || indexed.stamp.executable != entry.executable
            {
                return Ok(None);
            }
            stamps.insert(path.clone(), indexed.stamp);
        }
        Ok(Some(stamps))
    }

    pub(crate) fn cached_worktree_index_entries_for_paths(
        &self,
        paths: &[String],
    ) -> Result<BTreeMap<String, IndexedDiskManifest>> {
        let mut indexed = BTreeMap::new();
        for chunk in paths.chunks(WORKTREE_INDEX_STAMP_LOOKUP_CHUNK) {
            if chunk.is_empty() {
                continue;
            }
            let placeholders = (0..chunk.len()).map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT path, size_bytes, modified_ns, changed_ns, device_id, inode, executable, kind, content_hash \
                 FROM worktree_file_index WHERE path IN ({placeholders})"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows =
                stmt.query_map(params_from_iter(chunk.iter().map(String::as_str)), |row| {
                    let path: String = row.get(0)?;
                    let executable = row.get::<_, i64>(6)? != 0;
                    let kind_label: String = row.get(7)?;
                    let kind = file_kind_from_index(&kind_label).map_err(|err| {
                        rusqlite::Error::FromSqlConversionFailure(
                            7,
                            rusqlite::types::Type::Text,
                            Box::new(err),
                        )
                    })?;
                    Ok((
                        path,
                        IndexedDiskManifest {
                            stamp: WorktreeFileStamp {
                                size_bytes: row.get::<_, i64>(1)?.max(0) as u64,
                                modified_ns: row.get(2)?,
                                changed_ns: row.get(3)?,
                                device_id: row.get(4)?,
                                inode: row.get(5)?,
                                executable,
                            },
                            manifest: DiskManifest {
                                kind,
                                executable,
                                content_hash: row.get(8)?,
                            },
                        },
                    ))
                })?;
            for row in rows {
                let (path, entry) = row.map_err(Error::from)?;
                indexed.insert(path, entry);
            }
        }
        Ok(indexed)
    }

    fn scan_visible_worktree_paths(&self) -> Result<BTreeSet<String>> {
        let mut filesystem_metrics = OperationMetricsAccumulator::new(
            self.operation_metrics.as_ref(),
            OperationMetricsDelta {
                full_filesystem_walk_count: 1,
                ..OperationMetricsDelta::default()
            },
        );
        let root = self.workspace_root.canonicalize()?;
        let mut builder = WalkBuilder::new(&root);
        builder
            .hidden(false)
            .git_ignore(self.config.recording.ignore_gitignored)
            .git_exclude(self.config.recording.ignore_gitignored)
            .git_global(self.config.recording.ignore_gitignored)
            .add_custom_ignore_filename(".trailignore");

        let walker = builder.build();
        let mut paths = BTreeSet::new();
        for item in walker {
            let entry = item.map_err(|err| Error::InvalidInput(err.to_string()))?;
            let path = entry.path();
            if path == root {
                continue;
            }
            filesystem_metrics.delta.filesystem_entry_count = filesystem_metrics
                .delta
                .filesystem_entry_count
                .saturating_add(1);
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
            let stamp = WorktreeFileStamp::from_metadata(&metadata);
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

    /// Synchronize only the selected portion of the worktree cache. The
    /// metrics emitted here are complete for this SQL envelope, not for every
    /// SQLite statement issued by the containing Trail operation.
    pub(crate) fn sync_selected_worktree_index(
        &self,
        selections: &[String],
        paths: &[String],
        manifests: &BTreeMap<String, DiskManifest>,
    ) -> Result<()> {
        if selections.is_empty() && paths.is_empty() {
            return Ok(());
        }

        let mut sqlite_metrics = OperationMetricsAccumulator::new(
            self.operation_metrics.as_ref(),
            OperationMetricsDelta {
                selected_worktree_index_sqlite_envelope_count: 1,
                ..OperationMetricsDelta::default()
            },
        );
        note_selected_index_statement(&mut sqlite_metrics);
        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        sqlite_metrics
            .delta
            .selected_worktree_index_sqlite_transaction_count = sqlite_metrics
            .delta
            .selected_worktree_index_sqlite_transaction_count
            .saturating_add(1);

        let mut pending_row_deletes = 0u64;
        let mut pending_row_upserts = 0u64;
        let result = (|| {
            let minimal_selections = minimal_component_selections(selections);
            let mut cached_paths = BTreeSet::new();
            {
                let mut exact = self.conn.prepare(SELECT_WORKTREE_INDEX_EXACT_SQL)?;
                let mut descendants = self.conn.prepare(SELECT_WORKTREE_INDEX_DESCENDANTS_SQL)?;
                for selection in minimal_selections {
                    cached_paths.extend(query_selected_index_paths(
                        &mut exact,
                        params![selection.as_str()],
                        &mut sqlite_metrics,
                    )?);
                    let (lower, upper) = selected_path_descendant_bounds(&selection);
                    cached_paths.extend(query_selected_index_paths(
                        &mut descendants,
                        params![lower, upper],
                        &mut sqlite_metrics,
                    )?);
                }
            }

            let seen = paths.iter().map(String::as_str).collect::<HashSet<_>>();
            let paths_to_delete = cached_paths
                .into_iter()
                .filter(|path| !seen.contains(path.as_str()))
                .collect::<Vec<_>>();
            if paths_to_delete.is_empty() && paths.is_empty() {
                return Ok(());
            }

            {
                let mut clear_baseline = self
                    .conn
                    .prepare("DELETE FROM schema_meta WHERE key = ?1")?;
                execute_selected_index_statement(
                    &mut clear_baseline,
                    params![WORKTREE_INDEX_BASELINE_ROOT_KEY],
                    &mut sqlite_metrics,
                )?;
            }

            let scan_id = worktree_scan_id();
            let now = now_ts();
            let mut delete = self.conn.prepare(DELETE_WORKTREE_INDEX_PATH_SQL)?;
            let mut upsert = self.conn.prepare(UPSERT_WORKTREE_INDEX_PATH_SQL)?;
            for path in paths_to_delete {
                let deleted = execute_selected_index_statement(
                    &mut delete,
                    params![path],
                    &mut sqlite_metrics,
                )?;
                pending_row_deletes =
                    pending_row_deletes.saturating_add(saturating_u64_from_usize(deleted));
            }
            for path in paths {
                let abs = self.workspace_root.join(path_from_rel(path));
                let metadata = match fs::symlink_metadata(&abs) {
                    Ok(metadata) => metadata,
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                        let deleted = execute_selected_index_statement(
                            &mut delete,
                            params![path],
                            &mut sqlite_metrics,
                        )?;
                        pending_row_deletes =
                            pending_row_deletes.saturating_add(saturating_u64_from_usize(deleted));
                        continue;
                    }
                    Err(err) => return Err(Error::Io(err)),
                };
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    let deleted = execute_selected_index_statement(
                        &mut delete,
                        params![path],
                        &mut sqlite_metrics,
                    )?;
                    pending_row_deletes =
                        pending_row_deletes.saturating_add(saturating_u64_from_usize(deleted));
                    continue;
                }
                let stamp = WorktreeFileStamp::from_metadata(&metadata);
                let disk_manifest = manifests.get(path).ok_or_else(|| {
                    Error::Corrupt(format!("missing computed disk manifest for `{}`", path))
                })?;
                let upserted = execute_selected_index_statement(
                    &mut upsert,
                    params![
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
                    ],
                    &mut sqlite_metrics,
                )?;
                pending_row_upserts =
                    pending_row_upserts.saturating_add(saturating_u64_from_usize(upserted));
            }
            Ok(())
        })();

        match result {
            Ok(()) => {
                note_selected_index_statement(&mut sqlite_metrics);
                if let Err(err) = self.conn.execute_batch("COMMIT;") {
                    note_selected_index_statement(&mut sqlite_metrics);
                    let _ = self.conn.execute_batch("ROLLBACK;");
                    return Err(Error::from(err));
                }
                sqlite_metrics
                    .delta
                    .selected_worktree_index_sqlite_row_delete_count = pending_row_deletes;
                sqlite_metrics
                    .delta
                    .selected_worktree_index_sqlite_row_upsert_count = pending_row_upserts;
                Ok(())
            }
            Err(err) => {
                note_selected_index_statement(&mut sqlite_metrics);
                let _ = self.conn.execute_batch("ROLLBACK;");
                Err(err)
            }
        }
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
        self.cached_worktree_manifest(path, WorktreeFileStamp::from_metadata(metadata))
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

fn minimal_component_selections(selections: &[String]) -> Vec<String> {
    let unique = selections
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut minimal = BTreeSet::new();
    for selection in unique {
        let covered = selection
            .match_indices('/')
            .any(|(separator, _)| minimal.contains(&selection[..separator]));
        if !covered {
            minimal.insert(selection.to_string());
        }
    }
    minimal.into_iter().collect()
}

fn selected_path_descendant_bounds(selection: &str) -> (String, String) {
    let lower = format!("{selection}/");
    let mut upper = lower.as_bytes().to_vec();
    let terminal_separator = upper
        .last_mut()
        .expect("selected descendant lower bound always ends in slash");
    debug_assert_eq!(*terminal_separator, b'/');
    *terminal_separator = b'0';
    let upper = String::from_utf8(upper)
        .expect("incrementing an ASCII path separator preserves valid UTF-8");
    (lower, upper)
}

fn note_selected_index_statement(metrics: &mut OperationMetricsAccumulator) {
    metrics.delta.selected_worktree_index_sqlite_statement_count = metrics
        .delta
        .selected_worktree_index_sqlite_statement_count
        .saturating_add(1);
}

fn note_selected_index_full_scan(
    statement: &Statement<'_>,
    metrics: &mut OperationMetricsAccumulator,
) {
    if statement.get_status(StatementStatus::FullscanStep) > 0 {
        metrics.delta.selected_worktree_index_sqlite_full_scan_count = metrics
            .delta
            .selected_worktree_index_sqlite_full_scan_count
            .saturating_add(1);
    }
}

fn execute_selected_index_statement<P: Params>(
    statement: &mut Statement<'_>,
    params: P,
    metrics: &mut OperationMetricsAccumulator,
) -> Result<usize> {
    statement.reset_status(StatementStatus::FullscanStep);
    note_selected_index_statement(metrics);
    let result = statement.execute(params).map_err(Error::from);
    note_selected_index_full_scan(statement, metrics);
    result
}

fn query_selected_index_paths<P: Params>(
    statement: &mut Statement<'_>,
    params: P,
    metrics: &mut OperationMetricsAccumulator,
) -> Result<Vec<String>> {
    statement.reset_status(StatementStatus::FullscanStep);
    note_selected_index_statement(metrics);
    let result = (|| -> rusqlite::Result<Vec<String>> {
        let mut rows = statement.query(params)?;
        let mut paths = Vec::new();
        while let Some(row) = rows.next()? {
            paths.push(row.get::<_, String>(0)?);
            metrics.delta.selected_worktree_index_sqlite_row_read_count = metrics
                .delta
                .selected_worktree_index_sqlite_row_read_count
                .saturating_add(1);
        }
        Ok(paths)
    })()
    .map_err(Error::from);
    note_selected_index_full_scan(statement, metrics);
    result
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
    metrics: Option<&Arc<OperationMetricsState>>,
) -> Result<Vec<WorktreeIndexUpdate>> {
    if candidates.len() <= 1 {
        return candidates
            .iter()
            .map(|candidate| read_worktree_index_candidate(candidate, text_config, metrics))
            .collect();
    }

    candidates
        .par_iter()
        .map(|candidate| read_worktree_index_candidate(candidate, text_config, metrics))
        .collect()
}

fn read_worktree_index_candidate(
    candidate: &WorktreeIndexReadCandidate,
    text_config: &TextConfig,
    metrics: Option<&Arc<OperationMetricsState>>,
) -> Result<WorktreeIndexUpdate> {
    if let Some(metrics) = metrics {
        metrics.add(OperationMetricsDelta {
            filesystem_read_count: 1,
            ..OperationMetricsDelta::default()
        });
    }
    let bytes = fs::read(&candidate.abs_path)?;
    if let Some(metrics) = metrics {
        let bytes_len = saturating_u64_from_usize(bytes.len());
        metrics.add(OperationMetricsDelta {
            filesystem_read_bytes: bytes_len,
            filesystem_hash_count: 1,
            filesystem_hash_bytes: bytes_len,
            ..OperationMetricsDelta::default()
        });
    }
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
    fn start(
        workspace_root: &Path,
        db_dir: &Path,
        metrics: Option<Arc<OperationMetricsState>>,
    ) -> Result<Self> {
        let state = Arc::new(Mutex::new(DaemonWorktreeCacheState::default()));
        let root = workspace_root.to_path_buf();
        let persist = DaemonWorktreeCachePersist {
            path: daemon_worktree_snapshot_path(db_dir),
            workspace_root: workspace_root.to_path_buf(),
            pid: std::process::id(),
            active: Arc::new(AtomicBool::new(true)),
            metrics,
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
            watcher: Some(watcher),
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
            .is_some_and(|path| path == ".trailignore" || path == ".gitignore")
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
        let db = Trail::open_with_db_dir(&self.workspace_root, &self.db_dir)?;
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
    if !persist.active.load(Ordering::Acquire) {
        return;
    }
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
    let tmp = persist.path.with_file_name(format!(
        "{DAEMON_WORKTREE_SNAPSHOT_FILE}.{}.tmp",
        persist.pid
    ));
    let Ok(bytes) = serde_json::to_vec(&snapshot) else {
        return;
    };
    if let Some(metrics) = &persist.metrics {
        metrics.note_daemon_cumulative_rewrite(bytes.len());
    }
    if fs::write(&tmp, bytes).is_err() {
        return;
    }
    if !persist.active.load(Ordering::Acquire) {
        let _ = fs::remove_file(tmp);
        return;
    }
    let _ = fs::rename(tmp, &persist.path);
}

#[cfg(all(test, unix))]
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

impl Drop for DaemonWorktreeCache {
    fn drop(&mut self) {
        if let Some(persist) = &self.persist {
            persist.active.store(false, Ordering::Release);
        }
        drop(self.watcher.take());
        if let Some(persist) = &self.persist {
            let _ = fs::remove_file(&persist.path);
            let _ = fs::remove_file(persist.path.with_file_name(format!(
                "{DAEMON_WORKTREE_SNAPSHOT_FILE}.{}.tmp",
                persist.pid
            )));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed_worktree_index_paths(db: &Trail, paths: &[String]) {
        db.conn.execute_batch("BEGIN IMMEDIATE;").unwrap();
        {
            let mut insert = db
                .conn
                .prepare(
                    "INSERT OR REPLACE INTO worktree_file_index \
                     (path, size_bytes, modified_ns, changed_ns, device_id, inode, executable, kind, content_hash, last_seen_scan, updated_at) \
                     VALUES (?1, 1, 1, 1, 1, 1, 0, 'Text', 'seed', 1, 1)",
                )
                .unwrap();
            for path in paths {
                insert.execute(params![path]).unwrap();
            }
        }
        db.conn.execute_batch("COMMIT;").unwrap();
    }

    fn selected_sync_manifest(path: &str, bytes: &[u8]) -> BTreeMap<String, DiskManifest> {
        BTreeMap::from([(
            path.to_string(),
            DiskManifest {
                kind: FileKind::Text,
                executable: false,
                content_hash: sha256_hex(bytes),
            },
        )])
    }

    fn profile_selected_worktree_index_sync(
        db: &Trail,
        selections: &[String],
        paths: &[String],
        manifests: &BTreeMap<String, DiskManifest>,
    ) -> Result<OperationMetricsReport> {
        let metrics = Arc::clone(
            db.operation_metrics
                .as_ref()
                .expect("test operation metrics should be enabled"),
        );
        metrics.profile(OperationMetricsKind::Diff, || {
            db.sync_selected_worktree_index(selections, paths, manifests)
        })?;
        Ok(metrics.last_report())
    }

    fn selected_sync_scale_fixture(
        decoy_count: usize,
    ) -> (OperationMetricsReport, u64, Vec<String>, Option<ObjectId>) {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("live-dir")).unwrap();
        fs::write(temp.path().join("live-dir/keep.txt"), b"live\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();

        let mut seeded = (0..decoy_count)
            .map(|index| format!("decoy/{index:05}.txt"))
            .collect::<Vec<_>>();
        seeded.extend([
            "exact.txt".to_string(),
            "deleted-dir/a.txt".to_string(),
            "deleted-dir/nested/b.txt".to_string(),
            "live-dir/keep.txt".to_string(),
        ]);
        seed_worktree_index_paths(&db, &seeded);
        db.set_worktree_index_baseline(&ObjectId("selected-sync-baseline".to_string()))
            .unwrap();

        let selections = ["exact.txt", "deleted-dir", "live-dir"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let live_path = "live-dir/keep.txt".to_string();
        let paths = vec![live_path.clone()];
        let manifests = selected_sync_manifest(&live_path, b"live\n");
        let report =
            profile_selected_worktree_index_sync(&db, &selections, &paths, &manifests).unwrap();

        let decoys = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM worktree_file_index WHERE path >= 'decoy/' AND path < 'decoy0'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            .max(0) as u64;
        let selected_rows = db
            .conn
            .prepare(
                "SELECT path FROM worktree_file_index \
                 WHERE path = 'exact.txt' \
                    OR (path >= 'deleted-dir/' AND path < 'deleted-dir0') \
                    OR (path >= 'live-dir/' AND path < 'live-dir0') \
                 ORDER BY path",
            )
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        let baseline = db.worktree_index_baseline_root().unwrap();
        (report, decoys, selected_rows, baseline)
    }

    #[test]
    fn selected_worktree_index_sync_is_bounded_independent_of_repository_rows() {
        let (small, small_decoys, small_rows, small_baseline) = selected_sync_scale_fixture(0);
        let (large, large_decoys, large_rows, large_baseline) = selected_sync_scale_fixture(10_000);

        assert_eq!(small_decoys, 0);
        assert_eq!(large_decoys, 10_000);
        assert_eq!(small_rows, vec!["live-dir/keep.txt"]);
        assert_eq!(large_rows, small_rows);
        assert_eq!(small_baseline, None);
        assert_eq!(large_baseline, None);
        for report in [&small, &large] {
            assert!(report.selected_worktree_index_sqlite_accounting_complete);
            assert_eq!(report.selected_worktree_index_sqlite_envelope_count, 1);
            assert_eq!(report.selected_worktree_index_sqlite_full_scan_count, 0);
            assert_eq!(report.selected_worktree_index_sqlite_row_read_count, 4);
            assert_eq!(report.selected_worktree_index_sqlite_row_delete_count, 3);
            assert_eq!(report.selected_worktree_index_sqlite_row_upsert_count, 1);
            assert_eq!(report.selected_worktree_index_sqlite_statement_count, 13);
            assert_eq!(report.selected_worktree_index_sqlite_transaction_count, 1);
        }
    }

    #[test]
    fn selected_worktree_index_true_empty_input_preserves_baseline_without_an_envelope() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let baseline = ObjectId("empty-sync-baseline".to_string());
        db.set_worktree_index_baseline(&baseline).unwrap();

        let report = profile_selected_worktree_index_sync(&db, &[], &[], &BTreeMap::new()).unwrap();

        assert_eq!(db.worktree_index_baseline_root().unwrap(), Some(baseline));
        assert!(!report.selected_worktree_index_sqlite_accounting_complete);
        assert_eq!(report.selected_worktree_index_sqlite_envelope_count, 0);
        assert_eq!(report.selected_worktree_index_sqlite_statement_count, 0);
        assert_eq!(report.selected_worktree_index_sqlite_transaction_count, 0);
    }

    #[test]
    fn selected_worktree_index_queries_use_binary_primary_key_searches() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();

        let explain = |sql: &str, parameters: &[&str]| {
            let sql = format!("EXPLAIN QUERY PLAN {sql}");
            db.conn
                .prepare(&sql)
                .unwrap()
                .query_map(params_from_iter(parameters.iter().copied()), |row| {
                    row.get::<_, String>(3)
                })
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
        };
        let exact = explain(SELECT_WORKTREE_INDEX_EXACT_SQL, &["src"]);
        let range = explain(SELECT_WORKTREE_INDEX_DESCENDANTS_SQL, &["src/", "src0"]);

        assert!(exact
            .iter()
            .any(|detail| detail.contains("SEARCH worktree_file_index")));
        assert!(exact.iter().any(|detail| detail.contains("path=?")));
        assert!(exact
            .iter()
            .all(|detail| !detail.contains("SCAN worktree_file_index")));
        assert!(range
            .iter()
            .any(|detail| detail.contains("SEARCH worktree_file_index")));
        assert!(range
            .iter()
            .any(|detail| { detail.contains("path>?") && detail.contains("path<?") }));
        assert!(range
            .iter()
            .all(|detail| !detail.contains("SCAN worktree_file_index")));
    }

    #[test]
    fn selected_worktree_index_binary_ranges_preserve_unicode_and_special_paths() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let selected = "unicodé_%[目录]";
        let selected_rows = [
            selected.to_string(),
            format!("{selected}/child_%.txt"),
            format!("{selected}/nested/[leaf].txt"),
        ];
        let sibling_rows = [
            format!("{selected}-sibling.txt"),
            format!("{selected}.sibling.txt"),
            format!("{selected}0sibling.txt"),
            "UNICODÉ_%[目录]/case.txt".to_string(),
        ];
        let mut seeded = selected_rows.to_vec();
        seeded.extend(sibling_rows.clone());
        seed_worktree_index_paths(&db, &seeded);

        let report = profile_selected_worktree_index_sync(
            &db,
            &[selected.to_string()],
            &[],
            &BTreeMap::new(),
        )
        .unwrap();

        for path in selected_rows {
            assert_eq!(
                db.conn
                    .query_row(
                        "SELECT COUNT(*) FROM worktree_file_index WHERE path = ?1",
                        params![path],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                0,
                "selected path must be deleted"
            );
        }
        for path in sibling_rows {
            assert_eq!(
                db.conn
                    .query_row(
                        "SELECT COUNT(*) FROM worktree_file_index WHERE path = ?1",
                        params![path],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                1,
                "binary sibling must remain"
            );
        }
        assert_eq!(report.selected_worktree_index_sqlite_full_scan_count, 0);
        assert_eq!(report.selected_worktree_index_sqlite_row_read_count, 3);
        assert_eq!(report.selected_worktree_index_sqlite_row_delete_count, 3);
    }

    #[test]
    fn selected_worktree_index_sync_deduplicates_overlapping_component_selections() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        seed_worktree_index_paths(
            &db,
            &[
                "tree/a.txt".to_string(),
                "tree/sub/b.txt".to_string(),
                "treehouse/keep.txt".to_string(),
            ],
        );
        let selections = ["tree/sub", "tree", "tree/sub/b.txt", "tree"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();

        let report =
            profile_selected_worktree_index_sync(&db, &selections, &[], &BTreeMap::new()).unwrap();

        assert_eq!(report.selected_worktree_index_sqlite_row_read_count, 2);
        assert_eq!(report.selected_worktree_index_sqlite_row_delete_count, 2);
        assert_eq!(report.selected_worktree_index_sqlite_statement_count, 7);
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM worktree_file_index WHERE path = 'treehouse/keep.txt'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
    }

    #[test]
    fn selected_worktree_index_commit_failure_rolls_back_baseline_and_rows() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        fs::create_dir_all(temp.path().join("live")).unwrap();
        fs::write(temp.path().join("live/upsert.txt"), b"live\n").unwrap();
        let db = Trail::open(temp.path()).unwrap();
        seed_worktree_index_paths(&db, &["gone.txt".to_string()]);
        let baseline = ObjectId("rollback-baseline".to_string());
        db.set_worktree_index_baseline(&baseline).unwrap();
        db.conn
            .execute_batch(
                "CREATE TABLE selected_sync_commit_parent (id INTEGER PRIMARY KEY);
                 CREATE TABLE selected_sync_commit_child (
                    parent_id INTEGER REFERENCES selected_sync_commit_parent(id)
                        DEFERRABLE INITIALLY DEFERRED
                 );
                 CREATE TRIGGER selected_sync_fail_commit
                 AFTER INSERT ON worktree_file_index
                 WHEN NEW.path = 'live/upsert.txt'
                 BEGIN
                    INSERT INTO selected_sync_commit_child(parent_id) VALUES (1);
                 END;",
            )
            .unwrap();
        let live_path = "live/upsert.txt".to_string();
        let manifests = selected_sync_manifest(&live_path, b"live\n");
        let metrics = Arc::clone(db.operation_metrics.as_ref().unwrap());

        let result = metrics.profile(OperationMetricsKind::Diff, || {
            db.sync_selected_worktree_index(
                &["gone.txt".to_string(), "live".to_string()],
                &[live_path],
                &manifests,
            )
        });

        assert!(result.is_err());
        let report = metrics.last_report();
        assert!(report.selected_worktree_index_sqlite_accounting_complete);
        assert_eq!(report.selected_worktree_index_sqlite_transaction_count, 1);
        assert_eq!(report.selected_worktree_index_sqlite_row_read_count, 1);
        assert_eq!(report.selected_worktree_index_sqlite_row_delete_count, 0);
        assert_eq!(report.selected_worktree_index_sqlite_row_upsert_count, 0);
        assert_eq!(report.selected_worktree_index_sqlite_statement_count, 10);
        assert_eq!(db.worktree_index_baseline_root().unwrap(), Some(baseline));
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM worktree_file_index WHERE path = 'gone.txt'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM worktree_file_index WHERE path = 'live/upsert.txt'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
    }

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
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let mut daemon_db = Trail::open(temp.path()).unwrap();
        daemon_db.enable_daemon_worktree_cache().unwrap();
        let head = daemon_db.resolve_branch_ref("main").unwrap();
        let reader = Trail::open(temp.path()).unwrap();
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

        let reader = Trail::open(temp.path()).unwrap();
        match reader.daemon_worktree_snapshot().unwrap() {
            DaemonWorktreeSnapshot::Dirty { paths, .. } => {
                assert_eq!(paths, vec!["README.md".to_string()]);
            }
            other => panic!("expected persisted dirty snapshot, got {other:?}"),
        }

        drop(daemon_db);
        let reader = Trail::open(temp.path()).unwrap();
        assert!(reader.daemon_worktree_snapshot().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn stale_persisted_daemon_worktree_snapshot_is_ignored_and_removed() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
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

        let reader = Trail::open(temp.path()).unwrap();
        assert!(reader.daemon_worktree_snapshot().is_none());
        assert!(!path.exists());
    }

    #[test]
    fn corrupt_persisted_daemon_worktree_snapshot_is_ignored() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let path = daemon_worktree_snapshot_path(db.db_dir());
        fs::write(&path, b"not json").unwrap();

        let reader = Trail::open(temp.path()).unwrap();
        assert!(reader.daemon_worktree_snapshot().is_none());
        assert!(path.exists());
    }

    #[test]
    fn selected_worktree_snapshot_supports_directory_prefixes() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("src")).unwrap();
        fs::write(temp.path().join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
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
                let stamp =
                    WorktreeFileStamp::from_metadata(&fs::symlink_metadata(&abs_path).unwrap());
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

        let updates = read_worktree_index_candidates(&candidates, &text_config, None).unwrap();

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

    fn clean_index_stamp_reuse_fixture() -> (
        tempfile::TempDir,
        Trail,
        RefRecord,
        BTreeMap<String, FileEntry>,
    ) {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("a.txt"), "a1\n").unwrap();
        fs::create_dir(temp.path().join("src")).unwrap();
        fs::write(temp.path().join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        db.refresh_worktree_index().unwrap();
        let head = db.resolve_branch_ref("main").unwrap();
        db.set_worktree_index_baseline(&head.root_id).unwrap();
        let files = db.load_root_files(&head.root_id).unwrap();
        (temp, db, head, files)
    }

    #[test]
    fn clean_index_stamp_reuse_returns_stamps_for_matching_baseline() {
        let (_temp, db, head, files) = clean_index_stamp_reuse_fixture();

        let stamps = db
            .workspace_file_stamps_if_clean_index_matches(&head.root_id, &files)
            .unwrap()
            .unwrap();

        assert_eq!(stamps.len(), files.len());
        assert!(stamps.contains_key("a.txt"));
        assert!(stamps.contains_key("src/lib.rs"));
    }

    #[test]
    fn clean_index_stamp_reuse_misses_without_matching_baseline() {
        let (_temp, db, _head, files) = clean_index_stamp_reuse_fixture();

        let stamps = db
            .workspace_file_stamps_if_clean_index_matches(&ObjectId("other".to_string()), &files)
            .unwrap();

        assert!(stamps.is_none());
    }

    #[test]
    fn clean_index_stamp_reuse_misses_when_index_row_is_missing() {
        let (_temp, db, head, files) = clean_index_stamp_reuse_fixture();
        db.delete_worktree_index_path_row("a.txt").unwrap();

        let stamps = db
            .workspace_file_stamps_if_clean_index_matches(&head.root_id, &files)
            .unwrap();

        assert!(stamps.is_none());
    }

    #[test]
    fn clean_index_stamp_reuse_misses_when_index_manifest_differs() {
        let (temp, db, head, files) = clean_index_stamp_reuse_fixture();
        let metadata = fs::symlink_metadata(temp.path().join("a.txt")).unwrap();
        let stamp = WorktreeFileStamp::from_metadata(&metadata);
        db.upsert_worktree_index_manifest_for_scan(
            "a.txt",
            stamp,
            &DiskManifest {
                kind: FileKind::Text,
                executable: false,
                content_hash: sha256_hex(b"different\n"),
            },
            worktree_scan_id(),
        )
        .unwrap();

        let stamps = db
            .workspace_file_stamps_if_clean_index_matches(&head.root_id, &files)
            .unwrap();

        assert!(stamps.is_none());
    }

    #[test]
    fn daemon_diff_dirty_handles_deleted_directory_prefix() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("src")).unwrap();
        fs::write(temp.path().join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let mut db = Trail::open(temp.path()).unwrap();
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
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let mut db = Trail::open(temp.path()).unwrap();
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

    fn install_dirty_daemon_cache(db: &mut Trail, dirty_paths: &[&str]) {
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
            watcher: Some(watcher),
        });
    }
}
