use super::*;

impl Trail {
    pub(crate) fn scan_git_dirty_tracked_paths(&self) -> Result<Option<Vec<String>>> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.workspace_root)
            .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
            .output()
            .map_err(|err| Error::Git(err.to_string()))?;
        self.note_operation_metrics(OperationMetricsDelta {
            git_subprocess_count: 1,
            git_global_work_count: 1,
            git_output_bytes: saturating_u64_from_usize(
                output.stdout.len().saturating_add(output.stderr.len()),
            ),
            ..OperationMetricsDelta::default()
        });
        if !output.status.success() {
            return Ok(None);
        }

        let mut paths = BTreeSet::new();
        let records = output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
            .collect::<Vec<_>>();
        self.note_operation_metrics(OperationMetricsDelta {
            git_output_record_count: saturating_u64_from_usize(records.len()),
            ..OperationMetricsDelta::default()
        });
        let mut idx = 0;
        while idx < records.len() {
            let record = records[idx];
            if record.len() < 4 {
                return Ok(None);
            }
            let status = &record[..2];
            let path = normalize_relative_path(&String::from_utf8_lossy(&record[3..]))?;
            if path == ".trailignore" || path == ".gitignore" {
                return Ok(None);
            }
            if !self.ignore_check(&path)?.ignored {
                paths.insert(path);
            }
            if status == b"??" {
                idx += 1;
                continue;
            }
            if status.contains(&b'R') || status.contains(&b'C') {
                idx += 1;
                let Some(old_record) = records.get(idx) else {
                    return Ok(None);
                };
                let old_path = normalize_relative_path(&String::from_utf8_lossy(old_record))?;
                if old_path == ".trailignore" || old_path == ".gitignore" {
                    return Ok(None);
                }
                if !self.ignore_check(&old_path)?.ignored {
                    paths.insert(old_path);
                }
            }
            idx += 1;
        }
        self.note_operation_metrics(OperationMetricsDelta {
            input_path_count: saturating_u64_from_usize(records.len()),
            canonical_path_count: saturating_u64_from_usize(paths.len()),
            ..OperationMetricsDelta::default()
        });
        Ok(Some(paths.into_iter().collect()))
    }

    pub(crate) fn scan_visible_files_for_paths(&self, paths: &[String]) -> Result<Vec<DiskFile>> {
        let root = self.workspace_root.canonicalize()?;
        let mut files = BTreeMap::new();
        let mut filesystem_metrics = OperationMetricsAccumulator::new(
            self.operation_metrics.as_ref(),
            OperationMetricsDelta::default(),
        );
        for path in paths {
            if self.ignore_check(path)?.ignored {
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
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_file() {
                let (bytes, metadata) =
                    read_selected_regular_file(abs, self.operation_metrics.as_ref())?;
                files.insert(
                    path.clone(),
                    DiskFile {
                        path: path.clone(),
                        bytes,
                        executable: executable_from_metadata(&metadata),
                    },
                );
            } else if metadata.is_dir() {
                scan_files_under_selection(
                    &root,
                    &abs,
                    self.config.recording.ignore_gitignored,
                    self.operation_metrics.as_ref(),
                    &mut files,
                )?;
            }
        }
        filesystem_metrics.delta.expanded_path_count = saturating_u64_from_usize(files.len());
        Ok(files.into_values().collect())
    }

    pub(crate) fn scan_git_tracked_paths_impl(
        &self,
        required: bool,
    ) -> Result<Option<Vec<String>>> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.workspace_root)
            .arg("ls-files")
            .arg("-z")
            .output()
            .map_err(|err| Error::Git(err.to_string()))?;
        self.note_operation_metrics(OperationMetricsDelta {
            git_subprocess_count: 1,
            git_global_work_count: 1,
            git_output_bytes: saturating_u64_from_usize(
                output.stdout.len().saturating_add(output.stderr.len()),
            ),
            ..OperationMetricsDelta::default()
        });
        if !output.status.success() {
            if required {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(Error::Git(format!(
                    "git ls-files failed in {}: {}",
                    self.workspace_root.display(),
                    stderr.trim()
                )));
            }
            return Ok(None);
        }
        let output_record_count = output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|raw| !raw.is_empty())
            .count();
        self.note_operation_metrics(OperationMetricsDelta {
            git_output_record_count: saturating_u64_from_usize(output_record_count),
            ..OperationMetricsDelta::default()
        });
        let mut paths = Vec::new();
        for raw in output.stdout.split(|byte| *byte == 0) {
            if raw.is_empty() {
                continue;
            }
            let path = String::from_utf8_lossy(raw).to_string();
            let path = normalize_relative_path(&path)?;
            if is_default_ignored(&path) {
                continue;
            }
            paths.push(path);
        }
        paths.sort();
        self.note_operation_metrics(OperationMetricsDelta {
            input_path_count: saturating_u64_from_usize(output_record_count),
            canonical_path_count: saturating_u64_from_usize(paths.len()),
            ..OperationMetricsDelta::default()
        });
        Ok(Some(paths))
    }

    pub(crate) fn scan_worktree_file_paths(&self) -> Result<WorktreePathScan> {
        self.scan_file_paths_under(&self.workspace_root)
    }

    pub(crate) fn scan_files_under_for_paths(
        &self,
        root: &Path,
        paths: &[String],
    ) -> Result<Vec<DiskFile>> {
        let root = root.canonicalize()?;
        let mut files = BTreeMap::new();
        let mut filesystem_metrics = OperationMetricsAccumulator::new(
            self.operation_metrics.as_ref(),
            OperationMetricsDelta::default(),
        );
        for path in paths {
            let path = normalize_relative_path(path)?;
            if is_default_ignored(&path) {
                continue;
            }
            let abs = safe_join(&root, &path)?;
            filesystem_metrics.delta.filesystem_stat_count = filesystem_metrics
                .delta
                .filesystem_stat_count
                .saturating_add(1);
            let metadata = match fs::symlink_metadata(&abs) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => return Err(Error::Io(err)),
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_file() {
                let (bytes, metadata) =
                    read_selected_regular_file(abs, self.operation_metrics.as_ref())?;
                files.insert(
                    path.clone(),
                    DiskFile {
                        path,
                        bytes,
                        executable: executable_from_metadata(&metadata),
                    },
                );
            } else if metadata.is_dir() {
                scan_files_under_selection(
                    &root,
                    &abs,
                    self.config.recording.ignore_gitignored,
                    self.operation_metrics.as_ref(),
                    &mut files,
                )?;
            }
        }
        filesystem_metrics.delta.expanded_path_count = saturating_u64_from_usize(files.len());
        Ok(files.into_values().collect())
    }

    pub(crate) fn scan_files_under(&self, root: &Path) -> Result<Vec<DiskFile>> {
        let mut filesystem_metrics = OperationMetricsAccumulator::new(
            self.operation_metrics.as_ref(),
            OperationMetricsDelta {
                full_filesystem_walk_count: 1,
                ..OperationMetricsDelta::default()
            },
        );
        let root = root.canonicalize()?;
        let mut builder = WalkBuilder::new(&root);
        builder
            .hidden(false)
            .git_ignore(self.config.recording.ignore_gitignored)
            .git_exclude(self.config.recording.ignore_gitignored)
            .git_global(self.config.recording.ignore_gitignored)
            .add_custom_ignore_filename(".trailignore");
        let walker = builder.build();
        let mut files = Vec::new();
        for item in walker {
            let entry = item.map_err(|err| Error::InvalidInput(err.to_string()))?;
            let path = entry.path();
            if path == root {
                continue;
            }
            filesystem_metrics.delta.filesystem_entry_count = filesystem_metrics
                .delta
                .filesystem_entry_count
                .saturating_add(1);
            let rel = path
                .strip_prefix(&root)
                .map_err(|err| Error::InvalidInput(err.to_string()))?;
            let rel = normalize_relative_path(&rel.to_string_lossy())?;
            if entry.file_type().is_some_and(|kind| kind.is_dir()) && is_default_ignored(&rel) {
                continue;
            }
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            if is_default_ignored(&rel) {
                continue;
            }
            let (bytes, metadata) =
                read_selected_regular_file(path.to_path_buf(), self.operation_metrics.as_ref())?;
            files.push(DiskFile {
                path: rel,
                bytes,
                executable: executable_from_metadata(&metadata),
            });
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        filesystem_metrics.delta.expanded_path_count = saturating_u64_from_usize(files.len());
        Ok(files)
    }

    fn scan_file_paths_under(&self, root: &Path) -> Result<WorktreePathScan> {
        let mut filesystem_metrics = OperationMetricsAccumulator::new(
            self.operation_metrics.as_ref(),
            OperationMetricsDelta {
                full_filesystem_walk_count: 1,
                ..OperationMetricsDelta::default()
            },
        );
        let root = root.canonicalize()?;
        let mut builder = WalkBuilder::new(&root);
        builder
            .hidden(false)
            .git_ignore(self.config.recording.ignore_gitignored)
            .git_exclude(self.config.recording.ignore_gitignored)
            .git_global(self.config.recording.ignore_gitignored)
            .add_custom_ignore_filename(".trailignore");
        let walker = builder.build();
        let mut paths = Vec::new();
        let mut total_bytes = 0u64;
        for item in walker {
            let entry = item.map_err(|err| Error::InvalidInput(err.to_string()))?;
            let path = entry.path();
            if path == root {
                continue;
            }
            filesystem_metrics.delta.filesystem_entry_count = filesystem_metrics
                .delta
                .filesystem_entry_count
                .saturating_add(1);
            let rel = path
                .strip_prefix(&root)
                .map_err(|err| Error::InvalidInput(err.to_string()))?;
            let rel = normalize_relative_path(&rel.to_string_lossy())?;
            if entry.file_type().is_some_and(|kind| kind.is_dir()) && is_default_ignored(&rel) {
                continue;
            }
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            if is_default_ignored(&rel) {
                continue;
            }
            filesystem_metrics.delta.filesystem_stat_count = filesystem_metrics
                .delta
                .filesystem_stat_count
                .saturating_add(1);
            let metadata = match fs::symlink_metadata(path) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => return Err(Error::Io(err)),
            };
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                continue;
            }
            total_bytes = total_bytes.saturating_add(metadata.len());
            paths.push(rel);
        }
        paths.sort();
        filesystem_metrics.delta.expanded_path_count = saturating_u64_from_usize(paths.len());
        Ok(WorktreePathScan { paths, total_bytes })
    }
}

