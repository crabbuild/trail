use super::*;

impl Trail {
    pub fn restore_backup(
        workspace_root: impl AsRef<Path>,
        backup_path: impl AsRef<Path>,
        force: bool,
    ) -> Result<BackupRestoreReport> {
        fs::create_dir_all(workspace_root.as_ref())?;
        let workspace_root = workspace_root.as_ref().canonicalize()?;
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

        let temp_dir = workspace_root.join(format!(".trail.restore-{}", now_nanos()));
        if temp_dir.exists() {
            fs::remove_dir_all(&temp_dir)?;
        }

        let restore_result = (|| -> Result<()> {
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
            Ok(())
        })();
        if let Err(err) = restore_result {
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(err);
        }

        if replaced_existing {
            fs::remove_dir_all(&db_dir)?;
        }
        if let Err(err) = fs::rename(&temp_dir, &db_dir) {
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(Error::Io(err));
        }

        let backup_trailignore = backup_path.join(".trailignore");
        let workspace_trailignore = workspace_root.join(".trailignore");
        let restored_trailignore =
            if backup_trailignore.is_file() && (force || !workspace_trailignore.exists()) {
                fs::copy(&backup_trailignore, &workspace_trailignore)?;
                true
            } else {
                if !workspace_trailignore.exists() {
                    write_default_trailignore(&workspace_root)?;
                }
                false
            };

        let mut db = Trail::open(&workspace_root)?;
        let rewritten_workdirs = db.rewrite_restored_lane_workdir_paths()?;
        let fsck = db.fsck()?;
        if !fsck.errors.is_empty() {
            return Err(Error::Corrupt(format!(
                "restored backup failed fsck: {}",
                fsck.errors.join("; ")
            )));
        }

        Ok(BackupRestoreReport {
            workspace: workspace_root.to_string_lossy().to_string(),
            db_dir: db_dir.to_string_lossy().to_string(),
            backup_path: backup_path.to_string_lossy().to_string(),
            workspace_id: manifest.workspace_id,
            branch: manifest.branch,
            replaced_existing,
            restored_trailignore,
            rewritten_workdirs,
            checked_refs: fsck.checked_refs,
            checked_roots: fsck.checked_roots,
            checked_texts: fsck.checked_texts,
        })
    }
}
