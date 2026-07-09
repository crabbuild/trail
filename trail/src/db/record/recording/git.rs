use super::*;

impl Trail {
    pub fn git_import_update(
        &mut self,
        branch: Option<&str>,
        message: Option<String>,
    ) -> Result<GitImportReport> {
        let _lock = self.acquire_write_lock()?;
        let branch = branch.map(str::to_string).unwrap_or(self.current_branch()?);
        let ref_name = branch_ref(&branch);
        let head = self.get_ref(&ref_name)?;
        let tracked_paths = self.scan_git_tracked_paths_impl(true)?.ok_or_else(|| {
            Error::Git(format!(
                "git ls-files did not produce tracked paths in {}",
                self.workspace_root.display()
            ))
        })?;
        let actor = Actor::system();
        let change_id = self.allocate_change_id(&actor.id, "git-import-update")?;
        let built = self.build_root_from_git_tracked_paths_incremental(
            &tracked_paths,
            &head.root_id,
            &change_id,
        )?;
        let mut patch_left = BTreeMap::new();
        let mut patch_right = BTreeMap::new();
        let diff = self.diff_root_file_maps(
            &head.root_id,
            &built.root_id,
            &mut patch_left,
            &mut patch_right,
        )?;

        if diff.changes.is_empty() {
            let mapping =
                self.insert_git_mapping("import", &branch, &head.change_id, &head.root_id)?;
            return Ok(GitImportReport {
                branch,
                operation: None,
                root_id: head.root_id,
                imported: built.stats,
                changed_paths: Vec::new(),
                mapping,
            });
        }

        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind: OperationKind::GitImport,
            parents: vec![head.change_id.clone()],
            before_root: Some(head.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: branch.clone(),
            actor,
            session_id: None,
            message: message
                .map(|message| redact_sensitive_text(&message))
                .or_else(|| Some("Import Git-tracked workspace update".to_string())),
            changes: diff.changes,
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&head, &change_id, &built.root_id, &operation_id)?;
        self.update_worktree_index_from_paths_and_manifest(&tracked_paths, &built.disk_manifest)?;
        let mapping = self.insert_git_mapping("import", &branch, &change_id, &built.root_id)?;

        Ok(GitImportReport {
            branch,
            operation: Some(change_id),
            root_id: built.root_id,
            imported: built.stats,
            changed_paths: diff.summaries,
            mapping,
        })
    }

    pub fn git_mappings(&self, limit: usize) -> Result<Vec<GitMapping>> {
        let mut stmt = self.conn.prepare(
            "SELECT mapping_id, direction, branch, git_head, git_dirty, crab_change, crab_root, created_at \
             FROM git_mappings ORDER BY created_at DESC, rowid DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], git_mapping_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }
}
