use super::*;

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
        fs::create_dir_all(output.join("refs/lanes"))?;

        fs::copy(self.db_dir.join(CONFIG_FILE), output.join(CONFIG_FILE))?;
        fs::copy(self.db_dir.join(HEAD_FILE), output.join(HEAD_FILE))?;
        let trailignore = self.workspace_root.join(".trailignore");
        if trailignore.exists() {
            fs::copy(trailignore, output.join(".trailignore"))?;
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
}
