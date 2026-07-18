use super::*;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

impl Trail {
    pub(crate) fn with_workspace_authoritative_command_snapshot<T, F>(
        &self,
        mut consume: F,
    ) -> Result<(T, crate::db::change_ledger::FencedCandidateSnapshot)>
    where
        F: FnMut(
            &Trail,
            &crate::db::change_ledger::CompiledPolicy,
            &crate::db::change_ledger::CandidateSnapshot,
            Option<&crate::db::change_ledger::QualifiedGitCandidates>,
        ) -> Result<T>,
    {
        let git_bound: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM git_mappings LIMIT 1)",
            [],
            |row| row.get(0),
        )?;
        if git_bound {
            // The changed-path ledger is the command-path candidate authority.
            // Git contributes only a bounded structural fence here: running
            // `status` or `ls-files` would reintroduce O(repository files) work
            // on every warm status/diff/record operation.
            let captured = std::cell::RefCell::new(None);
            let held = std::cell::RefCell::new(None);
            let result = self.with_workspace_authoritative_snapshot(|db, policy, candidates| {
                let identity = db.capture_git_command_identity()?;
                *held.borrow_mut() = Some(GitCommandStructuralHold::open(db, policy, &identity)?);
                *captured.borrow_mut() = Some(identity);
                consume(db, policy, candidates, None)
            })?;
            #[cfg(debug_assertions)]
            run_git_qualification_after_c2_hook()?;
            let observed = self.capture_git_command_identity()?;
            let expected =
                captured
                    .into_inner()
                    .ok_or_else(|| Error::ChangeLedgerReconcileRequired {
                        scope: result.1.candidates.expected.scope_id.to_text(),
                        state: "untrusted_gap".into(),
                        reason: "Git command fence omitted its c1 structural identity".into(),
                        command: "trail index reconcile".into(),
                    })?;
            let path_matches = git_command_identity_matches(&observed, &expected);
            let held_matches = held
                .into_inner()
                .ok_or_else(|| Error::ChangeLedgerReconcileRequired {
                    scope: result.1.candidates.expected.scope_id.to_text(),
                    state: "untrusted_gap".into(),
                    reason: "Git command fence omitted its held descriptors".into(),
                    command: "trail index reconcile".into(),
                })?
                .verify(self)?;
            if !path_matches || !held_matches {
                return Err(Error::ChangeLedgerReconcileRequired {
                    scope: result.1.candidates.expected.scope_id.to_text(),
                    state: "untrusted_gap".into(),
                    reason: "Git structural identity changed across ledger c2".into(),
                    command: "trail index reconcile".into(),
                });
            }
            Ok(result)
        } else {
            self.with_workspace_authoritative_snapshot(|db, policy, candidates| {
                consume(db, policy, candidates, None)
            })
        }
    }

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

    fn add_git_blob_writes(&self, count: u64) {
        let mut metrics = self.git_handoff_metrics.get();
        metrics.blob_write_count = metrics.blob_write_count.saturating_add(count);
        self.git_handoff_metrics.set(metrics);
    }

    pub(crate) fn record_git_plumbing_command(&self) {
        let mut metrics = self.git_handoff_metrics.get();
        metrics.git_plumbing_command_count = metrics.git_plumbing_command_count.saturating_add(1);
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

    pub(crate) fn git_clean_worktree_index_matches_root_at_paths(
        &self,
        root_id: &ObjectId,
        paths: &[String],
    ) -> Result<bool> {
        let Some(git_paths) = self.scan_git_tracked_paths_impl(true)? else {
            return Ok(false);
        };
        let git_paths = git_paths.into_iter().collect::<BTreeSet<_>>();
        let root_files = self.load_root_files_for_paths(root_id, paths)?;
        let indexed = self.cached_worktree_index_entries_for_paths(paths)?;
        for path in paths {
            let Some(root_entry) = root_files.get(path) else {
                if git_paths.contains(path) {
                    return Ok(false);
                }
                continue;
            };
            if !git_paths.contains(path) {
                return Ok(false);
            }
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
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(Error::Io(error)),
            };
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Ok(false);
            }
            if WorktreeFileStamp::from_metadata(&metadata) != indexed_entry.stamp {
                return Ok(false);
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
        let batch_dir = tempfile::Builder::new()
            .prefix("git-delta-")
            .tempdir_in(&tmp_dir)?;
        let blob_dir = batch_dir.path().join("blobs");
        fs::create_dir(&blob_dir)?;
        let index_path = batch_dir.path().join("index");

        let result = (|| {
            self.record_git_plumbing_command();
            self.git_output_with_index(&["read-tree".to_string(), head.to_string()], &index_path)?;

            let mut additions = Vec::new();
            for (ordinal, path) in changed_paths.iter().enumerate() {
                if let Some(entry) = patch_right.get(path) {
                    let bytes = self.materialize_entry_bytes(entry)?;
                    let synthetic_name = format!("blob-{ordinal:020}");
                    let synthetic_path = blob_dir.join(&synthetic_name);
                    fs::write(&synthetic_path, bytes)?;
                    additions.push((
                        path.clone(),
                        if entry.executable { "100755" } else { "100644" },
                        synthetic_path,
                    ));
                }
            }

            let oids = if additions.is_empty() {
                Vec::new()
            } else {
                let mut hash_input = Vec::new();
                for (_, _, synthetic_path) in &additions {
                    append_git_stdin_path(&mut hash_input, synthetic_path)?;
                }
                self.record_git_plumbing_command();
                let output = self.git_output_bytes_with_input(
                    &["hash-object", "-w", "--no-filters", "--stdin-paths"],
                    &hash_input,
                    None,
                )?;
                let oids = parse_git_hash_object_oids(&output, additions.len())?;
                self.add_git_blob_writes(oids.len() as u64);
                oids
            };

            let oid_length = oids.first().map_or(head.len(), String::len);
            if !matches!(oid_length, 40 | 64) {
                return Err(Error::Git(format!(
                    "unsupported Git object ID length {oid_length}"
                )));
            }
            let zero_oid = "0".repeat(oid_length);
            let oid_by_path = additions
                .iter()
                .zip(oids)
                .map(|((path, mode, _), oid)| (path.as_str(), (*mode, oid)))
                .collect::<BTreeMap<_, _>>();
            let mut index_input = Vec::new();
            for path in &changed_paths {
                if path.as_bytes().contains(&0) {
                    return Err(Error::InvalidPath {
                        path: path.clone(),
                        reason: "Git index paths cannot contain NUL bytes".to_string(),
                    });
                }
                if let Some((mode, oid)) = oid_by_path.get(path.as_str()) {
                    index_input.extend_from_slice(mode.as_bytes());
                    index_input.push(b' ');
                    index_input.extend_from_slice(oid.as_bytes());
                } else {
                    index_input.extend_from_slice(b"0 ");
                    index_input.extend_from_slice(zero_oid.as_bytes());
                }
                index_input.push(b'\t');
                index_input.extend_from_slice(path.as_bytes());
                index_input.push(0);
            }

            self.record_git_plumbing_command();
            self.git_output_bytes_with_input(
                &["update-index", "-z", "--index-info"],
                &index_input,
                Some(&index_path),
            )?;
            self.record_git_plumbing_command();
            self.git_output_with_index(&["write-tree".to_string()], &index_path)
        })();
        let cleanup = batch_dir.close();
        match (result, cleanup) {
            (Ok(tree), Ok(())) => Ok(tree),
            (Ok(_), Err(err)) => Err(Error::Io(err)),
            (Err(err), _) => Err(err),
        }
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

    fn git_output_bytes_with_input(
        &self,
        args: &[&str],
        input: &[u8],
        index_path: Option<&Path>,
    ) -> Result<Vec<u8>> {
        let mut command = Command::new("git");
        command.arg("-C").arg(&self.workspace_root).args(args);
        if let Some(index_path) = index_path {
            command.env("GIT_INDEX_FILE", index_path);
        }
        let mut stdin = tempfile::tempfile()?;
        stdin.write_all(input)?;
        stdin.seek(SeekFrom::Start(0))?;
        let output = command
            .stdin(std::process::Stdio::from(stdin))
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .map_err(|err| Error::Git(err.to_string()))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Git(format!(
                "git {} failed in {}: {}",
                args.join(" "),
                self.workspace_root.display(),
                stderr.trim()
            )));
        }
        Ok(output.stdout)
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

    /// Git evidence is obtainable only while consuming the same fenced
    /// workspace snapshot used by status/diff/record. The private raw
    /// qualifier cannot be called by another producer with a fabricated or
    /// stale baseline.
    pub(crate) fn qualified_git_candidates(
        &self,
    ) -> Result<crate::db::change_ledger::QualifiedGitCandidates> {
        let (_, _, qualified) =
            self.with_workspace_authoritative_git_snapshot(None, |_, _, _, _qualified| Ok(()))?;
        Ok(qualified)
    }

    pub(crate) fn with_workspace_authoritative_git_snapshot<T, F>(
        &self,
        policy_fingerprint_override: Option<[u8; 32]>,
        mut consume: F,
    ) -> Result<(
        T,
        crate::db::change_ledger::FencedCandidateSnapshot,
        crate::db::change_ledger::QualifiedGitCandidates,
    )>
    where
        F: FnMut(
            &Trail,
            &crate::db::change_ledger::CompiledPolicy,
            &crate::db::change_ledger::CandidateSnapshot,
            &crate::db::change_ledger::QualifiedGitCandidates,
        ) -> Result<T>,
    {
        let final_identity = std::cell::RefCell::new(None);
        let held_files = std::cell::RefCell::new(None);
        let final_qualified = std::cell::RefCell::new(None);
        let result = self.with_workspace_authoritative_snapshot(|db, policy, candidates| {
            let (qualified, captured, pinned_worktree) =
                db.qualify_git_snapshot(policy, candidates, policy_fingerprint_override)?;
            *held_files.borrow_mut() =
                Some(GitStructuralHold::open(db, &captured, pinned_worktree)?);
            *final_identity.borrow_mut() = Some(qualified.qualification.clone());
            *final_qualified.borrow_mut() = Some(qualified.clone());
            let mut augmented = candidates.clone();
            for path in &qualified.exact_paths {
                augmented
                    .exact_paths
                    .push(crate::db::change_ledger::LedgerPath::parse(path)?);
            }
            augmented.exact_paths.sort();
            augmented.exact_paths.dedup();
            consume(db, policy, &augmented, &qualified)
        })?;
        #[cfg(debug_assertions)]
        run_git_qualification_after_c2_hook()?;
        let mut metrics = crate::db::change_ledger::GitStructuralMetrics::default();
        let observed = self.capture_git_repository_identity(&mut metrics)?;
        let expected =
            final_identity
                .into_inner()
                .ok_or_else(|| Error::ChangeLedgerReconcileRequired {
                    scope: result.1.candidates.expected.scope_id.to_text(),
                    state: "untrusted_gap".into(),
                    reason: "Git structural qualification omitted its c1 identity".into(),
                    command: "trail index reconcile".into(),
                })?;
        let identity_mismatches = git_identity_mismatches(&observed, &expected);
        let held_matches = held_files
            .into_inner()
            .ok_or_else(|| Error::ChangeLedgerReconcileRequired {
                scope: result.1.candidates.expected.scope_id.to_text(),
                state: "untrusted_gap".into(),
                reason: "Git structural descriptors were not retained through c2".into(),
                command: "trail index reconcile".into(),
            })?
            .verify(self, &mut metrics)?;
        if !identity_mismatches.is_empty() || !held_matches {
            let mut reasons = identity_mismatches;
            if !held_matches {
                reasons.push("held_descriptors");
            }
            return Err(Error::ChangeLedgerReconcileRequired {
                scope: result.1.candidates.expected.scope_id.to_text(),
                state: "untrusted_gap".into(),
                reason: format!(
                    "Git structural identity changed across ledger c2: {}",
                    reasons.join(", ")
                ),
                command: "trail index reconcile".into(),
            });
        }
        let mut qualified =
            final_qualified
                .into_inner()
                .ok_or_else(|| Error::ChangeLedgerReconcileRequired {
                    scope: result.1.candidates.expected.scope_id.to_text(),
                    state: "untrusted_gap".into(),
                    reason: "Git structural qualification omitted its returned evidence".into(),
                    command: "trail index reconcile".into(),
                })?;
        merge_git_structural_metrics(&mut qualified.metrics, &metrics);
        Ok((result.0, result.1, qualified))
    }

    #[cfg(debug_assertions)]
    pub(crate) fn qualified_git_candidates_for_test(
        &self,
        force_policy_mismatch: bool,
    ) -> Result<crate::db::change_ledger::QualifiedGitCandidates> {
        if force_policy_mismatch {
            let (_, _, qualified) = self
                .with_workspace_authoritative_git_snapshot(Some([0x5a; 32]), |_, _, _, _| Ok(()))?;
            Ok(qualified)
        } else {
            self.qualified_git_candidates()
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn git_qualification_full_scan_oracle_for_test(&self) -> Result<Vec<String>> {
        let branch = self.current_branch()?;
        let head = self.resolve_branch_ref(&branch)?;
        let disk_files = self.scan_workspace_files_preserving_root_paths(&head.root_id)?;
        let manifest = self.disk_manifest(&disk_files);
        Ok(self
            .diff_root_to_disk_manifest(&head.root_id, &manifest)?
            .into_iter()
            .map(|summary| summary.path)
            .collect())
    }

    fn qualify_git_snapshot(
        &self,
        policy: &crate::db::change_ledger::CompiledPolicy,
        snapshot: &crate::db::change_ledger::CandidateSnapshot,
        policy_fingerprint_override: Option<[u8; 32]>,
    ) -> Result<(
        crate::db::change_ledger::QualifiedGitCandidates,
        GitRepositoryQualificationIdentity,
        PinnedWorktreeRoot,
    )> {
        use crate::db::change_ledger::{
            GitEvidenceQualification, GitStructuralMetrics, QualifiedGitCandidates, TrustState,
        };

        let mut metrics = GitStructuralMetrics::default();
        let before = self.capture_git_repository_identity(&mut metrics)?;
        let index_bytes =
            self.git_read_structural_index(&before.index_path, false, &mut metrics)?;
        let index = self.git_index_semantics(&mut metrics, &index_bytes)?;
        let config = self.git_qualification_config(!policy.case_sensitive(), &mut metrics)?;
        let matcher = policy.recording_matcher()?;
        let porcelain = self.git_porcelain_v2(&matcher, &mut metrics)?;
        #[cfg(debug_assertions)]
        run_git_qualification_after_porcelain_hook()?;
        let after = self.capture_git_repository_identity(&mut metrics)?;

        let mapped = self.clean_git_mapping_for_evidence(&after.head_oid)?;
        let current_ref = self.conn.query_row(
            "SELECT change_id,root_id,generation FROM refs WHERE name=?1",
            [&snapshot.expected.ref_name],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    ObjectId(row.get(1)?),
                    row.get::<_, i64>(2)?,
                ))
            },
        )?;
        let mapped_trail_root = mapped
            .as_ref()
            .map(|(_, root)| root.clone())
            .unwrap_or_else(|| ObjectId(String::new()));
        let head_equivalent = before.head_oid == after.head_oid
            && before.head_identity == after.head_identity
            && mapped.as_ref().is_some_and(|(change, root)| {
                *root == snapshot.expected.baseline_root
                    && *change == current_ref.0
                    && *root == current_ref.1
            })
            && current_ref.1 == snapshot.expected.baseline_root
            && u64::try_from(current_ref.2).ok() == Some(snapshot.expected.ref_generation);
        let index_equivalent = before.index_path == after.index_path
            && before.index_identity == after.index_identity
            && before.shared_index_path == after.shared_index_path
            && before.shared_index_identity == after.shared_index_identity
            && (!index.split_index
                || (before.shared_index_identity.is_some()
                    && after.shared_index_identity.is_some()))
            && !index.assume_unchanged
            && !index.skip_worktree
            && !index.unresolved;
        let policy_fingerprint =
            policy_fingerprint_override.unwrap_or_else(|| policy.fingerprint());
        let policy_equivalent = snapshot.expected.policy_fingerprint == policy_fingerprint;
        let pinned = self.open_pinned_worktree_root(policy)?;
        let filesystem_identity = self.pinned_worktree_root_identity(&pinned);
        let worktree_equivalent = before.worktree_top_level == after.worktree_top_level
            && before.worktree_identity == after.worktree_identity
            && after.worktree_identity == filesystem_identity;
        let filesystem_equivalent = filesystem_identity == snapshot.expected.filesystem_identity
            && self.verify_pinned_worktree_root(&pinned)?
            && worktree_equivalent;
        let expected_case_insensitive = !policy.case_sensitive();
        let mode_equivalent = config.file_mode;
        // Trail records regular files only. A tracked Git symlink is therefore
        // structurally advisory even when Git itself has native symlink mode.
        let symlink_equivalent = config.symlinks && !index.symlink;
        let sparse_equivalent =
            !config.sparse_checkout && !index.skip_worktree && !index.sparse_index;
        let submodule_equivalent = !index.submodule && !porcelain.unresolved_submodule;
        let ignore_equivalent = self.config.recording.ignore_gitignored && policy_equivalent;
        let case_equivalent = config.ignore_case == expected_case_insensitive;
        let fsmonitor_qualified = !config.fsmonitor && !index.fsmonitor;
        let untracked_cache_qualified = !config.untracked_cache && !index.untracked_cache;
        metrics.fsmonitor_qualification_count = u64::from(fsmonitor_qualified);
        metrics.untracked_cache_qualification_count = u64::from(untracked_cache_qualified);
        self.note_operation_metrics(OperationMetricsDelta {
            git_fsmonitor_qualification_count: metrics.fsmonitor_qualification_count,
            git_untracked_cache_qualification_count: metrics.untracked_cache_qualification_count,
            ..OperationMetricsDelta::default()
        });

        let mut advisory_reasons = Vec::new();
        push_git_advisory(&mut advisory_reasons, !head_equivalent, "head_or_mapping");
        push_git_advisory(
            &mut advisory_reasons,
            !index_equivalent,
            "index_identity_or_flags",
        );
        push_git_advisory(
            &mut advisory_reasons,
            !filesystem_equivalent,
            "filesystem_identity",
        );
        push_git_advisory(
            &mut advisory_reasons,
            !worktree_equivalent,
            "git_worktree_identity",
        );
        push_git_advisory(
            &mut advisory_reasons,
            !policy_equivalent,
            "policy_fingerprint",
        );
        push_git_advisory(&mut advisory_reasons, !mode_equivalent, "file_mode");
        push_git_advisory(&mut advisory_reasons, !symlink_equivalent, "symlink");
        push_git_advisory(
            &mut advisory_reasons,
            !sparse_equivalent,
            "sparse_or_skip_worktree",
        );
        push_git_advisory(&mut advisory_reasons, !submodule_equivalent, "submodule");
        push_git_advisory(&mut advisory_reasons, !ignore_equivalent, "ignore_policy");
        push_git_advisory(&mut advisory_reasons, !case_equivalent, "case_policy");
        push_git_advisory(&mut advisory_reasons, !fsmonitor_qualified, "fsmonitor");
        push_git_advisory(
            &mut advisory_reasons,
            !untracked_cache_qualified,
            "untracked_cache",
        );
        push_git_advisory(
            &mut advisory_reasons,
            !porcelain.complete,
            "porcelain_incomplete",
        );
        push_git_advisory(
            &mut advisory_reasons,
            snapshot.trust != TrustState::Trusted,
            "ledger_not_trusted",
        );
        let clean_proof_allowed = advisory_reasons.is_empty();
        let qualified = QualifiedGitCandidates {
            qualification: GitEvidenceQualification {
                head_oid: after.head_oid.clone(),
                head_identity: after.head_identity.clone(),
                worktree_top_level: after.worktree_top_level.to_string_lossy().into_owned(),
                worktree_identity: after.worktree_identity.clone(),
                index_identity: after.index_identity.clone(),
                split_index_identity: index.split_index.then_some(after.index_identity.clone()),
                shared_index_identity: after.shared_index_identity.clone(),
                mapped_trail_root,
                ledger_baseline_root: snapshot.expected.baseline_root.clone(),
                filesystem_identity,
                policy_fingerprint,
                filesystem_equivalent,
                worktree_equivalent,
                policy_equivalent,
                head_equivalent,
                index_equivalent,
                mode_equivalent,
                symlink_equivalent,
                sparse_equivalent,
                submodule_equivalent,
                ignore_equivalent,
                case_equivalent,
                fsmonitor_qualified,
                untracked_cache_qualified,
                clean_proof_allowed,
                advisory_reasons,
            },
            exact_paths: porcelain.paths.into_iter().collect(),
            rename_pairs: porcelain.rename_pairs,
            metrics,
        };
        Ok((qualified, after, pinned))
    }

    fn capture_git_command_identity(&self) -> Result<GitCommandStructuralIdentity> {
        let mut metrics = crate::db::change_ledger::GitStructuralMetrics::default();
        let head_oid =
            self.git_qualification_text(&["rev-parse", "--verify", "HEAD"], &mut metrics)?;
        let worktree_top_level = self
            .git_qualification_path(&["rev-parse", "--show-toplevel"], &mut metrics)?
            .canonicalize()?;
        let worktree_identity = self.pinned_worktree_identity_for_path(&worktree_top_level)?;
        let index_path =
            self.git_qualification_path(&["rev-parse", "--git-path", "index"], &mut metrics)?;
        // Deliberately do not invoke `rev-parse --shared-index-path` here: Git
        // may inspect the repository index to answer it. The command path does
        // not consume index contents, and every usable split-index transition
        // rewrites/replaces the main index that this bounded metadata fence
        // already holds. Full qualification retains the shared-index check.
        let head_path =
            self.git_qualification_path(&["rev-parse", "--git-path", "HEAD"], &mut metrics)?;
        let packed_refs_path =
            self.git_qualification_path(&["rev-parse", "--git-path", "packed-refs"], &mut metrics)?;
        let symbolic_ref = self
            .git_qualification_optional_text(&["symbolic-ref", "--quiet", "HEAD"], &mut metrics)?;
        let symbolic_ref_path = match symbolic_ref {
            Some(reference) => Some(self.git_qualification_path(
                &["rev-parse", "--git-path", reference.as_str()],
                &mut metrics,
            )?),
            None => None,
        };
        let index_identity =
            git_file_metadata_identity_optional(&index_path)?.ok_or_else(|| {
                Error::Git(format!("Git index `{}` is missing", index_path.display()))
            })?;
        Ok(GitCommandStructuralIdentity {
            head_oid,
            head_path: head_path.clone(),
            head_identity: git_file_metadata_identity_optional(&head_path)?,
            symbolic_ref_path: symbolic_ref_path.clone(),
            symbolic_ref_identity: symbolic_ref_path
                .as_deref()
                .map(git_file_metadata_identity_optional)
                .transpose()?
                .flatten(),
            packed_refs_path: packed_refs_path.clone(),
            packed_refs_identity: git_file_metadata_identity_optional(&packed_refs_path)?,
            worktree_top_level,
            worktree_identity,
            index_path,
            index_identity,
        })
    }

    fn capture_git_repository_identity(
        &self,
        metrics: &mut crate::db::change_ledger::GitStructuralMetrics,
    ) -> Result<GitRepositoryQualificationIdentity> {
        let head_oid = self.git_qualification_text(&["rev-parse", "--verify", "HEAD"], metrics)?;
        let worktree_top_level = self
            .git_qualification_path(&["rev-parse", "--show-toplevel"], metrics)?
            .canonicalize()?;
        let worktree_identity = self.pinned_worktree_identity_for_path(&worktree_top_level)?;
        let index_path =
            self.git_qualification_path(&["rev-parse", "--git-path", "index"], metrics)?;
        let shared_index_path = self
            .git_qualification_optional_text(&["rev-parse", "--shared-index-path"], metrics)
            .map_err(|error| Error::Git(format!("Git shared_index discovery failed: {error}")))?
            .filter(|path| !path.is_empty())
            .map(|path| self.absolute_git_path(&path));
        let head_path =
            self.git_qualification_path(&["rev-parse", "--git-path", "HEAD"], metrics)?;
        let packed_refs =
            self.git_qualification_path(&["rev-parse", "--git-path", "packed-refs"], metrics)?;
        let symbolic_ref =
            self.git_qualification_optional_text(&["symbolic-ref", "--quiet", "HEAD"], metrics)?;
        let symbolic_ref_path = match symbolic_ref {
            Some(reference) => {
                let path = self.git_qualification_path(
                    &["rev-parse", "--git-path", reference.as_str()],
                    metrics,
                )?;
                Some(path)
            }
            None => None,
        };
        let head_file_identity = git_file_identity_optional(&head_path)?;
        let symbolic_ref_identity = symbolic_ref_path
            .as_deref()
            .map(git_file_identity_optional)
            .transpose()?
            .flatten();
        let packed_refs_identity = git_file_identity_optional(&packed_refs)?;
        let mut head_identity = head_oid.as_bytes().to_vec();
        for identity in [
            head_file_identity.as_ref(),
            symbolic_ref_identity.as_ref(),
            packed_refs_identity.as_ref(),
        ] {
            head_identity.push(0);
            if let Some(identity) = identity {
                head_identity.extend(identity);
            }
        }
        let index_identity = self
            .git_structural_index_identity(&index_path, false, metrics)?
            .ok_or_else(|| {
                Error::Git(format!("Git index `{}` is missing", index_path.display()))
            })?;
        let shared_index_identity = shared_index_path
            .as_deref()
            .map(|path| self.git_structural_index_identity(path, true, metrics))
            .transpose()?
            .flatten();
        Ok(GitRepositoryQualificationIdentity {
            head_oid,
            head_identity,
            head_path,
            head_file_identity,
            symbolic_ref_path,
            symbolic_ref_identity,
            packed_refs_path: packed_refs,
            packed_refs_identity,
            worktree_top_level,
            worktree_identity,
            index_path,
            index_identity,
            shared_index_path,
            shared_index_identity,
        })
    }

    fn git_read_structural_index(
        &self,
        path: &Path,
        shared: bool,
        metrics: &mut crate::db::change_ledger::GitStructuralMetrics,
    ) -> Result<Vec<u8>> {
        let mut file = open_git_structural_file_no_follow(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let byte_count = saturating_u64_from_usize(bytes.len());
        self.note_git_structural_read(shared, byte_count, metrics);
        Ok(bytes)
    }

    fn note_git_structural_read(
        &self,
        shared: bool,
        byte_count: u64,
        metrics: &mut crate::db::change_ledger::GitStructuralMetrics,
    ) {
        if shared {
            metrics.shared_index_read_count = metrics.shared_index_read_count.saturating_add(1);
            metrics.shared_index_bytes = metrics.shared_index_bytes.saturating_add(byte_count);
        } else {
            metrics.index_read_count = metrics.index_read_count.saturating_add(1);
            metrics.index_bytes = metrics.index_bytes.saturating_add(byte_count);
        }
        self.note_operation_metrics(OperationMetricsDelta {
            git_index_read_count: u64::from(!shared),
            git_index_bytes: if shared { 0 } else { byte_count },
            git_shared_index_read_count: u64::from(shared),
            git_shared_index_bytes: if shared { byte_count } else { 0 },
            filesystem_read_count: 1,
            filesystem_read_bytes: byte_count,
            ..OperationMetricsDelta::default()
        });
    }

    fn git_structural_index_identity(
        &self,
        path: &Path,
        shared: bool,
        metrics: &mut crate::db::change_ledger::GitStructuralMetrics,
    ) -> Result<Option<Vec<u8>>> {
        let mut file = match open_git_structural_file_no_follow(path) {
            Ok(file) => file,
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(None)
            }
            Err(error) => return Err(error),
        };
        let metadata = file.metadata()?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        self.note_git_structural_read(shared, saturating_u64_from_usize(bytes.len()), metrics);
        Ok(Some(git_file_identity(&metadata, &bytes)))
    }

    fn git_qualification_config(
        &self,
        default_ignore_case: bool,
        metrics: &mut crate::db::change_ledger::GitStructuralMetrics,
    ) -> Result<GitQualificationConfig> {
        Ok(GitQualificationConfig {
            file_mode: self.git_qualification_bool("core.filemode", true, metrics)?,
            symlinks: self.git_qualification_bool("core.symlinks", true, metrics)?,
            ignore_case: self.git_qualification_bool(
                "core.ignorecase",
                default_ignore_case,
                metrics,
            )?,
            sparse_checkout: self.git_qualification_bool("core.sparsecheckout", false, metrics)?,
            fsmonitor: self
                .git_qualification_optional_text(&["config", "--get", "core.fsmonitor"], metrics)?
                .is_some_and(|value| !matches!(value.as_str(), "" | "false" | "0")),
            untracked_cache: self
                .git_qualification_optional_text(
                    &["config", "--get", "core.untrackedcache"],
                    metrics,
                )?
                .is_some_and(|value| !matches!(value.as_str(), "" | "false" | "0")),
        })
    }

    fn git_qualification_bool(
        &self,
        key: &str,
        default: bool,
        metrics: &mut crate::db::change_ledger::GitStructuralMetrics,
    ) -> Result<bool> {
        let Some(value) =
            self.git_qualification_optional_text(&["config", "--bool", "--get", key], metrics)?
        else {
            return Ok(default);
        };
        match value.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(Error::Git(format!(
                "Git config `{key}` returned non-boolean value `{value}`"
            ))),
        }
    }

    fn git_index_semantics(
        &self,
        metrics: &mut crate::db::change_ledger::GitStructuralMetrics,
        index_bytes: &[u8],
    ) -> Result<GitIndexSemantics> {
        let records_before = metrics.output_record_count;
        let staged = self.git_qualification_output(
            &["ls-files", "--stage", "-t", "-z"],
            metrics,
            true,
            None,
        )?;
        let mut semantics = GitIndexSemantics {
            fsmonitor: bytes_contain(index_bytes, b"FSMN"),
            untracked_cache: bytes_contain(index_bytes, b"UNTR"),
            sparse_index: bytes_contain(index_bytes, b"sdir"),
            split_index: bytes_contain(index_bytes, b"link"),
            ..GitIndexSemantics::default()
        };
        for record in staged
            .stdout
            .split(|byte| *byte == 0)
            .filter(|raw| !raw.is_empty())
        {
            metrics.output_record_count = metrics.output_record_count.saturating_add(1);
            let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
                semantics.unresolved = true;
                continue;
            };
            let header = &record[..tab];
            let tag = header.first().copied().unwrap_or_default();
            semantics.skip_worktree |= tag == b'S';
            let mut fields = header[1..]
                .split(|byte| byte.is_ascii_whitespace())
                .filter(|part| !part.is_empty());
            let mode = fields.next().unwrap_or_default();
            let _oid = fields.next();
            let stage = fields.next().unwrap_or_default();
            semantics.unresolved |= stage != b"0";
            semantics.symlink |= mode == b"120000";
            semantics.submodule |= mode == b"160000";
        }
        let verbose =
            self.git_qualification_output(&["ls-files", "-v", "-z"], metrics, true, None)?;
        for record in verbose
            .stdout
            .split(|byte| *byte == 0)
            .filter(|raw| !raw.is_empty())
        {
            metrics.output_record_count = metrics.output_record_count.saturating_add(1);
            semantics.assume_unchanged |= record
                .first()
                .copied()
                .is_some_and(|tag| tag.is_ascii_lowercase());
        }
        self.note_operation_metrics(OperationMetricsDelta {
            git_output_record_count: metrics.output_record_count.saturating_sub(records_before),
            ..OperationMetricsDelta::default()
        });
        Ok(semantics)
    }

    fn git_porcelain_v2(
        &self,
        matcher: &crate::db::change_ledger::CompiledRecordingMatcher,
        metrics: &mut crate::db::change_ledger::GitStructuralMetrics,
    ) -> Result<GitPorcelainEvidence> {
        let trace = tempfile::NamedTempFile::new()?;
        let output = self.git_qualification_output(
            &[
                "status",
                "--porcelain=v2",
                "-z",
                "--untracked-files=all",
                "--ignore-submodules=none",
            ],
            metrics,
            true,
            Some(trace.path()),
        )?;
        let trace_bytes = fs::read(trace.path())?;
        metrics.trace2_bytes = metrics
            .trace2_bytes
            .saturating_add(saturating_u64_from_usize(trace_bytes.len()));
        for line in trace_bytes.split(|byte| *byte == b'\n') {
            if bytes_contain(line, b"\"event\":\"region_enter\"") {
                metrics.trace2_region_count = metrics.trace2_region_count.saturating_add(1);
            }
            if bytes_contain(line, b"\"label\":\"refresh\"") {
                metrics.index_refresh_count = metrics.index_refresh_count.saturating_add(1);
            }
        }
        self.note_operation_metrics(OperationMetricsDelta {
            git_trace2_region_count: metrics.trace2_region_count,
            git_trace2_bytes: metrics.trace2_bytes,
            git_index_refresh_count: metrics.index_refresh_count,
            ..OperationMetricsDelta::default()
        });
        let records_before = metrics.output_record_count;
        let evidence = parse_git_porcelain_v2(&output.stdout, matcher, metrics)?;
        self.note_operation_metrics(OperationMetricsDelta {
            git_output_record_count: metrics.output_record_count.saturating_sub(records_before),
            ..OperationMetricsDelta::default()
        });
        Ok(evidence)
    }

    fn git_qualification_path(
        &self,
        args: &[&str],
        metrics: &mut crate::db::change_ledger::GitStructuralMetrics,
    ) -> Result<PathBuf> {
        let value = self.git_qualification_text(args, metrics)?;
        Ok(self.absolute_git_path(&value))
    }

    fn absolute_git_path(&self, value: &str) -> PathBuf {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            path
        } else {
            self.workspace_root.join(path)
        }
    }

    fn git_qualification_text(
        &self,
        args: &[&str],
        metrics: &mut crate::db::change_ledger::GitStructuralMetrics,
    ) -> Result<String> {
        let output = self.git_qualification_output(args, metrics, false, None)?;
        if !output.status.success() {
            return Err(git_qualification_command_error(
                args,
                &self.workspace_root,
                &output,
            ));
        }
        String::from_utf8(output.stdout)
            .map(|value| value.trim().to_string())
            .map_err(|error| {
                Error::Git(format!(
                    "git {} returned non-UTF-8: {error}",
                    args.join(" ")
                ))
            })
    }

    fn git_qualification_optional_text(
        &self,
        args: &[&str],
        metrics: &mut crate::db::change_ledger::GitStructuralMetrics,
    ) -> Result<Option<String>> {
        let output = self.git_qualification_output(args, metrics, false, None)?;
        if output.status.success() {
            return String::from_utf8(output.stdout)
                .map(|value| Some(value.trim().to_string()))
                .map_err(|error| {
                    Error::Git(format!(
                        "git {} returned non-UTF-8: {error}",
                        args.join(" ")
                    ))
                });
        }
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        Err(git_qualification_command_error(
            args,
            &self.workspace_root,
            &output,
        ))
    }

    fn git_qualification_output(
        &self,
        args: &[&str],
        metrics: &mut crate::db::change_ledger::GitStructuralMetrics,
        global_work: bool,
        trace2: Option<&Path>,
    ) -> Result<std::process::Output> {
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(&self.workspace_root)
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0");
        for selector in GIT_REPOSITORY_SELECTOR_ENVIRONMENT {
            command.env_remove(selector);
        }
        if let Some(trace2) = trace2 {
            command.env("GIT_TRACE2_EVENT", trace2);
        }
        let output = command
            .output()
            .map_err(|error| Error::Git(error.to_string()))?;
        let bytes = output.stdout.len().saturating_add(output.stderr.len());
        metrics.subprocess_count = metrics.subprocess_count.saturating_add(1);
        metrics.output_bytes = metrics
            .output_bytes
            .saturating_add(saturating_u64_from_usize(bytes));
        if global_work {
            metrics.external_adapter_global_work =
                metrics.external_adapter_global_work.saturating_add(1);
        }
        self.note_operation_metrics(OperationMetricsDelta {
            git_subprocess_count: 1,
            git_global_work_count: u64::from(global_work),
            external_adapter_global_work: u64::from(global_work),
            git_output_bytes: saturating_u64_from_usize(bytes),
            ..OperationMetricsDelta::default()
        });
        if !output.status.success() && global_work {
            return Err(git_qualification_command_error(
                args,
                &self.workspace_root,
                &output,
            ));
        }
        Ok(output)
    }
}

