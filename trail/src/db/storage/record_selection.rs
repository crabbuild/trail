use super::*;

impl Trail {
    #[cfg(test)]
    pub(crate) fn scan_record_selection_files(
        &self,
        selected_paths: &[String],
        selections: &SelectionSet,
        allow_ignored: bool,
    ) -> Result<Vec<DiskFile>> {
        let policy = self.workspace_ignore_policy_snapshot();
        self.scan_record_selection_files_with_policy(
            selected_paths,
            selections,
            allow_ignored,
            &policy,
        )
    }

    pub(crate) fn scan_record_selection_files_with_policy(
        &self,
        selected_paths: &[String],
        selections: &SelectionSet,
        allow_ignored: bool,
        policy: &WorkspaceIgnorePolicySnapshot,
    ) -> Result<Vec<DiskFile>> {
        if allow_ignored
            && let Some(path) = selected_paths.iter().find(|path| is_internal_path(path))
        {
            return Err(Error::IgnoredPath(path.clone()));
        }
        let mut filesystem_metrics = OperationMetricsAccumulator::new(
            self.operation_metrics.as_ref(),
            OperationMetricsDelta::default(),
        );
        let mut selected_directory = false;
        if !allow_ignored {
            for path in selections.as_slice() {
                let abs = safe_join(&self.workspace_root, path)?;
                filesystem_metrics.delta.filesystem_stat_count = filesystem_metrics
                    .delta
                    .filesystem_stat_count
                    .saturating_add(1);
                match fs::symlink_metadata(abs) {
                    Ok(metadata) if metadata.is_dir() => {
                        selected_directory = true;
                        break;
                    }
                    Ok(_) => {}
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(err) => return Err(Error::Io(err)),
                }
            }
        }
        if selected_directory {
            let disk_files =
                self.scan_visible_files_for_paths_with_policy(selections.as_slice(), policy)?;
            return self.selected_record_disk_files_with_selection_set(
                &disk_files,
                selected_paths,
                selections,
                false,
            );
        }

        let mut selected = BTreeMap::new();
        for path in selections.as_slice() {
            if allow_ignored {
                for file in self.read_record_selection_unfiltered(path)? {
                    selected.insert(file.path.clone(), file);
                }
                continue;
            }

            let abs = safe_join(&self.workspace_root, path)?;
            filesystem_metrics.delta.filesystem_stat_count = filesystem_metrics
                .delta
                .filesystem_stat_count
                .saturating_add(1);
            let metadata = match fs::symlink_metadata(&abs) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => return Err(Error::Io(err)),
            };
            if is_internal_path(path)
                || metadata.file_type().is_symlink()
                || policy.check_with_is_dir(path, metadata.is_dir())?.ignored
            {
                return Err(Error::IgnoredPath(path.clone()));
            }
            if metadata.is_file() {
                filesystem_metrics.delta.filesystem_read_count = filesystem_metrics
                    .delta
                    .filesystem_read_count
                    .saturating_add(1);
                let bytes = fs::read(&abs)?;
                filesystem_metrics.delta.filesystem_read_bytes = filesystem_metrics
                    .delta
                    .filesystem_read_bytes
                    .saturating_add(saturating_u64_from_usize(bytes.len()));
                selected.insert(
                    path.clone(),
                    DiskFile {
                        path: path.clone(),
                        bytes,
                        executable: executable_from_metadata(&metadata),
                    },
                );
            }
        }
        Ok(selected.into_values().collect())
    }

    pub(crate) fn selected_record_disk_files_with_selection_set(
        &self,
        disk_files: &[DiskFile],
        explicit_paths: &[String],
        selections: &SelectionSet,
        allow_ignored: bool,
    ) -> Result<Vec<DiskFile>> {
        if allow_ignored
            && let Some(path) = explicit_paths.iter().find(|path| is_internal_path(path))
        {
            return Err(Error::IgnoredPath(path.clone()));
        }
        let mut filesystem_metrics = OperationMetricsAccumulator::new(
            self.operation_metrics.as_ref(),
            OperationMetricsDelta::default(),
        );
        let mut selected = BTreeMap::new();
        let mut selection_comparison_count = 0u64;
        for file in disk_files {
            let (matches, comparisons) = selections.contains_counted(&file.path);
            selection_comparison_count = selection_comparison_count.saturating_add(comparisons);
            if matches {
                selected.insert(file.path.clone(), file.clone());
            }
        }
        self.note_operation_metrics(OperationMetricsDelta {
            selection_comparison_count,
            ..OperationMetricsDelta::default()
        });

        if allow_ignored {
            for path in selections.as_slice() {
                for file in self.read_record_selection_unfiltered(path)? {
                    selected.insert(file.path.clone(), file);
                }
            }
        } else {
            for path in explicit_paths {
                if selected_map_has_selection_match(&selected, path) {
                    continue;
                }
                let abs = safe_join(&self.workspace_root, path)?;
                filesystem_metrics.delta.filesystem_stat_count = filesystem_metrics
                    .delta
                    .filesystem_stat_count
                    .saturating_add(1);
                if abs.exists() {
                    return Err(Error::IgnoredPath(path.clone()));
                }
            }
        }

        Ok(selected.into_values().collect())
    }

    pub(crate) fn read_record_selection_unfiltered(&self, path: &str) -> Result<Vec<DiskFile>> {
        let mut filesystem_metrics = OperationMetricsAccumulator::new(
            self.operation_metrics.as_ref(),
            OperationMetricsDelta::default(),
        );
        if is_internal_path(path) {
            return Err(Error::IgnoredPath(path.to_string()));
        }
        let abs = safe_join(&self.workspace_root, path)?;
        filesystem_metrics.delta.filesystem_stat_count = filesystem_metrics
            .delta
            .filesystem_stat_count
            .saturating_add(1);
        let metadata = match fs::symlink_metadata(&abs) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(Error::Io(err)),
        };
        if metadata.file_type().is_symlink() {
            return Ok(Vec::new());
        }
        if metadata.is_file() {
            filesystem_metrics.delta.filesystem_read_count = filesystem_metrics
                .delta
                .filesystem_read_count
                .saturating_add(1);
            let bytes = fs::read(&abs)?;
            filesystem_metrics.delta.filesystem_read_bytes = filesystem_metrics
                .delta
                .filesystem_read_bytes
                .saturating_add(saturating_u64_from_usize(bytes.len()));
            return Ok(vec![DiskFile {
                path: path.to_string(),
                bytes,
                executable: executable_from_metadata(&metadata),
            }]);
        }
        if !metadata.is_dir() {
            return Ok(Vec::new());
        }
        filesystem_metrics.delta.bounded_filesystem_walk_count = filesystem_metrics
            .delta
            .bounded_filesystem_walk_count
            .saturating_add(1);
        let mut files = Vec::new();
        self.read_record_dir_unfiltered_profiled(&abs, path, &mut files, &mut filesystem_metrics)?;
        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(files)
    }

    fn read_record_dir_unfiltered_profiled(
        &self,
        dir: &Path,
        rel_dir: &str,
        files: &mut Vec<DiskFile>,
        filesystem_metrics: &mut OperationMetricsAccumulator,
    ) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            filesystem_metrics.delta.filesystem_entry_count = filesystem_metrics
                .delta
                .filesystem_entry_count
                .saturating_add(1);
            let name = entry.file_name().to_string_lossy().to_string();
            let rel = format!("{rel_dir}/{name}");
            if is_internal_path(&rel) {
                continue;
            }
            let path = entry.path();
            filesystem_metrics.delta.filesystem_stat_count = filesystem_metrics
                .delta
                .filesystem_stat_count
                .saturating_add(1);
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                self.read_record_dir_unfiltered_profiled(&path, &rel, files, filesystem_metrics)?;
            } else if metadata.is_file() {
                filesystem_metrics.delta.filesystem_read_count = filesystem_metrics
                    .delta
                    .filesystem_read_count
                    .saturating_add(1);
                let bytes = fs::read(&path)?;
                filesystem_metrics.delta.filesystem_read_bytes = filesystem_metrics
                    .delta
                    .filesystem_read_bytes
                    .saturating_add(saturating_u64_from_usize(bytes.len()));
                files.push(DiskFile {
                    path: rel,
                    bytes,
                    executable: executable_from_metadata(&metadata),
                });
            }
        }
        Ok(())
    }
}

