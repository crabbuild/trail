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
        self.reset_git_handoff_metrics();
        let _lock = self.acquire_write_lock()?;
        let git_state = self.current_git_state()?.ok_or_else(|| {
            Error::Git(format!(
                "git export requires a Git working tree at {}",
                self.workspace_root.display()
            ))
        })?;
        self.git_export_commit_with_state(
            range,
            message,
            git_state,
            GitExportPolicy::AllowFullSnapshot,
        )
    }

    pub(crate) fn git_export_commit_mapped(
        &mut self,
        range: &str,
        message: &str,
        state: Option<GitState>,
    ) -> Result<GitExportReport> {
        let _lock = self.acquire_write_lock()?;
        let git_state = match state {
            Some(state) => state,
            None => self.current_git_state()?.ok_or_else(|| {
                Error::Git(format!(
                    "git export requires a Git working tree at {}",
                    self.workspace_root.display()
                ))
            })?,
        };
        self.git_export_commit_with_state(
            range,
            message,
            git_state,
            GitExportPolicy::RequireMappedDelta,
        )
    }

    fn git_export_commit_with_state(
        &mut self,
        range: &str,
        message: &str,
        git_state: GitState,
        policy: GitExportPolicy,
    ) -> Result<GitExportReport> {
        let message = message.trim();
        if message.is_empty() {
            return Err(Error::InvalidInput(
                "git export commit message cannot be empty".to_string(),
            ));
        }
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
        if let Some(lane) = branch.strip_prefix(LANE_REF_PREFIX) {
            let lane_head = self.get_ref(&branch)?;
            if lane_head.change_id != right_ref.change_id || lane_head.root_id != right_ref.root_id
            {
                return Err(Error::InvalidInput(format!(
                    "Git export from lane `{lane}` must use its current reviewed head, not an older or synthetic range endpoint"
                )));
            }
            self.ensure_lane_merge_readiness(lane)?;
        }
        let can_export_delta = match (git_state.head.as_deref(), git_state.dirty) {
            (Some(head), false) => {
                if self.git_clean_head_matches_root_mapping(head, &left_ref.root_id)? {
                    true
                } else {
                    match policy {
                        GitExportPolicy::RequireMappedDelta => {
                            return Err(Error::GitMappingRequired(format!(
                                "clean Git HEAD `{head}` has no mapping for Trail root `{}`",
                                left_ref.root_id.0
                            )));
                        }
                        GitExportPolicy::AllowFullSnapshot => self
                            .ensure_git_clean_head_root_mapping(
                                &branch,
                                &left_ref.change_id,
                                &left_ref.root_id,
                                head,
                            )?,
                    }
                }
            }
            _ => match policy {
                GitExportPolicy::RequireMappedDelta => {
                    return Err(Error::GitMappingRequired(format!(
                        "mapped delta export requires a clean Git HEAD for Trail root `{}`",
                        left_ref.root_id.0
                    )));
                }
                GitExportPolicy::AllowFullSnapshot => false,
            },
        };
        let mut patch_left = BTreeMap::new();
        let mut patch_right = BTreeMap::new();
        let diff = self.diff_root_file_maps(
            &left_ref.root_id,
            &right_ref.root_id,
            &mut patch_left,
            &mut patch_right,
        )?;
        self.set_git_changed_path_count(diff.summaries.len() as u64);
        let tree_oid = if let (true, Some(head)) = (can_export_delta, git_state.head.as_deref()) {
            self.set_git_export_mode(GitExportMode::MappedDelta);
            self.git_write_tree_from_head_delta(head, &patch_left, &patch_right)?
        } else {
            let files = self.load_root_files(&right_ref.root_id)?;
            self.set_git_export_mode(GitExportMode::FullSnapshot);
            self.add_git_full_root_file_count(files.len() as u64);
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
            performance: self.git_handoff_metrics_report(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn run_git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn mapped_git_export_requires_preexisting_clean_mapping() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        run_git(temp.path(), &["init"]);
        run_git(temp.path(), &["config", "user.email", "trail@example.test"]);
        run_git(temp.path(), &["config", "user.name", "Trail Test"]);
        fs::write(temp.path().join("README.md"), "one\n").unwrap();
        run_git(temp.path(), &["add", "README.md"]);
        run_git(temp.path(), &["commit", "-m", "initial"]);
        let init = Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        fs::write(temp.path().join("README.md"), "one\ntwo\n").unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let record = db
            .record(Some("main"), Some("change".into()), Actor::human(), false)
            .unwrap();
        run_git(temp.path(), &["checkout", "--", "README.md"]);
        let range = format!("{}..{}", init.operation.0, record.operation.unwrap().0);
        let err = db
            .git_export_commit_mapped(&range, "mapped", None)
            .unwrap_err();
        assert!(matches!(err, Error::GitMappingRequired(_)));
        assert!(db.git_mappings(10).unwrap().is_empty());
    }

    #[test]
    fn mapped_git_export_reports_mapped_delta_mode() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        run_git(temp.path(), &["init"]);
        run_git(temp.path(), &["config", "user.email", "trail@example.test"]);
        run_git(temp.path(), &["config", "user.name", "Trail Test"]);
        fs::write(temp.path().join("README.md"), "one\n").unwrap();
        run_git(temp.path(), &["add", "README.md"]);
        run_git(temp.path(), &["commit", "-m", "initial"]);
        let init = Trail::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();
        fs::write(temp.path().join("README.md"), "one\ntwo\n").unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let record = db
            .record(Some("main"), Some("change".into()), Actor::human(), false)
            .unwrap();
        run_git(temp.path(), &["checkout", "--", "README.md"]);
        let range = format!("{}..{}", init.operation.0, record.operation.unwrap().0);

        let report = db.git_export_commit_mapped(&range, "mapped", None).unwrap();

        assert_eq!(report.performance.export_mode, "mapped_delta");
    }

    #[test]
    fn general_git_export_reports_full_snapshot_mode_without_mapping() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        run_git(temp.path(), &["init"]);
        run_git(temp.path(), &["config", "user.email", "trail@example.test"]);
        run_git(temp.path(), &["config", "user.name", "Trail Test"]);
        fs::write(temp.path().join("README.md"), "one\n").unwrap();
        run_git(temp.path(), &["add", "README.md"]);
        run_git(temp.path(), &["commit", "-m", "initial"]);
        let init = Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        fs::write(temp.path().join("README.md"), "one\ntwo\n").unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        assert!(db.git_mappings(10).unwrap().is_empty());
        let record = db
            .record(Some("main"), Some("change".into()), Actor::human(), false)
            .unwrap();
        let range = format!("{}..{}", init.operation.0, record.operation.unwrap().0);

        let report = db.git_export_commit(&range, "full snapshot").unwrap();

        assert_eq!(report.performance.export_mode, "full_snapshot");
    }
}
