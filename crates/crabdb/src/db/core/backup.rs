use super::*;

impl CrabDb {
    pub fn create_backup(
        &self,
        output: impl AsRef<Path>,
        overwrite: bool,
    ) -> Result<BackupCreateReport> {
        let _lock = self.acquire_write_lock()?;
        let output = absolute_path(output.as_ref())?;
        if output.starts_with(&self.db_dir) {
            return Err(Error::InvalidInput(
                "backup output cannot be inside .crabdb".to_string(),
            ));
        }
        if output.exists() {
            if !overwrite {
                return Err(Error::WorkspaceExists(output));
            }
            if output.is_dir() {
                fs::remove_dir_all(&output)?;
            } else {
                fs::remove_file(&output)?;
            }
        }

        let result = self.create_backup_inner(&output);
        if result.is_err() {
            let _ = fs::remove_dir_all(&output);
        }
        result
    }

    pub(crate) fn create_backup_inner(&self, output: &Path) -> Result<BackupCreateReport> {
        fs::create_dir_all(output.join("index"))?;
        fs::create_dir_all(output.join("refs/branches"))?;
        fs::create_dir_all(output.join("refs/agents"))?;

        fs::copy(self.db_dir.join(CONFIG_FILE), output.join(CONFIG_FILE))?;
        fs::copy(self.db_dir.join(HEAD_FILE), output.join(HEAD_FILE))?;
        let crabignore = self.workspace_root.join(".crabignore");
        if crabignore.exists() {
            fs::copy(crabignore, output.join(".crabignore"))?;
        }

        let sqlite_path = output.join(DB_RELATIVE_PATH);
        let sqlite_path_text = sqlite_path.to_string_lossy().to_string();
        self.conn
            .execute("VACUUM main INTO ?1", params![sqlite_path_text])?;
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
            crabdb_version: env!("CARGO_PKG_VERSION").to_string(),
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
        };
        let manifest_path = backup_manifest_path(output);
        fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;

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
            fsck_errors: fsck.errors,
        })
    }

    pub fn verify_backup(path: impl AsRef<Path>) -> Result<BackupVerifyReport> {
        let path = absolute_path(path.as_ref())?;
        if !path.is_dir() {
            return Err(Error::WorkspaceNotFound(path));
        }
        let mut errors = Vec::new();
        let mut workspace_id = None;
        let mut branch = None;
        let mut checked_refs = 0;
        let mut checked_roots = 0;
        let mut checked_texts = 0;
        let mut sqlite_bytes = None;
        let mut sqlite_sha256 = None;

        let manifest = match read_backup_manifest(&path) {
            Ok(manifest) => {
                if manifest.format_version != 1 {
                    errors.push(format!(
                        "unsupported backup format version {}",
                        manifest.format_version
                    ));
                }
                workspace_id = Some(manifest.workspace_id.clone());
                branch = Some(manifest.branch.clone());
                Some(manifest)
            }
            Err(err) => {
                errors.push(format!("manifest invalid: {err}"));
                None
            }
        };

        for required in [CONFIG_FILE, HEAD_FILE] {
            if !path.join(required).is_file() {
                errors.push(format!("missing required file `{required}`"));
            }
        }

        let sqlite_path = backup_sqlite_path(&path);
        if sqlite_path.is_file() {
            match file_digest(&sqlite_path) {
                Ok((bytes, sha256)) => {
                    if let Some(manifest) = &manifest {
                        if manifest.sqlite_bytes != bytes {
                            errors.push(format!(
                                "SQLite byte size mismatch: manifest {}, actual {bytes}",
                                manifest.sqlite_bytes
                            ));
                        }
                        if manifest.sqlite_sha256 != sha256 {
                            errors.push("SQLite SHA-256 mismatch".to_string());
                        }
                    }
                    sqlite_bytes = Some(bytes);
                    sqlite_sha256 = Some(sha256);
                }
                Err(err) => errors.push(format!("could not hash SQLite store: {err}")),
            }
        } else {
            errors.push(format!("missing SQLite store `{}`", DB_RELATIVE_PATH));
        }

        if path.join(CONFIG_FILE).is_file()
            && path.join(HEAD_FILE).is_file()
            && sqlite_path.is_file()
        {
            let verify_dir = std::env::temp_dir().join(format!(
                "crabdb-backup-verify-{}-{}",
                std::process::id(),
                now_nanos()
            ));
            let verify_open = (|| -> Result<CrabDb> {
                fs::create_dir_all(verify_dir.join("index"))?;
                fs::copy(path.join(CONFIG_FILE), verify_dir.join(CONFIG_FILE))?;
                fs::copy(path.join(HEAD_FILE), verify_dir.join(HEAD_FILE))?;
                fs::copy(&sqlite_path, verify_dir.join(DB_RELATIVE_PATH))?;
                CrabDb::open_with_db_dir(&verify_dir, &verify_dir)
            })();
            match verify_open {
                Ok(db) => match db.fsck() {
                    Ok(fsck) => {
                        checked_refs = fsck.checked_refs;
                        checked_roots = fsck.checked_roots;
                        checked_texts = fsck.checked_texts;
                        errors.extend(fsck.errors);
                        workspace_id.get_or_insert_with(|| db.config.workspace.id.clone());
                        branch.get_or_insert(db.current_branch()?);
                    }
                    Err(err) => errors.push(format!("fsck failed: {err}")),
                },
                Err(err) => errors.push(format!("could not open backup store: {err}")),
            }
            let _ = fs::remove_dir_all(&verify_dir);
        }

        Ok(BackupVerifyReport {
            path: path.to_string_lossy().to_string(),
            valid: errors.is_empty(),
            workspace_id,
            branch,
            checked_refs,
            checked_roots,
            checked_texts,
            sqlite_bytes,
            sqlite_sha256,
            errors,
        })
    }

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
        let db_dir = workspace_root.join(".crabdb");
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

        let temp_dir = workspace_root.join(format!(".crabdb.restore-{}", now_nanos()));
        if temp_dir.exists() {
            fs::remove_dir_all(&temp_dir)?;
        }

        let restore_result = (|| -> Result<()> {
            fs::create_dir_all(temp_dir.join("index"))?;
            fs::create_dir_all(temp_dir.join("refs/branches"))?;
            fs::create_dir_all(temp_dir.join("refs/agents"))?;
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

        let backup_crabignore = backup_path.join(".crabignore");
        let workspace_crabignore = workspace_root.join(".crabignore");
        let restored_crabignore =
            if backup_crabignore.is_file() && (force || !workspace_crabignore.exists()) {
                fs::copy(&backup_crabignore, &workspace_crabignore)?;
                true
            } else {
                if !workspace_crabignore.exists() {
                    write_default_crabignore(&workspace_root)?;
                }
                false
            };

        let mut db = CrabDb::open(&workspace_root)?;
        let rewritten_workdirs = db.rewrite_restored_agent_workdir_paths()?;
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
            restored_crabignore,
            rewritten_workdirs,
            checked_refs: fsck.checked_refs,
            checked_roots: fsck.checked_roots,
            checked_texts: fsck.checked_texts,
        })
    }
}
