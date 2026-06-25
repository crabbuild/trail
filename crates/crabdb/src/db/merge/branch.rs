use super::*;

impl CrabDb {
    pub fn merge_branches(&mut self, source: &str, target: &str) -> Result<MergeReport> {
        self.merge_branches_with_options(source, target, false)
    }

    pub fn merge_branches_with_options(
        &mut self,
        source: &str,
        target: &str,
        dry_run: bool,
    ) -> Result<MergeReport> {
        let _lock = self.acquire_write_lock()?;
        self.merge_branches_unlocked(source, target, dry_run)
    }

    pub(crate) fn merge_branches_unlocked(
        &mut self,
        source: &str,
        target: &str,
        dry_run: bool,
    ) -> Result<MergeReport> {
        let source_ref_name = branch_ref(source);
        let target_ref_name = branch_ref(target);
        let source_ref = self.get_ref(&source_ref_name)?;
        let target_ref = self.get_ref(&target_ref_name)?;
        let base_change = self.common_parent_hint(&source_ref.change_id, &target_ref.change_id)?;
        let base_ref = self.ref_from_change(&base_change)?;
        let base_files = self.load_root_files(&base_ref.root_id)?;
        let source_files = self.load_root_files(&source_ref.root_id)?;
        let target_files = self.load_root_files(&target_ref.root_id)?;
        let actor = Actor::human();
        let change_id = self.allocate_change_id(&actor.id, "merge")?;
        let (merged_files, conflicts) =
            self.merge_file_maps(&base_files, &target_files, &source_files, &change_id)?;
        if !conflicts.is_empty() {
            if dry_run {
                return Ok(MergeReport {
                    operation: change_id,
                    source_ref: source_ref_name,
                    target_ref: target_ref_name,
                    root_id: target_ref.root_id,
                    dry_run,
                    changed_paths: Vec::new(),
                    conflicts,
                });
            }
            return Err(Error::Conflict(conflicts.join("; ")));
        }
        let built = self.build_root_from_file_entries(merged_files, &change_id)?;
        let diff = self.diff_file_maps(&target_files, &built.files)?;
        if dry_run {
            return Ok(MergeReport {
                operation: change_id,
                source_ref: source_ref_name,
                target_ref: target_ref_name,
                root_id: built.root_id,
                dry_run,
                changed_paths: diff.summaries,
                conflicts: Vec::new(),
            });
        }
        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind: OperationKind::Merge,
            parents: vec![target_ref.change_id.clone(), source_ref.change_id.clone()],
            before_root: Some(target_ref.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: target_ref_name.clone(),
            actor,
            session_id: None,
            message: Some(format!("Merge `{source}` into `{target}`")),
            changes: diff.changes,
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&target_ref, &change_id, &built.root_id, &operation_id)?;
        Ok(MergeReport {
            operation: change_id,
            source_ref: source_ref_name,
            target_ref: target_ref_name,
            root_id: built.root_id,
            dry_run,
            changed_paths: diff.summaries,
            conflicts: Vec::new(),
        })
    }
}
