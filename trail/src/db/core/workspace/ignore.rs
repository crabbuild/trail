use super::*;

impl WorkspaceIgnorePolicySnapshot {
    pub(crate) fn check(&self, path: &str) -> Result<IgnoreCheckReport> {
        let path = normalize_relative_path(path)?;
        if is_default_ignored(&path) {
            return Ok(IgnoreCheckReport {
                path,
                ignored: true,
                source: Some("hardcoded".to_string()),
            });
        }

        let abs = safe_join(&self.workspace_root, &path)?;
        if let Some(metrics) = &self.metrics {
            metrics.add(OperationMetricsDelta {
                filesystem_stat_count: 1,
                ..OperationMetricsDelta::default()
            });
        }
        let is_dir = match fs::symlink_metadata(abs) {
            Ok(metadata) => metadata.is_dir(),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
            Err(err) => return Err(Error::Io(err)),
        };
        self.check_normalized_with_is_dir(path, is_dir)
    }

    pub(crate) fn check_with_is_dir(&self, path: &str, is_dir: bool) -> Result<IgnoreCheckReport> {
        let path = normalize_relative_path(path)?;
        if is_default_ignored(&path) {
            return Ok(IgnoreCheckReport {
                path,
                ignored: true,
                source: Some("hardcoded".to_string()),
            });
        }
        self.check_normalized_with_is_dir(path, is_dir)
    }

    fn check_normalized_with_is_dir(
        &self,
        path: String,
        is_dir: bool,
    ) -> Result<IgnoreCheckReport> {
        let matcher = self
            .matcher
            .get_or_init(|| self.compile_matcher())
            .as_ref()
            .map_err(|message| Error::InvalidInput(message.clone()))?;
        let ignored = matcher
            .matched_path_or_any_parents(path_from_rel(&path), is_dir)
            .is_ignore();
        Ok(IgnoreCheckReport {
            path,
            ignored,
            source: ignored.then(|| "workspace".to_string()),
        })
    }

    fn compile_matcher(&self) -> std::result::Result<::ignore::gitignore::Gitignore, String> {
        let mut builder = ::ignore::gitignore::GitignoreBuilder::new(&self.workspace_root);
        let trailignore = self.workspace_root.join(".trailignore");
        let gitignore = self.workspace_root.join(".gitignore");
        // A single metadata probe per dependency preserves `Path::exists`
        // semantics while exposing exactly the files and bytes read by this
        // operation-local matcher.
        let trailignore_metadata = fs::metadata(&trailignore).ok();
        let gitignore_metadata = fs::metadata(&gitignore).ok();
        let trailignore_exists = trailignore_metadata.is_some();
        let gitignore_exists = gitignore_metadata.is_some();
        let dependency_bytes = trailignore_metadata
            .as_ref()
            .map(fs::Metadata::len)
            .unwrap_or(0)
            .saturating_add(
                gitignore_metadata
                    .as_ref()
                    .map(fs::Metadata::len)
                    .unwrap_or(0),
            );
        if let Some(metrics) = &self.metrics {
            metrics.add(OperationMetricsDelta {
                policy_build_count: 1,
                policy_dependency_file_count: u64::from(trailignore_exists)
                    .saturating_add(u64::from(gitignore_exists)),
                policy_dependency_bytes: dependency_bytes,
                filesystem_stat_count: 2,
                ..OperationMetricsDelta::default()
            });
        }
        if trailignore_exists {
            if let Some(err) = builder.add(trailignore) {
                return Err(err.to_string());
            }
        }
        if gitignore_exists {
            if let Some(err) = builder.add(gitignore) {
                return Err(err.to_string());
            }
        }
        builder.build().map_err(|err| err.to_string())
    }
}

impl Trail {
    pub(crate) fn workspace_ignore_policy_snapshot(&self) -> WorkspaceIgnorePolicySnapshot {
        WorkspaceIgnorePolicySnapshot {
            workspace_root: self.workspace_root.clone(),
            metrics: self.operation_metrics.clone(),
            matcher: OnceLock::new(),
        }
    }

