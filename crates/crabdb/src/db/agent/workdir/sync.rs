use super::*;

impl CrabDb {
    pub fn agent_workdir(&self, agent: &str) -> Result<AgentWorkdirReport> {
        validate_ref_segment(agent)?;
        let branch = self.agent_branch(agent)?;
        Ok(AgentWorkdirReport {
            agent_id: branch.agent_id,
            workdir: branch.workdir,
        })
    }

    pub fn sync_agent_workdir(
        &mut self,
        agent: &str,
        force: bool,
    ) -> Result<AgentWorkdirSyncReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;
        let branch = self.agent_branch(agent)?;
        let Some(workdir) = branch.workdir.clone() else {
            return Err(Error::InvalidInput(format!(
                "agent `{agent}` does not have a materialized workdir"
            )));
        };
        let workdir_path = PathBuf::from(&workdir);
        if workdir_path.exists() && !workdir_path.is_dir() {
            if force {
                fs::remove_file(&workdir_path)?;
            } else {
                return Err(Error::InvalidInput(format!(
                    "agent `{agent}` workdir path exists but is not a directory"
                )));
            }
        }
        let head = self.get_ref(&branch.ref_name)?;
        let target_files = self.load_root_files(&head.root_id)?;
        let workdir_exists = workdir_path.is_dir();
        let changed_paths = if workdir_exists {
            self.agent_workdir_changed_paths(&branch, &head)?
                .unwrap_or_default()
        } else {
            self.diff_file_maps(&BTreeMap::new(), &target_files)?
                .summaries
        };
        if workdir_exists && !changed_paths.is_empty() && !force {
            let preview = changed_paths
                .iter()
                .take(5)
                .map(|path| format!("{:?} {}", path.kind, path.path))
                .collect::<Vec<_>>()
                .join(", ");
            let suffix = if changed_paths.len() > 5 {
                format!(", ... {} more", changed_paths.len() - 5)
            } else {
                String::new()
            };
            return Err(Error::DirtyWorktreeWithMessage(format!(
                "agent `{agent}` workdir has unrecorded changes; run `crabdb agent record {agent}` or pass `--force` to sync: {preview}{suffix}"
            )));
        }
        if force && workdir_path.exists() {
            fs::remove_dir_all(&workdir_path)?;
        }
        fs::create_dir_all(&workdir_path)?;
        let previous = if force || !workdir_exists {
            BTreeMap::new()
        } else {
            target_files.clone()
        };
        materialize_into(
            &self.workspace_root,
            &workdir_path,
            &previous,
            &target_files,
            |entry| self.materialize_entry_bytes(entry),
        )?;
        self.insert_agent_event(
            &branch.agent_id,
            "workdir_synced",
            Some(&head.change_id),
            None,
            &serde_json::json!({
                "workdir": workdir.clone(),
                "forced": force,
                "changed_paths": changed_paths.iter().map(|item| item.path.clone()).collect::<Vec<_>>()
            }),
        )?;
        Ok(AgentWorkdirSyncReport {
            agent_id: branch.agent_id,
            workdir,
            head_change: head.change_id,
            root_id: head.root_id,
            forced: force,
            changed_paths,
        })
    }
}