const GIT_REPOSITORY_SELECTOR_ENVIRONMENT: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CEILING_DIRECTORIES",
    "GIT_DISCOVERY_ACROSS_FILESYSTEM",
    "GIT_PREFIX",
];

#[derive(Debug)]
struct GitCommandStructuralIdentity {
    head_oid: String,
    head_path: PathBuf,
    head_identity: Option<Vec<u8>>,
    symbolic_ref_path: Option<PathBuf>,
    symbolic_ref_identity: Option<Vec<u8>>,
    packed_refs_path: PathBuf,
    packed_refs_identity: Option<Vec<u8>>,
    worktree_top_level: PathBuf,
    worktree_identity: Vec<u8>,
    index_path: PathBuf,
    index_identity: Vec<u8>,
}

#[derive(Debug)]
struct GitRepositoryQualificationIdentity {
    head_oid: String,
    head_identity: Vec<u8>,
    head_path: PathBuf,
    head_file_identity: Option<Vec<u8>>,
    symbolic_ref_path: Option<PathBuf>,
    symbolic_ref_identity: Option<Vec<u8>>,
    packed_refs_path: PathBuf,
    packed_refs_identity: Option<Vec<u8>>,
    worktree_top_level: PathBuf,
    worktree_identity: Vec<u8>,
    index_path: PathBuf,
    index_identity: Vec<u8>,
    shared_index_path: Option<PathBuf>,
    shared_index_identity: Option<Vec<u8>>,
}

