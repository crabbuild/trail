use super::*;
use serde::{Deserialize, Serialize};
use std::thread;

const LAYER_BUILD_LEASE_SECS: i64 = 300;

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceLayerBinding {
    #[allow(dead_code)]
    pub(crate) layer_id: String,
    pub(crate) mount_path: String,
    pub(crate) storage_path: PathBuf,
    #[allow(dead_code)]
    pub(crate) priority: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct WorkspaceLayerManifest {
    version: u16,
    layer_id: String,
    kind: String,
    cache_key: String,
    adapter: String,
    adapter_version: u32,
    logical_bytes: u64,
    entries: BTreeMap<String, WorkspaceLayerEntry>,
    platform: String,
    architecture: String,
    portability_scope: String,
    producer_version: String,
    created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct WorkspaceLayerEntry {
    kind: String,
    mode: u32,
    size_bytes: u64,
    content_hash: Option<String>,
    symlink_target: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WorkspaceLayerPublishMarker {
    layer_id: String,
    cache_key: String,
    manifest_object_id: String,
    logical_bytes: u64,
    physical_bytes: u64,
    entry_count: u64,
}

impl Trail {
    pub fn workspace_layer_cache_key(&self, key: &WorkspaceLayerKeyV1) -> Result<String> {
        validate_layer_key(key)?;
        Ok(sha256_hex(&serde_json::to_vec(key)?))
    }

    pub(crate) fn build_workspace_layer_singleflight<F>(
        &self,
        key: &WorkspaceLayerKeyV1,
        builder: F,
    ) -> Result<WorkspaceLayerReport>
    where
        F: FnOnce(&Path) -> Result<PathBuf>,
    {
        let cache_key = self.workspace_layer_cache_key(key)?;
        if let Some(layer) = self.workspace_layer_by_cache_key(&cache_key)? {
            if layer.state == "ready" {
                return self.verify_workspace_layer(&layer.layer_id);
            }
        }
        let lock_path = self
            .db_dir
            .join("cache/staging/locks")
            .join(format!("{cache_key}.lock"));
        fs::create_dir_all(lock_path.parent().unwrap())?;
        let token = format!("{}:{}", std::process::id(), current_process_start_token());
        let deadline = Instant::now() + Duration::from_secs(LAYER_BUILD_LEASE_SECS as u64);
        let guard = loop {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(mut file) => {
                    file.write_all(token.as_bytes())?;
                    file.sync_all()?;
                    break CacheBuildKeyGuard {
                        path: lock_path.clone(),
                        token: token.clone(),
                    };
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    if let Some(layer) = self.workspace_layer_by_cache_key(&cache_key)? {
                        if layer.state == "ready" {
                            return self.verify_workspace_layer(&layer.layer_id);
                        }
                    }
                    if build_lock_is_stale(&lock_path)? {
                        let stale = lock_path.with_extension(format!("stale.{}", now_ts()));
                        let _ = fs::rename(&lock_path, stale);
                        continue;
                    }
                    if Instant::now() >= deadline {
                        return Err(Error::InvalidInput(format!(
                            "timed out waiting for workspace layer key {cache_key}"
                        )));
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                Err(err) => return Err(Error::Io(err)),
            }
        };
        let builder_limit = self.config().workspace_views.concurrent_cache_builders;
        if builder_limit > 0
            && active_cache_builder_count(lock_path.parent().unwrap())? > builder_limit
        {
            drop(guard);
            return Err(Error::InvalidInput(format!(
                "workspace cache builder quota exceeded (limit {builder_limit})"
            )));
        }
        if let Some(layer) = self.workspace_layer_by_cache_key(&cache_key)? {
            if layer.state == "ready" {
                drop(guard);
                return self.verify_workspace_layer(&layer.layer_id);
            }
        }
        let build_dir = self.db_dir.join("cache/staging").join(format!(
            "input_{}",
            crate::ids::short_hash(format!("{cache_key}:{token}").as_bytes(), 12)
        ));
        if build_dir.exists() {
            return Err(Error::InvalidInput(format!(
                "workspace layer build directory `{}` already exists",
                build_dir.display()
            )));
        }
        fs::create_dir_all(&build_dir)?;
        let output = match builder(&build_dir) {
            Ok(output) => output,
            Err(err) => {
                make_tree_writable(&build_dir);
                let failed = build_dir.with_extension(format!("failed.{}", now_ts()));
                let _ = fs::rename(&build_dir, failed);
                return Err(err);
            }
        };
        self.enforce_workspace_cache_build_quota(&output)?;
        let report = self.publish_workspace_layer_from_directory(key, &output)?;
        make_tree_writable(&build_dir);
        let _ = fs::remove_dir_all(&build_dir);
        drop(guard);
        Ok(report)
    }

    /// Publish a prebuilt directory as an immutable cache layer. The
    /// workspace write lock is the singleflight boundary: concurrent callers
    /// for the same canonical key observe one completed publish and reuse it.
    pub fn publish_workspace_layer_from_directory(
        &self,
        key: &WorkspaceLayerKeyV1,
        source: &Path,
    ) -> Result<WorkspaceLayerReport> {
        let cache_key = self.workspace_layer_cache_key(key)?;
        let _lock = self.acquire_write_lock()?;
        let layer_id = format!("layer_{}", &cache_key[..32]);
        let final_path = self.db_dir.join("cache/layers").join(&layer_id);
        if let Some(report) = self.workspace_layer_by_cache_key(&cache_key)? {
            if report.state == "ready" {
                self.verify_workspace_layer(&report.layer_id)?;
                return Ok(report);
            }
            if final_path.is_dir() {
                return self.recover_workspace_layer_publish(
                    key,
                    &cache_key,
                    &layer_id,
                    &final_path,
                );
            }
            if report.state == "building" && layer_builder_is_alive(&report.layer_id, &self.conn)? {
                return Err(Error::InvalidInput(format!(
                    "workspace layer `{}` is already being built",
                    report.layer_id
                )));
            }
        }
        let metadata = fs::symlink_metadata(source)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(Error::InvalidPath {
                path: source.to_string_lossy().into_owned(),
                reason: "workspace layer source must be a real directory".to_string(),
            });
        }
        self.enforce_workspace_cache_build_quota(source)?;
        let builder_id = format!("{}:{}", std::process::id(), current_process_start_token());
        let now = now_ts();
        let build_id = format!(
            "build_{}",
            crate::ids::short_hash(format!("{layer_id}:{builder_id}:{now}").as_bytes(), 12)
        );
        let staging = self.db_dir.join("cache/staging").join(build_id);
        if staging.exists() {
            return Err(Error::InvalidInput(format!(
                "workspace layer staging path `{}` already exists",
                staging.display()
            )));
        }
        fs::create_dir_all(staging.parent().unwrap())?;
        self.conn.execute(
            "INSERT INTO workspace_layers (layer_id, kind, cache_key, adapter, adapter_version, storage_path, state, logical_bytes, physical_bytes, entry_count, portability_scope, builder_id, lease_expires_at, last_used_at, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'building', 0, NULL, 0, ?7, ?8, ?9, ?10, ?10) \
             ON CONFLICT(cache_key) DO UPDATE SET state = 'building', builder_id = excluded.builder_id, lease_expires_at = excluded.lease_expires_at, last_used_at = excluded.last_used_at",
            params![
                layer_id,
                key.kind,
                cache_key,
                key.adapter,
                key.adapter_version,
                final_path.to_string_lossy(),
                key.portability_scope,
                builder_id,
                now + LAYER_BUILD_LEASE_SECS,
                now,
            ],
        )?;

        let publish = (|| -> Result<WorkspaceLayerReport> {
            copy_layer_tree(source, &staging)?;
            let _validated_entries = scan_layer_entries(&staging, false)?;
            sync_layer_tree(&staging)?;
            test_crash_point("layer_after_staging_sync");
            let entries = scan_layer_entries(&staging, true)?;
            let logical_bytes = entries.values().map(|entry| entry.size_bytes).sum();
            let manifest = WorkspaceLayerManifest {
                version: WORKSPACE_LAYER_MANIFEST_VERSION,
                layer_id: layer_id.clone(),
                kind: key.kind.clone(),
                cache_key: cache_key.clone(),
                adapter: key.adapter.clone(),
                adapter_version: key.adapter_version,
                logical_bytes,
                entries,
                platform: key.platform.clone(),
                architecture: key.architecture.clone(),
                portability_scope: key.portability_scope.clone(),
                producer_version: env!("CARGO_PKG_VERSION").to_string(),
                created_at: now,
            };
            let manifest_id = self.put_object(
                WORKSPACE_LAYER_MANIFEST_KIND,
                WORKSPACE_LAYER_MANIFEST_VERSION,
                &manifest,
            )?;
            let physical = layer_physical_bytes(&staging)?;
            let marker = WorkspaceLayerPublishMarker {
                layer_id: layer_id.clone(),
                cache_key: cache_key.clone(),
                manifest_object_id: manifest_id.0.clone(),
                logical_bytes,
                physical_bytes: physical,
                entry_count: manifest.entries.len() as u64,
            };
            write_file_atomic(
                &workspace_layer_marker_path(&final_path),
                &serde_json::to_vec_pretty(&marker)?,
                true,
            )?;
            test_crash_point("layer_after_publish_marker");
            fs::create_dir_all(final_path.parent().unwrap())?;
            if final_path.exists() {
                return Err(Error::Corrupt(format!(
                    "workspace layer destination `{}` exists without a ready matching database record",
                    final_path.display()
                )));
            }
            fs::rename(&staging, &final_path)?;
            set_layer_read_only(
                &final_path,
                true,
                layer_mode(&fs::symlink_metadata(&final_path)?),
            )?;
            sync_directory(final_path.parent().unwrap());
            test_crash_point("layer_after_atomic_rename");
            self.conn.execute(
                "UPDATE workspace_layers SET manifest_object_id = ?1, storage_path = ?2, state = 'ready', logical_bytes = ?3, physical_bytes = ?4, entry_count = ?5, builder_id = NULL, lease_expires_at = NULL, last_used_at = ?6 WHERE cache_key = ?7",
                params![
                    manifest_id.0,
                    final_path.to_string_lossy(),
                    logical_bytes as i64,
                    physical as i64,
                    manifest.entries.len() as i64,
                    now_ts(),
                    cache_key,
                ],
            )?;
            test_crash_point("layer_after_ready_state");
            self.workspace_layer_by_cache_key(&cache_key)?
                .ok_or_else(|| {
                    Error::Corrupt("published workspace layer row disappeared".to_string())
                })
        })();
        if let Err(err) = &publish {
            self.conn.execute(
                "UPDATE workspace_layers SET state = 'failed', builder_id = NULL, lease_expires_at = NULL, last_used_at = ?1 WHERE cache_key = ?2",
                params![now_ts(), cache_key],
            )?;
            let failure = staging.with_extension("failed");
            if staging.exists() && !failure.exists() {
                let _ = fs::rename(&staging, failure);
            }
            return Err(Error::InvalidInput(format!(
                "workspace layer publish failed for key {cache_key}: {err}"
            )));
        }
        publish
    }

    fn enforce_workspace_cache_build_quota(&self, source: &Path) -> Result<()> {
        let output_bytes = cache_tree_logical_bytes(source)?;
        let limit = self.config().workspace_views.cache_build_bytes;
        if limit > 0 && output_bytes > limit {
            return Err(Error::InvalidInput(format!(
                "workspace cache build output is {output_bytes} bytes, exceeding the configured limit of {limit} bytes"
            )));
        }
        let cache_limit = self.config().workspace_views.cache_max_bytes;
        if cache_limit > 0 {
            let current = cache_tree_usage(&self.db_dir.join("cache"))?;
            if current.saturating_add(output_bytes) > cache_limit {
                return Err(Error::InvalidInput(format!(
                    "publishing this workspace layer would exceed the {cache_limit}-byte cache quota; run `trail cache gc --dry-run` to inspect reclaimable data"
                )));
            }
        }
        Ok(())
    }

    fn recover_workspace_layer_publish(
        &self,
        key: &WorkspaceLayerKeyV1,
        cache_key: &str,
        layer_id: &str,
        final_path: &Path,
    ) -> Result<WorkspaceLayerReport> {
        let marker_path = workspace_layer_marker_path(final_path);
        let marker: WorkspaceLayerPublishMarker = serde_json::from_slice(&fs::read(&marker_path)?)?;
        if marker.layer_id != layer_id || marker.cache_key != cache_key {
            return Err(Error::Corrupt(format!(
                "workspace layer publish marker `{}` has the wrong identity",
                marker_path.display()
            )));
        }
        let manifest: WorkspaceLayerManifest = self.get_object(
            WORKSPACE_LAYER_MANIFEST_KIND,
            &ObjectId(marker.manifest_object_id.clone()),
        )?;
        let actual = scan_layer_entries(final_path, false)?;
        if manifest.entries != actual || manifest.cache_key != cache_key {
            return Err(Error::Corrupt(format!(
                "workspace layer `{layer_id}` cannot recover because its published tree is corrupt"
            )));
        }
        self.conn.execute(
            "INSERT INTO workspace_layers (layer_id, kind, cache_key, adapter, adapter_version, manifest_object_id, storage_path, state, logical_bytes, physical_bytes, entry_count, portability_scope, builder_id, lease_expires_at, last_used_at, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'ready', ?8, ?9, ?10, ?11, NULL, NULL, ?12, ?12) \
             ON CONFLICT(cache_key) DO UPDATE SET manifest_object_id = excluded.manifest_object_id, storage_path = excluded.storage_path, state = 'ready', logical_bytes = excluded.logical_bytes, physical_bytes = excluded.physical_bytes, entry_count = excluded.entry_count, builder_id = NULL, lease_expires_at = NULL, last_used_at = excluded.last_used_at",
            params![
                layer_id,
                key.kind,
                cache_key,
                key.adapter,
                key.adapter_version,
                marker.manifest_object_id,
                final_path.to_string_lossy(),
                marker.logical_bytes as i64,
                marker.physical_bytes as i64,
                marker.entry_count as i64,
                key.portability_scope,
                now_ts(),
            ],
        )?;
        self.workspace_layer_by_cache_key(cache_key)?
            .ok_or_else(|| Error::Corrupt("recovered workspace layer row disappeared".to_string()))
    }

    pub fn workspace_layer_by_cache_key(
        &self,
        cache_key: &str,
    ) -> Result<Option<WorkspaceLayerReport>> {
        self.conn
            .query_row(
                "SELECT layer_id, kind, cache_key, adapter, state, storage_path, logical_bytes, physical_bytes, entry_count, portability_scope FROM workspace_layers WHERE cache_key = ?1",
                params![cache_key],
                workspace_layer_from_row,
            )
            .optional()
            .map_err(Error::from)
    }

    pub fn list_workspace_layers(&self) -> Result<Vec<WorkspaceLayerReport>> {
        let mut stmt = self.conn.prepare(
            "SELECT layer_id, kind, cache_key, adapter, state, storage_path, logical_bytes, physical_bytes, entry_count, portability_scope FROM workspace_layers ORDER BY last_used_at DESC, layer_id ASC",
        )?;
        let rows = stmt
            .query_map([], workspace_layer_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        Ok(rows)
    }

    pub(crate) fn workspace_reclaimable_cache_bytes(&self) -> Result<u64> {
        let layer_bytes = self.conn.query_row(
            "SELECT COALESCE(SUM(COALESCE(l.physical_bytes, 0)), 0) FROM workspace_layers l \
             WHERE l.state != 'building' AND NOT EXISTS (SELECT 1 FROM workspace_view_layers b WHERE b.layer_id = l.layer_id)",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        let blobs = cache_tree_usage(&self.db_dir.join("cache/blobs"))?;
        Ok((layer_bytes.max(0) as u64).saturating_add(blobs))
    }

    pub fn workspace_cache_gc(
        &self,
        dry_run: bool,
        retention_secs: Option<u64>,
    ) -> Result<WorkspaceCacheGcReport> {
        let retention_secs =
            retention_secs.unwrap_or(self.config().workspace_views.cache_retention_secs);
        let cutoff = now_ts().saturating_sub(retention_secs.min(i64::MAX as u64) as i64);
        let cache_root = self.db_dir.join("cache");
        let cache_physical_bytes_before = cache_tree_usage(&cache_root)?;
        let mut found = Vec::<CacheGcCandidate>::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT l.layer_id, l.storage_path, COALESCE(l.physical_bytes, 0), l.last_used_at, l.state, \
                        EXISTS(SELECT 1 FROM workspace_view_layers b WHERE b.layer_id = l.layer_id) \
                 FROM workspace_layers l ORDER BY l.last_used_at ASC, l.layer_id ASC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?.max(0) as u64,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)? != 0,
                ))
            })?;
            for row in rows {
                let (id, path, physical_bytes, last_used_at, state, pinned) = row?;
                if pinned || state == "building" {
                    continue;
                }
                found.push(CacheGcCandidate {
                    entry: WorkspaceCacheGcEntry {
                        kind: "layer".to_string(),
                        id,
                        path,
                        physical_bytes,
                        pinned: false,
                        reason: if last_used_at <= cutoff {
                            "unreferenced_retention_expired".to_string()
                        } else {
                            "unreferenced_lru".to_string()
                        },
                    },
                    last_used_at,
                    retention_expired: last_used_at <= cutoff,
                });
            }
        }
        let blob_root = cache_root.join("blobs");
        if blob_root.exists() {
            for entry in walkdir::WalkDir::new(&blob_root).follow_links(false) {
                let entry = entry.map_err(|err| Error::InvalidInput(err.to_string()))?;
                if !entry.file_type().is_file() {
                    continue;
                }
                let metadata = entry.metadata().map_err(|err| Error::Io(err.into()))?;
                let last_used_at = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|value| value.as_secs().min(i64::MAX as u64) as i64)
                    .unwrap_or(0);
                let id = entry
                    .path()
                    .strip_prefix(&blob_root)
                    .unwrap_or(entry.path())
                    .to_string_lossy()
                    .into_owned();
                found.push(CacheGcCandidate {
                    entry: WorkspaceCacheGcEntry {
                        kind: "blob_projection".to_string(),
                        id,
                        path: entry.path().to_string_lossy().into_owned(),
                        physical_bytes: cache_file_physical_bytes(&metadata),
                        pinned: false,
                        reason: if last_used_at <= cutoff {
                            "projection_retention_expired".to_string()
                        } else {
                            "projection_lru".to_string()
                        },
                    },
                    last_used_at,
                    retention_expired: last_used_at <= cutoff,
                });
            }
        }
        found.sort_by(|left, right| {
            left.last_used_at
                .cmp(&right.last_used_at)
                .then_with(|| left.entry.kind.cmp(&right.entry.kind))
                .then_with(|| left.entry.id.cmp(&right.entry.id))
        });
        let max_bytes = self.config().workspace_views.cache_max_bytes;
        let mut projected_bytes = cache_physical_bytes_before;
        let mut selected = Vec::new();
        for candidate in found {
            if candidate.retention_expired || (max_bytes > 0 && projected_bytes > max_bytes) {
                projected_bytes = projected_bytes.saturating_sub(candidate.entry.physical_bytes);
                selected.push(candidate.entry);
            }
        }
        let reclaimable_bytes = selected
            .iter()
            .fold(0_u64, |sum, item| sum.saturating_add(item.physical_bytes));
        if dry_run {
            return Ok(WorkspaceCacheGcReport {
                dry_run,
                retention_secs,
                cache_physical_bytes_before,
                reclaimable_bytes,
                reclaimed_bytes: 0,
                candidates: selected,
                deleted: Vec::new(),
            });
        }

        let _lock = self.acquire_write_lock()?;
        let trash = cache_root.join("trash");
        fs::create_dir_all(&trash).map_err(|err| {
            Error::InvalidInput(format!(
                "failed to create workspace cache trash `{}`: {err}",
                trash.display()
            ))
        })?;
        let mut deleted = Vec::new();
        let mut reclaimed_bytes = 0_u64;
        for candidate in &selected {
            let path = PathBuf::from(&candidate.path);
            if candidate.kind == "layer" {
                let pinned = self.conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM workspace_view_layers WHERE layer_id = ?1)",
                    params![candidate.id],
                    |row| row.get::<_, i64>(0),
                )? != 0;
                let state = self
                    .conn
                    .query_row(
                        "SELECT state FROM workspace_layers WHERE layer_id = ?1",
                        params![candidate.id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?;
                if pinned || state.as_deref() == Some("building") {
                    continue;
                }
                if path.exists() {
                    let previous_state = state.as_deref().unwrap_or("ready");
                    self.conn.execute(
                        "UPDATE workspace_layers SET state = 'deleting' WHERE layer_id = ?1 AND NOT EXISTS (SELECT 1 FROM workspace_view_layers WHERE layer_id = ?1)",
                        params![candidate.id],
                    )?;
                    make_layer_root_writable(&path)?;
                    let trash_path = trash.join(format!(
                        "{}.{}",
                        candidate.id,
                        crate::ids::short_hash(
                            format!("{}:{}", candidate.id, now_nanos()).as_bytes(),
                            12
                        )
                    ));
                    if let Err(err) = fs::rename(&path, &trash_path) {
                        let _ = set_layer_read_only(
                            &path,
                            true,
                            layer_mode(&fs::symlink_metadata(&path)?),
                        );
                        self.conn.execute(
                            "UPDATE workspace_layers SET state = ?1 WHERE layer_id = ?2",
                            params![previous_state, candidate.id],
                        )?;
                        return Err(Error::InvalidInput(format!(
                            "failed to atomically quarantine workspace layer `{}`: {err}",
                            path.display()
                        )));
                    }
                    if let Err(err) = self.conn.execute(
                        "DELETE FROM workspace_layers WHERE layer_id = ?1 AND NOT EXISTS (SELECT 1 FROM workspace_view_layers WHERE layer_id = ?1)",
                        params![candidate.id],
                    ) {
                        let _ = fs::rename(&trash_path, &path);
                        return Err(Error::from(err));
                    }
                    make_tree_writable(&trash_path);
                    fs::remove_dir_all(&trash_path).map_err(|err| {
                        Error::InvalidInput(format!(
                            "failed to remove quarantined workspace layer `{}`: {err}",
                            trash_path.display()
                        ))
                    })?;
                } else {
                    self.conn.execute(
                        "DELETE FROM workspace_layers WHERE layer_id = ?1 AND NOT EXISTS (SELECT 1 FROM workspace_view_layers WHERE layer_id = ?1)",
                        params![candidate.id],
                    )?;
                    remove_workspace_layer_trash_entries(&trash, &candidate.id)?;
                }
                let _ = fs::remove_file(workspace_layer_marker_path(&path));
            } else {
                if !path.is_file() {
                    continue;
                }
                let trash_path = trash.join(format!(
                    "blob.{}",
                    crate::ids::short_hash(
                        format!("{}:{}", candidate.id, now_nanos()).as_bytes(),
                        16
                    )
                ));
                fs::rename(&path, &trash_path).map_err(|err| {
                    Error::InvalidInput(format!(
                        "failed to quarantine blob projection `{}`: {err}",
                        path.display()
                    ))
                })?;
                fs::remove_file(&trash_path).map_err(|err| {
                    Error::InvalidInput(format!(
                        "failed to remove quarantined blob projection `{}`: {err}",
                        trash_path.display()
                    ))
                })?;
            }
            reclaimed_bytes = reclaimed_bytes.saturating_add(candidate.physical_bytes);
            deleted.push(candidate.clone());
        }
        sync_directory(&cache_root);
        Ok(WorkspaceCacheGcReport {
            dry_run,
            retention_secs,
            cache_physical_bytes_before,
            reclaimable_bytes,
            reclaimed_bytes,
            candidates: selected,
            deleted,
        })
    }

    pub(crate) fn workspace_view_layer_reports(
        &self,
        view_id: &str,
    ) -> Result<Vec<WorkspaceLayerReport>> {
        let mut stmt = self.conn.prepare(
            "SELECT l.layer_id, l.kind, l.cache_key, l.adapter, l.state, l.storage_path, l.logical_bytes, l.physical_bytes, l.entry_count, l.portability_scope \
             FROM workspace_view_layers b JOIN workspace_layers l ON l.layer_id = b.layer_id \
             WHERE b.view_id = ?1 ORDER BY b.priority DESC, b.mount_path ASC",
        )?;
        let rows = stmt
            .query_map(params![view_id], workspace_layer_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn verify_workspace_layer(&self, layer_id: &str) -> Result<WorkspaceLayerReport> {
        let row = self.conn.query_row(
            "SELECT layer_id, kind, cache_key, adapter, state, storage_path, logical_bytes, physical_bytes, entry_count, portability_scope, manifest_object_id FROM workspace_layers WHERE layer_id = ?1",
            params![layer_id],
            |row| {
                Ok((
                    workspace_layer_from_row(row)?,
                    row.get::<_, Option<String>>(10)?,
                ))
            },
        ).optional()?;
        let Some((mut report, manifest_id)) = row else {
            return Err(Error::InvalidInput(format!(
                "workspace layer `{layer_id}` was not found"
            )));
        };
        let manifest_id = manifest_id.ok_or_else(|| {
            Error::Corrupt(format!("workspace layer `{layer_id}` has no manifest"))
        })?;
        let manifest: WorkspaceLayerManifest =
            self.get_object(WORKSPACE_LAYER_MANIFEST_KIND, &ObjectId(manifest_id))?;
        let actual = scan_layer_entries(Path::new(&report.storage_path), false)?;
        if manifest.layer_id != layer_id || manifest.entries != actual {
            report.state = "corrupt".to_string();
            return Err(Error::Corrupt(format!(
                "workspace layer `{layer_id}` does not match its immutable manifest"
            )));
        }
        Ok(report)
    }

    pub fn attach_workspace_layer(
        &self,
        lane: &str,
        layer_id: &str,
        mount_path: &str,
        adapter: &str,
        expected_key: &str,
    ) -> Result<LaneWorkspaceViewReport> {
        let _lock = self.acquire_write_lock()?;
        let layer = self.verify_workspace_layer(layer_id)?;
        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered workspace view"
            ))
        })?;
        if let (Some(pid), Some(token)) = (view.owner_pid, view.owner_start_token.as_deref()) {
            if process_matches_start_token(pid, token) {
                return Err(Error::InvalidInput(format!(
                    "workspace view `{}` has an active writer; unmount before changing layer bindings",
                    view.view_id
                )));
            }
        }
        let mount_path = normalize_relative_path(mount_path)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO workspace_view_layers (view_id, layer_id, mount_path, priority, read_only) VALUES (?1, ?2, ?3, 100, 1)",
            params![view.view_id, layer.layer_id, mount_path],
        )?;
        self.conn.execute(
            "UPDATE workspace_layers SET last_used_at = ?1 WHERE layer_id = ?2",
            params![now_ts(), layer.layer_id],
        )?;
        self.conn.execute(
            "INSERT OR REPLACE INTO workspace_environment_states (view_id, adapter, expected_key, attached_key, status, reason, updated_at) VALUES (?1, ?2, ?3, ?4, 'ready', NULL, ?5)",
            params![view.view_id, adapter, expected_key, layer.cache_key, now_ts()],
        )?;
        self.conn.execute(
            "UPDATE workspace_views SET generation = generation + 1, updated_at = ?1 WHERE view_id = ?2",
            params![now_ts(), view.view_id],
        )?;
        self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::Corrupt("workspace view disappeared after layer attach".to_string())
        })
    }

    pub(crate) fn workspace_layer_bindings_for_source_upper(
        &self,
        source_upper: &Path,
    ) -> Result<Vec<WorkspaceLayerBinding>> {
        let mut stmt = self.conn.prepare(
            "SELECT l.layer_id, b.mount_path, l.storage_path, b.priority \
             FROM workspace_views v JOIN workspace_view_layers b ON b.view_id = v.view_id \
             JOIN workspace_layers l ON l.layer_id = b.layer_id \
             WHERE v.source_upper = ?1 AND b.read_only = 1 AND l.state = 'ready' \
             ORDER BY length(b.mount_path) DESC, b.priority DESC",
        )?;
        let rows = stmt
            .query_map(params![source_upper.to_string_lossy()], |row| {
                Ok(WorkspaceLayerBinding {
                    layer_id: row.get(0)?,
                    mount_path: row.get(1)?,
                    storage_path: PathBuf::from(row.get::<_, String>(2)?),
                    priority: row.get(3)?,
                })
            })
            .map_err(Error::from)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

struct CacheGcCandidate {
    entry: WorkspaceCacheGcEntry,
    last_used_at: i64,
    retention_expired: bool,
}

fn cache_tree_usage(path: &Path) -> Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let mut bytes = 0_u64;
    for entry in walkdir::WalkDir::new(path).follow_links(false) {
        let entry = entry.map_err(|err| Error::InvalidInput(err.to_string()))?;
        if entry.file_type().is_file() {
            let metadata = entry.metadata().map_err(|err| Error::Io(err.into()))?;
            bytes = bytes.saturating_add(cache_file_physical_bytes(&metadata));
        }
    }
    Ok(bytes)
}