fn selected_map_has_selection_match(selected: &BTreeMap<String, DiskFile>, path: &str) -> bool {
    if selected.contains_key(path) {
        return true;
    }
    let lower = format!("{path}/");
    let upper = format!("{path}0");
    selected.range(lower..upper).next().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn assert_selected_record_rejects_symlink_ancestor(
        selected_path: &str,
        outside_path: &str,
        allow_ignored: bool,
    ) {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir_all(outside.path().join("docs")).unwrap();
        fs::write(outside.path().join("secret.txt"), "outside secret\n").unwrap();
        fs::write(
            outside.path().join("docs/secret.txt"),
            "outside directory secret\n",
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::Empty, false).unwrap();
        std::os::unix::fs::symlink(outside.path(), workspace.path().join("link")).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();

        let error = db
            .record_with_options(
                Some("main"),
                Some("reject symlink ancestor".to_string()),
                Actor::human(),
                RecordOptions {
                    paths: vec![selected_path.to_string()],
                    allow_ignored,
                    ..RecordOptions::default()
                },
            )
            .unwrap_err();

        assert!(
            matches!(
                &error,
                Error::InvalidPath { path, reason }
                    if path == selected_path && reason == "path uses a symlink ancestor"
            ),
            "selected {outside_path} through an ancestor symlink returned {error}"
        );
    }

    #[test]
    fn canonical_parent_selection_preserves_explicit_ignored_child_error() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("src")).unwrap();
        fs::write(temp.path().join("src/visible.txt"), "visible\n").unwrap();
        fs::write(temp.path().join("src/ignored.txt"), "ignored\n").unwrap();
        fs::write(temp.path().join(".trailignore"), "src/ignored.txt\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let paths = vec!["src".to_string(), "src/ignored.txt".to_string()];
        let selections = SelectionSet::from_paths(&paths).unwrap();

        let error = db
            .scan_record_selection_files(&paths, &selections, false)
            .unwrap_err();

        assert!(matches!(error, Error::IgnoredPath(path) if path == "src/ignored.txt"));
    }

    #[cfg(unix)]
    #[test]
    fn selected_record_rejects_exact_file_through_symlink_ancestor() {
        assert_selected_record_rejects_symlink_ancestor("link/secret.txt", "exact file", false);
    }

    #[cfg(unix)]
    #[test]
    fn selected_record_rejects_directory_through_symlink_ancestor() {
        assert_selected_record_rejects_symlink_ancestor("link/docs", "selected directory", false);
    }

    #[cfg(unix)]
    #[test]
    fn selected_record_allow_ignored_rejects_symlink_ancestors() {
        assert_selected_record_rejects_symlink_ancestor(
            "link/secret.txt",
            "allow-ignored exact file",
            true,
        );
        assert_selected_record_rejects_symlink_ancestor(
            "link/docs",
            "allow-ignored selected directory",
            true,
        );
    }

    #[cfg(unix)]
    #[test]
    fn selected_record_final_component_symlink_remains_ignored() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "outside secret\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::Empty, false).unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            workspace.path().join("secret-link"),
        )
        .unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();

        let error = db
            .record_with_options(
                Some("main"),
                Some("retain final symlink behavior".to_string()),
                Actor::human(),
                RecordOptions {
                    paths: vec!["secret-link".to_string()],
                    ..RecordOptions::default()
                },
            )
            .unwrap_err();

        assert!(matches!(error, Error::IgnoredPath(path) if path == "secret-link"));
    }
}
