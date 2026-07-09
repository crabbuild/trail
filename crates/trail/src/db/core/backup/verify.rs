use super::*;

impl Trail {
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
                "trail-backup-verify-{}-{}",
                std::process::id(),
                now_nanos()
            ));
            let verify_open = (|| -> Result<Trail> {
                fs::create_dir_all(verify_dir.join("index"))?;
                fs::copy(path.join(CONFIG_FILE), verify_dir.join(CONFIG_FILE))?;
                fs::copy(path.join(HEAD_FILE), verify_dir.join(HEAD_FILE))?;
                fs::copy(&sqlite_path, verify_dir.join(DB_RELATIVE_PATH))?;
                Trail::open_with_db_dir(&verify_dir, &verify_dir)
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
}