fn cache_tree_logical_bytes(path: &Path) -> Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    let mut bytes = 0_u64;
    for entry in walkdir::WalkDir::new(path).follow_links(false) {
        let entry = entry.map_err(|err| Error::InvalidInput(err.to_string()))?;
        if entry.file_type().is_file() {
            let metadata = entry.metadata().map_err(|err| Error::Io(err.into()))?;
            bytes = bytes.saturating_add(metadata.len());
        }
    }
    Ok(bytes)
}

fn active_cache_builder_count(lock_dir: &Path) -> Result<u64> {
    let mut active = 0_u64;
    for entry in fs::read_dir(lock_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file()
            || entry.path().extension().and_then(|value| value.to_str()) != Some("lock")
        {
            continue;
        }
        if build_lock_is_stale(&entry.path())? {
            let _ = fs::remove_file(entry.path());
        } else {
            active = active.saturating_add(1);
        }
    }
    Ok(active)
}

#[cfg(unix)]
fn cache_file_physical_bytes(metadata: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.blocks().saturating_mul(512)
}

#[cfg(not(unix))]
fn cache_file_physical_bytes(metadata: &fs::Metadata) -> u64 {
    metadata.len()
}

struct CacheBuildKeyGuard {
    path: PathBuf,
    token: String,
}