struct GitCommandStructuralHold {
    index: HeldGitMetadataFile,
    head: Option<HeldGitMetadataFile>,
    symbolic_ref: Option<HeldGitMetadataFile>,
    packed_refs: Option<HeldGitMetadataFile>,
    worktree: PinnedWorktreeRoot,
}

impl GitCommandStructuralHold {
    fn open(
        db: &Trail,
        policy: &crate::db::change_ledger::CompiledPolicy,
        identity: &GitCommandStructuralIdentity,
    ) -> Result<Self> {
        let index =
            HeldGitMetadataFile::open_required(&identity.index_path, &identity.index_identity)?;
        let head = HeldGitMetadataFile::open_optional(
            &identity.head_path,
            identity.head_identity.as_deref(),
        )?;
        let symbolic_ref = HeldGitMetadataFile::open_optional_pair(
            identity.symbolic_ref_path.as_deref(),
            identity.symbolic_ref_identity.as_deref(),
            "symbolic-ref",
        )?;
        let packed_refs = HeldGitMetadataFile::open_optional(
            &identity.packed_refs_path,
            identity.packed_refs_identity.as_deref(),
        )?;
        let worktree = db.open_pinned_worktree_root(policy)?;
        if db.pinned_worktree_root_identity(&worktree) != identity.worktree_identity
            || !db.verify_pinned_worktree_root(&worktree)?
        {
            return Err(Error::ChangeLedgerReconcileRequired {
                scope: "workspace".into(),
                state: "untrusted_gap".into(),
                reason: "Git worktree root changed before ledger command consumption".into(),
                command: "trail index reconcile".into(),
            });
        }
        Ok(Self {
            index,
            head,
            symbolic_ref,
            packed_refs,
            worktree,
        })
    }

