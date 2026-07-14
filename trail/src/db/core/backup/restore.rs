use super::*;
use crate::db::change_ledger::rotate_restored_scopes;
use crate::db::core::backup::publication::{
    publish_staged_tree, remove_any, remove_retained_tree, rollback_published_tree, sibling_stage,
    sync_directory_strict, sync_tree_bottom_up,
};

impl Trail {
    pub fn restore_backup(
        workspace_root: impl AsRef<Path>,
        backup_path: impl AsRef<Path>,
        force: bool,
    ) -> Result<BackupRestoreReport> {
        fs::create_dir_all(workspace_root.as_ref())?;
        let workspace_root = canonicalize_lossless(workspace_root.as_ref())?;
        let backup_path = absolute_path(backup_path.as_ref())?;
        let verification = Self::verify_backup(&backup_path)?;
        if !verification.valid {
            return Err(Error::Corrupt(format!(
                "backup verification failed: {}",
                verification.errors.join("; ")
            )));
        }
        let manifest = read_backup_manifest(&backup_path)?;
        let db_dir = workspace_root.join(".trail");
        let replaced_existing = db_dir.exists();
        if replaced_existing {
            if db_dir.join("lock").exists() {
                let holder = fs::read_to_string(db_dir.join("lock"))
                    .unwrap_or_else(|_| "unknown writer".to_string());
                return Err(Error::WorkspaceLocked(holder.trim().to_string()));
            }
            if !force {
                return Err(Error::WorkspaceExists(db_dir));
            }
        }
        let (previous_epoch, previous_continuity) = if replaced_existing {
            previous_scope_rotation(&db_dir)?
        } else {
            (0, 0)
        };
        let temp_dir = sibling_stage(&db_dir, "restore-stage")?;

        let restore_result = (|| -> Result<(u64, crate::model::FsckReport)> {
            fs::create_dir_all(temp_dir.join("index"))?;
            fs::create_dir_all(temp_dir.join("refs/branches"))?;
            fs::create_dir_all(temp_dir.join("refs/lanes"))?;
            fs::copy(backup_path.join(CONFIG_FILE), temp_dir.join(CONFIG_FILE))?;
            fs::copy(backup_path.join(HEAD_FILE), temp_dir.join(HEAD_FILE))?;
            fs::copy(
                backup_sqlite_path(&backup_path),
                temp_dir.join(DB_RELATIVE_PATH),
            )?;
            copy_dir_recursive(&backup_path.join("worktrees"), &temp_dir.join("worktrees"))?;
            let restored_conn = Connection::open(temp_dir.join(DB_RELATIVE_PATH))?;
            let filesystem_identity = fresh_restored_filesystem_identity(&workspace_root)?;
            let scope_root = restored_scope_root(&workspace_root);
            rotate_restored_scopes(
                &restored_conn,
                &filesystem_identity,
                &scope_root,
                previous_epoch,
                previous_continuity,
            )?;
            drop(restored_conn);
            test_crash_point("restore_after_ledger_rotation");

            let mut db = Trail::open_without_recovering_derived_paths(&workspace_root, &temp_dir)?;
            let rewritten_workdirs = {
                let _lock = db.acquire_write_lock()?;
                let rewritten = db.rewrite_restored_lane_workdir_paths()?;
                db.drain_pending_path_index_derived_repairs_from_restore_stage(&db_dir)?;
                rewritten
            };
            test_crash_point("restore_after_staged_workdir_rewrite");
            db.recover_after_open()?;
            test_crash_point("restore_after_staged_recovery");
            let fsck = db.fsck()?;
            if !fsck.errors.is_empty() {
                return Err(Error::Corrupt(format!(
                    "restored backup failed fsck: {}",
                    fsck.errors.join("; ")
                )));
            }
            let checkpoint_busy: i64 =
                db.conn
                    .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))?;
            if checkpoint_busy != 0 {
                return Err(Error::Conflict(
                    "restored SQLite checkpoint remained busy".into(),
                ));
            }
            drop(db);
            test_crash_point("restore_after_staged_checkpoint");
            sync_tree_bottom_up(&temp_dir)?;
            test_crash_point("restore_after_staged_sync");
            Ok((rewritten_workdirs, fsck))
        })();
        let (rewritten_workdirs, fsck) = match restore_result {
            Ok(prepared) => prepared,
            Err(err) => {
                let _ = remove_any(&temp_dir);
                return Err(err);
            }
        };
        let retained = match publish_staged_tree(&temp_dir, &db_dir) {
            Ok(retained) => retained,
            Err(error) => {
                let _ = remove_any(&temp_dir);
                return Err(error);
            }
        };
        let post_publish = (|| -> Result<BackupRestoreReport> {
            let backup_trailignore = backup_path.join(".trailignore");
            let workspace_trailignore = workspace_root.join(".trailignore");
            let restored_trailignore =
                if backup_trailignore.is_file() && (force || !workspace_trailignore.exists()) {
                    fs::copy(&backup_trailignore, &workspace_trailignore)?;
                    OpenOptions::new()
                        .read(true)
                        .open(&workspace_trailignore)?
                        .sync_all()?;
                    true
                } else {
                    if !workspace_trailignore.exists() {
                        write_default_trailignore(&workspace_root)?;
                        OpenOptions::new()
                            .read(true)
                            .open(&workspace_trailignore)?
                            .sync_all()?;
                    }
                    false
                };
            sync_directory_strict(&workspace_root)?;

            Ok(BackupRestoreReport {
                workspace: workspace_root.to_string_lossy().to_string(),
                db_dir: db_dir.to_string_lossy().to_string(),
                backup_path: backup_path.to_string_lossy().to_string(),
                workspace_id: manifest.workspace_id.clone(),
                branch: manifest.branch.clone(),
                replaced_existing,
                restored_trailignore,
                rewritten_workdirs,
                checked_refs: fsck.checked_refs,
                checked_roots: fsck.checked_roots,
                checked_texts: fsck.checked_texts,
            })
        })();
        match post_publish {
            Ok(report) => {
                let parent = db_dir
                    .parent()
                    .ok_or_else(|| Error::InvalidInput("restore target has no parent".into()))?;
                test_crash_point("restore_before_retained_cleanup");
                remove_retained_tree(retained, parent)?;
                test_crash_point("restore_after_retained_cleanup");
                Ok(report)
            }
            Err(error) => {
                rollback_published_tree(&db_dir, retained)?;
                Err(error)
            }
        }
    }
}