fn scan_files_under_selection(
    root: &Path,
    selected_root: &Path,
    use_git_ignores: bool,
    metrics: Option<&Arc<OperationMetricsState>>,
    files: &mut BTreeMap<String, DiskFile>,
) -> Result<()> {
    let mut filesystem_metrics = OperationMetricsAccumulator::new(
        metrics,
        OperationMetricsDelta {
            bounded_filesystem_walk_count: 1,
            ..OperationMetricsDelta::default()
        },
    );
    let mut builder = WalkBuilder::new(selected_root);
    builder
        .hidden(false)
        .git_ignore(use_git_ignores)
        .git_exclude(use_git_ignores)
        .git_global(use_git_ignores)
        .add_custom_ignore_filename(".trailignore");
    let walker = builder.build();
    for item in walker {
        let entry = item.map_err(|err| Error::InvalidInput(err.to_string()))?;
        let path = entry.path();
        if path == selected_root {
            continue;
        }
        filesystem_metrics.delta.filesystem_entry_count = filesystem_metrics
            .delta
            .filesystem_entry_count
            .saturating_add(1);
        let rel = path
            .strip_prefix(root)
            .map_err(|err| Error::InvalidInput(err.to_string()))?;
        let rel = normalize_relative_path(&rel.to_string_lossy())?;
        if entry.file_type().is_some_and(|kind| kind.is_dir()) && is_default_ignored(&rel) {
            continue;
        }
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        if is_default_ignored(&rel) {
            continue;
        }
        let (bytes, metadata) = read_selected_regular_file(path.to_path_buf(), metrics)?;
        files.insert(
            rel.clone(),
            DiskFile {
                path: rel,
                bytes,
                executable: executable_from_metadata(&metadata),
            },
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ObservedPathKind {
    RegularFile,
    Directory,
    Symlink,
    Other,
}

pub(crate) fn observed_exact_paths_for_candidates(
    root: &Path,
    candidate_paths: &[String],
    case_insensitive: bool,
) -> Result<BTreeMap<String, ObservedPathKind>> {
    observed_exact_paths_for_candidates_impl(root, candidate_paths, case_insensitive)
        .map(|(observed, _)| observed)
}

#[cfg(test)]
pub(crate) fn observed_exact_paths_for_candidates_with_scan_count(
    root: &Path,
    candidate_paths: &[String],
    case_insensitive: bool,
) -> Result<(BTreeMap<String, ObservedPathKind>, usize)> {
    observed_exact_paths_for_candidates_impl(root, candidate_paths, case_insensitive)
}

#[derive(Clone)]
struct CachedDirectoryEntry {
    name: String,
    path: PathBuf,
    file_type: fs::FileType,
}

#[derive(Default)]
struct CachedDirectory {
    exact: BTreeMap<String, Vec<CachedDirectoryEntry>>,
    folded: BTreeMap<String, Vec<CachedDirectoryEntry>>,
}

fn cached_directory<'a>(
    cache: &'a mut BTreeMap<PathBuf, CachedDirectory>,
    dir: &Path,
    directory_scans: &mut usize,
) -> Result<&'a CachedDirectory> {
    if !cache.contains_key(dir) {
        *directory_scans += 1;
        let mut cached = CachedDirectory::default();
        match fs::read_dir(dir) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry?;
                    let cached_entry = CachedDirectoryEntry {
                        name: entry.file_name().to_string_lossy().to_string(),
                        path: entry.path(),
                        file_type: entry.file_type()?,
                    };
                    cached
                        .exact
                        .entry(cached_entry.name.clone())
                        .or_default()
                        .push(cached_entry.clone());
                    cached
                        .folded
                        .entry(case_insensitive_path_key(&cached_entry.name))
                        .or_default()
                        .push(cached_entry);
                }
                for entries in cached.exact.values_mut() {
                    entries.sort_by(|left, right| left.name.cmp(&right.name));
                }
                for entries in cached.folded.values_mut() {
                    entries.sort_by(|left, right| left.name.cmp(&right.name));
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(Error::Io(err)),
        }
        cache.insert(dir.to_path_buf(), cached);
    }
    Ok(cache.get(dir).expect("cached directory exists"))
}