    fn verify(mut self, db: &Trail) -> Result<bool> {
        if !self.index.verify()? {
            return Ok(false);
        }
        for held in [
            &mut self.head,
            &mut self.symbolic_ref,
            &mut self.packed_refs,
        ]
        .into_iter()
        .flatten()
        {
            if !held.verify()? {
                return Ok(false);
            }
        }
        db.verify_pinned_worktree_root(&self.worktree)
    }
}

struct GitStructuralHold {
    index: HeldGitStructuralFile,
    shared_index: Option<HeldGitStructuralFile>,
    head: Option<HeldGitStructuralFile>,
    symbolic_ref: Option<HeldGitStructuralFile>,
    packed_refs: Option<HeldGitStructuralFile>,
    worktree: PinnedWorktreeRoot,
}

impl GitStructuralHold {
    fn open(
        db: &Trail,
        identity: &GitRepositoryQualificationIdentity,
        worktree: PinnedWorktreeRoot,
    ) -> Result<Self> {
        let index =
            HeldGitStructuralFile::open_required(&identity.index_path, &identity.index_identity)?;
        let shared_index = HeldGitStructuralFile::open_optional_pair(
            identity.shared_index_path.as_deref(),
            identity.shared_index_identity.as_deref(),
            "shared-index",
        )?;
        let head = HeldGitStructuralFile::open_optional(
            &identity.head_path,
            identity.head_file_identity.as_deref(),
        )?;
        let symbolic_ref = match (
            identity.symbolic_ref_path.as_deref(),
            identity.symbolic_ref_identity.as_deref(),
        ) {
            (Some(path), expected) => HeldGitStructuralFile::open_optional(path, expected)?,
            (None, None) => None,
            (None, Some(_)) => {
                return Err(Error::Git(
                    "Git symbolic-ref identity has no structural path".into(),
                ));
            }
        };
        let packed_refs = HeldGitStructuralFile::open_optional(
            &identity.packed_refs_path,
            identity.packed_refs_identity.as_deref(),
        )?;
        if db.pinned_worktree_root_identity(&worktree) != identity.worktree_identity
            || !db.verify_pinned_worktree_root(&worktree)?
        {
            return Err(Error::ChangeLedgerReconcileRequired {
                scope: "workspace".into(),
                state: "untrusted_gap".into(),
                reason: "Git worktree root changed before ledger consumption".into(),
                command: "trail index reconcile".into(),
            });
        }
        Ok(Self {
            index,
            shared_index,
            head,
            symbolic_ref,
            packed_refs,
            worktree,
        })
    }

