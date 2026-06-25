use super::*;

impl CrabDb {
    pub fn checkout(&mut self, change_or_ref: &str, force: bool) -> Result<CheckoutReport> {
        self.checkout_with_options(change_or_ref, force, false, None, false)
    }

    pub fn checkout_with_options(
        &mut self,
        change_or_ref: &str,
        force: bool,
        dry_run: bool,
        workdir: Option<&Path>,
        record_dirty: bool,
    ) -> Result<CheckoutReport> {
        let _lock = self.acquire_write_lock()?;
        if dry_run && record_dirty {
            return Err(Error::InvalidInput(
                "checkout --record-dirty cannot be combined with --dry-run".to_string(),
            ));
        }
        let mut recorded_dirty = None;
        if record_dirty {
            let current_branch = self.current_branch()?;
            let report = self.record_with_options_unlocked(
                Some(&current_branch),
                Some(format!(
                    "Record dirty worktree before checkout `{change_or_ref}`"
                )),
                Actor::human(),
                RecordOptions {
                    kind: Some(OperationKind::Checkout),
                    ..RecordOptions::default()
                },
            )?;
            recorded_dirty = report.operation;
        }
        let current = self.resolve_branch_ref(&self.current_branch()?)?;
        if !dry_run && workdir.is_none() && !force && !record_dirty {
            let status = self.status(None)?;
            if status.worktree_state != WorktreeState::Clean {
                return Err(Error::DirtyWorktree);
            }
        }
        let target = self.resolve_refish(change_or_ref)?;
        let current_files = self.load_root_files(&current.root_id)?;
        let target_files = self.load_root_files(&target.root_id)?;
        let diff = self.diff_file_maps(&current_files, &target_files)?;
        let output_root = workdir
            .map(|path| self.resolve_checkout_workdir_path(path))
            .transpose()?;
        if !dry_run {
            if let Some(output_root) = &output_root {
                prepare_checkout_workdir(output_root)?;
                materialize_into(
                    &self.workspace_root,
                    output_root,
                    &BTreeMap::new(),
                    &target_files,
                    |entry| self.materialize_entry_bytes(entry),
                )?;
            } else {
                self.materialize_files(&current_files, &target_files)?;
            }
        }
        Ok(CheckoutReport {
            change_id: target.change_id,
            root_id: target.root_id,
            written_files: if dry_run {
                0
            } else {
                target_files.len() as u64
            },
            dry_run,
            recorded_dirty,
            output_root: output_root.map(|path| path.to_string_lossy().to_string()),
            changed_paths: diff.summaries,
        })
    }
}