    pub fn ignore_list(&self) -> Result<IgnoreListReport> {
        let path = self.workspace_root.join(".trailignore");
        let patterns = read_ignore_patterns(&path)?;
        Ok(IgnoreListReport {
            path: path.to_string_lossy().to_string(),
            patterns,
        })
    }

    pub fn ignore_add(&mut self, pattern: &str) -> Result<IgnoreAddReport> {
        let _lock = self.acquire_write_lock()?;
        let pattern = normalize_ignore_pattern(pattern)?;
        write_default_trailignore(&self.workspace_root)?;
        let path = self.workspace_root.join(".trailignore");
        let mut content = fs::read_to_string(&path).unwrap_or_default();
        let exists = content
            .lines()
            .any(|line| line.trim() == pattern && !line.trim_start().starts_with('#'));
        if !exists {
            if !content.is_empty() && !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str(&pattern);
            content.push('\n');
            fs::write(&path, content)?;
        }
        Ok(IgnoreAddReport {
            path: path.to_string_lossy().to_string(),
            pattern,
            added: !exists,
        })
    }

    pub fn ignore_remove(&mut self, pattern: &str) -> Result<IgnoreRemoveReport> {
        let _lock = self.acquire_write_lock()?;
        let pattern = normalize_ignore_pattern(pattern)?;
        let path = self.workspace_root.join(".trailignore");
        let content = fs::read_to_string(&path).unwrap_or_default();
        let mut removed = false;
        let mut retained = Vec::new();
        for line in content.lines() {
            if line.trim() == pattern && !line.trim_start().starts_with('#') {
                removed = true;
            } else {
                retained.push(line.to_string());
            }
        }
        if removed {
            let mut next = retained.join("\n");
            if !next.is_empty() {
                next.push('\n');
            }
            fs::write(&path, next)?;
        }
        Ok(IgnoreRemoveReport {
            path: path.to_string_lossy().to_string(),
            pattern,
            removed,
        })
    }

    pub fn ignore_check(&self, path: &str) -> Result<IgnoreCheckReport> {
        self.workspace_ignore_policy_snapshot().check(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_scale_files(root: &Path, count: usize) {
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();
        for index in 0..count {
            fs::write(
                src.join(format!("file_{index:04}.txt")),
                format!("baseline {index}\n"),
            )
            .unwrap();
        }
    }

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_git_scale_workspace(file_count: usize) -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        git(temp.path(), &["init"]);
        git(temp.path(), &["config", "user.email", "trail@example.test"]);
        git(temp.path(), &["config", "user.name", "Trail Test"]);
        fs::write(temp.path().join(".trailignore"), "").unwrap();
        write_scale_files(temp.path(), file_count);
        git(temp.path(), &["add", ".trailignore", "src"]);
        git(temp.path(), &["commit", "-m", "baseline"]);
        Trail::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();
        temp
    }

    fn init_git_policy_visibility_workspace(
        policy_path: &str,
        policy_pattern: &str,
    ) -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        git(temp.path(), &["init"]);
        git(temp.path(), &["config", "user.email", "trail@example.test"]);
        git(temp.path(), &["config", "user.name", "Trail Test"]);
        fs::create_dir_all(temp.path().join("nested")).unwrap();
        if policy_path != ".trailignore" {
            fs::write(temp.path().join(".trailignore"), "").unwrap();
        }
        fs::write(temp.path().join(policy_path), policy_pattern).unwrap();
        fs::write(temp.path().join("nested/hidden.txt"), "hidden baseline\n").unwrap();
        git(
            temp.path(),
            &[
                "add",
                "-f",
                ".trailignore",
                policy_path,
                "nested/hidden.txt",
            ],
        );
        git(temp.path(), &["commit", "-m", "policy baseline"]);
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        fs::write(temp.path().join(".git/info/exclude"), ".trail/\n").unwrap();
        temp
    }