    fn verify(
        mut self,
        db: &Trail,
        metrics: &mut crate::db::change_ledger::GitStructuralMetrics,
    ) -> Result<bool> {
        let (index_matches, index_bytes) = self.index.verify()?;
        db.note_git_structural_read(false, index_bytes, metrics);
        if !index_matches {
            return Ok(false);
        }
        if let Some(mut shared) = self.shared_index {
            let (shared_matches, shared_bytes) = shared.verify()?;
            db.note_git_structural_read(true, shared_bytes, metrics);
            if !shared_matches {
                return Ok(false);
            }
        }
        for held in [
            &mut self.head,
            &mut self.symbolic_ref,
            &mut self.packed_refs,
        ]
        .into_iter()
        .flatten()
        {
            if !held.verify()?.0 {
                return Ok(false);
            }
        }
        db.verify_pinned_worktree_root(&self.worktree)
    }
}

struct HeldGitMetadataFile {
    file: File,
    expected_identity: Vec<u8>,
}

impl HeldGitMetadataFile {
    fn open_required(path: &Path, expected_identity: &[u8]) -> Result<Self> {
        let file = open_git_structural_file_no_follow(path)?;
        if git_file_metadata_identity(&file.metadata()?) != expected_identity {
            return Err(Error::ChangeLedgerReconcileRequired {
                scope: "workspace".into(),
                state: "untrusted_gap".into(),
                reason: format!(
                    "Git structural file `{}` changed before ledger command consumption",
                    path.display()
                ),
                command: "trail index reconcile".into(),
            });
        }
        Ok(Self {
            file,
            expected_identity: expected_identity.to_vec(),
        })
    }

