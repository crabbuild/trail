use super::*;

impl Trail {
    pub(crate) fn scan_git_dirty_tracked_paths(&self) -> Result<Option<Vec<String>>> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.workspace_root)
            .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
            .output()
            .map_err(|err| Error::Git(err.to_string()))?;
        if !output.status.success() {
            return Ok(None);
        }

        let mut paths = BTreeSet::new();
        let records = output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
            .collect::<Vec<_>>();
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
        Ok(Some(paths.into_iter().collect()))
    }

    pub(crate) fn scan_visible_files_for_paths(&self, paths: &[String]) -> Result<Vec<DiskFile>> {
        let root = self.workspace_root.canonicalize()?;
        let mut files = BTreeMap::new();
        for path in paths {
            if self.ignore_check(path)?.ignored {
                continue;
            }
            let abs = self.workspace_root.join(path_from_rel(path));
            let metadata = match fs::symlink_metadata(&abs) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => return Err(Error::Io(err)),
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_file() {
                files.insert(
                    path.clone(),
                    DiskFile {
                        path: path.clone(),
                        bytes: fs::read(&abs)?,
                        executable: executable_from_metadata(&metadata),
                    },
                );
            } else if metadata.is_dir() {
                scan_files_under_selection(
                    &root,
                    &abs,
                    self.config.recording.ignore_gitignored,
                    &mut files,
                )?;
            }
        }
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
        for path in paths {
            let path = normalize_relative_path(path)?;
            if is_default_ignored(&path) {
                continue;
            }
            let abs = safe_join(&root, &path)?;
            let metadata = match fs::symlink_metadata(&abs) {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => return Err(Error::Io(err)),
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_file() {
                files.insert(
                    path.clone(),
                    DiskFile {
                        path,
                        bytes: fs::read(&abs)?,
                        executable: executable_from_metadata(&metadata),
                    },
                );
            } else if metadata.is_dir() {
                scan_files_under_selection(
                    &root,
                    &abs,
                    self.config.recording.ignore_gitignored,
                    &mut files,
                )?;
            }
        }
        Ok(files.into_values().collect())
    }

    pub(crate) fn scan_files_under(&self, root: &Path) -> Result<Vec<DiskFile>> {
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
            files.push(DiskFile {
                path: rel,
                bytes: fs::read(path)?,
                executable: executable(path)?,
            });
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(files)
    }

    fn scan_file_paths_under(&self, root: &Path) -> Result<WorktreePathScan> {
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
        Ok(WorktreePathScan { paths, total_bytes })
    }
}

fn scan_files_under_selection(
    root: &Path,
    selected_root: &Path,
    use_git_ignores: bool,
    files: &mut BTreeMap<String, DiskFile>,
) -> Result<()> {
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
        files.insert(
            rel.clone(),
            DiskFile {
                path: rel,
                bytes: fs::read(path)?,
                executable: executable(path)?,
            },
        );
    }
    Ok(())
}