impl Drop for CacheBuildKeyGuard {
    fn drop(&mut self) {
        if fs::read_to_string(&self.path)
            .ok()
            .is_some_and(|value| value == self.token)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn build_lock_is_stale(path: &Path) -> Result<bool> {
    let value = match fs::read_to_string(path) {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(err) => return Err(Error::Io(err)),
    };
    let Some((pid, token)) = value.split_once(':') else {
        return Ok(true);
    };
    let Ok(pid) = pid.parse::<u32>() else {
        return Ok(true);
    };
    Ok(!process_matches_start_token(pid, token))
}

fn make_tree_writable(path: &Path) {
    if !path.exists() {
        return;
    }
    let entries = walkdir::WalkDir::new(path)
        .contents_first(true)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .collect::<Vec<_>>();
    for entry in entries {
        if let Ok(metadata) = fs::symlink_metadata(entry.path()) {
            if metadata.file_type().is_symlink() {
                continue;
            }
            let mut permissions = metadata.permissions();
            permissions.set_readonly(false);
            let _ = fs::set_permissions(entry.path(), permissions);
        }
    }
}

fn make_layer_root_writable(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(Error::Corrupt(format!(
            "workspace layer root `{}` is not a real directory",
            path.display()
        )));
    }
    let mut permissions = metadata.permissions();
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

fn remove_workspace_layer_trash_entries(trash: &Path, layer_id: &str) -> Result<()> {
    if !trash.is_dir() {
        return Ok(());
    }
    let prefix = format!("{layer_id}.");
    for entry in fs::read_dir(trash)? {
        let entry = entry?;
        if !entry.file_name().to_string_lossy().starts_with(&prefix) {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            make_tree_writable(&path);
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn workspace_layer_marker_path(layer_path: &Path) -> PathBuf {
    let name = layer_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("layer");
    layer_path.with_file_name(format!(".{name}.publish.json"))
}

fn validate_layer_key(key: &WorkspaceLayerKeyV1) -> Result<()> {
    if key.kind.trim().is_empty()
        || key.adapter.trim().is_empty()
        || key.platform.trim().is_empty()
        || key.architecture.trim().is_empty()
        || key.strategy.trim().is_empty()
    {
        return Err(Error::InvalidInput(
            "workspace layer key contains an empty identity field".to_string(),
        ));
    }
    if key
        .inputs
        .keys()
        .chain(key.tool_versions.keys())
        .any(|name| {
            let lowered = name.to_ascii_lowercase();
            lowered.contains("token")
                || lowered.contains("password")
                || lowered.contains("secret")
                || lowered.contains("private_key")
        })
    {
        return Err(Error::InvalidInput(
            "workspace layer keys cannot serialize secret-bearing inputs".to_string(),
        ));
    }
    Ok(())
}

fn workspace_layer_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspaceLayerReport> {
    Ok(WorkspaceLayerReport {
        layer_id: row.get(0)?,
        kind: row.get(1)?,
        cache_key: row.get(2)?,
        adapter: row.get(3)?,
        state: row.get(4)?,
        storage_path: row.get(5)?,
        logical_bytes: row.get::<_, i64>(6)?.max(0) as u64,
        physical_bytes: row
            .get::<_, Option<i64>>(7)?
            .map(|value| value.max(0) as u64),
        entry_count: row.get::<_, i64>(8)?.max(0) as u64,
        portability_scope: row.get(9)?,
    })
}

fn layer_builder_is_alive(layer_id: &str, conn: &Connection) -> Result<bool> {
    let value = conn
        .query_row(
            "SELECT builder_id FROM workspace_layers WHERE layer_id = ?1",
            params![layer_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let Some(value) = value else {
        return Ok(false);
    };
    let Some((pid, token)) = value.split_once(':') else {
        return Ok(false);
    };
    let Ok(pid) = pid.parse::<u32>() else {
        return Ok(false);
    };
    Ok(process_matches_start_token(pid, token))
}

fn copy_layer_tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    let mut folded = HashMap::<String, String>::new();
    for entry in walkdir::WalkDir::new(source).follow_links(false) {
        let entry = entry.map_err(|err| Error::InvalidInput(err.to_string()))?;
        if entry.path() == source {
            continue;
        }
        let rel = normalize_relative_path(
            &entry
                .path()
                .strip_prefix(source)
                .map_err(|err| Error::InvalidInput(err.to_string()))?
                .to_string_lossy(),
        )?;
        let folded_path = rel.to_lowercase();
        if let Some(previous) = folded.insert(folded_path, rel.clone()) {
            if previous != rel {
                return Err(Error::InvalidPath {
                    path: rel,
                    reason: format!("case-insensitive layer collision with `{previous}`"),
                });
            }
        }
        let target = safe_join(destination, &rel)?;
        let kind = entry.file_type();
        if kind.is_dir() {
            fs::create_dir_all(&target)?;
        } else if kind.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &target)?;
            preserve_layer_mode(entry.path(), &target)?;
        } else if kind.is_symlink() {
            let link = fs::read_link(entry.path())?;
            validate_layer_symlink(source, entry.path(), &link)?;
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            create_layer_symlink(&link, &target)?;
        } else {
            return Err(Error::InvalidPath {
                path: rel,
                reason: "workspace layers cannot contain devices, sockets, or special files"
                    .to_string(),
            });
        }
    }
    Ok(())
}

fn scan_layer_entries(
    root: &Path,
    make_read_only: bool,
) -> Result<BTreeMap<String, WorkspaceLayerEntry>> {
    if !root.is_dir() {
        return Err(Error::Corrupt(format!(
            "workspace layer directory `{}` is missing",
            root.display()
        )));
    }
    let mut entries = BTreeMap::new();
    let mut directories = Vec::new();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|err| Error::InvalidInput(err.to_string()))?;
        if entry.path() == root {
            continue;
        }
        let rel = normalize_relative_path(
            &entry
                .path()
                .strip_prefix(root)
                .map_err(|err| Error::InvalidInput(err.to_string()))?
                .to_string_lossy(),
        )?;
        let metadata = fs::symlink_metadata(entry.path())?;
        let mode = layer_mode(&metadata);
        let item = if metadata.file_type().is_symlink() {
            let target = fs::read_link(entry.path())?;
            validate_layer_symlink(root, entry.path(), &target)?;
            WorkspaceLayerEntry {
                kind: "symlink".to_string(),
                mode,
                size_bytes: 0,
                content_hash: None,
                symlink_target: Some(target.to_string_lossy().into_owned()),
            }
        } else if metadata.is_dir() {
            directories.push(entry.path().to_path_buf());
            WorkspaceLayerEntry {
                kind: "directory".to_string(),
                mode: if make_read_only {
                    immutable_layer_mode(true, mode)
                } else {
                    mode
                },
                size_bytes: 0,
                content_hash: None,
                symlink_target: None,
            }
        } else if metadata.is_file() {
            let hash = sha256_layer_file(entry.path())?;
            if make_read_only {
                set_layer_read_only(entry.path(), false, mode)?;
            }
            WorkspaceLayerEntry {
                kind: "file".to_string(),
                mode: if make_read_only {
                    immutable_layer_mode(false, mode)
                } else {
                    mode
                },
                size_bytes: metadata.len(),
                content_hash: Some(hash),
                symlink_target: None,
            }
        } else {
            return Err(Error::InvalidPath {
                path: rel,
                reason: "workspace layers cannot contain special files".to_string(),
            });
        };
        entries.insert(rel, item);
    }
    if make_read_only {
        directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
        for path in directories {
            let mode = layer_mode(&fs::symlink_metadata(&path)?);
            set_layer_read_only(&path, true, mode)?;
        }
    }
    Ok(entries)
}

fn sha256_layer_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut bytes = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut bytes)?;
        if read == 0 {
            break;
        }
        hasher.update(&bytes[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn validate_layer_symlink(root: &Path, path: &Path, target: &Path) -> Result<()> {
    if target.is_absolute() {
        return Err(Error::InvalidPath {
            path: path.to_string_lossy().into_owned(),
            reason: "workspace layer symlinks must be relative".to_string(),
        });
    }
    let parent = path.parent().unwrap_or(root);
    let resolved = normalize_absolute_lexically(&parent.join(target));
    let root = normalize_absolute_lexically(root);
    if !resolved.starts_with(&root) {
        return Err(Error::InvalidPath {
            path: path.to_string_lossy().into_owned(),
            reason: "workspace layer symlink escapes its immutable layer".to_string(),
        });
    }
    Ok(())
}

fn normalize_absolute_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(unix)]
fn create_layer_symlink(target: &Path, destination: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, destination).map_err(Error::from)
}

#[cfg(windows)]
fn create_layer_symlink(target: &Path, destination: &Path) -> Result<()> {
    let source = destination.parent().unwrap_or(Path::new(".")).join(target);
    if source.is_dir() {
        std::os::windows::fs::symlink_dir(target, destination).map_err(Error::from)
    } else {
        std::os::windows::fs::symlink_file(target, destination).map_err(Error::from)
    }
}

#[cfg(not(any(unix, windows)))]
fn create_layer_symlink(_target: &Path, destination: &Path) -> Result<()> {
    Err(Error::InvalidInput(format!(
        "workspace layer symlinks are not supported on this platform: {}",
        destination.display()
    )))
}

#[cfg(unix)]
fn preserve_layer_mode(source: &Path, destination: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(source)?.permissions().mode() & 0o777;
    fs::set_permissions(destination, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn preserve_layer_mode(_source: &Path, _destination: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn layer_mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn layer_mode(metadata: &fs::Metadata) -> u32 {
    if metadata.is_dir() {
        0o755
    } else if metadata.permissions().readonly() {
        0o444
    } else {
        0o644
    }
}

#[cfg(unix)]
fn set_layer_read_only(path: &Path, directory: bool, original_mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let executable = directory || original_mode & 0o111 != 0;
    fs::set_permissions(
        path,
        fs::Permissions::from_mode(if executable { 0o555 } else { 0o444 }),
    )?;
    Ok(())
}

#[cfg(unix)]
fn immutable_layer_mode(directory: bool, original_mode: u32) -> u32 {
    if directory || original_mode & 0o111 != 0 {
        0o555
    } else {
        0o444
    }
}

#[cfg(not(unix))]
fn set_layer_read_only(path: &Path, _directory: bool, _original_mode: u32) -> Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn immutable_layer_mode(directory: bool, _original_mode: u32) -> u32 {
    if directory {
        0o555
    } else {
        0o444
    }
}

fn sync_layer_tree(root: &Path) -> Result<()> {
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|err| Error::InvalidInput(err.to_string()))?;
        if entry.file_type().is_file() {
            fs::File::open(entry.path())?.sync_all()?;
        }
    }
    sync_directory(root);
    Ok(())
}

fn layer_physical_bytes(root: &Path) -> Result<u64> {
    let mut bytes = 0_u64;
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|err| Error::InvalidInput(err.to_string()))?;
        if entry.file_type().is_file() {
            let metadata = entry.metadata().map_err(|err| Error::Io(err.into()))?;
            bytes = bytes.saturating_add(layer_file_physical_bytes(&metadata));
        }
    }
    Ok(bytes)
}

