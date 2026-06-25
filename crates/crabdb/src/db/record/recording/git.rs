use super::*;

impl CrabDb {
    pub fn git_import_update(
        &mut self,
        branch: Option<&str>,
        message: Option<String>,
    ) -> Result<GitImportReport> {
        let _lock = self.acquire_write_lock()?;
        let branch = branch.map(str::to_string).unwrap_or(self.current_branch()?);
        let ref_name = branch_ref(&branch);
        let head = self.get_ref(&ref_name)?;
        let previous_files = self.load_root_files(&head.root_id)?;
        let disk_files = self.scan_git_tracked_files_required()?;
        let actor = Actor::system();
        let change_id = self.allocate_change_id(&actor.id, "git-import-update")?;
        let built =
            self.build_root_from_disk_files(&disk_files, &change_id, Some(&previous_files))?;
        let diff = self.diff_file_maps(&previous_files, &built.files)?;

        if diff.changes.is_empty() {
            return Ok(GitImportReport {
                branch,
                operation: None,
                root_id: head.root_id,
                imported: built.stats,
                changed_paths: Vec::new(),
                mapping: None,
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
