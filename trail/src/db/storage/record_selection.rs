use super::*;

impl Trail {
    pub(crate) fn scan_record_selection_files(
        &self,
        selected_paths: &[String],
        allow_ignored: bool,
    ) -> Result<Vec<DiskFile>> {
        let mut filesystem_metrics = OperationMetricsAccumulator::new(
            self.operation_metrics.as_ref(),
            OperationMetricsDelta::default(),
        );
        let mut selected_directory = false;
        if !allow_ignored {
            for path in selected_paths {
                filesystem_metrics.delta.filesystem_stat_count = filesystem_metrics
                    .delta
                    .filesystem_stat_count
                    .saturating_add(1);
                if self.workspace_root.join(path_from_rel(path)).is_dir() {
                    selected_directory = true;
                    break;
                }
            }
        }
        if selected_directory {
            let disk_files = self.scan_visible_files_for_paths(selected_paths)?;
            return self.selected_record_disk_files(&disk_files, selected_paths, false);
        }

        let mut selected = BTreeMap::new();
        for path in selected_paths {
            if allow_ignored {
                for file in self.read_record_selection_unfiltered(path)? {
                    selected.insert(file.path.clone(), file);
                }
                continue;
            }

            let abs = self.workspace_root.join(path_from_rel(path));
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
                || self.ignore_check(path)?.ignored
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

    pub(crate) fn selected_record_disk_files(
        &self,
        disk_files: &[DiskFile],
        selected_paths: &[String],
        allow_ignored: bool,
    ) -> Result<Vec<DiskFile>> {
        let mut filesystem_metrics = OperationMetricsAccumulator::new(
            self.operation_metrics.as_ref(),
            OperationMetricsDelta::default(),
        );
        let mut selected = BTreeMap::new();
        for file in disk_files {
            if selected_paths
                .iter()
                .any(|path| path_matches_selection(&file.path, path))
            {
                selected.insert(file.path.clone(), file.clone());
            }
        }

        for path in selected_paths {
            let had_visible_match = selected
                .keys()
                .any(|candidate| path_matches_selection(candidate, path));
            if allow_ignored {
                for file in self.read_record_selection_unfiltered(path)? {
                    selected.insert(file.path.clone(), file);
                }
            } else if !had_visible_match {
                let abs = self.workspace_root.join(path_from_rel(path));
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
        let abs = self.workspace_root.join(path_from_rel(path));
        filesystem_metrics.delta.filesystem_stat_count = filesystem_metrics
            .delta
            .filesystem_stat_count
            .saturating_add(1);
        if !abs.exists() {
            return Ok(Vec::new());
        }
        filesystem_metrics.delta.filesystem_stat_count = filesystem_metrics
            .delta
            .filesystem_stat_count
            .saturating_add(1);
        let metadata = fs::symlink_metadata(&abs)?;
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
