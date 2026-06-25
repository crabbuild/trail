use super::*;

impl CrabDb {
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
            if status == b"??" {
                return Ok(None);
            }
            let path = normalize_relative_path(&String::from_utf8_lossy(&record[3..]))?;
            if path == ".crabignore" || path == ".gitignore" {
                return Ok(None);
            }
            if !self.ignore_check(&path)?.ignored {
                paths.insert(path);
            }
            if status.contains(&b'R') || status.contains(&b'C') {
                idx += 1;
                let Some(old_record) = records.get(idx) else {
                    return Ok(None);
                };
                let old_path = normalize_relative_path(&String::from_utf8_lossy(old_record))?;
                if old_path == ".crabignore" || old_path == ".gitignore" {
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
            }
        }
        Ok(files.into_values().collect())
    }

    pub(crate) fn scan_git_tracked_files(&self) -> Result<Vec<DiskFile>> {
        self.scan_git_tracked_files_impl(false)
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
}