    fn assert_selected_locality(report: &OperationMetricsReport) {
        assert_eq!(report.full_filesystem_walk_count, 0);
        assert_eq!(report.full_root_range_count, 0);
        assert_eq!(report.selected_worktree_index_sqlite_full_scan_count, 0);
    }

    fn assert_operation_report(
        report: &OperationMetricsReport,
        operation: &str,
        git_global_work_count: u64,
        policy_build_count: u64,
    ) {
        assert_eq!(report.operation, operation);
        assert_eq!(report.outcome, OperationMetricsOutcome::Success);
        assert_eq!(report.git_global_work_count, git_global_work_count);
        assert_eq!(report.git_subprocess_count, git_global_work_count);
        assert_eq!(report.policy_build_count, policy_build_count);
    }

    fn assert_selected_sqlite_accounting(report: &OperationMetricsReport) {
        assert!(report.selected_worktree_index_sqlite_accounting_complete);
        assert!(report.selected_worktree_index_sqlite_envelope_count > 0);
        assert_eq!(report.selected_worktree_index_sqlite_full_scan_count, 0);
    }

    fn assert_no_daemon_persistence(report: &OperationMetricsReport) {
        assert_eq!(report.daemon_cumulative_rewrite_count, 0);
        assert_eq!(report.daemon_cumulative_rewrite_bytes, 0);
        assert_eq!(report.daemon_cumulative_rewrite_count_total, 0);
        assert_eq!(report.daemon_cumulative_rewrite_bytes_total, 0);
    }

    fn install_daemon_cache(
        db: &mut Trail,
        dirty_paths: &[&str],
        overflow: bool,
        baseline_root_id: Option<ObjectId>,
    ) {
        db.daemon_worktree_cache = Some(DaemonWorktreeCache {
            state: Arc::new(Mutex::new(DaemonWorktreeCacheState {
                dirty_paths: dirty_paths.iter().map(|path| path.to_string()).collect(),
                overflow,
                initialized: true,
                baseline_root_id,
                generation: 1,
                policy_invalidation_index: None,
            })),
            persist: None,
            watcher: None,
        });
    }

    #[test]
    fn workspace_ignore_policy_snapshot_reuses_matcher_and_invalidates_between_operations() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();
        fs::write(temp.path().join(".trailignore"), "old.txt\n").unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let metrics = db.operation_metrics.as_ref().unwrap();

        profile_operation_metrics(Some(metrics), OperationMetricsKind::Status, || {
            let policy = db.workspace_ignore_policy_snapshot();
            assert!(policy.check("old.txt")?.ignored);
            assert!(!policy.check("new.txt")?.ignored);
            fs::write(temp.path().join(".trailignore"), "new.txt\n")?;
            assert!(policy.check("old.txt")?.ignored);
            assert!(!policy.check("new.txt")?.ignored);
            Ok::<(), Error>(())
        })
        .unwrap();
        let first = operation_metrics_report(Some(metrics)).unwrap();
        assert_eq!(first.policy_build_count, 1);

