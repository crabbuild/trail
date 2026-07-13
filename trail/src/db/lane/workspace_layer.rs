use super::workdir::{PreparedLayerMountReset, ViewCore};
use super::*;
use serde::{Deserialize, Serialize};
use std::thread;

const LAYER_BUILD_LEASE_SECS: i64 = 300;
const WORKSPACE_LAYER_VERIFICATION_STAMP_VERSION: u16 = 1;
const WORKSPACE_LAYER_SIDECAR_MAX_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceLayerBinding {
    /// Durable identity used by filesystem-side activation recovery. For an
    /// immutable binding this is the layer ID; writable-private bindings use a
    /// component/output/key-derived identity and intentionally have no layer.
    pub(crate) binding_identity: String,
    #[allow(dead_code)]
    pub(crate) layer_id: Option<String>,
    pub(crate) mount_path: String,
    pub(crate) storage_path: Option<PathBuf>,
    pub(crate) kind: String,
    #[allow(dead_code)]
    pub(crate) priority: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct EnvironmentLayerActivation {
    pub(crate) layer_id: Option<String>,
    pub(crate) outputs: Vec<EnvironmentLayerOutputActivation>,
    pub(crate) component_id: String,
    pub(crate) adapter_identity: String,
    pub(crate) adapter_version: u32,
    pub(crate) implementation_version: String,
    pub(crate) distribution_digest: String,
    pub(crate) kind: String,
    /// Direct dependencies, exact upstream generation keys, and typed edge
    /// semantics. Only identity-bearing edge keys also occur in the canonical
    /// component identity.
    pub(crate) dependencies: Vec<(String, String, String)>,
    pub(crate) caches: Vec<EnvironmentCacheReport>,
    pub(crate) external_artifacts: Vec<EnvironmentExternalArtifactReport>,
    pub(crate) runtime_resources: Vec<EnvironmentRuntimeDeclarationReport>,
    pub(crate) expected_key: String,
    pub(crate) canonical_key: WorkspaceLayerKeyV1,
}

#[derive(Clone, Debug)]
pub(crate) struct EnvironmentLayerOutputActivation {
    pub(crate) name: String,
    pub(crate) mount_path: String,
    pub(crate) policy: String,
    pub(crate) binding_identity: String,
    /// Staged initial content for a replaced writable-private output. None
    /// means preserve the compatible lane-private directory in place.
    pub(crate) private_seed: Option<PathBuf>,
    /// Empty for the historical single-output layer layout.
    pub(crate) layer_subpath: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct WorkspaceLayerManifest {
    version: u16,
    layer_id: String,
    kind: String,
    cache_key: String,
    #[serde(default)]
    layer_key: Option<WorkspaceLayerKeyV1>,
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

/// Durable evidence that a complete verification was performed for this exact
/// immutable directory identity. Routine attachment can validate this bounded
/// record instead of recursively reopening every file in a large dependency tree.
/// Explicit `cache verify` and readiness checks still perform a full scan.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct WorkspaceLayerVerificationStamp {
    version: u16,
    layer_id: String,
    manifest_object_id: String,
    root_identity: WorkspaceLayerRootIdentity,
    logical_bytes: u64,
    entry_count: u64,
    verified_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct WorkspaceLayerRootIdentity {
    platform: String,
    values: Vec<u64>,
}

#[cfg(test)]
std::thread_local! {
    static WORKSPACE_LAYER_FULL_SCAN_COUNT: Cell<u64> = const { Cell::new(0) };
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
                return self.verify_workspace_layer_for_attach(&layer.layer_id);
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
                            return self.verify_workspace_layer_for_attach(&layer.layer_id);
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
                return self.verify_workspace_layer_for_attach(&layer.layer_id);
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
                self.verify_workspace_layer_for_attach(&report.layer_id)?;
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
                layer_key: Some(key.clone()),
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
            let report = self
                .workspace_layer_by_cache_key(&cache_key)?
                .ok_or_else(|| {
                    Error::Corrupt("published workspace layer row disappeared".to_string())
                })?;
            // The layer is already durable and ready. A sidecar failure must
            // not invalidate correct shared content; the next attach safely
            // falls back to a full verification and retries the stamp.
            let _ = write_workspace_layer_verification_stamp(&report, &manifest_id.0);
            Ok(report)
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
        let marker: WorkspaceLayerPublishMarker = read_workspace_layer_sidecar(&marker_path)?;
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
        let manifest_key_matches = manifest
            .layer_key
            .as_ref()
            .map(|layer_key| self.workspace_layer_cache_key(layer_key))
            .transpose()?
            .is_none_or(|canonical| canonical == cache_key);
        if manifest.entries != actual || manifest.cache_key != cache_key || !manifest_key_matches {
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
        let report = self
            .workspace_layer_by_cache_key(cache_key)?
            .ok_or_else(|| {
                Error::Corrupt("recovered workspace layer row disappeared".to_string())
            })?;
        let _ = write_workspace_layer_verification_stamp(&report, &marker.manifest_object_id);
        Ok(report)
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

    pub(crate) fn workspace_layer_key_by_cache_key(
        &self,
        cache_key: &str,
    ) -> Result<Option<WorkspaceLayerKeyV1>> {
        let layer_id = self
            .conn
            .query_row(
                "SELECT layer_id FROM workspace_layers WHERE cache_key = ?1",
                params![cache_key],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(layer_id) = layer_id {
            let (_, _, manifest) = self.workspace_layer_verification_record(&layer_id)?;
            if manifest.layer_key.is_some() {
                return Ok(manifest.layer_key);
            }
        }
        let bytes = self
            .conn
            .query_row(
                "SELECT canonical_key_json FROM environment_component_key_provenance
                 WHERE component_key = ?1",
                params![cache_key],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        let Some(bytes) = bytes else {
            return Ok(None);
        };
        let key: WorkspaceLayerKeyV1 = serde_json::from_slice(&bytes).map_err(|error| {
            Error::Corrupt(format!(
                "environment key provenance `{cache_key}` is malformed: {error}"
            ))
        })?;
        if self.workspace_layer_cache_key(&key)? != cache_key {
            return Err(Error::Corrupt(format!(
                "environment key provenance `{cache_key}` does not match its content identity"
            )));
        }
        Ok(Some(key))
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
             WHERE l.state != 'building'
               AND NOT EXISTS (SELECT 1 FROM workspace_view_layers b WHERE b.layer_id = l.layer_id)
               AND NOT EXISTS (SELECT 1 FROM environment_generation_components g WHERE g.layer_id = l.layer_id)",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        let blobs = cache_tree_usage(&self.db_dir.join("cache/blobs"))?;
        let environment_caches = cache_tree_usage(&self.db_dir.join("cache/namespaces"))?;
        Ok((layer_bytes.max(0) as u64)
            .saturating_add(blobs)
            .saturating_add(environment_caches))
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
                        (EXISTS(SELECT 1 FROM workspace_view_layers b WHERE b.layer_id = l.layer_id)
                         OR EXISTS(SELECT 1 FROM environment_generation_components g WHERE g.layer_id = l.layer_id)) \
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
        {
            let mut stmt = self.conn.prepare(
                "SELECT namespace_id, storage_path, last_used_at
                 FROM environment_cache_namespaces
                 ORDER BY last_used_at ASC, namespace_id ASC",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?;
            for row in rows {
                let (namespace_id, storage_path, last_used_at) = row?;
                if super::workspace_environment::environment_cache_namespace_has_live_leases(
                    &self.db_dir,
                    &namespace_id,
                    !dry_run,
                )? {
                    continue;
                }
                let expected = cache_root.join("namespaces").join(&namespace_id);
                let path = PathBuf::from(&storage_path);
                if path != expected {
                    return Err(Error::Corrupt(format!(
                        "environment cache namespace `{namespace_id}` has invalid storage path `{storage_path}`"
                    )));
                }
                found.push(CacheGcCandidate {
                    entry: WorkspaceCacheGcEntry {
                        kind: "environment_cache".to_string(),
                        id: namespace_id,
                        path: storage_path,
                        physical_bytes: cache_tree_usage(&path)?,
                        pinned: false,
                        reason: if last_used_at <= cutoff {
                            "performance_cache_retention_expired".to_string()
                        } else {
                            "performance_cache_lru".to_string()
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
                    "SELECT (EXISTS(SELECT 1 FROM workspace_view_layers WHERE layer_id = ?1)
                             OR EXISTS(SELECT 1 FROM environment_generation_components WHERE layer_id = ?1))",
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
                        "UPDATE workspace_layers SET state = 'deleting' WHERE layer_id = ?1
                         AND NOT EXISTS (SELECT 1 FROM workspace_view_layers WHERE layer_id = ?1)
                         AND NOT EXISTS (SELECT 1 FROM environment_generation_components WHERE layer_id = ?1)",
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
                        "DELETE FROM workspace_layers WHERE layer_id = ?1
                         AND NOT EXISTS (SELECT 1 FROM workspace_view_layers WHERE layer_id = ?1)
                         AND NOT EXISTS (SELECT 1 FROM environment_generation_components WHERE layer_id = ?1)",
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
                        "DELETE FROM workspace_layers WHERE layer_id = ?1
                         AND NOT EXISTS (SELECT 1 FROM workspace_view_layers WHERE layer_id = ?1)
                         AND NOT EXISTS (SELECT 1 FROM environment_generation_components WHERE layer_id = ?1)",
                        params![candidate.id],
                    )?;
                    remove_workspace_layer_trash_entries(&trash, &candidate.id)?;
                }
                let _ = fs::remove_file(workspace_layer_marker_path(&path));
                let _ = fs::remove_file(workspace_layer_verification_stamp_path(&path));
            } else if candidate.kind == "environment_cache" {
                let Some(_maintenance) =
                    acquire_environment_cache_maintenance(&self.db_dir, &candidate.id)?
                else {
                    continue;
                };
                if super::workspace_environment::environment_cache_namespace_has_live_leases(
                    &self.db_dir,
                    &candidate.id,
                    true,
                )? {
                    continue;
                }
                let expected = cache_root.join("namespaces").join(&candidate.id);
                if path != expected {
                    return Err(Error::Corrupt(format!(
                        "environment cache namespace `{}` escaped cache storage",
                        candidate.id
                    )));
                }
                let trash_path = trash.join(format!(
                    "environment-cache.{}.{}",
                    candidate.id,
                    crate::ids::short_hash(
                        format!("{}:{}", candidate.id, now_nanos()).as_bytes(),
                        12
                    )
                ));
                if path.exists() {
                    if !path.is_dir() {
                        return Err(Error::Corrupt(format!(
                            "environment cache namespace `{}` is not a directory",
                            candidate.id
                        )));
                    }
                    fs::rename(&path, &trash_path).map_err(|error| {
                        Error::InvalidInput(format!(
                            "failed to atomically quarantine environment cache `{}`: {error}",
                            path.display()
                        ))
                    })?;
                }
                if let Err(error) = self.conn.execute(
                    "DELETE FROM environment_cache_namespaces WHERE namespace_id = ?1",
                    params![candidate.id],
                ) {
                    if trash_path.exists() {
                        let _ = fs::rename(&trash_path, &path);
                    }
                    return Err(Error::from(error));
                }
                if trash_path.exists() {
                    make_tree_writable(&trash_path);
                    fs::remove_dir_all(&trash_path).map_err(|error| {
                        Error::InvalidInput(format!(
                            "failed to remove quarantined environment cache `{}`: {error}",
                            trash_path.display()
                        ))
                    })?;
                }
                let _ = fs::remove_dir_all(
                    self.db_dir
                        .join("cache/namespace-leases")
                        .join(&candidate.id),
                );
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
            "SELECT DISTINCT l.layer_id, l.kind, l.cache_key, l.adapter, l.state, l.storage_path, l.logical_bytes, l.physical_bytes, l.entry_count, l.portability_scope \
             FROM workspace_view_layers b JOIN workspace_layers l ON l.layer_id = b.layer_id \
             WHERE b.view_id = ?1 ORDER BY b.priority DESC, b.mount_path ASC",
        )?;
        let rows = stmt
            .query_map(params![view_id], workspace_layer_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn workspace_layer_verification_record(
        &self,
        layer_id: &str,
    ) -> Result<(WorkspaceLayerReport, String, WorkspaceLayerManifest)> {
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
        let manifest: WorkspaceLayerManifest = self.get_object(
            WORKSPACE_LAYER_MANIFEST_KIND,
            &ObjectId(manifest_id.clone()),
        )?;
        let manifest_key_matches = manifest
            .layer_key
            .as_ref()
            .map(|key| self.workspace_layer_cache_key(key))
            .transpose()
            .map_err(|error| {
                Error::Corrupt(format!(
                    "workspace layer `{layer_id}` contains an invalid canonical key: {error}"
                ))
            })?
            .is_none_or(|cache_key| cache_key == report.cache_key);
        if manifest.version != WORKSPACE_LAYER_MANIFEST_VERSION
            || !manifest_key_matches
            || manifest.layer_id != layer_id
            || manifest.kind != report.kind
            || manifest.cache_key != report.cache_key
            || manifest.adapter != report.adapter
            || manifest.logical_bytes != report.logical_bytes
            || manifest.entries.len() as u64 != report.entry_count
            || manifest.portability_scope != report.portability_scope
        {
            report.state = "corrupt".to_string();
            return Err(Error::Corrupt(format!(
                "workspace layer `{layer_id}` metadata does not match its immutable manifest"
            )));
        }
        Ok((report, manifest_id, manifest))
    }

    /// Bounded verification used by routine cache reuse and attachment. A
    /// missing or stale stamp safely falls back to one complete verification,
    /// which refreshes the durable stamp for later attaches.
    pub(crate) fn verify_workspace_layer_for_attach(
        &self,
        layer_id: &str,
    ) -> Result<WorkspaceLayerReport> {
        let (report, manifest_id, manifest) = self.workspace_layer_verification_record(layer_id)?;
        if report.state != "ready" {
            return Err(Error::Corrupt(format!(
                "workspace layer `{layer_id}` is `{}` and cannot be attached",
                report.state
            )));
        }
        let storage_path = Path::new(&report.storage_path);
        let root_identity = workspace_layer_root_identity(storage_path)?;
        let stamp_path = workspace_layer_verification_stamp_path(storage_path);
        let marker_path = workspace_layer_marker_path(storage_path);
        let stamp = read_workspace_layer_sidecar::<WorkspaceLayerVerificationStamp>(&stamp_path);
        let marker = read_workspace_layer_sidecar::<WorkspaceLayerPublishMarker>(&marker_path);
        let stamp_matches = stamp.is_ok_and(|stamp| {
            stamp.version == WORKSPACE_LAYER_VERIFICATION_STAMP_VERSION
                && stamp.layer_id == layer_id
                && stamp.manifest_object_id == manifest_id
                && stamp.root_identity == root_identity
                && stamp.logical_bytes == report.logical_bytes
                && stamp.entry_count == report.entry_count
        });
        let marker_matches = marker.is_ok_and(|marker| {
            marker.layer_id == layer_id
                && marker.cache_key == report.cache_key
                && marker.manifest_object_id == manifest_id
                && marker.logical_bytes == report.logical_bytes
                && marker.entry_count == report.entry_count
                && report
                    .physical_bytes
                    .is_none_or(|bytes| bytes == marker.physical_bytes)
        });
        if stamp_matches && marker_matches && manifest.entries.len() as u64 == report.entry_count {
            return Ok(report);
        }
        self.verify_workspace_layer(layer_id)
    }

    /// Perform an explicit complete verification of every layer entry and
    /// refresh the bounded attach-tier stamp only after all hashes match.
    pub fn verify_workspace_layer(&self, layer_id: &str) -> Result<WorkspaceLayerReport> {
        let (mut report, manifest_id, manifest) =
            self.workspace_layer_verification_record(layer_id)?;
        let actual = scan_layer_entries(Path::new(&report.storage_path), false)?;
        if manifest.entries != actual {
            report.state = "corrupt".to_string();
            return Err(Error::Corrupt(format!(
                "workspace layer `{layer_id}` does not match its immutable manifest"
            )));
        }
        let _ = write_workspace_layer_verification_stamp(&report, &manifest_id);
        let _ = write_workspace_layer_publish_marker_from_report(&report, &manifest_id);
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
        self.bind_workspace_layer(
            lane,
            layer_id,
            mount_path,
            adapter,
            expected_key,
            false,
            None,
        )
    }

    #[cfg(test)]
    pub(crate) fn replace_workspace_layer(
        &self,
        lane: &str,
        layer_id: &str,
        mount_path: &str,
        adapter: &str,
        expected_key: &str,
    ) -> Result<LaneWorkspaceViewReport> {
        self.bind_workspace_layer(
            lane,
            layer_id,
            mount_path,
            adapter,
            expected_key,
            true,
            None,
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn replace_declared_workspace_layer(
        &self,
        lane: &str,
        layer_id: &str,
        mount_path: &str,
        component_id: &str,
        adapter_identity: &str,
        adapter_version: u32,
        implementation_version: &str,
        distribution_digest: &str,
        kind: &str,
        expected_key: &str,
    ) -> Result<LaneWorkspaceViewReport> {
        self.replace_declared_workspace_layers(
            lane,
            &[EnvironmentLayerActivation {
                layer_id: Some(layer_id.to_string()),
                outputs: vec![EnvironmentLayerOutputActivation {
                    name: "primary".to_string(),
                    mount_path: mount_path.to_string(),
                    policy: "immutable_seed_private".to_string(),
                    binding_identity: layer_id.to_string(),
                    private_seed: None,
                    layer_subpath: String::new(),
                }],
                component_id: component_id.to_string(),
                adapter_identity: adapter_identity.to_string(),
                adapter_version,
                implementation_version: implementation_version.to_string(),
                distribution_digest: distribution_digest.to_string(),
                kind: kind.to_string(),
                dependencies: Vec::new(),
                caches: Vec::new(),
                external_artifacts: Vec::new(),
                runtime_resources: Vec::new(),
                expected_key: expected_key.to_string(),
                canonical_key: self
                    .workspace_layer_key_by_cache_key(expected_key)?
                    .ok_or_else(|| {
                        Error::Corrupt(format!(
                            "workspace layer `{layer_id}` has no canonical key provenance"
                        ))
                    })?,
            }],
        )
    }

    #[cfg(test)]
    pub(crate) fn replace_declared_workspace_layers(
        &self,
        lane: &str,
        requested: &[EnvironmentLayerActivation],
    ) -> Result<LaneWorkspaceViewReport> {
        self.replace_declared_workspace_layers_with_removals(lane, requested, &[])
    }

    pub(crate) fn replace_declared_workspace_layers_at_source(
        &self,
        lane: &str,
        requested: &[EnvironmentLayerActivation],
        source_root: &ObjectId,
    ) -> Result<LaneWorkspaceViewReport> {
        self.replace_declared_workspace_layers_with_removals_internal(
            lane,
            requested,
            &[],
            Some(source_root),
        )
    }

    #[cfg(test)]
    pub(crate) fn replace_declared_workspace_layers_with_removals(
        &self,
        lane: &str,
        requested: &[EnvironmentLayerActivation],
        removed_components: &[String],
    ) -> Result<LaneWorkspaceViewReport> {
        self.replace_declared_workspace_layers_with_removals_internal(
            lane,
            requested,
            removed_components,
            None,
        )
    }

    pub(crate) fn replace_declared_workspace_layers_with_removals_at_source(
        &self,
        lane: &str,
        requested: &[EnvironmentLayerActivation],
        removed_components: &[String],
        source_root: &ObjectId,
    ) -> Result<LaneWorkspaceViewReport> {
        self.replace_declared_workspace_layers_with_removals_internal(
            lane,
            requested,
            removed_components,
            Some(source_root),
        )
    }

    fn replace_declared_workspace_layers_with_removals_internal(
        &self,
        lane: &str,
        requested: &[EnvironmentLayerActivation],
        removed_components: &[String],
        source_root: Option<&ObjectId>,
    ) -> Result<LaneWorkspaceViewReport> {
        if requested.is_empty() && removed_components.is_empty() {
            return Err(Error::InvalidInput(
                "environment activation requires at least one replacement or removal".to_string(),
            ));
        }
        let _lock = self.acquire_write_lock()?;
        if let Some(expected) = source_root {
            let branch = self.lane_branch(lane)?;
            let current = self.get_ref(&branch.ref_name)?.root_id;
            if &current != expected {
                return Err(Error::InvalidInput(format!(
                    "lane `{lane}` advanced from pinned source root `{expected}` to `{current}` before environment activation; retry against the new lane head"
                )));
            }
        }
        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered workspace view"
            ))
        })?;
        if let (Some(pid), Some(token)) = (view.owner_pid, view.owner_start_token.as_deref()) {
            if process_matches_start_token(pid, token) {
                return Err(Error::InvalidInput(format!(
                    "workspace view `{}` for lane `{lane}` has an active writer; run `trail lane unmount {lane}` before changing environment bindings",
                    view.view_id
                )));
            }
        }

        let mut requested_keys = BTreeMap::new();
        for activation in requested {
            super::workspace_environment::validate_environment_component_identity(
                &activation.component_id,
            )?;
            if requested_keys
                .insert(
                    activation.component_id.clone(),
                    activation.expected_key.clone(),
                )
                .is_some()
            {
                return Err(Error::InvalidInput(format!(
                    "environment activation contains component `{}` more than once",
                    activation.component_id
                )));
            }
        }
        let component_ids = requested_keys.keys().cloned().collect::<BTreeSet<_>>();
        let mut removed_ids = BTreeSet::new();
        for component_id in removed_components {
            super::workspace_environment::validate_environment_component_identity(component_id)?;
            if component_ids.contains(component_id) || !removed_ids.insert(component_id.clone()) {
                return Err(Error::InvalidInput(format!(
                    "environment activation both replaces or repeats removed component `{component_id}`"
                )));
            }
        }
        let replaced_component_ids = component_ids
            .union(&removed_ids)
            .cloned()
            .collect::<BTreeSet<_>>();
        let dependent_components =
            self.environment_dependency_descendants(&view.view_id, &replaced_component_ids)?;
        let mut removed_bindings = Vec::with_capacity(removed_ids.len());
        for component_id in &removed_ids {
            let mut stmt = self.conn.prepare(
                "SELECT mount_path, kind, policy, binding_identity
                 FROM environment_component_output_bindings
                 WHERE view_id = ?1 AND component_id = ?2
                 ORDER BY output_name",
            )?;
            let bindings = stmt
                .query_map(params![&view.view_id, component_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            removed_bindings.push((component_id.clone(), bindings));
        }
        let mut resolved = Vec::with_capacity(requested.len());
        let mut mount_paths = Vec::<(String, String)>::new();
        for activation in requested {
            if self.workspace_layer_cache_key(&activation.canonical_key)? != activation.expected_key
            {
                return Err(Error::Corrupt(format!(
                    "environment component `{}` canonical key does not match its expected key",
                    activation.component_id
                )));
            }
            let mut dependency_ids = BTreeSet::new();
            for (dependency, component_key, edge_type) in &activation.dependencies {
                super::workspace_environment::validate_environment_component_identity(dependency)?;
                if dependency == &activation.component_id
                    || !dependency_ids.insert(dependency.clone())
                {
                    return Err(Error::InvalidInput(format!(
                        "environment component `{}` has an invalid or duplicate dependency `{dependency}`",
                        activation.component_id
                    )));
                }
                let identity_input = match edge_type.as_str() {
                    "build_requires" => Some(format!("dependency:{dependency}")),
                    "invalidates_with" => Some(format!("dependency:invalidates_with:{dependency}")),
                    "runtime_requires" | "binds_after" => None,
                    other => {
                        return Err(Error::InvalidInput(format!(
                            "environment component `{}` has unknown dependency edge type `{other}`",
                            activation.component_id
                        )))
                    }
                };
                if identity_input.as_ref().is_some_and(|input| {
                    activation.canonical_key.inputs.get(input) != Some(component_key)
                }) {
                    return Err(Error::Corrupt(format!(
                        "environment component `{}` dependency `{dependency}` does not match its canonical key",
                        activation.component_id
                    )));
                }
                if let Some(requested_key) = requested_keys.get(dependency) {
                    if requested_key != component_key {
                        return Err(Error::InvalidInput(format!(
                            "environment component `{}` requires `{dependency}` at `{component_key}`, but this activation provides `{requested_key}`",
                            activation.component_id
                        )));
                    }
                    continue;
                }
                let attached = self
                    .conn
                    .query_row(
                        "SELECT attached_key, status FROM environment_component_states
                         WHERE view_id = ?1 AND component_id = ?2",
                        params![&view.view_id, dependency],
                        |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
                    )
                    .optional()?;
                if !matches!(attached, Some((Some(ref key), ref status)) if key == component_key && status == "ready")
                {
                    return Err(Error::InvalidInput(format!(
                        "environment component `{}` requires `{dependency}` at `{component_key}`, but that dependency is not ready in the lane or this activation",
                        activation.component_id
                    )));
                }
            }
            let mut cache_names = BTreeSet::new();
            for cache in &activation.caches {
                if !cache_names.insert(cache.name.clone())
                    || !cache.namespace_id.starts_with("cache_")
                    || cache.namespace_id.len() != "cache_".len() + 64
                    || !cache.namespace_id["cache_".len()..]
                        .chars()
                        .all(|character| character.is_ascii_hexdigit())
                    || !matches!(
                        cache.protocol.as_str(),
                        "content_store" | "compiler_cache" | "locked_index"
                    )
                    || !matches!(cache.access.as_str(), "tool_concurrent" | "host_exclusive")
                    || cache.authority != "performance_only"
                    || cache.scope != "workspace"
                    || cache.compatibility.is_empty()
                {
                    return Err(Error::InvalidInput(format!(
                        "environment component `{}` has an invalid cache declaration `{}`",
                        activation.component_id, cache.name
                    )));
                }
            }
            let mut external_artifact_names = BTreeSet::new();
            for artifact in &activation.external_artifacts {
                super::workspace_environment::validate_environment_external_artifact_report(
                    artifact,
                )?;
                if !external_artifact_names.insert(&artifact.name) {
                    return Err(Error::InvalidInput(format!(
                        "environment component `{}` repeats external artifact `{}`",
                        activation.component_id, artifact.name
                    )));
                }
            }
            let mut runtime_resource_names = BTreeSet::new();
            for resource in &activation.runtime_resources {
                super::workspace_environment::validate_environment_runtime_declaration_report(
                    resource,
                )?;
                if !runtime_resource_names.insert(&resource.name) {
                    return Err(Error::InvalidInput(format!(
                        "environment component `{}` repeats runtime resource `{}`",
                        activation.component_id, resource.name
                    )));
                }
                if !external_artifact_names.contains(&resource.artifact_name) {
                    return Err(Error::InvalidInput(format!(
                        "environment component `{}` runtime resource `{}` references missing external artifact `{}`",
                        activation.component_id, resource.name, resource.artifact_name
                    )));
                }
            }
            let layer = activation
                .layer_id
                .as_deref()
                .map(|layer_id| self.verify_workspace_layer_for_attach(layer_id))
                .transpose()?;
            if let Some(layer) = &layer {
                if layer.kind != activation.kind {
                    return Err(Error::Corrupt(format!(
                        "component `{}` declares kind `{}` but layer `{}` has kind `{}`",
                        activation.component_id, activation.kind, layer.layer_id, layer.kind
                    )));
                }
            }
            let mut previous_stmt = self.conn.prepare(
                "SELECT mount_path, kind, policy, binding_identity
                 FROM environment_component_output_bindings
                 WHERE view_id = ?1 AND component_id = ?2 ORDER BY output_name",
            )?;
            let previous_bindings = previous_stmt
                .query_map(params![&view.view_id, &activation.component_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if activation.outputs.is_empty()
                && activation.external_artifacts.is_empty()
                && activation.runtime_resources.is_empty()
            {
                return Err(Error::InvalidInput(format!(
                    "environment component `{}` activation has neither outputs nor external/runtime resources",
                    activation.component_id
                )));
            }
            if (!activation.external_artifacts.is_empty()
                || !activation.runtime_resources.is_empty())
                && (layer.is_some() || !activation.outputs.is_empty())
            {
                return Err(Error::InvalidInput(format!(
                    "environment component `{}` mixes external/runtime resources with filesystem layer outputs",
                    activation.component_id
                )));
            }
            let mut names = BTreeSet::new();
            let mut outputs = Vec::with_capacity(activation.outputs.len());
            for output in &activation.outputs {
                if !names.insert(output.name.clone()) {
                    return Err(Error::InvalidInput(format!(
                        "environment component `{}` activation repeats output `{}`",
                        activation.component_id, output.name
                    )));
                }
                let mount_path = normalize_relative_path(&output.mount_path)?;
                let layer_subpath = if output.layer_subpath.is_empty() {
                    String::new()
                } else {
                    normalize_relative_path(&output.layer_subpath)?
                };
                if output.binding_identity.trim().is_empty() {
                    return Err(Error::InvalidInput(format!(
                        "environment component `{}` output `{}` has an empty binding identity",
                        activation.component_id, output.name
                    )));
                }
                match output.policy.as_str() {
                    "immutable_seed_private" => {
                        let layer = layer.as_ref().ok_or_else(|| {
                            Error::Corrupt(format!(
                                "environment component `{}` immutable output `{}` has no layer",
                                activation.component_id, output.name
                            ))
                        })?;
                        if output.private_seed.is_some()
                            || output.binding_identity != layer.layer_id
                        {
                            return Err(Error::Corrupt(format!(
                                "environment component `{}` immutable output `{}` has inconsistent storage identity",
                                activation.component_id, output.name
                            )));
                        }
                        let source = if layer_subpath.is_empty() {
                            PathBuf::from(&layer.storage_path)
                        } else {
                            safe_join(Path::new(&layer.storage_path), &layer_subpath)?
                        };
                        if !source.is_dir() {
                            return Err(Error::Corrupt(format!(
                                "environment component `{}` output `{}` refers to missing layer directory `{layer_subpath}`",
                                activation.component_id, output.name
                            )));
                        }
                    }
                    "writable_private" => {
                        if layer.is_some() || !layer_subpath.is_empty() {
                            return Err(Error::Corrupt(format!(
                                "environment component `{}` writable-private output `{}` must not reference an immutable layer",
                                activation.component_id, output.name
                            )));
                        }
                        if output
                            .private_seed
                            .as_ref()
                            .is_some_and(|path| !path.is_dir())
                        {
                            return Err(Error::InvalidInput(format!(
                                "environment component `{}` writable-private seed for `{}` is not a directory",
                                activation.component_id, output.name
                            )));
                        }
                    }
                    other => {
                        return Err(Error::InvalidInput(format!(
                        "environment component `{}` output `{}` has unsupported policy `{other}`",
                        activation.component_id, output.name
                    )))
                    }
                }
                for (owner, existing) in &mount_paths {
                    if mount_paths_overlap(&mount_path, existing) {
                        return Err(Error::InvalidInput(format!(
                            "environment component `{}` mount `{mount_path}` overlaps batch component `{owner}` mount `{existing}`",
                            activation.component_id
                        )));
                    }
                }
                mount_paths.push((activation.component_id.clone(), mount_path.clone()));
                outputs.push(EnvironmentLayerOutputActivation {
                    name: output.name.clone(),
                    mount_path,
                    layer_subpath,
                    policy: output.policy.clone(),
                    binding_identity: output.binding_identity.clone(),
                    private_seed: output.private_seed.clone(),
                });
            }
            resolved.push((activation.clone(), layer, outputs, previous_bindings));
        }
        self.validate_environment_batch_mount_ownership(
            &view.view_id,
            &replaced_component_ids,
            &mount_paths,
        )?;

        let mut core = ViewCore::new_lazy(
            // The outer environment mutation owns the workspace writer lock
            // and its Trail handle already completed open-time recovery.
            Trail::open_without_recovering_derived_paths(self.workspace_root(), self.db_dir())?,
            PathBuf::from(&view.source_upper),
            view.base_root.clone(),
        )?;
        let mut resets: Vec<PreparedLayerMountReset> = Vec::new();
        for (_, bindings) in &removed_bindings {
            for (mount_path, kind, _, _) in bindings {
                match core.prepare_declared_layer_unmount_path(mount_path, kind) {
                    Ok(reset) => resets.push(reset),
                    Err(err) => {
                        for reset in resets.into_iter().rev() {
                            let _ = reset.rollback(&mut core);
                        }
                        return Err(err);
                    }
                }
            }
        }
        for (component, layer, outputs, previous_bindings) in &resolved {
            for output in outputs {
                if output.policy == "writable_private"
                    && output.private_seed.is_none()
                    && previous_bindings
                        .iter()
                        .any(|(mount, _, policy, binding_identity)| {
                            mount == &output.mount_path
                                && policy == &output.policy
                                && binding_identity == &output.binding_identity
                        })
                {
                    if let Err(err) =
                        core.ensure_declared_private_mount_path(&output.mount_path, &component.kind)
                    {
                        for reset in resets.into_iter().rev() {
                            let _ = reset.rollback(&mut core);
                        }
                        return Err(err);
                    }
                    continue;
                }
                let prepared = if output.policy == "writable_private" {
                    core.prepare_declared_private_mount_path(
                        &output.mount_path,
                        &component.kind,
                        &output.binding_identity,
                    )
                } else {
                    let layer = layer.as_ref().ok_or_else(|| {
                        Error::Corrupt(format!(
                            "immutable output `{}` lost its prepared layer",
                            output.name
                        ))
                    })?;
                    core.prepare_declared_layer_mount_path(
                        &output.mount_path,
                        &layer.kind,
                        &layer.layer_id,
                    )
                };
                match prepared {
                    Ok(reset) => {
                        let install = if let Some(seed) = &output.private_seed {
                            reset.install_private_directory(seed)
                        } else if output.policy == "writable_private" {
                            core.ensure_declared_private_mount_path(
                                &output.mount_path,
                                &component.kind,
                            )
                        } else {
                            Ok(())
                        };
                        if let Err(err) = install {
                            let _ = reset.rollback(&mut core);
                            for reset in resets.into_iter().rev() {
                                let _ = reset.rollback(&mut core);
                            }
                            return Err(err);
                        }
                        resets.push(reset);
                    }
                    Err(err) => {
                        for reset in resets.into_iter().rev() {
                            let _ = reset.rollback(&mut core);
                        }
                        return Err(err);
                    }
                }
            }
            let replacement_mounts = outputs
                .iter()
                .map(|output| output.mount_path.as_str())
                .collect::<BTreeSet<_>>();
            for (previous_mount, previous_kind, _, _) in previous_bindings {
                if replacement_mounts.contains(previous_mount.as_str()) {
                    continue;
                }
                match core.prepare_declared_layer_unmount_path(previous_mount, previous_kind) {
                    Ok(reset) => resets.push(reset),
                    Err(err) => {
                        for reset in resets.into_iter().rev() {
                            let _ = reset.rollback(&mut core);
                        }
                        return Err(err);
                    }
                }
            }
        }
        test_crash_point("environment_after_upper_resets");

        self.conn
            .execute_batch("SAVEPOINT trail_environment_activation")?;
        let activation = (|| -> Result<()> {
            for (component_id, bindings) in &removed_bindings {
                for (mount_path, _, _, _) in bindings {
                    self.conn.execute(
                        "DELETE FROM workspace_view_layers WHERE view_id = ?1 AND mount_path = ?2",
                        params![&view.view_id, mount_path],
                    )?;
                }
                self.conn.execute(
                    "DELETE FROM environment_component_output_bindings WHERE view_id = ?1 AND component_id = ?2",
                    params![&view.view_id, component_id],
                )?;
                self.conn.execute(
                    "DELETE FROM environment_component_bindings WHERE view_id = ?1 AND component_id = ?2",
                    params![&view.view_id, component_id],
                )?;
                self.conn.execute(
                    "DELETE FROM environment_component_dependencies WHERE view_id = ?1 AND component_id = ?2",
                    params![&view.view_id, component_id],
                )?;
                self.conn.execute(
                    "DELETE FROM environment_component_caches WHERE view_id = ?1 AND component_id = ?2",
                    params![&view.view_id, component_id],
                )?;
                self.conn.execute(
                    "DELETE FROM environment_component_external_artifacts WHERE view_id = ?1 AND component_id = ?2",
                    params![&view.view_id, component_id],
                )?;
                self.conn.execute(
                    "DELETE FROM environment_component_runtime_secrets WHERE view_id = ?1 AND component_id = ?2",
                    params![&view.view_id, component_id],
                )?;
                self.conn.execute(
                    "DELETE FROM environment_component_runtime_resources WHERE view_id = ?1 AND component_id = ?2",
                    params![&view.view_id, component_id],
                )?;
                self.conn.execute(
                    "DELETE FROM environment_component_states WHERE view_id = ?1 AND component_id = ?2",
                    params![&view.view_id, component_id],
                )?;
                self.conn.execute(
                    "DELETE FROM workspace_environment_states WHERE view_id = ?1 AND adapter = ?2",
                    params![&view.view_id, component_id],
                )?;
            }
            for (component, layer, outputs, previous_bindings) in &resolved {
                self.conn.execute(
                    "INSERT OR IGNORE INTO environment_component_key_provenance
                     (component_key, canonical_key_json, created_at) VALUES (?1, ?2, ?3)",
                    params![
                        &component.expected_key,
                        serde_json::to_vec(&component.canonical_key)?,
                        now_ts()
                    ],
                )?;
                let stored_key = self.conn.query_row(
                    "SELECT canonical_key_json FROM environment_component_key_provenance
                     WHERE component_key = ?1",
                    params![&component.expected_key],
                    |row| row.get::<_, Vec<u8>>(0),
                )?;
                let stored_key: WorkspaceLayerKeyV1 =
                    serde_json::from_slice(&stored_key).map_err(|error| {
                        Error::Corrupt(format!(
                            "environment key provenance `{}` is malformed: {error}",
                            component.expected_key
                        ))
                    })?;
                if stored_key != component.canonical_key {
                    return Err(Error::Corrupt(format!(
                        "environment key provenance `{}` does not match its content identity",
                        component.expected_key
                    )));
                }
                for (previous_mount, _, _, _) in previous_bindings {
                    self.conn.execute(
                        "DELETE FROM workspace_view_layers WHERE view_id = ?1 AND mount_path = ?2",
                        params![&view.view_id, previous_mount],
                    )?;
                }
                self.conn.execute(
                    "DELETE FROM environment_component_output_bindings WHERE view_id = ?1 AND component_id = ?2",
                    params![&view.view_id, &component.component_id],
                )?;
                self.conn.execute(
                    "DELETE FROM environment_component_bindings WHERE view_id = ?1 AND component_id = ?2",
                    params![&view.view_id, &component.component_id],
                )?;
                self.conn.execute(
                    "DELETE FROM environment_component_dependencies WHERE view_id = ?1 AND component_id = ?2",
                    params![&view.view_id, &component.component_id],
                )?;
                self.conn.execute(
                    "DELETE FROM environment_component_caches WHERE view_id = ?1 AND component_id = ?2",
                    params![&view.view_id, &component.component_id],
                )?;
                self.conn.execute(
                    "DELETE FROM environment_component_external_artifacts WHERE view_id = ?1 AND component_id = ?2",
                    params![&view.view_id, &component.component_id],
                )?;
                self.conn.execute(
                    "DELETE FROM environment_component_runtime_secrets WHERE view_id = ?1 AND component_id = ?2",
                    params![&view.view_id, &component.component_id],
                )?;
                self.conn.execute(
                    "DELETE FROM environment_component_runtime_resources WHERE view_id = ?1 AND component_id = ?2",
                    params![&view.view_id, &component.component_id],
                )?;
                for (dependency, dependency_key, edge_type) in &component.dependencies {
                    self.conn.execute(
                        "INSERT INTO environment_component_dependencies
                         (view_id, component_id, dependency_component_id, dependency_component_key, edge_type, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![
                            &view.view_id,
                            &component.component_id,
                            dependency,
                            dependency_key,
                            edge_type,
                            now_ts()
                        ],
                    )?;
                }
                for cache in &component.caches {
                    let compatibility_json = serde_json::to_vec(&cache.compatibility)?;
                    let storage_path = self
                        .db_dir
                        .join("cache/namespaces")
                        .join(&cache.namespace_id)
                        .to_string_lossy()
                        .into_owned();
                    self.conn.execute(
                        "INSERT OR IGNORE INTO environment_cache_namespaces
                         (namespace_id, adapter_identity, cache_name, protocol, access, authority, scope, compatibility_json, storage_path, last_used_at, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, 'performance_only', 'workspace', ?6, ?7, ?8, ?8)",
                        params![
                            &cache.namespace_id,
                            &component.adapter_identity,
                            &cache.name,
                            &cache.protocol,
                            &cache.access,
                            &compatibility_json,
                            &storage_path,
                            now_ts()
                        ],
                    )?;
                    let stored = self.conn.query_row(
                        "SELECT adapter_identity, cache_name, protocol, access, authority, scope, compatibility_json, storage_path
                         FROM environment_cache_namespaces WHERE namespace_id = ?1",
                        params![&cache.namespace_id],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, String>(4)?,
                                row.get::<_, String>(5)?,
                                row.get::<_, Vec<u8>>(6)?,
                                row.get::<_, String>(7)?,
                            ))
                        },
                    )?;
                    if stored
                        != (
                            component.adapter_identity.clone(),
                            cache.name.clone(),
                            cache.protocol.clone(),
                            cache.access.clone(),
                            "performance_only".to_string(),
                            "workspace".to_string(),
                            compatibility_json.clone(),
                            storage_path,
                        )
                    {
                        return Err(Error::Corrupt(format!(
                            "environment cache namespace `{}` has conflicting provenance",
                            cache.namespace_id
                        )));
                    }
                    self.conn.execute(
                        "INSERT INTO environment_component_caches
                         (view_id, component_id, cache_name, namespace_id, protocol, access, compatibility_json, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        params![
                            &view.view_id,
                            &component.component_id,
                            &cache.name,
                            &cache.namespace_id,
                            &cache.protocol,
                            &cache.access,
                            &compatibility_json,
                            now_ts()
                        ],
                    )?;
                }
                for artifact in &component.external_artifacts {
                    self.conn.execute(
                        "INSERT INTO environment_component_external_artifacts
                         (view_id, component_id, artifact_name, artifact_type, provider, reference, digest, platform, cleanup_owner, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                        params![
                            &view.view_id,
                            &component.component_id,
                            &artifact.name,
                            &artifact.artifact_type,
                            &artifact.provider,
                            &artifact.reference,
                            &artifact.digest,
                            &artifact.platform,
                            &artifact.cleanup_owner,
                            now_ts()
                        ],
                    )?;
                }
                for resource in &component.runtime_resources {
                    self.conn.execute(
                        "INSERT INTO environment_component_runtime_resources
                         (view_id, component_id, resource_name, runtime_type, provider, artifact_name,
                          container_port, protocol, health_type, health_timeout_ms, restart_policy,
                          cleanup_owner, volume_target, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                        params![
                            &view.view_id,
                            &component.component_id,
                            &resource.name,
                            &resource.runtime_type,
                            &resource.provider,
                            &resource.artifact_name,
                            resource.container_port,
                            &resource.protocol,
                            &resource.health_type,
                            resource.health_timeout_ms,
                            &resource.restart_policy,
                            &resource.cleanup_owner,
                            &resource.volume_target,
                            now_ts()
                        ],
                    )?;
                    for secret in &resource.secrets {
                        self.conn.execute(
                            "INSERT INTO environment_component_runtime_secrets
                             (view_id, component_id, resource_name, secret_name, provider,
                              reference, version, purpose, injection, target, environment,
                              required, updated_at)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                            params![
                                &view.view_id,
                                &component.component_id,
                                &resource.name,
                                &secret.name,
                                &secret.provider,
                                &secret.reference,
                                &secret.version,
                                &secret.purpose,
                                &secret.injection,
                                &secret.target,
                                &secret.environment,
                                secret.required,
                                now_ts()
                            ],
                        )?;
                    }
                }
                for output in outputs {
                    if output.policy == "immutable_seed_private" {
                        let layer = layer.as_ref().ok_or_else(|| {
                            Error::Corrupt(format!(
                                "immutable output `{}` lost its layer during activation",
                                output.name
                            ))
                        })?;
                        self.conn.execute(
                            "INSERT OR REPLACE INTO workspace_view_layers (view_id, layer_id, mount_path, priority, read_only, source_path) VALUES (?1, ?2, ?3, 100, 1, ?4)",
                            params![
                                &view.view_id,
                                &layer.layer_id,
                                &output.mount_path,
                                &output.layer_subpath
                            ],
                        )?;
                    }
                    self.conn.execute(
                        "INSERT INTO environment_component_output_bindings
                         (view_id, component_id, output_name, mount_path, layer_subpath, policy, binding_identity, kind, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                        params![
                            &view.view_id,
                            &component.component_id,
                            &output.name,
                            &output.mount_path,
                            &output.layer_subpath,
                            &output.policy,
                            &output.binding_identity,
                            &component.kind,
                            now_ts()
                        ],
                    )?;
                }
                if let Some(layer) = layer {
                    self.conn.execute(
                        "UPDATE workspace_layers SET last_used_at = ?1 WHERE layer_id = ?2",
                        params![now_ts(), &layer.layer_id],
                    )?;
                }
                self.conn.execute(
                    "INSERT OR REPLACE INTO workspace_environment_states (view_id, adapter, expected_key, attached_key, status, reason, updated_at) VALUES (?1, ?2, ?3, ?4, 'ready', NULL, ?5)",
                    params![
                        &view.view_id,
                        &component.component_id,
                        &component.expected_key,
                        &component.expected_key,
                        now_ts()
                    ],
                )?;
                self.conn.execute(
                    "INSERT OR REPLACE INTO environment_component_states (view_id, component_id, adapter_identity, adapter_version, implementation_version, distribution_digest, kind, expected_key, attached_key, status, reason, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'ready', NULL, ?10)",
                    params![
                        &view.view_id,
                        &component.component_id,
                        &component.adapter_identity,
                        component.adapter_version,
                        &component.implementation_version,
                        &component.distribution_digest,
                        &component.kind,
                        &component.expected_key,
                        &component.expected_key,
                        now_ts()
                    ],
                )?;
                if let Some(primary_output) = outputs.first() {
                    self.conn.execute(
                        "INSERT OR REPLACE INTO environment_component_bindings (view_id, component_id, mount_path, kind, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![
                            &view.view_id,
                            &component.component_id,
                            &primary_output.mount_path,
                            &component.kind,
                            now_ts()
                        ],
                    )?;
                }
            }
            for dependent in dependent_components
                .iter()
                .filter(|component| !replaced_component_ids.contains(*component))
            {
                let reason = format!(
                    "an upstream environment dependency changed; run `trail env sync-all {lane}`"
                );
                self.conn.execute(
                    "UPDATE environment_component_states
                     SET status = 'stale', reason = ?1, updated_at = ?2
                     WHERE view_id = ?3 AND component_id = ?4 AND attached_key IS NOT NULL",
                    params![&reason, now_ts(), &view.view_id, dependent],
                )?;
                self.conn.execute(
                    "UPDATE workspace_environment_states
                     SET status = 'stale', reason = ?1, updated_at = ?2
                     WHERE view_id = ?3 AND adapter = ?4 AND attached_key IS NOT NULL",
                    params![&reason, now_ts(), &view.view_id, dependent],
                )?;
            }
            // Runtime/order-only edges select an upstream instance for this
            // generation but do not define the consumer artifact. Advance
            // those exact keys without rebuilding or marking the consumer
            // stale when an upstream component is replaced.
            for (component_id, component_key) in &requested_keys {
                self.conn.execute(
                    "UPDATE environment_component_dependencies
                     SET dependency_component_key = ?1, updated_at = ?2
                     WHERE view_id = ?3 AND dependency_component_id = ?4
                       AND edge_type IN ('runtime_requires', 'binds_after')",
                    params![component_key, now_ts(), &view.view_id, component_id],
                )?;
            }
            self.record_environment_generation(lane, &view.view_id)?;
            self.conn.execute(
                "UPDATE workspace_views SET generation = generation + 1, updated_at = ?1 WHERE view_id = ?2",
                params![now_ts(), &view.view_id],
            )?;
            Ok(())
        })();
        let activation = activation.and_then(|()| {
            self.conn
                .execute_batch("RELEASE SAVEPOINT trail_environment_activation")
                .map_err(Error::from)
        });
        match activation {
            Ok(()) => {
                test_crash_point("environment_after_generation_commit");
                for reset in resets {
                    reset.commit();
                }
            }
            Err(err) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO SAVEPOINT trail_environment_activation; RELEASE SAVEPOINT trail_environment_activation",
                );
                let mut rollback_failure = None;
                for reset in resets.into_iter().rev() {
                    if let Err(reset_err) = reset.rollback(&mut core) {
                        rollback_failure = Some(reset_err);
                        break;
                    }
                }
                if let Some(reset_err) = rollback_failure {
                    return Err(Error::Corrupt(format!(
                        "environment activation failed ({err}); restoring private generated state also failed ({reset_err}); durable recovery intents were retained"
                    )));
                }
                return Err(err);
            }
        }
        self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::Corrupt("workspace view disappeared after environment activation".to_string())
        })
    }

    fn bind_workspace_layer(
        &self,
        lane: &str,
        layer_id: &str,
        mount_path: &str,
        component_id: &str,
        expected_key: &str,
        replace_private_upper: bool,
        normalized_environment: Option<(&str, u32, &str, &str, &str)>,
    ) -> Result<LaneWorkspaceViewReport> {
        let _lock = self.acquire_write_lock()?;
        let layer = self.verify_workspace_layer_for_attach(layer_id)?;
        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered workspace view"
            ))
        })?;
        if let (Some(pid), Some(token)) = (view.owner_pid, view.owner_start_token.as_deref()) {
            if process_matches_start_token(pid, token) {
                return Err(Error::InvalidInput(format!(
                    "workspace view `{}` for lane `{lane}` has an active writer; run `trail lane unmount {lane}` before changing layer bindings",
                    view.view_id,
                )));
            }
        }
        let mount_path = normalize_relative_path(mount_path)?;
        if normalized_environment.is_some() {
            self.validate_environment_mount_ownership(&view.view_id, component_id, &mount_path)?;
        }
        let mut prepared_reset = if replace_private_upper {
            let mut core = ViewCore::new_lazy(
                // The outer layer mutation owns the workspace writer lock and
                // its Trail handle already completed open-time recovery.
                Trail::open_without_recovering_derived_paths(self.workspace_root(), self.db_dir())?,
                PathBuf::from(&view.source_upper),
                view.base_root.clone(),
            )?;
            let reset =
                core.prepare_declared_layer_mount_path(&mount_path, &layer.kind, &layer.layer_id)?;
            test_crash_point("layer_after_upper_reset");
            Some((core, reset))
        } else {
            None
        };

        self.conn
            .execute_batch("SAVEPOINT trail_layer_activation")?;
        let activation = (|| -> Result<()> {
            if normalized_environment.is_some() {
                let previous_mount = self
                    .conn
                    .query_row(
                        "SELECT mount_path FROM environment_component_bindings WHERE view_id = ?1 AND component_id = ?2",
                        params![&view.view_id, component_id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?;
                if previous_mount
                    .as_deref()
                    .is_some_and(|path| path != mount_path)
                {
                    self.conn.execute(
                        "DELETE FROM workspace_view_layers WHERE view_id = ?1 AND mount_path = ?2",
                        params![&view.view_id, previous_mount],
                    )?;
                }
            }
            self.conn.execute(
                "INSERT OR REPLACE INTO workspace_view_layers (view_id, layer_id, mount_path, priority, read_only) VALUES (?1, ?2, ?3, 100, 1)",
                params![&view.view_id, &layer.layer_id, &mount_path],
            )?;
            self.conn.execute(
                "UPDATE workspace_layers SET last_used_at = ?1 WHERE layer_id = ?2",
                params![now_ts(), &layer.layer_id],
            )?;
            self.conn.execute(
                "INSERT OR REPLACE INTO workspace_environment_states (view_id, adapter, expected_key, attached_key, status, reason, updated_at) VALUES (?1, ?2, ?3, ?4, 'ready', NULL, ?5)",
                params![&view.view_id, component_id, expected_key, &layer.cache_key, now_ts()],
            )?;
            if let Some((
                adapter_identity,
                adapter_version,
                implementation_version,
                distribution_digest,
                kind,
            )) = normalized_environment
            {
                self.conn.execute(
                    "INSERT OR REPLACE INTO environment_component_states (view_id, component_id, adapter_identity, adapter_version, implementation_version, distribution_digest, kind, expected_key, attached_key, status, reason, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'ready', NULL, ?10)",
                    params![
                        &view.view_id,
                        component_id,
                        adapter_identity,
                        adapter_version,
                        implementation_version,
                        distribution_digest,
                        kind,
                        expected_key,
                        &layer.cache_key,
                        now_ts()
                    ],
                )?;
                self.conn.execute(
                    "INSERT OR REPLACE INTO environment_component_bindings (view_id, component_id, mount_path, kind, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![&view.view_id, component_id, &mount_path, kind, now_ts()],
                )?;
                self.record_environment_generation(lane, &view.view_id)?;
            }
            self.conn.execute(
                "UPDATE workspace_views SET generation = generation + 1, updated_at = ?1 WHERE view_id = ?2",
                params![now_ts(), &view.view_id],
            )?;
            Ok(())
        })();
        let activation = activation.and_then(|()| {
            self.conn
                .execute_batch("RELEASE SAVEPOINT trail_layer_activation")
                .map_err(Error::from)
        });
        match activation {
            Ok(()) => {
                test_crash_point("layer_after_binding_commit");
                if let Some((_core, reset)) = prepared_reset.take() {
                    reset.commit();
                }
            }
            Err(err) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO SAVEPOINT trail_layer_activation; RELEASE SAVEPOINT trail_layer_activation",
                );
                if let Some((mut core, reset)) = prepared_reset.take() {
                    if let Err(rollback_err) = reset.rollback(&mut core) {
                        return Err(Error::Corrupt(format!(
                            "workspace layer activation failed ({err}); restoring private generated state also failed ({rollback_err}); durable recovery intent was retained"
                        )));
                    }
                }
                return Err(err);
            }
        }
        self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::Corrupt("workspace view disappeared after layer attach".to_string())
        })
    }

    fn environment_dependency_descendants(
        &self,
        view_id: &str,
        roots: &BTreeSet<String>,
    ) -> Result<BTreeSet<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT component_id, dependency_component_id
             FROM environment_component_dependencies
             WHERE view_id = ?1
               AND edge_type IN ('build_requires', 'invalidates_with')
             ORDER BY dependency_component_id, component_id",
        )?;
        let edges = stmt
            .query_map(params![view_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut adjacency = BTreeMap::<String, Vec<String>>::new();
        for (component, dependency) in edges {
            adjacency.entry(dependency).or_default().push(component);
        }
        let mut queue = roots.iter().cloned().collect::<VecDeque<_>>();
        let mut descendants = BTreeSet::new();
        while let Some(component) = queue.pop_front() {
            if let Some(children) = adjacency.get(&component) {
                for child in children {
                    if descendants.insert(child.clone()) {
                        queue.push_back(child.clone());
                    }
                }
            }
        }
        Ok(descendants)
    }

    fn validate_environment_mount_ownership(
        &self,
        view_id: &str,
        component_id: &str,
        mount_path: &str,
    ) -> Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT component_id, mount_path FROM environment_component_bindings WHERE view_id = ?1",
        )?;
        let bindings = stmt
            .query_map(params![view_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (owner, existing) in bindings {
            if owner != component_id && mount_paths_overlap(mount_path, &existing) {
                return Err(Error::InvalidInput(format!(
                    "environment component `{component_id}` mount `{mount_path}` overlaps `{existing}` owned by component `{owner}`"
                )));
            }
        }
        let mut stmt = self
            .conn
            .prepare("SELECT mount_path FROM workspace_view_layers WHERE view_id = ?1")?;
        let existing_mounts = stmt
            .query_map(params![view_id], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for existing in existing_mounts {
            let owned_by_component = self.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM environment_component_bindings WHERE view_id = ?1 AND component_id = ?2 AND mount_path = ?3)",
                params![view_id, component_id, &existing],
                |row| row.get::<_, bool>(0),
            )?;
            if !owned_by_component
                && existing != mount_path
                && mount_paths_overlap(mount_path, &existing)
            {
                return Err(Error::InvalidInput(format!(
                    "environment component `{component_id}` mount `{mount_path}` overlaps existing layer mount `{existing}`"
                )));
            }
        }
        Ok(())
    }

    fn validate_environment_batch_mount_ownership(
        &self,
        view_id: &str,
        replaced_components: &BTreeSet<String>,
        requested_mounts: &[(String, String)],
    ) -> Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT component_id, mount_path FROM environment_component_output_bindings WHERE view_id = ?1",
        )?;
        let bindings = stmt
            .query_map(params![view_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (owner, existing) in &bindings {
            if replaced_components.contains(owner) {
                continue;
            }
            for (component, requested) in requested_mounts {
                if mount_paths_overlap(requested, existing) {
                    return Err(Error::InvalidInput(format!(
                        "environment component `{component}` mount `{requested}` overlaps `{existing}` owned by component `{owner}`"
                    )));
                }
            }
        }

        let known_mounts = bindings
            .iter()
            .map(|(_, mount)| mount.as_str())
            .collect::<BTreeSet<_>>();
        let mut stmt = self
            .conn
            .prepare("SELECT mount_path FROM workspace_view_layers WHERE view_id = ?1")?;
        let layer_mounts = stmt
            .query_map(params![view_id], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for existing in layer_mounts {
            if known_mounts.contains(existing.as_str()) {
                continue;
            }
            for (component, requested) in requested_mounts {
                if existing != *requested && mount_paths_overlap(requested, &existing) {
                    return Err(Error::InvalidInput(format!(
                        "environment component `{component}` mount `{requested}` overlaps existing layer mount `{existing}`"
                    )));
                }
            }
        }
        Ok(())
    }

    fn record_environment_generation(&self, lane: &str, view_id: &str) -> Result<String> {
        let branch = self.lane_branch(lane)?;
        let head = self.get_ref(&branch.ref_name)?;
        let mut stmt = self.conn.prepare(
            "SELECT s.component_id, s.adapter_identity, s.kind,
                    COALESCE(s.attached_key, s.expected_key), l.layer_id, b.mount_path
             FROM environment_component_states s
             LEFT JOIN environment_component_bindings b
               ON b.view_id = s.view_id AND b.component_id = s.component_id
             LEFT JOIN workspace_view_layers l
               ON l.view_id = b.view_id AND l.mount_path = b.mount_path
             WHERE s.view_id = ?1
               AND s.attached_key IS NOT NULL
             ORDER BY s.component_id",
        )?;
        let mut components = stmt
            .query_map(params![view_id], |row| {
                Ok(EnvironmentGenerationComponentReport {
                    component_id: row.get(0)?,
                    adapter_identity: row.get(1)?,
                    kind: row.get(2)?,
                    component_key: row.get(3)?,
                    layer_id: row.get(4)?,
                    mount_path: row.get(5)?,
                    dependencies: Vec::new(),
                    outputs: Vec::new(),
                    caches: Vec::new(),
                    external_artifacts: Vec::new(),
                    runtime_resources: Vec::new(),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for component in &mut components {
            let dependencies = {
                let mut dependency_stmt = self.conn.prepare(
                    "SELECT dependency_component_id, dependency_component_key, edge_type
                     FROM environment_component_dependencies
                     WHERE view_id = ?1 AND component_id = ?2
                     ORDER BY dependency_component_id",
                )?;
                let dependencies = dependency_stmt
                    .query_map(params![view_id, &component.component_id], |row| {
                        Ok(EnvironmentGenerationDependencyReport {
                            component_id: row.get(0)?,
                            component_key: row.get(1)?,
                            edge_type: row.get(2)?,
                        })
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                dependencies
            };
            component.dependencies = dependencies;
            let mut output_stmt = self.conn.prepare(
                "SELECT b.output_name, b.policy, b.binding_identity, l.layer_id,
                        b.mount_path, b.layer_subpath
                 FROM environment_component_output_bindings b
                 LEFT JOIN workspace_view_layers l
                   ON l.view_id = b.view_id AND l.mount_path = b.mount_path
                 WHERE b.view_id = ?1 AND b.component_id = ?2
                 ORDER BY b.output_name",
            )?;
            component.outputs = output_stmt
                .query_map(params![view_id, &component.component_id], |row| {
                    Ok(EnvironmentGenerationOutputReport {
                        name: row.get(0)?,
                        policy: row.get(1)?,
                        storage_identity: row.get(2)?,
                        layer_id: row.get(3)?,
                        mount_path: row.get(4)?,
                        layer_subpath: row.get(5)?,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let mut cache_stmt = self.conn.prepare(
                "SELECT cache_name, namespace_id, protocol, access, compatibility_json
                 FROM environment_component_caches
                 WHERE view_id = ?1 AND component_id = ?2
                 ORDER BY cache_name",
            )?;
            component.caches = cache_stmt
                .query_map(params![view_id, &component.component_id], |row| {
                    let compatibility = row.get::<_, Vec<u8>>(4)?;
                    let compatibility =
                        serde_json::from_slice(&compatibility).map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                4,
                                rusqlite::types::Type::Blob,
                                Box::new(error),
                            )
                        })?;
                    Ok(EnvironmentCacheReport {
                        name: row.get(0)?,
                        namespace_id: row.get(1)?,
                        protocol: row.get(2)?,
                        access: row.get(3)?,
                        authority: "performance_only".to_string(),
                        scope: "workspace".to_string(),
                        compatibility,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let mut external_stmt = self.conn.prepare(
                "SELECT artifact_name, artifact_type, provider, reference, digest, platform, cleanup_owner
                 FROM environment_component_external_artifacts
                 WHERE view_id = ?1 AND component_id = ?2
                 ORDER BY artifact_name",
            )?;
            component.external_artifacts = external_stmt
                .query_map(params![view_id, &component.component_id], |row| {
                    Ok(EnvironmentExternalArtifactReport {
                        name: row.get(0)?,
                        artifact_type: row.get(1)?,
                        provider: row.get(2)?,
                        reference: row.get(3)?,
                        digest: row.get(4)?,
                        platform: row.get(5)?,
                        cleanup_owner: row.get(6)?,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
        }
        let specification_digest = sha256_hex(&serde_json::to_vec(&components)?);
        let sequence = self.conn.query_row(
            "SELECT COALESCE(MAX(generation_sequence), 0) + 1 FROM environment_generations WHERE view_id = ?1",
            params![view_id],
            |row| row.get::<_, u64>(0),
        )?;
        let predecessor = self
            .conn
            .query_row(
                "SELECT generation_id FROM environment_view_generations WHERE view_id = ?1",
                params![view_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let generation_id = format!(
            "envgen_{}",
            &sha256_hex(
                format!(
                    "{view_id}:{sequence}:{}:{specification_digest}",
                    head.root_id.0
                )
                .as_bytes()
            )[..32]
        );
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO environment_generations
             (generation_id, view_id, generation_sequence, source_root, specification_digest, predecessor_generation_id, state, created_at, activated_at, retired_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?7, NULL)",
            params![
                &generation_id,
                view_id,
                sequence,
                head.root_id.0,
                &specification_digest,
                predecessor.as_deref(),
                now
            ],
        )?;
        for component in &components {
            self.conn.execute(
                "INSERT INTO environment_generation_components
                 (generation_id, component_id, adapter_identity, kind, component_key, layer_id, mount_path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    &generation_id,
                    &component.component_id,
                    &component.adapter_identity,
                    &component.kind,
                    &component.component_key,
                    &component.layer_id,
                    &component.mount_path
                ],
            )?;
            for output in &component.outputs {
                self.conn.execute(
                    "INSERT INTO environment_generation_outputs
                     (generation_id, component_id, output_name, policy, storage_identity, layer_id, mount_path, layer_subpath)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        &generation_id,
                        &component.component_id,
                        &output.name,
                        &output.policy,
                        &output.storage_identity,
                        &output.layer_id,
                        &output.mount_path,
                        &output.layer_subpath
                    ],
                )?;
            }
            for dependency in &component.dependencies {
                self.conn.execute(
                    "INSERT INTO environment_generation_edges
                     (generation_id, component_id, dependency_component_id, dependency_component_key, edge_type)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        &generation_id,
                        &component.component_id,
                        &dependency.component_id,
                        &dependency.component_key,
                        &dependency.edge_type
                    ],
                )?;
            }
            for cache in &component.caches {
                self.conn.execute(
                    "INSERT INTO environment_generation_caches
                     (generation_id, component_id, cache_name, namespace_id, protocol, access, compatibility_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        &generation_id,
                        &component.component_id,
                        &cache.name,
                        &cache.namespace_id,
                        &cache.protocol,
                        &cache.access,
                        serde_json::to_vec(&cache.compatibility)?
                    ],
                )?;
            }
            for artifact in &component.external_artifacts {
                self.conn.execute(
                    "INSERT INTO environment_generation_external_artifacts
                     (generation_id, component_id, artifact_name, artifact_type, provider, reference, digest, platform, cleanup_owner)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        &generation_id,
                        &component.component_id,
                        &artifact.name,
                        &artifact.artifact_type,
                        &artifact.provider,
                        &artifact.reference,
                        &artifact.digest,
                        &artifact.platform,
                        &artifact.cleanup_owner
                    ],
                )?;
            }
            let mut runtime_stmt = self.conn.prepare(
                "SELECT r.resource_name, r.runtime_type, r.provider, r.artifact_name,
                        a.reference, a.digest, a.platform, r.container_port, r.protocol,
                        r.health_type, r.health_timeout_ms, r.restart_policy,
                        r.cleanup_owner, r.volume_target
                 FROM environment_component_runtime_resources r
                 JOIN environment_component_external_artifacts a
                   ON a.view_id = r.view_id AND a.component_id = r.component_id
                  AND a.artifact_name = r.artifact_name
                 WHERE r.view_id = ?1 AND r.component_id = ?2
                 ORDER BY r.resource_name",
            )?;
            let runtime_resources = runtime_stmt
                .query_map(params![view_id, &component.component_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, u16>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, u64>(10)?,
                        row.get::<_, String>(11)?,
                        row.get::<_, String>(12)?,
                        row.get::<_, Option<String>>(13)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for (
                resource_name,
                runtime_type,
                provider,
                artifact_name,
                image_reference,
                image_digest,
                image_platform,
                container_port,
                protocol,
                health_type,
                health_timeout_ms,
                restart_policy,
                cleanup_owner,
                volume_target,
            ) in runtime_resources
            {
                let names = environment_runtime_allocation_names(
                    &self.config.workspace.id.0,
                    view_id,
                    &generation_id,
                    &component.component_id,
                    &resource_name,
                    volume_target.is_some(),
                );
                self.conn.execute(
                    "INSERT INTO environment_generation_runtime_resources
                     (generation_id, component_id, resource_name, runtime_type, provider,
                      artifact_name, image_reference, image_digest, image_platform,
                      container_port, protocol, health_type, health_timeout_ms, restart_policy,
                      cleanup_owner, volume_target, allocation_id, provider_resource_id,
                      container_name, network_name, volume_name, host_port, status,
                      health_status, reason, cleanup_token, owner_pid, owner_start_token,
                      created_at, updated_at, started_at, stopped_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                             ?14, ?15, ?16, ?17, NULL, ?18, ?19, ?20, NULL, 'pending',
                             'pending', NULL, ?21, NULL, NULL, ?22, ?22, NULL, NULL)",
                    params![
                        &generation_id,
                        &component.component_id,
                        &resource_name,
                        &runtime_type,
                        &provider,
                        &artifact_name,
                        &image_reference,
                        &image_digest,
                        &image_platform,
                        container_port,
                        &protocol,
                        &health_type,
                        health_timeout_ms,
                        &restart_policy,
                        &cleanup_owner,
                        &volume_target,
                        &names.allocation_id,
                        &names.container_name,
                        &names.network_name,
                        &names.volume_name,
                        &names.cleanup_token,
                        now
                    ],
                )?;
                let mut secret_stmt = self.conn.prepare(
                    "SELECT secret_name, provider, reference, version, purpose, injection,
                            target, environment, required
                     FROM environment_component_runtime_secrets
                     WHERE view_id = ?1 AND component_id = ?2 AND resource_name = ?3
                     ORDER BY secret_name",
                )?;
                let secrets = secret_stmt
                    .query_map(
                        params![view_id, &component.component_id, &resource_name],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, Option<String>>(3)?,
                                row.get::<_, String>(4)?,
                                row.get::<_, String>(5)?,
                                row.get::<_, String>(6)?,
                                row.get::<_, Option<String>>(7)?,
                                row.get::<_, bool>(8)?,
                            ))
                        },
                    )?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                for (
                    secret_name,
                    secret_provider,
                    reference,
                    version,
                    purpose,
                    injection,
                    target,
                    environment,
                    required,
                ) in secrets
                {
                    self.conn.execute(
                        "INSERT INTO environment_generation_runtime_secrets
                         (generation_id, component_id, resource_name, secret_name, provider,
                          reference, version, purpose, injection, target, environment, required,
                          status, reason, resolved_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                                 'pending', NULL, NULL, ?13)",
                        params![
                            &generation_id,
                            &component.component_id,
                            &resource_name,
                            &secret_name,
                            &secret_provider,
                            &reference,
                            &version,
                            &purpose,
                            &injection,
                            &target,
                            &environment,
                            required,
                            now
                        ],
                    )?;
                }
            }
        }
        if let Some(predecessor) = predecessor {
            self.conn.execute(
                "UPDATE environment_generations SET state = 'retired', retired_at = ?1 WHERE generation_id = ?2 AND state = 'active'",
                params![now, predecessor],
            )?;
        }
        self.conn.execute(
            "INSERT OR REPLACE INTO environment_view_generations (view_id, generation_id, updated_at) VALUES (?1, ?2, ?3)",
            params![view_id, &generation_id, now],
        )?;
        Ok(generation_id)
    }

    pub(crate) fn workspace_layer_bindings_for_source_upper(
        &self,
        source_upper: &Path,
    ) -> Result<Vec<WorkspaceLayerBinding>> {
        let mut stmt = self.conn.prepare(
            "SELECT l.layer_id, b.mount_path, l.storage_path, b.source_path, l.kind, b.priority \
             FROM workspace_views v JOIN workspace_view_layers b ON b.view_id = v.view_id \
             JOIN workspace_layers l ON l.layer_id = b.layer_id \
             WHERE v.source_upper = ?1 AND b.read_only = 1 AND l.state = 'ready' \
             ORDER BY length(b.mount_path) DESC, b.priority DESC",
        )?;
        let rows = stmt
            .query_map(params![source_upper.to_string_lossy()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(Error::from)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut bindings = rows
            .into_iter()
            .map(
                |(layer_id, mount_path, storage_path, source_path, kind, priority)| {
                    let storage_path = if source_path.is_empty() {
                        PathBuf::from(storage_path)
                    } else {
                        safe_join(Path::new(&storage_path), &source_path)?
                    };
                    Ok(WorkspaceLayerBinding {
                        binding_identity: layer_id.clone(),
                        layer_id: Some(layer_id),
                        mount_path,
                        storage_path: Some(storage_path),
                        kind,
                        priority,
                    })
                },
            )
            .collect::<Result<Vec<_>>>()?;
        let mut stmt = self.conn.prepare(
            "SELECT o.binding_identity, o.mount_path, o.kind
             FROM workspace_views v
             JOIN environment_component_output_bindings o ON o.view_id = v.view_id
             WHERE v.source_upper = ?1 AND o.policy = 'writable_private'
             ORDER BY length(o.mount_path) DESC, o.mount_path",
        )?;
        let private = stmt
            .query_map(params![source_upper.to_string_lossy()], |row| {
                Ok(WorkspaceLayerBinding {
                    binding_identity: row.get(0)?,
                    layer_id: None,
                    mount_path: row.get(1)?,
                    storage_path: None,
                    kind: row.get(2)?,
                    priority: 100,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        bindings.extend(private);
        bindings.sort_by(|left, right| {
            right
                .mount_path
                .len()
                .cmp(&left.mount_path.len())
                .then_with(|| right.priority.cmp(&left.priority))
                .then_with(|| left.mount_path.cmp(&right.mount_path))
        });
        Ok(bindings)
    }
}

struct EnvironmentRuntimeAllocationNames {
    allocation_id: String,
    container_name: String,
    network_name: String,
    volume_name: Option<String>,
    cleanup_token: String,
}

fn environment_runtime_allocation_names(
    workspace_id: &str,
    view_id: &str,
    generation_id: &str,
    component_id: &str,
    resource_name: &str,
    has_volume: bool,
) -> EnvironmentRuntimeAllocationNames {
    let identity = sha256_hex(
        format!("{workspace_id}\0{view_id}\0{generation_id}\0{component_id}\0{resource_name}")
            .as_bytes(),
    );
    let network_identity =
        sha256_hex(format!("{workspace_id}\0{view_id}\0{generation_id}").as_bytes());
    let volume_identity = sha256_hex(
        format!("{workspace_id}\0{view_id}\0{component_id}\0{resource_name}\0volume").as_bytes(),
    );
    let mut slug = resource_name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    slug.truncate(20);
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "service" } else { slug };
    EnvironmentRuntimeAllocationNames {
        allocation_id: format!("runtime_{}", &identity[..32]),
        container_name: format!("trail-{slug}-{}", &identity[..16]),
        network_name: format!("trail-net-{}", &network_identity[..20]),
        // A private data volume belongs to the logical lane service rather
        // than one immutable generation. New container/image generations can
        // therefore roll forward without silently discarding database state.
        volume_name: has_volume.then(|| format!("trail-vol-{}", &volume_identity[..20])),
        cleanup_token: format!("cleanup_{}", &identity[32..]),
    }
}

struct CacheGcCandidate {
    entry: WorkspaceCacheGcEntry,
    last_used_at: i64,
    retention_expired: bool,
}

fn mount_paths_overlap(left: &str, right: &str) -> bool {
    left == right
        || left
            .strip_prefix(right)
            .is_some_and(|rest| rest.starts_with('/'))
        || right
            .strip_prefix(left)
            .is_some_and(|rest| rest.starts_with('/'))
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
        // create_new makes ownership exclusive before the builder can write
        // and sync its token. A contender may observe that short empty-file
        // window; do not steal a freshly created lock as malformed.
        return malformed_build_lock_is_stale(path);
    };
    let Ok(pid) = pid.parse::<u32>() else {
        return malformed_build_lock_is_stale(path);
    };
    Ok(!process_matches_start_token(pid, token))
}

fn acquire_environment_cache_maintenance(
    db_dir: &Path,
    namespace_id: &str,
) -> Result<Option<CacheBuildKeyGuard>> {
    let lock_dir = db_dir.join("cache/namespace-maintenance");
    fs::create_dir_all(&lock_dir)?;
    let path = lock_dir.join(format!("{namespace_id}.lock"));
    let token = format!("{}:{}", std::process::id(), current_process_start_token());
    loop {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(token.as_bytes())?;
                file.sync_all()?;
                return Ok(Some(CacheBuildKeyGuard { path, token }));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if !build_lock_is_stale(&path)? {
                    return Ok(None);
                }
                let stale = path.with_extension(format!(
                    "stale.{}",
                    crate::ids::short_hash(
                        format!("{}:{}", namespace_id, now_nanos()).as_bytes(),
                        16
                    )
                ));
                match fs::rename(&path, stale) {
                    Ok(()) => continue,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(Error::Io(error)),
                }
            }
            Err(error) => return Err(Error::Io(error)),
        }
    }
}

fn malformed_build_lock_is_stale(path: &Path) -> Result<bool> {
    const MALFORMED_LOCK_GRACE: Duration = Duration::from_secs(5);
    let modified = fs::metadata(path)?.modified()?;
    Ok(SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default()
        >= MALFORMED_LOCK_GRACE)
}

pub(crate) fn make_tree_writable(path: &Path) {
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

fn workspace_layer_verification_stamp_path(layer_path: &Path) -> PathBuf {
    let name = layer_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("layer");
    layer_path.with_file_name(format!(".{name}.verified.json"))
}

fn read_workspace_layer_sidecar<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(Error::Corrupt(format!(
            "workspace layer sidecar `{}` is not a regular file",
            path.display()
        )));
    }
    if metadata.len() > WORKSPACE_LAYER_SIDECAR_MAX_BYTES {
        return Err(Error::Corrupt(format!(
            "workspace layer sidecar `{}` exceeds {WORKSPACE_LAYER_SIDECAR_MAX_BYTES} bytes",
            path.display()
        )));
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn write_workspace_layer_verification_stamp(
    report: &WorkspaceLayerReport,
    manifest_object_id: &str,
) -> Result<()> {
    let storage_path = Path::new(&report.storage_path);
    let stamp = WorkspaceLayerVerificationStamp {
        version: WORKSPACE_LAYER_VERIFICATION_STAMP_VERSION,
        layer_id: report.layer_id.clone(),
        manifest_object_id: manifest_object_id.to_string(),
        root_identity: workspace_layer_root_identity(storage_path)?,
        logical_bytes: report.logical_bytes,
        entry_count: report.entry_count,
        verified_at: now_ts(),
    };
    write_file_atomic(
        &workspace_layer_verification_stamp_path(storage_path),
        &serde_json::to_vec_pretty(&stamp)?,
        true,
    )
}

fn write_workspace_layer_publish_marker_from_report(
    report: &WorkspaceLayerReport,
    manifest_object_id: &str,
) -> Result<()> {
    let storage_path = Path::new(&report.storage_path);
    let marker = WorkspaceLayerPublishMarker {
        layer_id: report.layer_id.clone(),
        cache_key: report.cache_key.clone(),
        manifest_object_id: manifest_object_id.to_string(),
        logical_bytes: report.logical_bytes,
        physical_bytes: match report.physical_bytes {
            Some(bytes) => bytes,
            None => layer_physical_bytes(storage_path)?,
        },
        entry_count: report.entry_count,
    };
    write_file_atomic(
        &workspace_layer_marker_path(storage_path),
        &serde_json::to_vec_pretty(&marker)?,
        true,
    )
}

#[cfg(unix)]
fn workspace_layer_root_identity(path: &Path) -> Result<WorkspaceLayerRootIdentity> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(Error::Corrupt(format!(
            "workspace layer directory `{}` is missing or not a real directory",
            path.display()
        )));
    }
    Ok(WorkspaceLayerRootIdentity {
        platform: std::env::consts::OS.to_string(),
        values: vec![
            metadata.dev(),
            metadata.ino(),
            metadata.mode() as u64,
            metadata.nlink(),
            metadata.uid() as u64,
            metadata.gid() as u64,
            metadata.mtime() as u64,
            metadata.mtime_nsec() as u64,
            metadata.ctime() as u64,
            metadata.ctime_nsec() as u64,
        ],
    })
}

#[cfg(windows)]
fn workspace_layer_root_identity(path: &Path) -> Result<WorkspaceLayerRootIdentity> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(Error::Corrupt(format!(
            "workspace layer directory `{}` is missing or not a real directory",
            path.display()
        )));
    }
    let identity = windows_file_identity(path)?;
    Ok(WorkspaceLayerRootIdentity {
        platform: std::env::consts::OS.to_string(),
        values: vec![
            identity.attributes as u64,
            identity.volume_serial_number as u64,
            identity.file_index,
            identity.number_of_links as u64,
            identity.creation_time,
            identity.last_write_time,
        ],
    })
}

#[cfg(not(any(unix, windows)))]
fn workspace_layer_root_identity(path: &Path) -> Result<WorkspaceLayerRootIdentity> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(Error::Corrupt(format!(
            "workspace layer directory `{}` is missing or not a real directory",
            path.display()
        )));
    }
    let modified = metadata
        .modified()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    Ok(WorkspaceLayerRootIdentity {
        platform: std::env::consts::OS.to_string(),
        values: vec![
            metadata.len(),
            modified.as_secs(),
            modified.subsec_nanos() as u64,
            metadata.permissions().readonly() as u64,
        ],
    })
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
    #[cfg(test)]
    WORKSPACE_LAYER_FULL_SCAN_COUNT.with(|count| count.set(count.get().saturating_add(1)));
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
    fn legacy_layer_manifests_without_canonical_keys_remain_readable() {
        let manifest = WorkspaceLayerManifest {
            version: WORKSPACE_LAYER_MANIFEST_VERSION,
            layer_id: "layer_legacy".to_string(),
            kind: "dependency".to_string(),
            cache_key: "legacy-key".to_string(),
            layer_key: Some(key()),
            adapter: "node".to_string(),
            adapter_version: 1,
            logical_bytes: 0,
            entries: BTreeMap::new(),
            platform: "legacy".to_string(),
            architecture: "legacy".to_string(),
            portability_scope: "legacy".to_string(),
            producer_version: "legacy".to_string(),
            created_at: 1,
        };
        let mut value = serde_json::to_value(manifest).unwrap();
        value.as_object_mut().unwrap().remove("layer_key");
        let decoded: WorkspaceLayerManifest = serde_json::from_value(value).unwrap();
        assert!(decoded.layer_key.is_none());
    }

    #[test]
    fn dependency_activation_is_atomic_marks_descendants_stale_and_preserves_edge_history() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "dependencies",
            Some("main"),
            if cfg!(target_os = "macos") {
                LaneWorkdirMode::NfsCow
            } else if cfg!(target_os = "windows") {
                LaneWorkdirMode::DokanCow
            } else {
                LaneWorkdirMode::FuseCow
            },
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();

        let make_activation = |component: &str,
                               revision: &str,
                               dependencies: Vec<(String, String)>,
                               mount_path: &str,
                               seed: &Path|
         -> EnvironmentLayerActivation {
            let mut inputs = BTreeMap::from([("revision".to_string(), revision.to_string())]);
            for (dependency, component_key) in &dependencies {
                inputs.insert(format!("dependency:{dependency}"), component_key.clone());
            }
            let canonical_key = WorkspaceLayerKeyV1 {
                kind: "generated".to_string(),
                adapter: "test".to_string(),
                adapter_version: 1,
                inputs,
                tool_versions: BTreeMap::new(),
                platform: std::env::consts::OS.to_string(),
                architecture: std::env::consts::ARCH.to_string(),
                portability_scope: "host".to_string(),
                strategy: "private-dependency-test".to_string(),
            };
            let expected_key = db.workspace_layer_cache_key(&canonical_key).unwrap();
            EnvironmentLayerActivation {
                layer_id: None,
                outputs: vec![EnvironmentLayerOutputActivation {
                    name: "output".to_string(),
                    mount_path: mount_path.to_string(),
                    policy: "writable_private".to_string(),
                    binding_identity: format!("private-{component}-{revision}"),
                    private_seed: Some(seed.to_path_buf()),
                    layer_subpath: String::new(),
                }],
                component_id: component.to_string(),
                adapter_identity: "trail/test@1".to_string(),
                adapter_version: 1,
                implementation_version: "test".to_string(),
                distribution_digest: "builtin:test".to_string(),
                kind: "generated".to_string(),
                dependencies: dependencies
                    .into_iter()
                    .map(|(component_id, component_key)| {
                        (component_id, component_key, "build_requires".to_string())
                    })
                    .collect(),
                caches: Vec::new(),
                external_artifacts: Vec::new(),
                runtime_resources: Vec::new(),
                expected_key,
                canonical_key,
            }
        };

        let seed_a1 = tempfile::tempdir().unwrap();
        let seed_b1 = tempfile::tempdir().unwrap();
        fs::write(seed_a1.path().join("value"), "a1\n").unwrap();
        fs::write(seed_b1.path().join("value"), "b1\n").unwrap();
        let a1 = make_activation("a", "1", Vec::new(), ".generated/a", seed_a1.path());
        let b1 = make_activation(
            "b",
            "1",
            vec![("a".to_string(), a1.expected_key.clone())],
            ".generated/b",
            seed_b1.path(),
        );
        let wrong_source = ObjectId("object_not_the_lane_head".to_string());
        let error = db
            .replace_declared_workspace_layers_at_source(
                "dependencies",
                &[a1.clone(), b1.clone()],
                &wrong_source,
            )
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("advanced from pinned source root"));
        assert!(db
            .active_environment_generation("dependencies")
            .unwrap()
            .is_none());
        db.replace_declared_workspace_layers("dependencies", &[a1.clone(), b1.clone()])
            .unwrap();
        let predecessor = db
            .active_environment_generation("dependencies")
            .unwrap()
            .unwrap();
        assert_eq!(
            predecessor.components[1].dependencies[0].component_key,
            a1.expected_key
        );

        let mut invalid_b = b1.clone();
        invalid_b.dependencies[0].1 = "wrong-key".to_string();
        assert!(db
            .replace_declared_workspace_layers("dependencies", &[invalid_b])
            .is_err());
        assert_eq!(
            db.active_environment_generation("dependencies")
                .unwrap()
                .unwrap()
                .generation_id,
            predecessor.generation_id
        );

        let seed_a2 = tempfile::tempdir().unwrap();
        fs::write(seed_a2.path().join("value"), "a2\n").unwrap();
        let a2 = make_activation("a", "2", Vec::new(), ".generated/a", seed_a2.path());
        db.replace_declared_workspace_layers("dependencies", &[a2.clone()])
            .unwrap();
        let current = db
            .active_environment_generation("dependencies")
            .unwrap()
            .unwrap();
        let current_b = current
            .components
            .iter()
            .find(|component| component.component_id == "b")
            .unwrap();
        assert_eq!(current_b.dependencies[0].component_key, a1.expected_key);
        assert_ne!(current_b.dependencies[0].component_key, a2.expected_key);
        assert_eq!(
            db.environment_component_status("dependencies")
                .unwrap()
                .into_iter()
                .find(|state| state.component.component_id == "b")
                .unwrap()
                .status,
            "stale"
        );

        db.replace_declared_workspace_layers_with_removals("dependencies", &[], &["b".to_string()])
            .unwrap();
        let retired = db
            .active_environment_generation("dependencies")
            .unwrap()
            .unwrap();
        assert_eq!(
            retired
                .components
                .iter()
                .map(|component| component.component_id.as_str())
                .collect::<Vec<_>>(),
            ["a"]
        );
        assert!(db
            .environment_component_status("dependencies")
            .unwrap()
            .into_iter()
            .all(|state| state.component.component_id != "b"));
        db.replace_declared_workspace_layers_with_removals("dependencies", &[], &["a".to_string()])
            .unwrap();
        assert!(db
            .active_environment_generation("dependencies")
            .unwrap()
            .unwrap()
            .components
            .is_empty());
    }

    #[test]
    fn runtime_dependency_replacement_advances_generation_without_rebuilding_consumer() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "runtime-edges",
            Some("main"),
            if cfg!(target_os = "macos") {
                LaneWorkdirMode::NfsCow
            } else if cfg!(target_os = "windows") {
                LaneWorkdirMode::DokanCow
            } else {
                LaneWorkdirMode::FuseCow
            },
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let make_activation = |component: &str,
                               revision: &str,
                               dependencies: Vec<(String, String, String)>,
                               mount_path: &str,
                               seed: &Path| {
            let canonical_key = WorkspaceLayerKeyV1 {
                kind: "generated".to_string(),
                adapter: "test".to_string(),
                adapter_version: 1,
                inputs: BTreeMap::from([("revision".to_string(), revision.to_string())]),
                tool_versions: BTreeMap::new(),
                platform: std::env::consts::OS.to_string(),
                architecture: std::env::consts::ARCH.to_string(),
                portability_scope: "host".to_string(),
                strategy: "runtime-edge-test".to_string(),
            };
            let expected_key = db.workspace_layer_cache_key(&canonical_key).unwrap();
            EnvironmentLayerActivation {
                layer_id: None,
                outputs: vec![EnvironmentLayerOutputActivation {
                    name: "output".to_string(),
                    mount_path: mount_path.to_string(),
                    policy: "writable_private".to_string(),
                    binding_identity: format!("private-{component}-{revision}"),
                    private_seed: Some(seed.to_path_buf()),
                    layer_subpath: String::new(),
                }],
                component_id: component.to_string(),
                adapter_identity: "trail/test@1".to_string(),
                adapter_version: 1,
                implementation_version: "test".to_string(),
                distribution_digest: "builtin:test".to_string(),
                kind: "generated".to_string(),
                dependencies,
                caches: Vec::new(),
                external_artifacts: Vec::new(),
                runtime_resources: Vec::new(),
                expected_key,
                canonical_key,
            }
        };

        let provider_seed_1 = tempfile::tempdir().unwrap();
        let provider_seed_2 = tempfile::tempdir().unwrap();
        let consumer_seed = tempfile::tempdir().unwrap();
        fs::write(provider_seed_1.path().join("value"), "provider-1\n").unwrap();
        fs::write(provider_seed_2.path().join("value"), "provider-2\n").unwrap();
        fs::write(consumer_seed.path().join("value"), "consumer\n").unwrap();
        let provider_1 = make_activation(
            "provider",
            "1",
            Vec::new(),
            ".generated/provider",
            provider_seed_1.path(),
        );
        let consumer = make_activation(
            "consumer",
            "1",
            vec![(
                "provider".to_string(),
                provider_1.expected_key.clone(),
                "runtime_requires".to_string(),
            )],
            ".generated/consumer",
            consumer_seed.path(),
        );
        db.replace_declared_workspace_layers(
            "runtime-edges",
            &[provider_1.clone(), consumer.clone()],
        )
        .unwrap();

        let provider_2 = make_activation(
            "provider",
            "2",
            Vec::new(),
            ".generated/provider",
            provider_seed_2.path(),
        );
        db.replace_declared_workspace_layers("runtime-edges", &[provider_2.clone()])
            .unwrap();
        let generation = db
            .active_environment_generation("runtime-edges")
            .unwrap()
            .unwrap();
        let current_consumer = generation
            .components
            .iter()
            .find(|component| component.component_id == "consumer")
            .unwrap();
        assert_eq!(current_consumer.component_key, consumer.expected_key);
        assert_eq!(
            current_consumer.dependencies[0].edge_type,
            "runtime_requires"
        );
        assert_eq!(
            current_consumer.dependencies[0].component_key,
            provider_2.expected_key
        );
        assert_eq!(
            db.environment_component_status("runtime-edges")
                .unwrap()
                .into_iter()
                .find(|state| state.component.component_id == "consumer")
                .unwrap()
                .status,
            "ready"
        );
    }

    fn reset_full_scan_count() {
        WORKSPACE_LAYER_FULL_SCAN_COUNT.with(|count| count.set(0));
    }

    fn full_scan_count() -> u64 {
        WORKSPACE_LAYER_FULL_SCAN_COUNT.with(Cell::get)
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
            } else if cfg!(target_os = "windows") {
                LaneWorkdirMode::DokanCow
            } else {
                LaneWorkdirMode::FuseCow
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
    fn warm_attach_uses_durable_verification_stamp_and_legacy_fallback_scans_once() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let built = tempfile::tempdir().unwrap();
        fs::create_dir_all(built.path().join("pkg")).unwrap();
        for index in 0..128 {
            fs::write(
                built.path().join(format!("pkg/{index:03}.js")),
                format!("module.exports = {index};\n"),
            )
            .unwrap();
        }
        let layer = db
            .publish_workspace_layer_from_directory(&key(), built.path())
            .unwrap();
        let storage = Path::new(&layer.storage_path);
        let stamp = workspace_layer_verification_stamp_path(storage);
        assert!(stamp.is_file());

        reset_full_scan_count();
        let reused = db
            .publish_workspace_layer_from_directory(&key(), built.path())
            .unwrap();
        assert_eq!(reused.layer_id, layer.layer_id);
        db.verify_workspace_layer_for_attach(&layer.layer_id)
            .unwrap();
        db.verify_workspace_layer_for_attach(&layer.layer_id)
            .unwrap();
        assert_eq!(full_scan_count(), 0);

        fs::remove_file(&stamp).unwrap();
        reset_full_scan_count();
        db.verify_workspace_layer_for_attach(&layer.layer_id)
            .unwrap();
        assert_eq!(full_scan_count(), 1);
        assert!(stamp.is_file());

        reset_full_scan_count();
        db.verify_workspace_layer_for_attach(&layer.layer_id)
            .unwrap();
        assert_eq!(full_scan_count(), 0);

        fs::write(
            &stamp,
            vec![b'x'; WORKSPACE_LAYER_SIDECAR_MAX_BYTES as usize + 1],
        )
        .unwrap();
        reset_full_scan_count();
        db.verify_workspace_layer_for_attach(&layer.layer_id)
            .unwrap();
        assert_eq!(full_scan_count(), 1);
        assert!(fs::metadata(&stamp).unwrap().len() < WORKSPACE_LAYER_SIDECAR_MAX_BYTES);
    }

    #[test]
    fn attach_stamp_invalidates_when_layer_root_identity_changes() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let built = tempfile::tempdir().unwrap();
        fs::write(built.path().join("artifact"), "immutable\n").unwrap();
        let layer = db
            .publish_workspace_layer_from_directory(&key(), built.path())
            .unwrap();
        let storage = Path::new(&layer.storage_path);
        make_layer_root_writable(storage).unwrap();
        fs::write(storage.join("injected"), "corrupt\n").unwrap();
        set_layer_read_only(
            storage,
            true,
            layer_mode(&fs::symlink_metadata(storage).unwrap()),
        )
        .unwrap();

        reset_full_scan_count();
        let error = db
            .verify_workspace_layer_for_attach(&layer.layer_id)
            .unwrap_err();
        assert!(error.to_string().contains("immutable manifest"));
        assert_eq!(full_scan_count(), 1);
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
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
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

        let private = views[0]
            .1
            .create(package_a, "private.js", 0o644, true)
            .unwrap();
        views[0].1.write(private.ino, 0, b"private\n").unwrap();
        assert!(views[0]
            .0
            .generated_upper
            .join("node_modules/pkg/private.js")
            .is_file());
        drop(views);

        db.replace_workspace_layer(
            "node-a",
            &layer.layer_id,
            "node_modules",
            "node",
            &layer.cache_key,
        )
        .unwrap();
        let branch = db.lane_branch("node-a").unwrap();
        let head = db.get_ref(&branch.ref_name).unwrap();
        let paths = db.workspace_view_paths_for_lane("node-a").unwrap();
        let mut replaced = ViewCore::new_lazy(
            Trail::open(workspace.path()).unwrap(),
            paths.source_upper,
            head.root_id,
        )
        .unwrap();
        let modules = replaced.lookup(VIEW_ROOT_INO, "node_modules").unwrap();
        let package = replaced.lookup(modules, "pkg").unwrap();
        let restored = replaced.lookup(package, "index.js").unwrap();
        assert_eq!(replaced.read(restored, 0, 64).unwrap().0, b"original\n");
        assert!(replaced.lookup(package, "private.js").is_err());
        assert!(!paths.generated_upper.join("node_modules").exists());
        assert!(replaced.checkpoint_candidates().unwrap().paths.is_empty());
    }

    #[test]
    fn retained_path_index_mirror_intent_does_not_self_lock_layer_replacement() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "retained-repair",
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
        fs::create_dir_all(built.path().join("pkg")).unwrap();
        fs::write(built.path().join("pkg/index.js"), "immutable\n").unwrap();
        let layer = db
            .publish_workspace_layer_from_directory(&key(), built.path())
            .unwrap();
        db.attach_workspace_layer(
            "retained-repair",
            &layer.layer_id,
            "node_modules",
            "node",
            &layer.cache_key,
        )
        .unwrap();

        let branch = db.lane_branch("retained-repair").unwrap();
        let head = db.get_ref(&branch.ref_name).unwrap();
        let manifest = db
            .workspace_view_paths_for_lane("retained-repair")
            .unwrap()
            .meta_dir
            .join("workdir-manifest.json");
        if manifest.exists() {
            fs::remove_file(&manifest).unwrap();
        }
        fs::create_dir(&manifest).unwrap();
        db.conn
            .execute(
                "INSERT INTO pending_path_index_derived_repairs \
                 (ref_name, repair_kind, old_root, new_root, new_change, created_at) \
                 VALUES (?1, 'lane_manifest', ?2, ?2, ?3, ?4)",
                params![branch.ref_name, head.root_id.0, head.change_id.0, now_ts()],
            )
            .unwrap();

        db.replace_workspace_layer(
            "retained-repair",
            &layer.layer_id,
            "node_modules",
            "node",
            &layer.cache_key,
        )
        .unwrap();
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM pending_path_index_derived_repairs",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );

        fs::remove_dir(&manifest).unwrap();
        db.rebuild_indexes().unwrap();
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM pending_path_index_derived_repairs",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
    }

    #[test]
    fn declared_dependency_binding_classifies_nonstandard_mount_as_generated_upper() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "python-a",
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
        fs::create_dir_all(built.path().join("lib/python/site-packages/pkg")).unwrap();
        fs::write(
            built
                .path()
                .join("lib/python/site-packages/pkg/__init__.py"),
            "VALUE = 1\n",
        )
        .unwrap();
        let layer = db
            .publish_workspace_layer_from_directory(&key(), built.path())
            .unwrap();
        let paths = db.workspace_view_paths_for_lane("python-a").unwrap();
        fs::create_dir_all(paths.source_upper.join(".venv")).unwrap();
        fs::write(paths.source_upper.join(".venv/preexisting.txt"), "source\n").unwrap();
        let overlap = db
            .replace_declared_workspace_layer(
                "python-a",
                &layer.layer_id,
                ".venv",
                "python-env",
                "trail/python-test@1",
                1,
                "test",
                "builtin:python-test",
                "dependency",
                &layer.cache_key,
            )
            .unwrap_err();
        assert!(overlap.to_string().contains("pre-existing source-upper"));
        assert!(paths.source_upper.join(".venv/preexisting.txt").is_file());
        fs::remove_dir_all(paths.source_upper.join(".venv")).unwrap();
        db.attach_workspace_layer(
            "python-a",
            &layer.layer_id,
            ".venv",
            "python-env",
            &layer.cache_key,
        )
        .unwrap();

        let branch = db.lane_branch("python-a").unwrap();
        let head = db.get_ref(&branch.ref_name).unwrap();
        let mut view = ViewCore::new_lazy(
            Trail::open(workspace.path()).unwrap(),
            paths.source_upper.clone(),
            head.root_id,
        )
        .unwrap();
        let venv = view.lookup(VIEW_ROOT_INO, ".venv").unwrap();
        let marker = view.create(venv, "private.txt", 0o644, true).unwrap();
        view.write(marker.ino, 0, b"private\n").unwrap();
        assert!(paths.generated_upper.join(".venv/private.txt").is_file());
        assert!(!paths.source_upper.join(".venv/private.txt").exists());
        assert!(view.checkpoint_candidates().unwrap().paths.is_empty());
        drop(view);

        db.replace_declared_workspace_layer(
            "python-a",
            &layer.layer_id,
            ".venv",
            "python-env",
            "trail/python-test@1",
            1,
            "test",
            "builtin:python-test",
            "dependency",
            &layer.cache_key,
        )
        .unwrap();
        assert!(!paths.generated_upper.join(".venv").exists());
        let overlap = db
            .replace_declared_workspace_layer(
                "python-a",
                &layer.layer_id,
                ".venv/lib",
                "python-tools",
                "trail/python-test@1",
                1,
                "test",
                "builtin:python-test",
                "dependency",
                &layer.cache_key,
            )
            .unwrap_err();
        assert!(overlap.to_string().contains("overlaps"));
    }

    #[test]
    fn committed_binding_recovery_finishes_interrupted_private_upper_reset() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "reset-commit",
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
        fs::create_dir_all(built.path().join("pkg")).unwrap();
        fs::write(built.path().join("pkg/index.js"), "immutable\n").unwrap();
        let layer = db
            .publish_workspace_layer_from_directory(&key(), built.path())
            .unwrap();
        let view_report = db.lane_workspace_view("reset-commit").unwrap().unwrap();
        let paths = db.workspace_view_paths_for_lane("reset-commit").unwrap();
        fs::create_dir_all(paths.generated_upper.join("node_modules/pkg")).unwrap();
        fs::write(
            paths.generated_upper.join("node_modules/pkg/private.js"),
            "private\n",
        )
        .unwrap();
        let branch = db.lane_branch("reset-commit").unwrap();
        let head = db.get_ref(&branch.ref_name).unwrap();
        let mut core = ViewCore::new_lazy(
            Trail::open(workspace.path()).unwrap(),
            paths.source_upper.clone(),
            head.root_id.clone(),
        )
        .unwrap();
        let reset = core
            .prepare_declared_layer_mount_path("node_modules", "dependency", &layer.layer_id)
            .unwrap();
        drop(reset); // crash after the filesystem half, before local cleanup
        drop(core);
        db.conn
            .execute(
                "INSERT OR REPLACE INTO workspace_view_layers (view_id, layer_id, mount_path, priority, read_only) VALUES (?1, ?2, 'node_modules', 100, 1)",
                params![view_report.view_id, layer.layer_id],
            )
            .unwrap();

        let mut reopened = ViewCore::new_lazy(
            Trail::open(workspace.path()).unwrap(),
            paths.source_upper,
            head.root_id,
        )
        .unwrap();
        assert!(!paths.generated_upper.join("node_modules").exists());
        let modules = reopened.lookup(VIEW_ROOT_INO, "node_modules").unwrap();
        let pkg = reopened.lookup(modules, "pkg").unwrap();
        let index = reopened.lookup(pkg, "index.js").unwrap();
        assert_eq!(reopened.read(index, 0, 64).unwrap().0, b"immutable\n");
        assert!(fs::read_dir(paths.meta_dir.join("layer-reset-intents"))
            .unwrap()
            .next()
            .is_none());
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
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
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
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
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
        let verification_stamp =
            workspace_layer_verification_stamp_path(Path::new(&layer.storage_path));
        assert!(verification_stamp.is_file());
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
        assert!(!verification_stamp.exists());
        assert!(db.list_workspace_layers().unwrap().is_empty());
    }

    #[test]
    fn cache_gc_preserves_layers_referenced_only_by_retired_environment_generations() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let built = tempfile::tempdir().unwrap();
        fs::write(built.path().join("artifact"), "retained\n").unwrap();
        let layer = db
            .publish_workspace_layer_from_directory(&key(), built.path())
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO environment_generations
                 (generation_id, view_id, generation_sequence, source_root, specification_digest, predecessor_generation_id, state, created_at, activated_at, retired_at)
                 VALUES ('retired-generation', 'retired-view', 1, 'root', 'spec', NULL, 'retired', 1, 1, 2)",
                [],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO environment_generation_components
                 (generation_id, component_id, adapter_identity, kind, component_key, layer_id, mount_path)
                 VALUES ('retired-generation', 'component', 'trail/test@1', 'dependency', 'key', ?1, 'vendor')",
                params![layer.layer_id],
            )
            .unwrap();

        let report = db.workspace_cache_gc(false, Some(0)).unwrap();
        assert!(!report
            .deleted
            .iter()
            .any(|entry| entry.id == layer.layer_id));
        assert!(Path::new(&layer.storage_path).is_dir());
        db.verify_workspace_layer(&layer.layer_id).unwrap();
    }

    #[test]
    fn corrupt_bound_layer_is_an_exact_readiness_blocker() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
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