    fn open_optional(path: &Path, expected_identity: Option<&[u8]>) -> Result<Option<Self>> {
        match expected_identity {
            Some(expected) => Self::open_required(path, expected).map(Some),
            None => match open_git_structural_file_no_follow(path) {
                Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
                Ok(_) => Err(Error::ChangeLedgerReconcileRequired {
                    scope: "workspace".into(),
                    state: "untrusted_gap".into(),
                    reason: format!(
                        "Git structural file `{}` appeared before ledger command consumption",
                        path.display()
                    ),
                    command: "trail index reconcile".into(),
                }),
            },
        }
    }

    fn open_optional_pair(
        path: Option<&Path>,
        expected_identity: Option<&[u8]>,
        label: &str,
    ) -> Result<Option<Self>> {
        match (path, expected_identity) {
            (Some(path), expected) => Self::open_optional(path, expected),
            (None, None) => Ok(None),
            (None, Some(_)) => Err(Error::Git(format!(
                "Git {label} identity has no structural path"
            ))),
        }
    }

    fn verify(&mut self) -> Result<bool> {
        Ok(git_file_metadata_identity(&self.file.metadata()?) == self.expected_identity)
    }
}

struct HeldGitStructuralFile {
    file: File,
    expected_identity: Vec<u8>,
}

impl HeldGitStructuralFile {
    fn open_required(path: &Path, expected_identity: &[u8]) -> Result<Self> {
        let mut file = open_git_structural_file_no_follow(path)?;
        let (observed, _) = held_git_file_identity(&mut file)?;
        if observed != expected_identity {
            return Err(Error::ChangeLedgerReconcileRequired {
                scope: "workspace".into(),
                state: "untrusted_gap".into(),
                reason: format!(
                    "Git structural file `{}` changed before ledger consumption",
                    path.display()
                ),
                command: "trail index reconcile".into(),
            });
        }
        Ok(Self {
            file,
            expected_identity: expected_identity.to_vec(),
        })
    }

    fn open_optional(path: &Path, expected_identity: Option<&[u8]>) -> Result<Option<Self>> {
        match expected_identity {
            Some(expected) => Self::open_required(path, expected).map(Some),
            None => match open_git_structural_file_no_follow(path) {
                Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
                Ok(_) => Err(Error::ChangeLedgerReconcileRequired {
                    scope: "workspace".into(),
                    state: "untrusted_gap".into(),
                    reason: format!(
                        "Git structural file `{}` appeared before ledger consumption",
                        path.display()
                    ),
                    command: "trail index reconcile".into(),
                }),
            },
        }
    }

    fn open_optional_pair(
        path: Option<&Path>,
        expected_identity: Option<&[u8]>,
        label: &str,
    ) -> Result<Option<Self>> {
        match (path, expected_identity) {
            (Some(path), expected) => Self::open_optional(path, expected),
            (None, None) => Ok(None),
            (None, Some(_)) => Err(Error::Git(format!(
                "Git {label} identity has no structural path"
            ))),
        }
    }