fn restored_scope_root(workspace_root: &Path) -> String {
    workspace_root
        .to_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            format!(
                "os-bytes:{}",
                hex::encode(workspace_root.as_os_str().as_encoded_bytes())
            )
        })
}

fn previous_scope_rotation(db_dir: &Path) -> Result<(u64, u64)> {
    let sqlite = db_dir.join(DB_RELATIVE_PATH);
    if !sqlite.is_file() {
        return Ok((0, 0));
    }
    let flags =
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(sqlite, flags)?;
    let (epoch, continuity): (i64, i64) = conn.query_row(
        "SELECT COALESCE(MAX(epoch),0),COALESCE(MAX(continuity_generation),0)
         FROM changed_path_scopes",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok((
        u64::try_from(epoch).map_err(|_| Error::Corrupt("negative scope epoch".into()))?,
        u64::try_from(continuity)
            .map_err(|_| Error::Corrupt("negative scope continuity".into()))?,
    ))
}

fn fresh_restored_filesystem_identity(workspace_root: &Path) -> Result<Vec<u8>> {
    let mut nonce = [0_u8; 32];
    getrandom::getrandom(&mut nonce)
        .map_err(|error| Error::Io(std::io::Error::other(error.to_string())))?;
    let mut identity = b"trail-restored-filesystem-v2\0".to_vec();
    identity.extend_from_slice(&nonce);
    identity.extend_from_slice(workspace_root.as_os_str().as_encoded_bytes());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::metadata(workspace_root)?;
        identity.extend_from_slice(&metadata.dev().to_be_bytes());
        identity.extend_from_slice(&metadata.ino().to_be_bytes());
    }
    #[cfg(windows)]
    {
        let platform = windows_file_identity(workspace_root)?;
        identity.extend_from_slice(&platform.volume_serial_number.to_be_bytes());
        identity.extend_from_slice(&platform.file_index.to_be_bytes());
    }
    Ok(identity)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    #[test]
    fn restored_scope_root_encodes_non_utf8_bytes_losslessly() {
        let path = PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/trail-\xff".to_vec()));
        let encoded = restored_scope_root(&path);

        assert_eq!(encoded, "os-bytes:2f746d702f747261696c2dff");
        assert_eq!(
            hex::decode(encoded.strip_prefix("os-bytes:").unwrap()).unwrap(),
            path.as_os_str().as_bytes()
        );
    }
}
