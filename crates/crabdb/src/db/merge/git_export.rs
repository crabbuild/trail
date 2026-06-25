use super::*;

impl CrabDb {
    pub fn export_patch(&self, range: &str) -> Result<String> {
        let summary = self.diff_range(range, true)?;
        let mut out = String::new();
        for file in summary.files {
            if let Some(patch) = file.patch {
                out.push_str(&patch);
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
        }
        Ok(out)
    }

    pub fn git_export_commit(&mut self, range: &str, message: &str) -> Result<GitExportReport> {
        let _lock = self.acquire_write_lock()?;
        let message = message.trim();
        if message.is_empty() {
            return Err(Error::InvalidInput(
                "git export commit message cannot be empty".to_string(),
            ));
        }
        let Some(git_state) = self.current_git_state()? else {
            return Err(Error::Git(format!(
                "git export requires a Git working tree at {}",
                self.workspace_root.display()
            )));
        };
        let (left, right) = parse_range(range)?;
        let left_ref = self.resolve_refish(left)?;
        let right_ref = self.resolve_refish(right)?;
        if !self
            .ancestor_set(&right_ref.change_id)?
            .contains(&left_ref.change_id.0)
        {
            return Err(Error::InvalidInput(format!(
                "range `{range}` is not an ancestor range"
            )));
        }
        let files = self.load_root_files(&right_ref.root_id)?;
        let tree_oid = self.git_write_tree(&files)?;
        let commit = self.git_commit_tree(&tree_oid, git_state.head.as_deref(), message)?;
        let operation = self.operation(&right_ref.change_id)?;
        let branch = operation.branch.clone();
        let mapping = self.insert_git_mapping_for_state(
            "export",
            &branch,
            &right_ref.change_id,
            &right_ref.root_id,
            Some(commit.clone()),
            git_state.dirty,
        )?;
        Ok(GitExportReport {
            range: range.to_string(),
            branch,
            operation: right_ref.change_id,
            root_id: right_ref.root_id,
            commit,
            parent: git_state.head,
            mapping,
        })
    }

    pub fn write_patch_to(&self, range: &str, output: &Path) -> Result<()> {
        let patch = self.export_patch(range)?;
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(output, patch)?;
        Ok(())
    }

    pub(crate) fn insert_git_mapping(
        &self,
        direction: &str,
        branch: &str,
        change_id: &ChangeId,
        root_id: &ObjectId,
    ) -> Result<Option<GitMapping>> {
        let Some(state) = self.current_git_state()? else {
            return Ok(None);
        };
        self.insert_git_mapping_for_state(
            direction,
            branch,
            change_id,
            root_id,
            state.head,
            state.dirty,
        )
    }

    pub(crate) fn insert_git_mapping_for_state(
        &self,
        direction: &str,
        branch: &str,
        change_id: &ChangeId,
        root_id: &ObjectId,
        git_head: Option<String>,
        git_dirty: bool,
    ) -> Result<Option<GitMapping>> {
        let created_at = now_ts();
        let seed = format!(
            "{direction}:{branch}:{:?}:{}:{}:{created_at}",
            git_head, change_id.0, root_id.0
        );
        let hash = sha256_hex(seed.as_bytes());
        let mapping_id = format!("gitmap_{}", &hash[..16]);
        self.conn.execute(
            "INSERT INTO git_mappings \
             (mapping_id, direction, branch, git_head, git_dirty, crab_change, crab_root, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                mapping_id,
                direction,
                branch,
                git_head.as_deref(),
                if git_dirty { 1_i64 } else { 0_i64 },
                change_id.0,
                root_id.0,
                created_at
            ],
        )?;
        Ok(Some(GitMapping {
            mapping_id,
            direction: direction.to_string(),
            branch: branch.to_string(),
            git_head,
            git_dirty,
            crab_change: change_id.clone(),
            crab_root: root_id.clone(),
            created_at,
        }))
    }
}
