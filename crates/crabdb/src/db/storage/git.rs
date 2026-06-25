use super::*;

impl CrabDb {
    pub(crate) fn scan_git_tracked_files(&self) -> Result<Vec<DiskFile>> {
        self.scan_git_tracked_files_impl(false)
    }

    pub(crate) fn current_git_state(&self) -> Result<Option<GitState>> {
        let inside = Command::new("git")
            .arg("-C")
            .arg(&self.workspace_root)
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .map_err(|err| Error::Git(err.to_string()))?;
        if !inside.status.success() {
            return Ok(None);
        }

        let head_output = Command::new("git")
            .arg("-C")
            .arg(&self.workspace_root)
            .args(["rev-parse", "--verify", "HEAD"])
            .output()
            .map_err(|err| Error::Git(err.to_string()))?;
        let head = if head_output.status.success() {
            Some(
                String::from_utf8_lossy(&head_output.stdout)
                    .trim()
                    .to_string(),
            )
            .filter(|head| !head.is_empty())
        } else {
            None
        };

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

        Ok(Some(GitState {
            head,
            dirty: !status.stdout.is_empty(),
        }))
    }

    pub(crate) fn git_write_tree(&self, files: &BTreeMap<String, FileEntry>) -> Result<String> {
        let mut root = GitTreeNode::default();
        for (path, entry) in files {
            let bytes = self.materialize_entry_bytes(entry)?;
            let oid = self.git_output_with_input(&["hash-object", "-w", "--stdin"], &bytes)?;
            let blob = GitBlobEntry {
                mode: if entry.executable { "100755" } else { "100644" },
                oid,
            };
            Self::git_insert_tree_path(&mut root, path, blob)?;
        }
        self.git_write_tree_node(&root)
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

    pub(crate) fn scan_git_tracked_files_required(&self) -> Result<Vec<DiskFile>> {
        self.scan_git_tracked_files_impl(true)
    }

    pub(crate) fn scan_git_tracked_files_impl(&self, required: bool) -> Result<Vec<DiskFile>> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.workspace_root)
            .arg("ls-files")
            .arg("-z")
            .output()
            .map_err(|err| Error::Git(err.to_string()))?;
        if !output.status.success() {
            if required {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(Error::Git(format!(
                    "git ls-files failed in {}: {}",
                    self.workspace_root.display(),
                    stderr.trim()
                )));
            }
            return self.scan_worktree_files();
        }
        let mut files = Vec::new();
        for raw in output.stdout.split(|byte| *byte == 0) {
            if raw.is_empty() {
                continue;
            }
            let path = String::from_utf8_lossy(raw).to_string();
            let path = normalize_relative_path(&path)?;
            if is_default_ignored(&path) {
                continue;
            }
            let abs = self.workspace_root.join(path_from_rel(&path));
            let metadata = match fs::symlink_metadata(&abs) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => return Err(Error::Io(err)),
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_file() {
                files.push(DiskFile {
                    path,
                    bytes: fs::read(&abs)?,
                    executable: executable_from_metadata(&metadata),
                });
            }
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(files)
    }

    pub(crate) fn scan_worktree_files(&self) -> Result<Vec<DiskFile>> {
        self.scan_files_under(&self.workspace_root)
    }

    pub(crate) fn scan_files_under(&self, root: &Path) -> Result<Vec<DiskFile>> {
        let root = root.canonicalize()?;
        let mut builder = WalkBuilder::new(&root);
        builder
            .hidden(false)
            .git_ignore(self.config.recording.ignore_gitignored)
            .git_exclude(self.config.recording.ignore_gitignored)
            .git_global(self.config.recording.ignore_gitignored)
            .add_custom_ignore_filename(".crabignore");
        let walker = builder.build();
        let mut files = Vec::new();
        for item in walker {
            let entry = item.map_err(|err| Error::InvalidInput(err.to_string()))?;
            let path = entry.path();
            if path == root {
                continue;
            }
            let rel = path
                .strip_prefix(&root)
                .map_err(|err| Error::InvalidInput(err.to_string()))?;
            let rel = normalize_relative_path(&rel.to_string_lossy())?;
            if entry.file_type().is_some_and(|kind| kind.is_dir()) {
                if is_default_ignored(&rel) {
                    continue;
                }
            }
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            if is_default_ignored(&rel) {
                continue;
            }
            files.push(DiskFile {
                path: rel,
                bytes: fs::read(path)?,
                executable: executable(path)?,
            });
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(files)
    }

    pub(crate) fn disk_manifest(&self, disk_files: &[DiskFile]) -> BTreeMap<String, DiskManifest> {
        disk_files
            .iter()
            .map(|file| {
                (
                    file.path.clone(),
                    DiskManifest {
                        kind: classify_file_kind(&file.bytes, &self.config.text),
                        executable: file.executable,
                        content_hash: sha256_hex(&file.bytes),
                    },
                )
            })
            .collect()
    }

    pub(crate) fn diff_file_maps_to_manifest(
        &self,
        left: &BTreeMap<String, FileEntry>,
        right: &BTreeMap<String, DiskManifest>,
    ) -> Vec<FileDiffSummary> {
        let mut paths = BTreeSet::new();
        paths.extend(left.keys().cloned());
        paths.extend(right.keys().cloned());
        let mut summaries = Vec::new();
        for path in paths {
            match (left.get(&path), right.get(&path)) {
                (None, Some(new_entry)) => summaries.push(FileDiffSummary {
                    path,
                    old_path: None,
                    kind: FileChangeKind::Added,
                    before_hash: None,
                    after_hash: Some(new_entry.content_hash.clone()),
                    additions: 0,
                    deletions: 0,
                    line_changes: Vec::new(),
                    patch: None,
                }),
                (Some(old_entry), None) => summaries.push(FileDiffSummary {
                    path,
                    old_path: None,
                    kind: FileChangeKind::Deleted,
                    before_hash: Some(old_entry.content_hash.clone()),
                    after_hash: None,
                    additions: 0,
                    deletions: 0,
                    line_changes: Vec::new(),
                    patch: None,
                }),
                (Some(old_entry), Some(new_entry)) => {
                    if old_entry.content_hash == new_entry.content_hash
                        && old_entry.executable == new_entry.executable
                        && old_entry.kind == new_entry.kind
                    {
                        continue;
                    }
                    summaries.push(FileDiffSummary {
                        path,
                        old_path: None,
                        kind: if old_entry.kind == new_entry.kind {
                            FileChangeKind::Modified
                        } else {
                            FileChangeKind::TypeChanged
                        },
                        before_hash: Some(old_entry.content_hash.clone()),
                        after_hash: Some(new_entry.content_hash.clone()),
                        additions: 0,
                        deletions: 0,
                        line_changes: Vec::new(),
                        patch: None,
                    });
                }
                (None, None) => {}
            }
        }
        summaries
    }
}