fn observed_exact_paths_for_candidates_impl(
    root: &Path,
    candidate_paths: &[String],
    case_insensitive: bool,
) -> Result<(BTreeMap<String, ObservedPathKind>, usize)> {
    let root = root.canonicalize()?;
    let mut observed = BTreeMap::new();
    let mut directory_cache = BTreeMap::new();
    let mut directory_scans = 0;
    for candidate in candidate_paths {
        let candidate = normalize_relative_path(candidate)?;
        let components = path_from_rel(&candidate)
            .components()
            .map(|component| component.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        let mut parents = vec![(root.clone(), Vec::<String>::new())];
        for (index, expected) in components.iter().enumerate() {
            let is_final = index + 1 == components.len();
            let mut next_parents = Vec::new();
            for (dir, actual_components) in parents {
                let directory = cached_directory(&mut directory_cache, &dir, &mut directory_scans)?;
                let matches = if case_insensitive {
                    directory.folded.get(&case_insensitive_path_key(expected))
                } else {
                    directory.exact.get(expected)
                }
                .into_iter()
                .flatten()
                .map(|entry| (entry.name.clone(), entry.path.clone(), entry.file_type))
                .collect::<Vec<_>>();
                if is_final {
                    for (name, _, file_type) in matches {
                        let mut actual = actual_components.clone();
                        actual.push(name);
                        let path = normalize_relative_path(&actual.join("/"))?;
                        let kind = if file_type.is_file() {
                            ObservedPathKind::RegularFile
                        } else if file_type.is_dir() {
                            ObservedPathKind::Directory
                        } else if file_type.is_symlink() {
                            ObservedPathKind::Symlink
                        } else {
                            ObservedPathKind::Other
                        };
                        observed.insert(path, kind);
                    }
                    continue;
                }

                for (name, next, file_type) in matches {
                    if file_type.is_symlink() || !file_type.is_dir() {
                        return Err(Error::InvalidPath {
                            path: candidate.clone(),
                            reason: format!("parent component `{name}` is not a safe directory"),
                        });
                    }
                    let canonical = next.canonicalize()?;
                    if !canonical.starts_with(&root) {
                        return Err(Error::InvalidPath {
                            path: candidate.clone(),
                            reason: "parent directory escapes the worktree".to_string(),
                        });
                    }
                    let mut actual = actual_components.clone();
                    actual.push(name);
                    next_parents.push((canonical, actual));
                }
            }
            if is_final {
                break;
            }
            parents = next_parents;
            if parents.is_empty() {
                break;
            }
        }
    }
    Ok((observed, directory_scans))
}

fn read_selected_regular_file(
    path: PathBuf,
    metrics: Option<&Arc<OperationMetricsState>>,
) -> Result<(Vec<u8>, fs::Metadata)> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    #[cfg(not(unix))]
    {
        if let Some(metrics) = metrics {
            metrics.add(OperationMetricsDelta {
                filesystem_stat_count: 1,
                ..OperationMetricsDelta::default()
            });
        }
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(Error::InvalidInput(format!(
                "selected path `{}` changed file type during scan",
                path.display()
            )));
        }
    }
    let mut file = options.open(&path)?;
    if let Some(metrics) = metrics {
        metrics.add(OperationMetricsDelta {
            filesystem_stat_count: 1,
            ..OperationMetricsDelta::default()
        });
    }
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(Error::InvalidInput(format!(
            "selected path `{}` changed file type during scan",
            path.display()
        )));
    }
    let mut bytes = Vec::new();
    if let Some(metrics) = metrics {
        metrics.add(OperationMetricsDelta {
            filesystem_read_count: 1,
            ..OperationMetricsDelta::default()
        });
    }
    let read_result = file.read_to_end(&mut bytes);
    if let Some(metrics) = metrics {
        metrics.add(OperationMetricsDelta {
            filesystem_read_bytes: saturating_u64_from_usize(bytes.len()),
            ..OperationMetricsDelta::default()
        });
    }
    read_result?;
    #[cfg(not(unix))]
    {
        if let Some(metrics) = metrics {
            metrics.add(OperationMetricsDelta {
                filesystem_stat_count: 1,
                ..OperationMetricsDelta::default()
            });
        }
        let final_metadata = fs::symlink_metadata(&path)?;
        if final_metadata.file_type().is_symlink() || !final_metadata.is_file() {
            return Err(Error::InvalidInput(format!(
                "selected path `{}` changed file type during scan",
                path.display()
            )));
        }
    }
    Ok((bytes, metadata))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn selected_regular_file_read_rejects_symlink_swap() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("target.txt"), "target\n").unwrap();
        std::os::unix::fs::symlink("target.txt", temp.path().join("selected.txt")).unwrap();

        assert!(read_selected_regular_file(temp.path().join("selected.txt"), None).is_err());
    }

    #[test]
    fn exact_path_observation_enumerates_one_shared_parent_once() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("present.txt"), "present\n").unwrap();
        let candidates = (0..10_000)
            .map(|index| format!("candidate-{index}.txt"))
            .chain(std::iter::once("present.txt".to_string()))
            .collect::<Vec<_>>();

        let (observed, directory_scans) =
            observed_exact_paths_for_candidates_with_scan_count(temp.path(), &candidates, false)
                .unwrap();

        assert_eq!(directory_scans, 1);
        assert_eq!(
            observed.get("present.txt"),
            Some(&ObservedPathKind::RegularFile)
        );
    }

    #[test]
    fn exact_path_observation_preserves_nested_parent_and_file_spelling() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("Src")).unwrap();
        fs::write(temp.path().join("Src/readme.md"), "present\n").unwrap();

        let observed =
            observed_exact_paths_for_candidates(temp.path(), &["src/README.md".to_string()], true)
                .unwrap();

        assert_eq!(
            observed.get("Src/readme.md"),
            Some(&ObservedPathKind::RegularFile)
        );
        assert_eq!(observed.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn exact_path_observation_rejects_symlinked_parent() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("file.txt"), "outside\n").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("linked")).unwrap();

        let err = observed_exact_paths_for_candidates(
            root.path(),
            &["linked/file.txt".to_string()],
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("safe directory"));
    }
}
