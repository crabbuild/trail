use super::*;

impl CrabDb {
    pub fn diff_range(&self, spec: &str, patches: bool) -> Result<DiffSummary> {
        self.diff_range_with_options(spec, patches, false)
    }

    pub fn diff_range_with_options(
        &self,
        spec: &str,
        patches: bool,
        line_changes: bool,
    ) -> Result<DiffSummary> {
        let (left, right) = parse_range(spec)?;
        self.diff_refs_with_options(left, right, patches, line_changes)
    }

    pub fn diff_refs(&self, left: &str, right: &str, patches: bool) -> Result<DiffSummary> {
        self.diff_refs_with_options(left, right, patches, false)
    }

    pub fn diff_refs_with_options(
        &self,
        left: &str,
        right: &str,
        patches: bool,
        line_changes: bool,
    ) -> Result<DiffSummary> {
        let left_ref = self.resolve_refish(left)?;
        let right_ref = self.resolve_refish(right)?;
        self.diff_root_files(
            left.to_string(),
            right.to_string(),
            &left_ref.root_id,
            &right_ref.root_id,
            patches,
            line_changes,
        )
    }

    pub fn diff_roots(&self, spec: &str, patches: bool, line_changes: bool) -> Result<DiffSummary> {
        let (left, right) = parse_range(spec)?;
        let left_id = ObjectId(left.to_string());
        let right_id = ObjectId(right.to_string());
        self.diff_root_files(
            left.to_string(),
            right.to_string(),
            &left_id,
            &right_id,
            patches,
            line_changes,
        )
    }

    pub fn diff_dirty(&mut self, patches: bool, line_changes: bool) -> Result<DiffSummary> {
        let _lock = self.acquire_write_lock()?;
        let branch = self.current_branch()?;
        let head = self.resolve_branch_ref(&branch)?;
        let previous_files = self.load_root_files(&head.root_id)?;
        let fast_dirty_paths = self.scan_git_dirty_tracked_paths()?;
        let disk_files;
        let build_selected_paths;
        if let Some(paths) = fast_dirty_paths {
            if paths.is_empty() {
                return Ok(DiffSummary {
                    from: branch,
                    to: "dirty".to_string(),
                    files: Vec::new(),
                });
            }
            let snapshot = self.selected_worktree_snapshot(&previous_files, &paths)?;
            if snapshot.paths.is_empty() {
                return Ok(DiffSummary {
                    from: branch,
                    to: "dirty".to_string(),
                    files: Vec::new(),
                });
            }
            disk_files = snapshot.files;
            build_selected_paths = Some(snapshot.paths);
        } else {
            disk_files = self.scan_worktree_files()?;
            build_selected_paths = None;
        }
        let change_id = self.allocate_change_id("crabdb", "dirty-diff")?;
        let built = if let Some(paths) = build_selected_paths.as_deref() {
            self.build_root_for_selected_record_incremental(
                &head.root_id,
                &previous_files,
                &disk_files,
                paths,
                false,
                &change_id,
            )?
        } else {
            self.build_root_from_disk_files(&disk_files, &change_id, Some(&previous_files))?
        };
        self.diff_files(
            branch,
            "dirty".to_string(),
            &previous_files,
            &built.files,
            patches,
            line_changes,
        )
    }
}
