use super::*;
use crate::db::change_ledger::mark_backup_scopes_untrusted;
use crate::db::core::backup::publication::{
    publish_staged_tree, remove_any, remove_retained_tree, sibling_stage, sync_file_for_publication,
};
use crate::db::lane::ViewMutationBarrier;

impl Trail {
    pub fn create_backup(
        &self,
        output: impl AsRef<Path>,
        overwrite: bool,
    ) -> Result<BackupCreateReport> {
        let _lock = self.acquire_write_lock()?;
        let output = absolute_path(output.as_ref())?;
        if output.starts_with(&self.db_dir) {
            return Err(Error::InvalidInput(
                "backup output cannot be inside .trail".to_string(),
            ));
        }
        self.changed_path_ledger().recover()?;
        if output.exists() && !overwrite {
            return Err(Error::WorkspaceExists(output));
        }
        let parent = output
            .parent()
            .ok_or_else(|| Error::InvalidInput("backup output has no parent".into()))?;
        fs::create_dir_all(parent)?;
        let stage = sibling_stage(&output, "backup-stage")?;
        let mut report = match self.create_backup_inner(&stage) {
            Ok(report) => report,
            Err(error) => {
                let _ = remove_any(&stage);
                return Err(error);
            }
        };
        let retained = match publish_staged_tree(&stage, &output) {
            Ok(retained) => retained,
            Err(error) => {
                let _ = remove_any(&stage);
                return Err(error);
            }
        };
        remove_retained_tree(retained, parent)?;
        report.path = output.to_string_lossy().to_string();
        report.manifest_path = backup_manifest_path(&output).to_string_lossy().to_string();
        report.sqlite_path = backup_sqlite_path(&output).to_string_lossy().to_string();
        Ok(report)
    }

    pub(crate) fn create_backup_inner(&self, output: &Path) -> Result<BackupCreateReport> {
        fs::create_dir_all(output.join("index"))?;
        fs::write(output.join("index").join(SCHEMA_EXCLUSION_FILE), [])?;
        fs::write(output.join("index").join(SCHEMA_VALIDATION_LEADER_FILE), [])?;
        fs::create_dir_all(output.join("refs/branches"))?;
        fs::create_dir_all(output.join("refs/lanes"))?;

        fs::copy(self.db_dir.join(CONFIG_FILE), output.join(CONFIG_FILE))?;
        fs::copy(self.db_dir.join(HEAD_FILE), output.join(HEAD_FILE))?;
        let trailignore = self.workspace_root.join(".trailignore");
        if trailignore.exists() {
            fs::copy(trailignore, output.join(".trailignore"))?;
        }

        let retained_views = retained_private_views(self)?;
        let mut view_barriers = Vec::with_capacity(retained_views.len());
        for view in &retained_views {
            view_barriers.push(ViewMutationBarrier::exclusive(&view.meta_dir)?);
        }
        let (retained_private_views, retained_private_bytes) =
            copy_retained_private_views(&retained_views, output)?;
        let (sealed_private_bytes, retained_private_sha256) =
            portable_tree_digest(&output.join("views"))?;
        if sealed_private_bytes != retained_private_bytes {
            return Err(Error::Conflict(
                "retained private view bytes changed while creating the backup".into(),
            ));
        }

        let sqlite_path = output.join(DB_RELATIVE_PATH);
        let sqlite_path_text = sqlite_path.to_string_lossy().to_string();
        self.conn
            .execute("VACUUM main INTO ?1", params![sqlite_path_text])?;
        let backup_conn = Connection::open(&sqlite_path)?;
        let rebuildable = sanitize_portable_backup_database(&backup_conn)?;
        mark_backup_scopes_untrusted(&backup_conn)?;
        let checkpoint_busy: i64 =
            backup_conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))?;
        if checkpoint_busy != 0 {
            return Err(Error::Conflict(
                "backup SQLite checkpoint remained busy".into(),
            ));
        }
        drop(backup_conn);
        drop(view_barriers);
        sync_file_for_publication(&sqlite_path)?;
        let (sqlite_bytes, sqlite_sha256) = file_digest(&sqlite_path)?;

        let worktree_bytes =
            copy_dir_recursive(&self.db_dir.join("worktrees"), &output.join("worktrees"))?;

        let fsck = self.fsck()?;
        let branch = self.current_branch()?;
        let ref_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM refs", [], |row| row.get(0))?;
        let operation_count: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM operations", [], |row| row.get(0))?;

        let manifest = BackupManifest {
            format_version: 1,
            trail_version: env!("CARGO_PKG_VERSION").to_string(),
            created_at: now_ts(),
            source_workspace: self.workspace_root.to_string_lossy().to_string(),
            source_db_dir: self.db_dir.to_string_lossy().to_string(),
            workspace_id: self.config.workspace.id.clone(),
            branch: branch.clone(),
            ref_count: ref_count as u64,
            operation_count: operation_count as u64,
            sqlite_bytes,
            sqlite_sha256: sqlite_sha256.clone(),
            worktree_bytes,
            retained_private_views,
            retained_private_bytes,
            retained_private_sha256,
            rebuildable_materializations: rebuildable.materializations,
            rebuildable_materialization_bytes: rebuildable.materialization_bytes,
            rebuildable_performance_caches: rebuildable.performance_caches,
        };
        let manifest_path = backup_manifest_path(output);
        fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
        sync_file_for_publication(&manifest_path)?;

        Ok(BackupCreateReport {
            path: output.to_string_lossy().to_string(),
            manifest_path: manifest_path.to_string_lossy().to_string(),
            sqlite_path: sqlite_path.to_string_lossy().to_string(),
            workspace_id: manifest.workspace_id,
            branch,
            ref_count: ref_count as u64,
            operation_count: operation_count as u64,
            sqlite_bytes,
            sqlite_sha256,
            worktree_bytes,
            retained_private_views,
            retained_private_bytes,
            rebuildable_materializations: rebuildable.materializations,
            rebuildable_materialization_bytes: rebuildable.materialization_bytes,
            rebuildable_performance_caches: rebuildable.performance_caches,
            fsck_errors: fsck.errors,
        })
    }
}