    fn verify(&mut self) -> Result<(bool, u64)> {
        let (observed, bytes) = held_git_file_identity(&mut self.file)?;
        Ok((observed == self.expected_identity, bytes))
    }
}

#[derive(Debug)]
struct GitQualificationConfig {
    file_mode: bool,
    symlinks: bool,
    ignore_case: bool,
    sparse_checkout: bool,
    fsmonitor: bool,
    untracked_cache: bool,
}

#[derive(Debug, Default)]
struct GitIndexSemantics {
    assume_unchanged: bool,
    skip_worktree: bool,
    unresolved: bool,
    symlink: bool,
    submodule: bool,
    fsmonitor: bool,
    untracked_cache: bool,
    sparse_index: bool,
    split_index: bool,
}

#[derive(Debug, Default)]
struct GitPorcelainEvidence {
    paths: BTreeSet<String>,
    rename_pairs: Vec<(String, String)>,
    unresolved_submodule: bool,
    complete: bool,
}

fn parse_git_porcelain_v2(
    output: &[u8],
    matcher: &crate::db::change_ledger::CompiledRecordingMatcher,
    metrics: &mut crate::db::change_ledger::GitStructuralMetrics,
) -> Result<GitPorcelainEvidence> {
    let records = output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .collect::<Vec<_>>();
    let mut evidence = GitPorcelainEvidence {
        complete: true,
        ..GitPorcelainEvidence::default()
    };
    let mut index = 0;
    while index < records.len() {
        let record = records[index];
        metrics.output_record_count = metrics.output_record_count.saturating_add(1);
        let parsed = match record.first().copied() {
            Some(b'1') => split_git_record(record, 9).map(|fields| {
                evidence.unresolved_submodule |= fields[2] != b"N...";
                vec![fields[8]]
            }),
            Some(b'2') => split_git_record(record, 10).and_then(|fields| {
                evidence.unresolved_submodule |= fields[2] != b"N...";
                index = index.saturating_add(1);
                records.get(index).map(|old| vec![fields[9], *old])
            }),
            Some(b'u') => split_git_record(record, 11).map(|fields| {
                evidence.complete = false;
                evidence.unresolved_submodule = true;
                vec![fields[10]]
            }),
            Some(b'?') if record.get(1) == Some(&b' ') => Some(vec![&record[2..]]),
            Some(b'#') => Some(Vec::new()),
            _ => None,
        };
        let Some(paths) = parsed else {
            evidence.complete = false;
            index = index.saturating_add(1);
            continue;
        };
        let mut normalized = Vec::new();
        for raw in paths {
            let path = match std::str::from_utf8(raw) {
                Ok(path) => match normalize_relative_path(path) {
                    Ok(path) => path,
                    Err(_) => {
                        evidence.complete = false;
                        continue;
                    }
                },
                Err(_) => {
                    evidence.complete = false;
                    continue;
                }
            };
            if !matcher.is_ignored(&path, false)? {
                evidence.paths.insert(path.clone());
            }
            normalized.push(path);
        }
        if record.first() == Some(&b'2') && normalized.len() == 2 {
            evidence
                .rename_pairs
                .push((normalized[1].clone(), normalized[0].clone()));
        }
        index = index.saturating_add(1);
    }
    Ok(evidence)
}

fn split_git_record(record: &[u8], expected_fields: usize) -> Option<Vec<&[u8]>> {
    let fields = record
        .splitn(expected_fields, |byte| *byte == b' ')
        .collect::<Vec<_>>();
    (fields.len() == expected_fields).then_some(fields)
}

fn git_file_identity_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    let mut file = match open_git_structural_file_no_follow(path) {
        Ok(file) => file,
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    held_git_file_identity(&mut file).map(|(identity, _)| Some(identity))
}

fn git_file_metadata_identity_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    let file = match open_git_structural_file_no_follow(path) {
        Ok(file) => file,
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(Some(git_file_metadata_identity(&file.metadata()?)))
}

fn open_git_structural_file_no_follow(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(Error::Git(format!(
            "Git structural file `{}` is not a regular file",
            path.display()
        )));
    }
    Ok(file)
}

fn git_file_metadata_identity(metadata: &fs::Metadata) -> Vec<u8> {
    let mut digest = Sha256::new();
    digest.update(b"trail-git-file-metadata-identity-v1\0");
    #[cfg(unix)]
    {
        digest.update(metadata.dev().to_le_bytes());
        digest.update(metadata.ino().to_le_bytes());
        digest.update(metadata.mode().to_le_bytes());
        digest.update(metadata.mtime().to_le_bytes());
        digest.update(metadata.mtime_nsec().to_le_bytes());
        digest.update(metadata.ctime().to_le_bytes());
        digest.update(metadata.ctime_nsec().to_le_bytes());
    }
    #[cfg(not(unix))]
    if let Ok(modified) = metadata.modified() {
        if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
            digest.update(duration.as_nanos().to_le_bytes());
        }
    }
    digest.update(metadata.len().to_le_bytes());
    digest.finalize().to_vec()
}

fn git_file_identity(metadata: &fs::Metadata, bytes: &[u8]) -> Vec<u8> {
    let mut digest = Sha256::new();
    digest.update(b"trail-git-file-identity-v1\0");
    #[cfg(unix)]
    {
        digest.update(metadata.dev().to_le_bytes());
        digest.update(metadata.ino().to_le_bytes());
        digest.update(metadata.mode().to_le_bytes());
    }
    digest.update(metadata.len().to_le_bytes());
    digest.update(Sha256::digest(bytes));
    digest.finalize().to_vec()
}

fn held_git_file_identity(file: &mut File) -> Result<(Vec<u8>, u64)> {
    file.seek(SeekFrom::Start(0))?;
    let metadata = file.metadata()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    file.seek(SeekFrom::Start(0))?;
    Ok((
        git_file_identity(&metadata, &bytes),
        saturating_u64_from_usize(bytes.len()),
    ))
}

fn merge_git_structural_metrics(
    target: &mut crate::db::change_ledger::GitStructuralMetrics,
    additional: &crate::db::change_ledger::GitStructuralMetrics,
) {
    target.subprocess_count = target
        .subprocess_count
        .saturating_add(additional.subprocess_count);
    target.index_refresh_count = target
        .index_refresh_count
        .saturating_add(additional.index_refresh_count);
    target.trace2_region_count = target
        .trace2_region_count
        .saturating_add(additional.trace2_region_count);
    target.trace2_bytes = target.trace2_bytes.saturating_add(additional.trace2_bytes);
    target.output_bytes = target.output_bytes.saturating_add(additional.output_bytes);
    target.output_record_count = target
        .output_record_count
        .saturating_add(additional.output_record_count);
    target.index_read_count = target
        .index_read_count
        .saturating_add(additional.index_read_count);
    target.index_bytes = target.index_bytes.saturating_add(additional.index_bytes);
    target.shared_index_read_count = target
        .shared_index_read_count
        .saturating_add(additional.shared_index_read_count);
    target.shared_index_bytes = target
        .shared_index_bytes
        .saturating_add(additional.shared_index_bytes);
    target.fsmonitor_qualification_count = target
        .fsmonitor_qualification_count
        .saturating_add(additional.fsmonitor_qualification_count);
    target.untracked_cache_qualification_count = target
        .untracked_cache_qualification_count
        .saturating_add(additional.untracked_cache_qualification_count);
    target.external_adapter_global_work = target
        .external_adapter_global_work
        .saturating_add(additional.external_adapter_global_work);
}

