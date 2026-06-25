use super::*;

impl CrabDb {
    pub(crate) fn scan_record_selection_files(
        &self,
        selected_paths: &[String],
        allow_ignored: bool,
    ) -> Result<Vec<DiskFile>> {
        if !allow_ignored
            && selected_paths
                .iter()
                .any(|path| self.workspace_root.join(path_from_rel(path)).is_dir())
        {
            let disk_files = self.scan_worktree_files()?;
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
                selected.insert(
                    path.clone(),
                    DiskFile {
                        path: path.clone(),
                        bytes: fs::read(&abs)?,
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
                if abs.exists() {
                    return Err(Error::IgnoredPath(path.clone()));
                }
            }
        }

        Ok(selected.into_values().collect())
    }

    pub(crate) fn read_record_selection_unfiltered(&self, path: &str) -> Result<Vec<DiskFile>> {
        if is_internal_path(path) {
            return Err(Error::IgnoredPath(path.to_string()));
        }
        let abs = self.workspace_root.join(path_from_rel(path));
        if !abs.exists() {
            return Ok(Vec::new());
        }
        let metadata = fs::symlink_metadata(&abs)?;
        if metadata.file_type().is_symlink() {
            return Ok(Vec::new());
        }
        if metadata.is_file() {
            return Ok(vec![DiskFile {
                path: path.to_string(),
                bytes: fs::read(&abs)?,
                executable: executable_from_metadata(&metadata),
            }]);
        }
        if !metadata.is_dir() {
            return Ok(Vec::new());
        }
        let mut files = Vec::new();
        self.read_record_dir_unfiltered(&abs, path, &mut files)?;
        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(files)
    }

    pub(crate) fn read_record_dir_unfiltered(
        &self,
        dir: &Path,
        rel_dir: &str,
        files: &mut Vec<DiskFile>,
    ) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            let rel = format!("{rel_dir}/{name}");
            if is_internal_path(&rel) {
                continue;
            }
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                self.read_record_dir_unfiltered(&path, &rel, files)?;
            } else if metadata.is_file() {
                files.push(DiskFile {
                    path: rel,
                    bytes: fs::read(&path)?,
                    executable: executable_from_metadata(&metadata),
                });
            }
        }
        Ok(())
    }
}