#[cfg(unix)]
fn layer_file_physical_bytes(metadata: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.blocks().saturating_mul(512)
}

#[cfg(not(unix))]
fn layer_file_physical_bytes(metadata: &fs::Metadata) -> u64 {
    metadata.len()
}

#[cfg(test)]
mod tests {
    use super::super::workdir::{ViewCore, VIEW_ROOT_INO};
    use super::*;
    use std::process::Stdio;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn key() -> WorkspaceLayerKeyV1 {
        WorkspaceLayerKeyV1 {
            kind: "dependency".to_string(),
            adapter: "node".to_string(),
            adapter_version: 1,
            inputs: BTreeMap::from([("package-lock.json".to_string(), "abc".to_string())]),
            tool_versions: BTreeMap::from([("node".to_string(), "22.0.0".to_string())]),
            platform: std::env::consts::OS.to_string(),
            architecture: std::env::consts::ARCH.to_string(),
            portability_scope: "platform".to_string(),
            strategy: "npm-ci-ignore-scripts".to_string(),
        }
    }

    #[test]
    fn cache_publish_crash_helper() {
        let Some(workspace) = std::env::var_os("TRAIL_TEST_CRASH_WORKSPACE") else {
            return;
        };
        let source = PathBuf::from(
            std::env::var_os("TRAIL_TEST_CRASH_LAYER_SOURCE")
                .expect("layer crash helper requires a source directory"),
        );
        let db = Trail::open(PathBuf::from(workspace)).unwrap();
        let _ = db.publish_workspace_layer_from_directory(&key(), &source);
        panic!("cache publish crash helper passed its requested crash point");
    }