fn git_identity_mismatches(
    observed: &GitRepositoryQualificationIdentity,
    expected: &crate::db::change_ledger::GitEvidenceQualification,
) -> Vec<&'static str> {
    let mut mismatches = Vec::new();
    if observed.head_oid != expected.head_oid || observed.head_identity != expected.head_identity {
        mismatches.push("HEAD");
    }
    if observed.worktree_top_level.to_string_lossy() != expected.worktree_top_level
        || observed.worktree_identity != expected.worktree_identity
    {
        mismatches.push("worktree");
    }
    if observed.index_identity != expected.index_identity {
        mismatches.push("index");
    }
    if observed.shared_index_identity != expected.shared_index_identity {
        mismatches.push("shared_index");
    }
    mismatches
}

fn git_command_identity_matches(
    observed: &GitCommandStructuralIdentity,
    expected: &GitCommandStructuralIdentity,
) -> bool {
    observed.head_oid == expected.head_oid
        && observed.head_path == expected.head_path
        && observed.head_identity == expected.head_identity
        && observed.symbolic_ref_path == expected.symbolic_ref_path
        && observed.symbolic_ref_identity == expected.symbolic_ref_identity
        && observed.packed_refs_path == expected.packed_refs_path
        && observed.packed_refs_identity == expected.packed_refs_identity
        && observed.worktree_top_level == expected.worktree_top_level
        && observed.worktree_identity == expected.worktree_identity
        && observed.index_path == expected.index_path
        && observed.index_identity == expected.index_identity
}

fn bytes_contain(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn push_git_advisory(reasons: &mut Vec<String>, condition: bool, reason: &str) {
    if condition {
        reasons.push(reason.to_string());
    }
}

fn git_qualification_command_error(
    args: &[&str],
    workspace: &Path,
    output: &std::process::Output,
) -> Error {
    Error::Git(format!(
        "git {} failed in {}: {}",
        args.join(" "),
        workspace.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

#[cfg(debug_assertions)]
type GitQualificationHook = Box<dyn FnOnce() -> Result<()> + Send>;

#[cfg(debug_assertions)]
static GIT_QUALIFICATION_AFTER_PORCELAIN_HOOK: std::sync::OnceLock<
    std::sync::Mutex<Option<GitQualificationHook>>,
> = std::sync::OnceLock::new();

#[cfg(debug_assertions)]
static GIT_QUALIFICATION_AFTER_C2_HOOK: std::sync::OnceLock<
    std::sync::Mutex<Option<GitQualificationHook>>,
> = std::sync::OnceLock::new();

#[cfg(debug_assertions)]
pub(crate) fn install_git_qualification_after_porcelain_hook(
    hook: impl FnOnce() -> Result<()> + Send + 'static,
) {
    *GIT_QUALIFICATION_AFTER_PORCELAIN_HOOK
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = Some(Box::new(hook));
}

#[cfg(debug_assertions)]
pub(crate) fn install_git_qualification_after_c2_hook(
    hook: impl FnOnce() -> Result<()> + Send + 'static,
) {
    *GIT_QUALIFICATION_AFTER_C2_HOOK
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner()) = Some(Box::new(hook));
}

#[cfg(debug_assertions)]
fn run_git_qualification_after_porcelain_hook() -> Result<()> {
    let hook = GIT_QUALIFICATION_AFTER_PORCELAIN_HOOK
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take();
    match hook {
        Some(hook) => hook(),
        None => Ok(()),
    }
}

#[cfg(debug_assertions)]
fn run_git_qualification_after_c2_hook() -> Result<()> {
    let hook = GIT_QUALIFICATION_AFTER_C2_HOOK
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take();
    match hook {
        Some(hook) => hook(),
        None => Ok(()),
    }
}

fn append_git_stdin_path(input: &mut Vec<u8>, path: &Path) -> Result<()> {
    #[cfg(unix)]
    let bytes = path.as_os_str().as_bytes();
    #[cfg(not(unix))]
    let bytes = path
        .to_str()
        .ok_or_else(|| Error::Git("Git blob batch path is not valid UTF-8".to_string()))?
        .as_bytes();
    if bytes.contains(&b'\n') || bytes.contains(&b'\r') {
        return Err(Error::Git(
            "Git blob batch path contains a line separator".to_string(),
        ));
    }
    input.extend_from_slice(bytes);
    input.push(b'\n');
    Ok(())
}

fn parse_git_hash_object_oids(output: &[u8], expected_count: usize) -> Result<Vec<String>> {
    let output = std::str::from_utf8(output)
        .map_err(|err| Error::Git(format!("git hash-object returned non-UTF-8 output: {err}")))?;
    let mut lines = output.split('\n').collect::<Vec<_>>();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    if lines.len() != expected_count {
        return Err(Error::Git(format!(
            "git hash-object returned {} object IDs for {expected_count} paths",
            lines.len()
        )));
    }
    let mut oid_length = None;
    let mut oids = Vec::with_capacity(lines.len());
    for (index, oid) in lines.into_iter().enumerate() {
        if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(Error::Git(format!(
                "git hash-object returned invalid object ID at position {index}: `{oid}`"
            )));
        }
        if oid_length
            .replace(oid.len())
            .is_some_and(|length| length != oid.len())
        {
            return Err(Error::Git(
                "git hash-object returned mixed object ID lengths".to_string(),
            ));
        }
        oids.push(oid.to_string());
    }
    Ok(oids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn git_stdin_path_preserves_non_utf8_bytes() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let path = PathBuf::from(OsString::from_vec(b"/tmp/trail-\xff/blob".to_vec()));
        let mut input = Vec::new();

        append_git_stdin_path(&mut input, &path).unwrap();

        assert_eq!(input, b"/tmp/trail-\xff/blob\n");
    }

    #[test]
    fn git_hash_batch_drains_output_larger_than_pipe_capacity() {
        const CHILD_ENV: &str = "TRAIL_GIT_HASH_BATCH_DEADLOCK_CHILD";
        const TEST_NAME: &str =
            "db::storage::git::tests::git_hash_batch_drains_output_larger_than_pipe_capacity";
        if std::env::var_os(CHILD_ENV).is_none() {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST_NAME, "--nocapture"])
                .env(CHILD_ENV, "1")
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert!(status.success(), "bounded Git hash batch child failed");
                    return;
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("Git hash batch deadlocked while stdout exceeded pipe capacity");
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }

        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        Command::new("git")
            .arg("-C")
            .arg(temp.path())
            .arg("init")
            .output()
            .unwrap();
        fs::write(temp.path().join("batch-source"), b"batch bytes\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let mut input = Vec::with_capacity(1_300_000);
        for _ in 0..100_000 {
            input.extend_from_slice(b"batch-source\n");
        }

        let output = db
            .git_output_bytes_with_input(&["hash-object", "-w", "--stdin-paths"], &input, None)
            .unwrap();

        assert!(output.len() > 4_000_000);
        assert_eq!(
            parse_git_hash_object_oids(&output, 100_000).unwrap().len(),
            100_000
        );
    }

    #[test]
    fn git_hash_batch_output_preserves_order_and_validates_count() {
        let first = "1".repeat(40);
        let second = "a".repeat(40);
        let output = format!("{first}\n{second}\n");
        assert_eq!(
            parse_git_hash_object_oids(output.as_bytes(), 2).unwrap(),
            vec![first, second]
        );
        assert!(matches!(
            parse_git_hash_object_oids(output.as_bytes(), 1),
            Err(Error::Git(message)) if message.contains("2 object IDs for 1 paths")
        ));
    }

    #[test]
    fn git_hash_batch_output_rejects_invalid_or_mixed_oids() {
        assert!(matches!(
            parse_git_hash_object_oids(b"not-an-oid\n", 1),
            Err(Error::Git(message)) if message.contains("invalid object ID")
        ));
        let mixed = format!("{}\n{}\n", "1".repeat(40), "2".repeat(64));
        assert!(matches!(
            parse_git_hash_object_oids(mixed.as_bytes(), 2),
            Err(Error::Git(message)) if message.contains("mixed object ID lengths")
        ));
    }

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
