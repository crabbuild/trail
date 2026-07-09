use super::*;

impl Trail {
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
        let actor = Actor::human();
        let change_id = self.allocate_change_id(&actor.id, "merge")?;
        let merged = self.merge_root_maps_for_changed_paths(
            &base_ref.root_id,
            &target_ref.root_id,
            &source_ref.root_id,
            &change_id,
        )?;
        if !merged.conflicts.is_empty() {
            if dry_run {
                return Ok(MergeReport {
                    operation: change_id,
                    source_ref: source_ref_name,
                    target_ref: target_ref_name,
                    root_id: target_ref.root_id,
                    dry_run,
                    changed_paths: Vec::new(),
                    conflicts: merged.conflicts,
                });
            }
            return Err(Error::Conflict(merged.conflicts.join("; ")));
        }
        if merged.merged_files == merged.target_files {
            return Ok(MergeReport {
                operation: target_ref.change_id,
                source_ref: source_ref_name,
                target_ref: target_ref_name,
                root_id: target_ref.root_id,
                dry_run,
                changed_paths: Vec::new(),
                conflicts: Vec::new(),
            });
        }
        let built = self.build_root_from_touched_file_entries_incremental(
            &target_ref.root_id,
            &merged.target_files,
            &merged.merged_files,
            &change_id,
        )?;
        let diff = self.diff_file_maps(&merged.target_files, &merged.merged_files)?;
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
