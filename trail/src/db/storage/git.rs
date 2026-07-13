use super::*;

impl Trail {
    pub(crate) fn reset_git_handoff_metrics(&self) {
        self.git_handoff_metrics.set(GitHandoffMetrics::default());
    }

    pub(crate) fn git_handoff_metrics_report(&self) -> GitHandoffMetricsReport {
        self.git_handoff_metrics.get().into()
    }

    pub(crate) fn set_git_export_mode(&self, export_mode: GitExportMode) {
        let mut metrics = self.git_handoff_metrics.get();
        metrics.export_mode = export_mode;
        self.git_handoff_metrics.set(metrics);
    }

    pub(crate) fn set_git_changed_path_count(&self, changed_path_count: u64) {
        let mut metrics = self.git_handoff_metrics.get();
        metrics.changed_path_count = changed_path_count;
        self.git_handoff_metrics.set(metrics);
    }

    pub(crate) fn add_git_full_root_file_count(&self, full_root_file_count: u64) {
        let mut metrics = self.git_handoff_metrics.get();
        metrics.full_root_file_count = metrics
            .full_root_file_count
            .saturating_add(full_root_file_count);
        self.git_handoff_metrics.set(metrics);
    }

    fn record_git_blob_write(&self) {
        let mut metrics = self.git_handoff_metrics.get();
        metrics.blob_write_count = metrics.blob_write_count.saturating_add(1);
        self.git_handoff_metrics.set(metrics);
    }

    fn record_tracked_git_status(&self) {
        let mut metrics = self.git_handoff_metrics.get();
        metrics.tracked_status_count = metrics.tracked_status_count.saturating_add(1);
        self.git_handoff_metrics.set(metrics);
    }

    pub(crate) fn current_git_identity(&self) -> Result<Option<GitIdentity>> {
        let head_output = Command::new("git")
            .arg("-C")
            .arg(&self.workspace_root)
            .args(["rev-parse", "--verify", "HEAD"])
            .output()
            .map_err(|err| Error::Git(err.to_string()))?;
        if !head_output.status.success() {
            return Ok(None);
        }
        let head = String::from_utf8_lossy(&head_output.stdout)
            .trim()
            .to_string();
        if head.is_empty() {
            return Ok(None);
        }

        let branch_output = Command::new("git")
            .arg("-C")
            .arg(&self.workspace_root)
            .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
            .output()
            .map_err(|err| Error::Git(err.to_string()))?;
        let branch = branch_output
            .status
            .success()
            .then(|| {
                String::from_utf8_lossy(&branch_output.stdout)
                    .trim()
                    .to_string()
            })
            .filter(|branch| !branch.is_empty());
        Ok(Some(GitIdentity { head, branch }))
    }

    pub(crate) fn tracked_git_state(&self, identity: &GitIdentity) -> Result<GitState> {
        self.tracked_git_state_for_head(Some(identity.head.clone()))
    }

    fn tracked_git_state_for_head(&self, head: Option<String>) -> Result<GitState> {
        self.record_tracked_git_status();
        let status = Command::new("git")
            .arg("-C")
            .arg(&self.workspace_root)
            .args(["status", "--porcelain", "--untracked-files=no"])
            .output()
            .map_err(|err| Error::Git(err.to_string()))?;
        if !status.status.success() {
            let stderr = String::from_utf8_lossy(&status.stderr);
            return Err(Error::Git(format!(
                "git status failed in {}: {}",
                self.workspace_root.display(),
                stderr.trim()
            )));
        }
        Ok(GitState {
            head,
            dirty: !status.stdout.is_empty(),
        })
    }

    pub(crate) fn current_git_state(&self) -> Result<Option<GitState>> {
        if let Some(identity) = self.current_git_identity()? {
            return self.tracked_git_state(&identity).map(Some);
        }
        let inside = Command::new("git")
            .arg("-C")
            .arg(&self.workspace_root)
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .map_err(|err| Error::Git(err.to_string()))?;
        if !inside.status.success() {
            return Ok(None);
        }
        self.tracked_git_state_for_head(None).map(Some)
    }