#[derive(Debug)]
struct RetainedPrivateView {
    view_id: String,
    source_upper: PathBuf,
    meta_dir: PathBuf,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct RebuildableBackupState {
    pub(super) materializations: u64,
    pub(super) materialization_bytes: u64,
    pub(super) performance_caches: u64,
}

fn retained_private_views(db: &Trail) -> Result<Vec<RetainedPrivateView>> {
    let mut statement = db
        .conn
        .prepare("SELECT view_id,source_upper,meta_dir FROM workspace_views ORDER BY view_id")?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                PathBuf::from(row.get::<_, String>(1)?),
                PathBuf::from(row.get::<_, String>(2)?),
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut retained = Vec::new();
    for (view_id, source_upper, meta_dir) in rows {
        let mut components = Path::new(&view_id).components();
        if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
            return Err(Error::Corrupt(format!(
                "workspace view ID `{view_id}` is not a confined backup path"
            )));
        }
        let view_dir = db.db_dir.join("views").join(&view_id);
        let expected_source = view_dir.join("source-upper");
        let expected_meta = view_dir.join("meta");
        if !source_upper.is_dir() {
            continue;
        }
        if !meta_dir.is_dir() {
            return Err(Error::Corrupt(format!(
                "workspace view `{view_id}` source upper has no recovery metadata"
            )));
        }
        if fs::canonicalize(&source_upper)? != fs::canonicalize(&expected_source)?
            || fs::canonicalize(&meta_dir)? != fs::canonicalize(&expected_meta)?
        {
            return Err(Error::Corrupt(format!(
                "workspace view `{view_id}` has noncanonical private-state paths"
            )));
        }
        retained.push(RetainedPrivateView {
            view_id,
            source_upper,
            meta_dir,
        });
    }
    Ok(retained)
}

fn copy_retained_private_views(views: &[RetainedPrivateView], output: &Path) -> Result<(u64, u64)> {
    let mut bytes = 0_u64;
    for view in views {
        let destination = output.join("views").join(&view.view_id);
        bytes = bytes.saturating_add(copy_dir_recursive(
            &view.source_upper,
            &destination.join("source-upper"),
        )?);
        bytes = bytes.saturating_add(copy_dir_recursive(
            &view.meta_dir,
            &destination.join("meta"),
        )?);
        bytes = bytes.saturating_sub(scrub_ephemeral_view_metadata(&destination.join("meta"))?);
    }
    Ok((views.len() as u64, bytes))
}

fn scrub_ephemeral_view_metadata(meta_dir: &Path) -> Result<u64> {
    if !meta_dir.is_dir() {
        return Ok(0);
    }
    let mut removed_bytes = 0_u64;
    for entry in fs::read_dir(meta_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("checkpoint-barrier.")
            || matches!(
                name.as_ref(),
                "mount.json" | "unmount-request.json" | "view.json"
            )
        {
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.is_file() {
                removed_bytes = removed_bytes.saturating_add(metadata.len());
            }
            remove_any(&entry.path())?;
        }
    }
    Ok(removed_bytes)
}

pub(super) fn sanitize_portable_backup_database(
    conn: &Connection,
) -> Result<RebuildableBackupState> {
    conn.execute_batch("BEGIN IMMEDIATE;")?;
    let result = (|| -> Result<RebuildableBackupState> {
        let (artifact_count, artifact_bytes): (i64, i64) = conn.query_row(
            "SELECT COUNT(*),COALESCE(SUM(COALESCE(physical_bytes,logical_bytes)),0)
             FROM artifact_materializations",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let (layer_count, layer_bytes): (i64, i64) = conn.query_row(
            "SELECT COUNT(*),COALESCE(SUM(COALESCE(physical_bytes,logical_bytes)),0)
             FROM workspace_layers",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let performance_caches: i64 = conn.query_row(
            "SELECT COUNT(*) FROM environment_cache_namespaces",
            [],
            |row| row.get(0),
        )?;
        conn.execute_batch(
            "DELETE FROM artifact_materializations;
             DELETE FROM environment_hot_access_sessions;
             DELETE FROM environment_hot_sets;
             DELETE FROM environment_generation_caches;
             DELETE FROM environment_component_caches;
             DELETE FROM environment_cache_namespaces;",
        )?;
        Ok(RebuildableBackupState {
            materializations: u64::try_from(artifact_count.saturating_add(layer_count))
                .map_err(|_| Error::Corrupt("negative rebuildable materialization count".into()))?,
            materialization_bytes: u64::try_from(artifact_bytes.saturating_add(layer_bytes))
                .map_err(|_| Error::Corrupt("negative rebuildable materialization bytes".into()))?,
            performance_caches: u64::try_from(performance_caches)
                .map_err(|_| Error::Corrupt("negative rebuildable cache count".into()))?,
        })
    })();
    match result {
        Ok(summary) => {
            conn.execute_batch("COMMIT;")?;
            Ok(summary)
        }
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(error)
        }
    }
}