    #[test]
    fn killing_cache_publish_at_each_durable_phase_preserves_source_and_recovers() {
        for phase in [
            "layer_after_staging_sync",
            "layer_after_publish_marker",
            "layer_after_atomic_rename",
            "layer_after_ready_state",
        ] {
            let workspace = tempfile::tempdir().unwrap();
            fs::write(workspace.path().join("README.md"), "root\n").unwrap();
            Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let mut db = Trail::open(workspace.path()).unwrap();
            let mode = if cfg!(target_os = "macos") {
                LaneWorkdirMode::NfsCow
            } else {
                LaneWorkdirMode::OverlayCow
            };
            db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                "crash-source",
                Some("main"),
                mode,
                None,
                None,
                None,
                &[],
                false,
            )
            .unwrap();
            let source_upper = db
                .workspace_view_paths_for_lane("crash-source")
                .unwrap()
                .source_upper;
            fs::write(source_upper.join("uncheckpointed.rs"), "keep me\n").unwrap();
            drop(db);

            let layer_source = tempfile::tempdir().unwrap();
            fs::create_dir_all(layer_source.path().join("pkg")).unwrap();
            fs::write(layer_source.path().join("pkg/index.js"), "cached\n").unwrap();
            let ready = workspace.path().join(format!("{phase}.ready"));
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "db::lane::workspace_layer::tests::cache_publish_crash_helper",
                    "--nocapture",
                ])
                .env("RUST_TEST_THREADS", "1")
                .env("TRAIL_TEST_CRASH_AT", phase)
                .env("TRAIL_TEST_CRASH_READY", &ready)
                .env("TRAIL_TEST_CRASH_WORKSPACE", workspace.path())
                .env("TRAIL_TEST_CRASH_LAYER_SOURCE", layer_source.path())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            wait_for_crash_handshake(&mut child, &ready, phase);
            child.kill().unwrap();
            let _ = child.wait().unwrap();

            assert_eq!(
                fs::read_to_string(source_upper.join("uncheckpointed.rs")).unwrap(),
                "keep me\n"
            );
            let reopened = Trail::open(workspace.path()).unwrap();
            let layer = reopened
                .publish_workspace_layer_from_directory(&key(), layer_source.path())
                .unwrap();
            assert_eq!(layer.state, "ready", "failed recovery at {phase}");
            reopened.verify_workspace_layer(&layer.layer_id).unwrap();
            assert_eq!(
                fs::read_to_string(source_upper.join("uncheckpointed.rs")).unwrap(),
                "keep me\n"
            );
        }
    }

    fn wait_for_crash_handshake(child: &mut std::process::Child, ready: &Path, phase: &str) {
        for _ in 0..1_000 {
            if ready.is_file() {
                return;
            }
            if let Some(status) = child.try_wait().unwrap() {
                panic!("crash helper exited at {phase} before handshake: {status}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = child.kill();
        panic!("timed out waiting for crash helper at {phase}");
    }

    #[test]
    fn identical_layer_keys_publish_once_and_reuse_read_only_tree() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let built = tempfile::tempdir().unwrap();
        fs::create_dir_all(built.path().join("pkg")).unwrap();
        fs::write(built.path().join("pkg/index.js"), "module.exports = 1;\n").unwrap();

        let first = db
            .publish_workspace_layer_from_directory(&key(), built.path())
            .unwrap();
        let second = db
            .publish_workspace_layer_from_directory(&key(), built.path())
            .unwrap();
        assert_eq!(first.layer_id, second.layer_id);
        assert_eq!(db.list_workspace_layers().unwrap().len(), 1);
        assert!(
            fs::metadata(Path::new(&first.storage_path).join("pkg/index.js"))
                .unwrap()
                .permissions()
                .readonly()
        );
        db.verify_workspace_layer(&first.layer_id).unwrap();
    }

    #[test]
    fn layer_publish_rejects_escaping_symlink_and_secret_key_names() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let mut secret = key();
        secret
            .inputs
            .insert("registry_token".to_string(), "hidden".to_string());
        assert!(db.workspace_layer_cache_key(&secret).is_err());

        #[cfg(unix)]
        {
            let built = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink("../../outside", built.path().join("escape")).unwrap();
            assert!(db
                .publish_workspace_layer_from_directory(&key(), built.path())
                .is_err());
        }
    }

    #[test]
    fn two_views_share_one_layer_but_copy_writes_to_private_generated_uppers() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else {
            LaneWorkdirMode::OverlayCow
        };
        for lane in ["node-a", "node-b"] {
            db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                lane,
                Some("main"),
                mode.clone(),
                None,
                None,
                None,
                &[],
                false,
            )
            .unwrap();
        }
        let built = tempfile::tempdir().unwrap();
        fs::create_dir_all(built.path().join("pkg")).unwrap();
        fs::write(built.path().join("pkg/index.js"), "original\n").unwrap();
        let layer = db
            .publish_workspace_layer_from_directory(&key(), built.path())
            .unwrap();
        for lane in ["node-a", "node-b"] {
            db.attach_workspace_layer(
                lane,
                &layer.layer_id,
                "node_modules",
                "node",
                &layer.cache_key,
            )
            .unwrap();
        }

        let mut views = Vec::new();
        for lane in ["node-a", "node-b"] {
            let branch = db.lane_branch(lane).unwrap();
            let head = db.get_ref(&branch.ref_name).unwrap();
            let paths = db.workspace_view_paths_for_lane(lane).unwrap();
            views.push((
                paths.clone(),
                ViewCore::new_lazy(
                    Trail::open(workspace.path()).unwrap(),
                    paths.source_upper,
                    head.root_id,
                )
                .unwrap(),
            ));
        }
        let lookup_index = |view: &mut ViewCore| {
            let modules = view.lookup(VIEW_ROOT_INO, "node_modules").unwrap();
            let package = view.lookup(modules, "pkg").unwrap();
            view.lookup(package, "index.js").unwrap()
        };
        let index_a = lookup_index(&mut views[0].1);
        assert_ne!(
            views[0].1.attr("node_modules/pkg/index.js").unwrap().mode & 0o200,
            0
        );
        assert!(
            fs::metadata(Path::new(&layer.storage_path).join("pkg/index.js"))
                .unwrap()
                .permissions()
                .readonly()
        );
        views[0].1.setattr(index_a, Some(0), None).unwrap();
        views[0].1.write(index_a, 0, b"lane-a\n").unwrap();
        let index_b = lookup_index(&mut views[1].1);
        assert_eq!(views[1].1.read(index_b, 0, 64).unwrap().0, b"original\n");
        assert_eq!(
            fs::read(Path::new(&layer.storage_path).join("pkg/index.js")).unwrap(),
            b"original\n"
        );
        assert_eq!(
            fs::read(views[0].0.generated_upper.join("node_modules/pkg/index.js")).unwrap(),
            b"lane-a\n"
        );
        assert!(!views[1]
            .0
            .generated_upper
            .join("node_modules/pkg/index.js")
            .exists());
        assert!(views[0].1.checkpoint_candidates().unwrap().paths.is_empty());

        let modules_a = views[0].1.lookup(VIEW_ROOT_INO, "node_modules").unwrap();
        let package_a = views[0].1.lookup(modules_a, "pkg").unwrap();
        views[0].1.remove(package_a, "index.js").unwrap();
        assert_eq!(
            views[0].1.node_kind("node_modules/pkg/index.js").unwrap(),
            None
        );
        assert_eq!(views[1].1.read(index_b, 0, 64).unwrap().0, b"original\n");
        assert_eq!(
            fs::read(Path::new(&layer.storage_path).join("pkg/index.js")).unwrap(),
            b"original\n"
        );
    }

    #[test]
    fn concurrent_missing_key_runs_exactly_one_layer_builder() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let workspace_path = workspace.path().to_path_buf();
        let builds = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let workspace_path = workspace_path.clone();
            let builds = Arc::clone(&builds);
            let key = key();
            workers.push(std::thread::spawn(move || {
                let db = Trail::open(&workspace_path).unwrap();
                db.build_workspace_layer_singleflight(&key, |build_dir| {
                    builds.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(150));
                    let output = build_dir.join("output");
                    fs::create_dir_all(&output).unwrap();
                    fs::write(output.join("index.js"), "shared\n").unwrap();
                    Ok(output)
                })
                .unwrap()
            }));
        }
        let reports = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert_eq!(reports[0].layer_id, reports[1].layer_id);
    }

    #[test]
    fn publish_recovery_adopts_atomic_tree_without_touching_source_upper() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else {
            LaneWorkdirMode::OverlayCow
        };
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "recover",
            Some("main"),
            mode,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let source_upper = db
            .workspace_view_paths_for_lane("recover")
            .unwrap()
            .source_upper;
        fs::write(source_upper.join("uncheckpointed.rs"), "keep me\n").unwrap();
        let built = tempfile::tempdir().unwrap();
        fs::write(built.path().join("index.js"), "published\n").unwrap();
        let first = db
            .publish_workspace_layer_from_directory(&key(), built.path())
            .unwrap();

        // This is the durable state immediately after staging was renamed but
        // before the final SQLite transition. The adjacent marker and
        // content-addressed manifest are sufficient to adopt it.
        db.conn
            .execute(
                "UPDATE workspace_layers SET state = 'building', manifest_object_id = NULL, builder_id = NULL WHERE layer_id = ?1",
                params![first.layer_id],
            )
            .unwrap();
        drop(db);
        let reopened = Trail::open(workspace.path()).unwrap();
        let recovered = reopened
            .publish_workspace_layer_from_directory(&key(), built.path())
            .unwrap();
        assert_eq!(recovered.layer_id, first.layer_id);
        assert_eq!(recovered.state, "ready");
        assert_eq!(
            fs::read_to_string(source_upper.join("uncheckpointed.rs")).unwrap(),
            "keep me\n"
        );
    }

    #[test]
    fn cache_gc_never_selects_pinned_layers_and_reclaims_unpinned_layers() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else {
            LaneWorkdirMode::OverlayCow
        };
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "pinned",
            Some("main"),
            mode,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let built = tempfile::tempdir().unwrap();
        fs::write(built.path().join("index.js"), "cached\n").unwrap();
        let layer = db
            .publish_workspace_layer_from_directory(&key(), built.path())
            .unwrap();
        db.attach_workspace_layer(
            "pinned",
            &layer.layer_id,
            "node_modules",
            "node",
            &layer.cache_key,
        )
        .unwrap();
        let pinned = db.workspace_cache_gc(true, Some(0)).unwrap();
        assert!(!pinned
            .candidates
            .iter()
            .any(|candidate| candidate.id == layer.layer_id));

        db.conn
            .execute(
                "DELETE FROM workspace_view_layers WHERE layer_id = ?1",
                params![layer.layer_id],
            )
            .unwrap();
        let preview = db.workspace_cache_gc(true, Some(0)).unwrap();
        assert!(preview
            .candidates
            .iter()
            .any(|candidate| candidate.id == layer.layer_id));
        assert!(preview.reclaimable_bytes > 0);
        let collected = db.workspace_cache_gc(false, Some(0)).unwrap();
        assert!(collected
            .deleted
            .iter()
            .any(|candidate| candidate.id == layer.layer_id));
        assert!(!Path::new(&layer.storage_path).exists());
        assert!(db.list_workspace_layers().unwrap().is_empty());
    }

    #[test]
    fn corrupt_bound_layer_is_an_exact_readiness_blocker() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else {
            LaneWorkdirMode::OverlayCow
        };
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "corrupt",
            Some("main"),
            mode,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let built = tempfile::tempdir().unwrap();
        fs::write(built.path().join("index.js"), "immutable\n").unwrap();
        let layer = db
            .publish_workspace_layer_from_directory(&key(), built.path())
            .unwrap();
        db.attach_workspace_layer(
            "corrupt",
            &layer.layer_id,
            "vendor-cache",
            "manual",
            &layer.cache_key,
        )
        .unwrap();
        let file = Path::new(&layer.storage_path).join("index.js");
        let mut permissions = fs::metadata(&file).unwrap().permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&file, permissions).unwrap();
        fs::write(&file, "tampered!\n").unwrap();
        let readiness = db.lane_readiness("corrupt").unwrap();
        assert!(readiness
            .blockers
            .iter()
            .any(|issue| issue.code == "workspace_layer_corrupt"));
    }

    #[test]
    fn cache_gc_resumes_after_crash_between_quarantine_and_row_delete() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let built = tempfile::tempdir().unwrap();
        fs::write(built.path().join("index.js"), "cached\n").unwrap();
        let layer = db
            .publish_workspace_layer_from_directory(&key(), built.path())
            .unwrap();
        let original = PathBuf::from(&layer.storage_path);
        let trash = db.db_dir.join("cache/trash");
        fs::create_dir_all(&trash).unwrap();
        let quarantined = trash.join(format!("{}.crash", layer.layer_id));
        db.conn
            .execute(
                "UPDATE workspace_layers SET state = 'deleting' WHERE layer_id = ?1",
                params![layer.layer_id],
            )
            .unwrap();
        make_layer_root_writable(&original).unwrap();
        fs::rename(&original, &quarantined).unwrap();
        drop(db);

        let reopened = Trail::open(workspace.path()).unwrap();
        let report = reopened.workspace_cache_gc(false, Some(0)).unwrap();
        assert!(report
            .deleted
            .iter()
            .any(|candidate| candidate.id == layer.layer_id));
        assert!(!quarantined.exists());
        assert!(reopened.list_workspace_layers().unwrap().is_empty());
    }
}
