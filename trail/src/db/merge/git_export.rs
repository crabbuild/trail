use super::*;

impl Trail {
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
        let operation = self.operation(&right_ref.change_id)?;
        let branch = operation.branch.clone();
        let mut patch_left = BTreeMap::new();
        let mut patch_right = BTreeMap::new();
        let _diff = self.diff_root_file_maps(
            &left_ref.root_id,
            &right_ref.root_id,
            &mut patch_left,
            &mut patch_right,
        )?;
        let can_export_delta = git_state
            .head
            .as_deref()
            .filter(|_| !git_state.dirty)
            .map(|head| {
                self.ensure_git_clean_head_root_mapping(
                    &branch,
                    &left_ref.change_id,
                    &left_ref.root_id,
                    head,
                )
            })
            .transpose()?
            .unwrap_or(false);
        let tree_oid = if let (true, Some(head)) = (can_export_delta, git_state.head.as_deref()) {
            self.git_write_tree_from_head_delta(head, &patch_left, &patch_right)?
        } else {
            let files = self.load_root_files(&right_ref.root_id)?;
            self.git_write_tree(&files)?
        };
        let commit = self.git_commit_tree(&tree_oid, git_state.head.as_deref(), message)?;
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
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let seed = format!(
            "{direction}:{branch}:{:?}:{}:{}:{created_at}:{nonce}",
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

    pub(crate) fn git_clean_head_matches_root_mapping(
        &self,
        git_head: &str,
        root_id: &ObjectId,
    ) -> Result<bool> {
        let exists = self
            .conn
            .query_row(
                "SELECT 1 FROM git_mappings \
                 WHERE git_head = ?1 AND git_dirty = 0 AND crab_root = ?2 \
                 LIMIT 1",
                params![git_head, root_id.0],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        Ok(exists.is_some())
    }

    pub(crate) fn ensure_git_clean_head_root_mapping(
        &self,
        branch: &str,
        change_id: &ChangeId,
        root_id: &ObjectId,
        git_head: &str,
    ) -> Result<bool> {
        if self.git_clean_head_matches_root_mapping(git_head, root_id)? {
            return Ok(true);
        }
        if !self.git_clean_worktree_index_matches_root(root_id)? {
            return Ok(false);
        }
        self.insert_git_mapping_for_state(
            "verify-index",
            branch,
            change_id,
            root_id,
            Some(git_head.to_string()),
            false,
        )?;
        Ok(true)
    }
}
