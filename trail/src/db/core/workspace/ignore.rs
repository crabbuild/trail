use super::*;

impl Trail {
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
        let path = normalize_relative_path(path)?;
        if is_default_ignored(&path) {
            return Ok(IgnoreCheckReport {
                path,
                ignored: true,
                source: Some("hardcoded".to_string()),
            });
        }
        let abs = self.workspace_root.join(path_from_rel(&path));
        let is_dir = abs.is_dir();
        let mut builder = ::ignore::gitignore::GitignoreBuilder::new(&self.workspace_root);
        let trailignore = self.workspace_root.join(".trailignore");
        let gitignore = self.workspace_root.join(".gitignore");
        // A single metadata probe preserves `Path::exists` semantics (all
        // errors mean absent) while also exposing the bytes the matcher will
        // read, without adding a metrics-only syscall.
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
        self.note_operation_metrics(OperationMetricsDelta {
            policy_build_count: 1,
            policy_dependency_file_count: u64::from(trailignore_exists)
                .saturating_add(u64::from(gitignore_exists)),
            policy_dependency_bytes: dependency_bytes,
            // Candidate directory classification plus the two dependency
            // probes above. `Path::is_dir` also treats probe errors as false.
            filesystem_stat_count: 3,
            ..OperationMetricsDelta::default()
        });
        if trailignore_exists {
            if let Some(err) = builder.add(trailignore) {
                return Err(Error::InvalidInput(err.to_string()));
            }
        }
        if gitignore_exists {
            if let Some(err) = builder.add(gitignore) {
                return Err(Error::InvalidInput(err.to_string()));
            }
        }
        let matcher = builder
            .build()
            .map_err(|err| Error::InvalidInput(err.to_string()))?;
        let ignored = matcher
            .matched_path_or_any_parents(path_from_rel(&path), is_dir)
            .is_ignore();
        Ok(IgnoreCheckReport {
            path,
            ignored,
            source: ignored.then(|| "workspace".to_string()),
        })
    }
}
