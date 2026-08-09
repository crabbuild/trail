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
        let mut retained_private_views = 0;
        let mut retained_private_bytes = 0;
        let mut rebuildable_materializations = 0;
        let mut rebuildable_materialization_bytes = 0;
        let mut rebuildable_performance_caches = 0;

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
                retained_private_views = manifest.retained_private_views;
                retained_private_bytes = manifest.retained_private_bytes;
                rebuildable_materializations = manifest.rebuildable_materializations;
                rebuildable_materialization_bytes = manifest.rebuildable_materialization_bytes;
                rebuildable_performance_caches = manifest.rebuildable_performance_caches;
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

        if let Some(manifest) = &manifest
            && !manifest.retained_private_sha256.is_empty()
        {
            match portable_tree_digest(&path.join("views")) {
                Ok((bytes, digest)) => {
                    if bytes != manifest.retained_private_bytes {
                        errors.push(format!(
                            "retained private byte size mismatch: manifest {}, actual {bytes}",
                            manifest.retained_private_bytes
                        ));
                    }
                    if digest != manifest.retained_private_sha256 {
                        errors.push("retained private SHA-256 mismatch".to_string());
                    }
                    let view_count = fs::read_dir(path.join("views"))
                        .map(|entries| {
                            entries
                                .filter_map(std::result::Result::ok)
                                .filter(|entry| entry.path().is_dir())
                                .count() as u64
                        })
                        .unwrap_or(0);
                    if view_count != manifest.retained_private_views {
                        errors.push(format!(
                            "retained private view count mismatch: manifest {}, actual {view_count}",
                            manifest.retained_private_views
                        ));
                    }
                }
                Err(error) => {
                    errors.push(format!("could not verify retained private state: {error}"))
                }
            }
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
                fs::write(verify_dir.join("index").join(SCHEMA_EXCLUSION_FILE), [])?;
                fs::write(
                    verify_dir.join("index").join(SCHEMA_VALIDATION_LEADER_FILE),
                    [],
                )?;
                fs::copy(path.join(CONFIG_FILE), verify_dir.join(CONFIG_FILE))?;
                fs::copy(path.join(HEAD_FILE), verify_dir.join(HEAD_FILE))?;
                fs::copy(&sqlite_path, verify_dir.join(DB_RELATIVE_PATH))?;
                copy_dir_recursive(&path.join("views"), &verify_dir.join("views"))?;
                super::open_staged_copy(&verify_dir, &verify_dir)
            })();
            match verify_open {
                Ok(mut db) => {
                    let rewrite = (|| -> Result<()> {
                        let _lock = db.acquire_write_lock()?;
                        db.rewrite_restored_lane_workdir_paths()?;
                        Ok(())
                    })();
                    if let Err(err) = rewrite {
                        errors.push(format!("could not prepare portable backup view: {err}"));
                    }
                    match db.fsck() {
                        Ok(fsck) => {
                            checked_refs = fsck.checked_refs;
                            checked_roots = fsck.checked_roots;
                            checked_texts = fsck.checked_texts;
                            errors.extend(fsck.errors);
                            workspace_id.get_or_insert_with(|| db.config.workspace.id.clone());
                            branch.get_or_insert(db.current_branch()?);
                            let trusted_scopes: i64 = db.conn.query_row(
                                "SELECT COUNT(*) FROM changed_path_scopes
                             WHERE retired_at IS NULL AND trust_state='trusted'",
                                [],
                                |row| row.get(0),
                            )?;
                            let live_observers: i64 = db.conn.query_row(
                                "SELECT (SELECT COUNT(*) FROM changed_path_observer_owners)
                                  + (SELECT COUNT(*) FROM changed_path_observer_segments)",
                                [],
                                |row| row.get(0),
                            )?;
                            if trusted_scopes != 0 || live_observers != 0 {
                                errors.push(
                                    "changed-path backup is not fenced or marked untrusted"
                                        .to_string(),
                                );
                            }
                        }
                        Err(err) => errors.push(format!("fsck failed: {err}")),
                    }
                }
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
            retained_private_views,
            retained_private_bytes,
            rebuildable_materializations,
            rebuildable_materialization_bytes,
            rebuildable_performance_caches,
            errors,
        })
    }
}