        profile_operation_metrics(Some(metrics), OperationMetricsKind::Status, || {
            let policy = db.workspace_ignore_policy_snapshot();
            assert!(!policy.check("old.txt")?.ignored);
            assert!(policy.check("new.txt")?.ignored);
            Ok::<(), Error>(())
        })
        .unwrap();
        let second = operation_metrics_report(Some(metrics)).unwrap();
        assert_eq!(second.policy_build_count, 1);
    }

    #[test]
    fn workspace_ignore_policy_snapshot_does_not_compile_for_hardcoded_ignore() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let metrics = db.operation_metrics.as_ref().unwrap();

        profile_operation_metrics(Some(metrics), OperationMetricsKind::Status, || {
            let policy = db.workspace_ignore_policy_snapshot();
            assert!(policy.check(".trail/index/trail.sqlite")?.ignored);
            Ok::<(), Error>(())
        })
        .unwrap();
        let report = operation_metrics_report(Some(metrics)).unwrap();
        assert_eq!(report.policy_build_count, 0);
        assert_eq!(report.policy_dependency_file_count, 0);
        assert_eq!(report.filesystem_stat_count, 0);
    }

    #[test]
    fn workspace_ignore_policy_snapshot_caches_exact_error_and_retry_rebuilds() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();
        let trailignore = temp.path().join(".trailignore");
        fs::write(&trailignore, "invalid\\\n").unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let mut expected_builder = ::ignore::gitignore::GitignoreBuilder::new(&db.workspace_root);
        let expected_message = expected_builder
            .add(db.workspace_root.join(".trailignore"))
            .expect("fixture must be rejected by the existing ignore parser")
            .to_string();
        let metrics = db.operation_metrics.as_ref().unwrap();

        let error = profile_operation_metrics(
            Some(metrics),
            OperationMetricsKind::Status,
            || -> Result<()> {
                let policy = db.workspace_ignore_policy_snapshot();
                for _ in 0..2 {
                    match policy.check("candidate.txt").unwrap_err() {
                        Error::InvalidInput(message) => {
                            assert_eq!(message, expected_message);
                        }
                        other => panic!("unexpected policy error: {other}"),
                    }
                }
                Err(Error::InvalidInput(expected_message.clone()))
            },
        )
        .unwrap_err();
        assert!(matches!(error, Error::InvalidInput(message) if message == expected_message));
        let failed = operation_metrics_report(Some(metrics)).unwrap();
        assert_eq!(failed.outcome, OperationMetricsOutcome::Error);
        assert_eq!(failed.policy_build_count, 1);

        fs::write(&trailignore, "candidate.txt\n").unwrap();
        profile_operation_metrics(Some(metrics), OperationMetricsKind::Status, || {
            let policy = db.workspace_ignore_policy_snapshot();
            assert!(policy.check("candidate.txt")?.ignored);
            Ok::<(), Error>(())
        })
        .unwrap();
        let retry = operation_metrics_report(Some(metrics)).unwrap();
        assert_eq!(retry.outcome, OperationMetricsOutcome::Success);
        assert_eq!(retry.policy_build_count, 1);
    }

    #[test]
    fn git_selected_status_1k_reuses_one_policy_snapshot() {
        let temp = init_git_scale_workspace(1_000);
        fs::write(temp.path().join("src/file_0001.txt"), "changed one\n").unwrap();
        fs::write(temp.path().join("src/file_0002.txt"), "changed two\n").unwrap();
        let db = Trail::open(temp.path()).unwrap();

        let status = db.status(None).unwrap();
        assert_eq!(status.changed_paths.len(), 2);
        let report = operation_metrics_report(db.operation_metrics.as_ref()).unwrap();
        assert_selected_locality(&report);
        assert_selected_sqlite_accounting(&report);
        assert_eq!(report.policy_build_count, 1);
        assert_eq!(report.git_subprocess_count, 1);
        assert_eq!(report.git_global_work_count, 1);
    }

    #[test]
    fn explicit_selected_record_1k_reuses_one_policy_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        write_scale_files(temp.path(), 1_000);
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        fs::write(temp.path().join("src/file_0001.txt"), "changed one\n").unwrap();
        fs::write(temp.path().join("src/file_0002.txt"), "changed two\n").unwrap();
        let mut db = Trail::open(temp.path()).unwrap();

        let recorded = db
            .record_with_options(
                Some("main"),
                Some("selected policy reuse".to_string()),
                Actor::human(),
                RecordOptions {
                    paths: vec!["src".to_string(), "src/file_0001.txt".to_string()],
                    ..RecordOptions::default()
                },
            )
            .unwrap();
        assert_eq!(recorded.changed_paths.len(), 2);
        let report = operation_metrics_report(db.operation_metrics.as_ref()).unwrap();
        assert_selected_locality(&report);
        assert_eq!(report.policy_build_count, 2);
        assert_eq!(report.git_global_work_count, 0);
        assert_eq!(report.input_path_count, 2);
        assert_eq!(report.canonical_path_count, 1);
        assert_eq!(report.filesystem_read_count, 1_000);
    }

    #[test]
    fn git_policy_file_change_is_labeled_as_global_fallback() {
        let temp = init_git_scale_workspace(8);
        fs::write(temp.path().join(".trailignore"), "generated/\n").unwrap();
        let db = Trail::open(temp.path()).unwrap();

        let _status = db.status(None).unwrap();
        let report = operation_metrics_report(db.operation_metrics.as_ref()).unwrap();
        // The fallback performs the dirty-policy probe plus one tracked-path
        // inventory so ignored-but-tracked files remain visible.
        assert_eq!(report.git_global_work_count, 2);
        assert_eq!(report.full_filesystem_walk_count, 1);
        assert_eq!(report.policy_build_count, 1);
    }

    #[test]
    fn git_root_and_nested_policy_changes_force_full_fallback_before_filtering() {
        for (policy_path, pattern) in [
            ("nested/.trailignore", "hidden.txt\n"),
            ("nested/.gitignore", "hidden.txt\n"),
            (".trailignore", "nested/hidden.txt\n"),
            (".gitignore", "nested/hidden.txt\n"),
        ] {
            let temp = init_git_policy_visibility_workspace(policy_path, pattern);
            fs::write(temp.path().join(policy_path), "").unwrap();
            let db = Trail::open(temp.path()).unwrap();
            let policy = db.workspace_ignore_policy_snapshot();

            assert!(
                db.scan_git_dirty_tracked_paths_with_policy(&policy)
                    .unwrap()
                    .is_none(),
                "policy candidate {policy_path} must force global fallback"
            );

            let status = db.status(None).unwrap();
            assert!(
                status
                    .changed_paths
                    .iter()
                    .any(|change| change.path == "nested/hidden.txt"),
                "policy candidate {policy_path} produced {:?}",
                status.changed_paths
            );
            let report = operation_metrics_report(db.operation_metrics.as_ref()).unwrap();
            assert_eq!(report.git_global_work_count, 2, "{policy_path}");
            assert_eq!(report.full_filesystem_walk_count, 1, "{policy_path}");
            assert!(
                (3..=4).contains(&report.input_path_count),
                "{policy_path}: {report:?}"
            );
            assert!(
                (1..=4).contains(&report.canonical_path_count),
                "{policy_path}: {report:?}"
            );
            assert_eq!(report.policy_build_count, 1, "{policy_path}");
        }
    }

    #[test]
    fn git_regular_rename_candidates_preserve_both_endpoints() {
        let temp = init_git_scale_workspace(8);
        git(
            temp.path(),
            &["mv", "src/file_0001.txt", "src/renamed_0001.txt"],
        );
        let db = Trail::open(temp.path()).unwrap();
        let policy = db.workspace_ignore_policy_snapshot();

        let paths = db
            .scan_git_dirty_tracked_paths_with_policy(&policy)
            .unwrap()
            .expect("regular rename remains eligible for selected handling");

        assert_eq!(
            paths,
            vec![
                "src/file_0001.txt".to_string(),
                "src/renamed_0001.txt".to_string()
            ]
        );
    }

    #[test]
    fn git_candidate_metrics_keep_exact_endpoints_but_canonicalize_parent_overlap() {
        let temp = tempfile::tempdir().unwrap();
        git(temp.path(), &["init"]);
        git(temp.path(), &["config", "user.email", "trail@example.test"]);
        git(temp.path(), &["config", "user.name", "Trail Test"]);
        fs::write(temp.path().join(".trailignore"), "").unwrap();
        fs::write(temp.path().join("node"), "baseline node\n").unwrap();
        git(temp.path(), &["add", ".trailignore", "node"]);
        git(temp.path(), &["commit", "-m", "overlap baseline"]);
        Trail::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();
        fs::write(temp.path().join(".git/info/exclude"), ".trail/\n").unwrap();
        fs::remove_file(temp.path().join("node")).unwrap();
        fs::create_dir(temp.path().join("node")).unwrap();
        fs::write(temp.path().join("node/child.txt"), "replacement child\n").unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let metrics = db.operation_metrics.as_ref().unwrap();

        let paths = profile_operation_metrics(
            Some(metrics),
            OperationMetricsKind::Status,
            || -> Result<Vec<String>> {
                let policy = db.workspace_ignore_policy_snapshot();
                Ok(db
                    .scan_git_dirty_tracked_paths_with_policy(&policy)?
                    .expect("valid Git overlap candidates"))
            },
        )
        .unwrap();

        assert_eq!(
            paths,
            vec!["node".to_string(), "node/child.txt".to_string()]
        );
        let report = operation_metrics_report(Some(metrics)).unwrap();
        assert_eq!(report.input_path_count, 2);
        assert_eq!(report.canonical_path_count, 1);
    }

    #[test]
    fn git_candidate_metrics_report_raw_input_before_policy_error() {
        let temp = init_git_scale_workspace(1);
        fs::write(temp.path().join(".git/info/exclude"), ".trail/\n").unwrap();
        fs::write(temp.path().join(".trailignore"), "invalid\\\n").unwrap();
        git(temp.path(), &["add", ".trailignore"]);
        git(temp.path(), &["commit", "-m", "invalid policy"]);
        fs::write(temp.path().join("src/file_0000.txt"), "dirty candidate\n").unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let metrics = db.operation_metrics.as_ref().unwrap();

        let result = profile_operation_metrics(
            Some(metrics),
            OperationMetricsKind::Status,
            || -> Result<()> {
                let policy = db.workspace_ignore_policy_snapshot();
                db.scan_git_dirty_tracked_paths_with_policy(&policy)?;
                Ok(())
            },
        );

        assert!(matches!(result, Err(Error::InvalidInput(_))));
        let report = operation_metrics_report(Some(metrics)).unwrap();
        assert_eq!(report.outcome, OperationMetricsOutcome::Error);
        assert_eq!(report.input_path_count, 1);
        assert_eq!(report.canonical_path_count, 0);
    }

    #[test]
    fn selected_directory_keeps_nested_walkbuilder_ignore_semantics() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("src/nested")).unwrap();
        fs::write(temp.path().join("src/keep.txt"), "base\n").unwrap();
        fs::write(temp.path().join("src/nested/.trailignore"), "ignored.log\n").unwrap();
        fs::write(temp.path().join("src/nested/ignored.log"), "ignored base\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        fs::write(temp.path().join("src/keep.txt"), "changed\n").unwrap();
        fs::write(
            temp.path().join("src/nested/ignored.log"),
            "ignored changed\n",
        )
        .unwrap();
        let mut db = Trail::open(temp.path()).unwrap();

        let recorded = db
            .record_with_options(
                Some("main"),
                Some("nested ignore compatibility".to_string()),
                Actor::human(),
                RecordOptions {
                    paths: vec!["src".to_string()],
                    ..RecordOptions::default()
                },
            )
            .unwrap();
        assert_eq!(
            recorded.changed_paths.len(),
            1,
            "unexpected nested-ignore changes: {:?}",
            recorded.changed_paths
        );
        assert_eq!(recorded.changed_paths[0].path, "src/keep.txt");
        let report = operation_metrics_report(db.operation_metrics.as_ref()).unwrap();
        assert_operation_report(&report, "record", 0, 2);
        assert_eq!(report.bounded_filesystem_walk_count, 1);
    }

    #[test]
    fn daemon_selected_status_diff_and_record_1k_structural_matrix() {
        let temp = tempfile::tempdir().unwrap();
        write_scale_files(temp.path(), 996);
        fs::create_dir_all(temp.path().join("overlap/nested")).unwrap();
        fs::write(temp.path().join("overlap/nested/a.txt"), "overlap base\n").unwrap();
        fs::create_dir_all(temp.path().join("delete/sub")).unwrap();
        fs::write(temp.path().join("delete/sub/b.txt"), "delete base\n").unwrap();
        fs::write(temp.path().join("clean_a.txt"), "clean a\n").unwrap();
        fs::write(temp.path().join("clean_b.txt"), "clean b\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let head = db.resolve_branch_ref("main").unwrap();

        install_daemon_cache(&mut db, &[], false, Some(head.root_id.clone()));
        assert!(db.status(None).unwrap().changed_paths.is_empty());
        let clean_status = operation_metrics_report(db.operation_metrics.as_ref()).unwrap();
        assert_selected_locality(&clean_status);
        assert_operation_report(&clean_status, "status", 0, 0);
        assert_no_daemon_persistence(&clean_status);

        assert!(db.diff_dirty(false, false).unwrap().files.is_empty());
        let clean_diff = operation_metrics_report(db.operation_metrics.as_ref()).unwrap();
        assert_selected_locality(&clean_diff);
        assert_operation_report(&clean_diff, "diff", 0, 0);
        assert_no_daemon_persistence(&clean_diff);

        assert!(db
            .record(
                Some("main"),
                Some("daemon clean no-op".to_string()),
                Actor::human(),
                false,
            )
            .unwrap()
            .operation
            .is_none());
        let clean_record = operation_metrics_report(db.operation_metrics.as_ref()).unwrap();
        assert_selected_locality(&clean_record);
        assert_operation_report(&clean_record, "record", 0, 0);
        assert_no_daemon_persistence(&clean_record);

        fs::write(
            temp.path().join("overlap/nested/a.txt"),
            "overlap changed\n",
        )
        .unwrap();
        install_daemon_cache(&mut db, &["overlap", "overlap/nested/a.txt"], false, None);
        let dirty_status = db.status(None).unwrap();
        assert_eq!(dirty_status.changed_paths.len(), 1);
        let status_metrics = operation_metrics_report(db.operation_metrics.as_ref()).unwrap();
        assert_selected_locality(&status_metrics);
        assert_selected_sqlite_accounting(&status_metrics);
        assert_operation_report(&status_metrics, "status", 0, 2);
        assert_eq!(status_metrics.bounded_filesystem_walk_count, 1);
        assert_eq!(status_metrics.bounded_root_range_count, 1);
        assert_eq!(status_metrics.filesystem_read_count, 1);
        assert_eq!(status_metrics.input_path_count, 2);
        assert_eq!(status_metrics.canonical_path_count, 1);
        assert_no_daemon_persistence(&status_metrics);

        fs::write(temp.path().join("overlap/nested/a.txt"), "overlap base\n").unwrap();
        fs::remove_dir_all(temp.path().join("delete")).unwrap();
        install_daemon_cache(&mut db, &["delete", "delete/sub"], false, None);
        let deleted = db.diff_dirty(false, false).unwrap();
        assert_eq!(deleted.files.len(), 1);
        assert_eq!(deleted.files[0].path, "delete/sub/b.txt");
        assert_eq!(deleted.files[0].kind, FileChangeKind::Deleted);
        let diff_metrics = operation_metrics_report(db.operation_metrics.as_ref()).unwrap();
        assert_selected_locality(&diff_metrics);
        assert_selected_sqlite_accounting(&diff_metrics);
        assert_operation_report(&diff_metrics, "diff", 0, 1);
        assert_eq!(diff_metrics.bounded_root_range_count, 1);
        assert_no_daemon_persistence(&diff_metrics);

        fs::write(temp.path().join("src/file_0001.txt"), "daemon record\n").unwrap();
        install_daemon_cache(&mut db, &["src/file_0001.txt"], false, None);
        let recorded = db
            .record(
                Some("main"),
                Some("daemon selected record".to_string()),
                Actor::human(),
                false,
            )
            .unwrap();
        assert!(recorded.operation.is_some());
        assert_eq!(recorded.changed_paths.len(), 1);
        let record_metrics = operation_metrics_report(db.operation_metrics.as_ref()).unwrap();
        assert_selected_locality(&record_metrics);
        assert_selected_sqlite_accounting(&record_metrics);
        assert_operation_report(&record_metrics, "record", 0, 1);
        assert_no_daemon_persistence(&record_metrics);

        fs::write(temp.path().join(".trailignore"), "invalid\\\n").unwrap();
        fs::write(temp.path().join("src/file_0002.txt"), "daemon retry\n").unwrap();
        install_daemon_cache(&mut db, &["src/file_0002.txt"], false, None);
        assert!(matches!(db.status(None), Err(Error::InvalidInput(_))));
        let failed = operation_metrics_report(db.operation_metrics.as_ref()).unwrap();
        assert_eq!(failed.operation, "status");
        assert_eq!(failed.outcome, OperationMetricsOutcome::Error);
        assert_eq!(failed.policy_build_count, 1);
        assert_selected_locality(&failed);

        fs::write(temp.path().join(".trailignore"), "").unwrap();
        install_daemon_cache(&mut db, &["src/file_0002.txt"], false, None);
        assert_eq!(db.status(None).unwrap().changed_paths.len(), 1);
        let retry = operation_metrics_report(db.operation_metrics.as_ref()).unwrap();
        assert_selected_locality(&retry);
        assert_selected_sqlite_accounting(&retry);
        assert_operation_report(&retry, "status", 0, 1);

        fs::write(temp.path().join(".trailignore"), "generated/\n").unwrap();
        install_daemon_cache(&mut db, &[], true, None);
        let _fallback = db.status(None).unwrap();
        let fallback = operation_metrics_report(db.operation_metrics.as_ref()).unwrap();
        assert_eq!(fallback.operation, "status");
        assert_eq!(fallback.outcome, OperationMetricsOutcome::Success);
        assert_eq!(fallback.full_filesystem_walk_count, 1);
        assert_eq!(fallback.git_global_work_count, 2);
        assert_eq!(fallback.policy_build_count, 1);
    }

    #[test]
    fn git_selected_diff_record_and_noop_1k_structural_matrix() {
        let temp = init_git_scale_workspace(1_000);
        let mut db = Trail::open(temp.path()).unwrap();

        assert!(db.status(None).unwrap().changed_paths.is_empty());
        let clean = operation_metrics_report(db.operation_metrics.as_ref()).unwrap();
        assert_selected_locality(&clean);
        assert_operation_report(&clean, "status", 1, 0);

        git(
            temp.path(),
            &["mv", "src/file_0001.txt", "src/renamed_0001.txt"],
        );
        let renamed = db.diff_dirty(false, false).unwrap();
        assert_eq!(renamed.files.len(), 1);
        assert_eq!(renamed.files[0].path, "src/renamed_0001.txt");
        assert_eq!(
            renamed.files[0].old_path.as_deref(),
            Some("src/file_0001.txt")
        );
        assert_eq!(renamed.files[0].kind, FileChangeKind::Renamed);
        let diff = operation_metrics_report(db.operation_metrics.as_ref()).unwrap();
        assert_selected_locality(&diff);
        assert_selected_sqlite_accounting(&diff);
        assert_operation_report(&diff, "diff", 1, 1);
        assert_eq!(diff.root_point_key_count, 2);

        git(temp.path(), &["reset", "--hard", "HEAD"]);
        fs::write(temp.path().join("src/file_0002.txt"), "git record\n").unwrap();
        let recorded = db
            .record(
                Some("main"),
                Some("Git selected record".to_string()),
                Actor::human(),
                false,
            )
            .unwrap();
        assert!(recorded.operation.is_some());
        assert_eq!(recorded.changed_paths.len(), 1);
        let record = operation_metrics_report(db.operation_metrics.as_ref()).unwrap();
        assert_selected_locality(&record);
        assert_selected_sqlite_accounting(&record);
        assert_operation_report(&record, "record", 1, 1);

        let noop = db
            .record(
                Some("main"),
                Some("Git candidate no-op".to_string()),
                Actor::human(),
                false,
            )
            .unwrap();
        assert!(noop.operation.is_none());
        let noop_metrics = operation_metrics_report(db.operation_metrics.as_ref()).unwrap();
        assert_selected_locality(&noop_metrics);
        assert_selected_sqlite_accounting(&noop_metrics);
        assert_operation_report(&noop_metrics, "record", 1, 1);
    }
}