    pub(crate) fn git_write_tree(&self, files: &BTreeMap<String, FileEntry>) -> Result<String> {
        let mut root = GitTreeNode::default();
        for (path, entry) in files {
            let bytes = self.materialize_entry_bytes(entry)?;
            let oid = self.git_output_with_input(&["hash-object", "-w", "--stdin"], &bytes)?;
            self.record_git_blob_write();
            let blob = GitBlobEntry {
                mode: if entry.executable { "100755" } else { "100644" },
                oid,
            };
            Self::git_insert_tree_path(&mut root, path, blob)?;
        }
        self.git_write_tree_node(&root)
    }

    pub(crate) fn git_clean_worktree_index_matches_root(&self, root_id: &ObjectId) -> Result<bool> {
        let Some(git_paths) = self.scan_git_tracked_paths_impl(true)? else {
            return Ok(false);
        };
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        if git_paths.len() as u64 != root.file_count {
            return Ok(false);
        }
        let root_paths = self.load_root_paths(root_id)?;
        if root_paths != git_paths {
            return Ok(false);
        }

        for paths in root_paths.chunks(512) {
            let root_files = self.load_root_files_for_paths(root_id, paths)?;
            if root_files.len() != paths.len() {
                return Ok(false);
            }
            let indexed = self.cached_worktree_index_entries_for_paths(paths)?;
            if indexed.len() != paths.len() {
                return Ok(false);
            }
            for path in paths {
                let Some(root_entry) = root_files.get(path) else {
                    return Ok(false);
                };
                let Some(indexed_entry) = indexed.get(path) else {
                    return Ok(false);
                };
                if indexed_entry.manifest.kind != root_entry.kind
                    || indexed_entry.manifest.executable != root_entry.executable
                    || indexed_entry.manifest.content_hash != root_entry.content_hash
                {
                    return Ok(false);
                }

                let abs = self.workspace_root.join(path_from_rel(path));
                let metadata = match fs::symlink_metadata(&abs) {
                    Ok(metadata) => metadata,
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                    Err(err) => return Err(Error::Io(err)),
                };
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Ok(false);
                }
                if WorktreeFileStamp::from_metadata(&metadata) != indexed_entry.stamp {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    pub(crate) fn git_write_tree_from_head_delta(
        &self,
        head: &str,
        patch_left: &BTreeMap<String, FileEntry>,
        patch_right: &BTreeMap<String, FileEntry>,
    ) -> Result<String> {
        let mut changed_paths = BTreeSet::new();
        changed_paths.extend(patch_left.keys().cloned());
        changed_paths.extend(patch_right.keys().cloned());
        if changed_paths.is_empty() {
            return self.git_output(&["rev-parse".to_string(), format!("{head}^{{tree}}")]);
        }

        let tmp_dir = self.db_dir.join("tmp");
        fs::create_dir_all(&tmp_dir)?;
        let index_path = tmp_dir.join(format!("git-index-{}-{}", std::process::id(), now_ts()));

        let result = (|| {
            self.git_output_with_index(&["read-tree".to_string(), head.to_string()], &index_path)?;
            for path in changed_paths {
                if let Some(entry) = patch_right.get(&path) {
                    let bytes = self.materialize_entry_bytes(entry)?;
                    let oid =
                        self.git_output_with_input(&["hash-object", "-w", "--stdin"], &bytes)?;
                    self.record_git_blob_write();
                    let mode = if entry.executable { "100755" } else { "100644" };
                    self.git_output_with_index(
                        &[
                            "update-index".to_string(),
                            "--add".to_string(),
                            "--cacheinfo".to_string(),
                            mode.to_string(),
                            oid,
                            path,
                        ],
                        &index_path,
                    )?;
                } else {
                    self.git_output_with_index(
                        &[
                            "update-index".to_string(),
                            "--force-remove".to_string(),
                            "--".to_string(),
                            path,
                        ],
                        &index_path,
                    )?;
                }
            }
            self.git_output_with_index(&["write-tree".to_string()], &index_path)
        })();

        let _ = fs::remove_file(&index_path);
        let _ = fs::remove_file(index_path.with_extension("lock"));
        result
    }

    pub(crate) fn git_insert_tree_path(
        root: &mut GitTreeNode,
        path: &str,
        blob: GitBlobEntry,
    ) -> Result<()> {
        let mut parts = path.split('/').collect::<Vec<_>>();
        if parts.is_empty()
            || parts
                .iter()
                .any(|part| part.is_empty() || *part == "." || *part == "..")
        {
            return Err(Error::InvalidPath {
                path: path.to_string(),
                reason: "path cannot be represented in a Git tree".to_string(),
            });
        }
        let name = parts.pop().unwrap();
        let mut node = root;
        for part in parts {
            if node.blobs.contains_key(part) {
                return Err(Error::InvalidPath {
                    path: path.to_string(),
                    reason: "path conflicts with a file in the Git tree".to_string(),
                });
            }
            node = node.dirs.entry(part.to_string()).or_default();
        }
        if node.dirs.contains_key(name) || node.blobs.insert(name.to_string(), blob).is_some() {
            return Err(Error::InvalidPath {
                path: path.to_string(),
                reason: "duplicate path in Git tree export".to_string(),
            });
        }
        Ok(())
    }

    pub(crate) fn git_write_tree_node(&self, node: &GitTreeNode) -> Result<String> {
        let mut entries = Vec::new();
        for (name, blob) in &node.blobs {
            entries.push((
                name.clone(),
                format!("{} blob {}\t{}\n", blob.mode, blob.oid, name),
            ));
        }
        for (name, child) in &node.dirs {
            let oid = self.git_write_tree_node(child)?;
            entries.push((name.clone(), format!("040000 tree {}\t{}\n", oid, name)));
        }
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        let input = entries
            .into_iter()
            .map(|(_, line)| line)
            .collect::<String>();
        self.git_output_with_input(&["mktree"], input.as_bytes())
    }

    pub(crate) fn git_commit_tree(
        &self,
        tree_oid: &str,
        parent: Option<&str>,
        message: &str,
    ) -> Result<String> {
        let mut args = vec!["commit-tree".to_string(), tree_oid.to_string()];
        if let Some(parent) = parent {
            args.push("-p".to_string());
            args.push(parent.to_string());
        }
        args.push("-m".to_string());
        args.push(message.to_string());
        self.git_output(&args)
    }

    pub(crate) fn git_output(&self, args: &[String]) -> Result<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.workspace_root)
            .args(args)
            .output()
            .map_err(|err| Error::Git(err.to_string()))?;
        self.git_checked_output(args, output)
    }

    pub(crate) fn git_output_with_index(
        &self,
        args: &[String],
        index_path: &Path,
    ) -> Result<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.workspace_root)
            .args(args)
            .env("GIT_INDEX_FILE", index_path)
            .output()
            .map_err(|err| Error::Git(err.to_string()))?;
        self.git_checked_output(args, output)
    }

    pub(crate) fn git_output_with_input(&self, args: &[&str], input: &[u8]) -> Result<String> {
        let mut child = Command::new("git")
            .arg("-C")
            .arg(&self.workspace_root)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|err| Error::Git(err.to_string()))?;
        {
            let stdin = child
                .stdin
                .as_mut()
                .ok_or_else(|| Error::Git("failed to open git stdin".to_string()))?;
            stdin.write_all(input)?;
        }
        let output = child
            .wait_with_output()
            .map_err(|err| Error::Git(err.to_string()))?;
        let args = args
            .iter()
            .map(|arg| (*arg).to_string())
            .collect::<Vec<_>>();
        self.git_checked_output(&args, output)
    }

    pub(crate) fn git_checked_output(
        &self,
        args: &[String],
        output: std::process::Output,
    ) -> Result<String> {
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Git(format!(
                "git {} failed in {}: {}",
                args.join(" "),
                self.workspace_root.display(),
                stderr.trim()
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_publication_state_rejects_changed_head() {
        assert!(matches!(
            validate_git_publication_state(
                "old",
                &GitState {
                    head: Some("new".into()),
                    dirty: false,
                }
            ),
            Err(Error::GitHeadChanged(_))
        ));
    }

    #[test]
    fn git_publication_state_rejects_dirty_worktree() {
        assert!(matches!(
            validate_git_publication_state(
                "head",
                &GitState {
                    head: Some("head".into()),
                    dirty: true,
                }
            ),
            Err(Error::GitWorktreeDirty(_))
        ));
    }
}
