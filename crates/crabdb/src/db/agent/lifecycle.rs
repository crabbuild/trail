use super::*;

impl CrabDb {
    pub fn spawn_agent(
        &mut self,
        name: &str,
        from: Option<&str>,
        materialize: bool,
        provider: Option<String>,
        model: Option<String>,
    ) -> Result<AgentSpawnReport> {
        self.spawn_agent_with_workdir(name, from, materialize, provider, model, None)
    }

    pub fn spawn_agent_with_workdir(
        &mut self,
        name: &str,
        from: Option<&str>,
        materialize: bool,
        provider: Option<String>,
        model: Option<String>,
        workdir: Option<PathBuf>,
    ) -> Result<AgentSpawnReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(name)?;
        if workdir.is_some() && !materialize {
            return Err(Error::InvalidInput(
                "custom agent workdir requires materialization to be enabled".to_string(),
            ));
        }
        let source = match from {
            Some(refish) => self.resolve_refish(refish)?,
            None => self.resolve_branch_ref(&self.current_branch()?)?,
        };
        let agent_id = format!("agent_{}", crate::ids::short_hash(name.as_bytes(), 8));
        let ref_name = agent_ref(name);
        if self.try_get_ref(&ref_name)?.is_some() {
            return Err(Error::InvalidInput(format!(
                "agent `{name}` already exists"
            )));
        }
        let workdir_path = if materialize {
            Some(self.resolve_agent_workdir_path(name, workdir.as_deref())?)
        } else {
            None
        };
        let materialized_workdir = if let Some(dir) = &workdir_path {
            self.materialize_agent_workdir_at(&source.root_id, dir, workdir.is_some())?;
            Some(dir.to_string_lossy().to_string())
        } else {
            None
        };
        self.set_ref(
            &ref_name,
            &source.change_id,
            &source.root_id,
            &source.operation_id,
        )?;
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO agents (agent_id, name, kind, provider, model, created_at, metadata_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                agent_id,
                name,
                "coding-agent",
                provider,
                model,
                now,
                Option::<String>::None
            ],
        )?;
        self.conn.execute(
            "INSERT INTO agent_branches \
             (agent_id, ref_name, base_change, head_change, base_root, head_root, session_id, workdir, status, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'active', ?9, ?9)",
            params![
                agent_id,
                ref_name,
                source.change_id.0,
                source.change_id.0,
                source.root_id.0,
                source.root_id.0,
                Option::<String>::None,
                materialized_workdir,
                now
            ],
        )?;
        self.insert_agent_event(
            &agent_id,
            "agent_spawned",
            Some(&source.change_id),
            None,
            &serde_json::json!({
                "ref_name": ref_name.clone(),
                "base_root": source.root_id.0.clone(),
                "workdir": materialized_workdir.clone()
            }),
        )?;
        Ok(AgentSpawnReport {
            agent_id,
            ref_name,
            base_change: source.change_id,
            workdir: materialized_workdir,
        })
    }

    pub(crate) fn materialize_agent_workdir(
        &self,
        name: &str,
        root_id: &ObjectId,
        custom_workdir: Option<&Path>,
    ) -> Result<PathBuf> {
        let dir = self.resolve_agent_workdir_path(name, custom_workdir)?;
        self.materialize_agent_workdir_at(root_id, &dir, custom_workdir.is_some())?;
        Ok(dir)
    }

    pub(crate) fn materialize_agent_workdir_at(
        &self,
        root_id: &ObjectId,
        dir: &Path,
        custom_workdir: bool,
    ) -> Result<()> {
        prepare_agent_workdir(dir, custom_workdir)?;
        let empty = BTreeMap::new();
        let files = self.load_root_files(root_id)?;
        materialize_into(&self.workspace_root, dir, &empty, &files, |entry| {
            self.materialize_entry_bytes(entry)
        })
    }

    pub(crate) fn resolve_agent_workdir_path(
        &self,
        name: &str,
        custom_workdir: Option<&Path>,
    ) -> Result<PathBuf> {
        let raw = match custom_workdir {
            Some(path) if path.is_absolute() => path.to_path_buf(),
            Some(path) => self.workspace_root.join(path),
            None => self.default_agent_workdir_path(name)?,
        };
        let normalized = normalize_workdir_path(&raw)?;
        let normalized = canonicalize_existing_workdir_prefix(&normalized)?;
        self.validate_agent_workdir_path(&normalized)?;
        Ok(normalized)
    }

    pub(crate) fn default_agent_workdir_path(&self, name: &str) -> Result<PathBuf> {
        Ok(self.default_agent_worktrees_base()?.join(name))
    }

    pub(crate) fn default_agent_worktrees_base(&self) -> Result<PathBuf> {
        let rel = normalize_relative_path(&self.config.agent.worktrees_dir)?;
        normalize_workdir_path(&self.workspace_root.join(path_from_rel(&rel)))
    }

    pub(crate) fn validate_agent_workdir_path(&self, path: &Path) -> Result<()> {
        if path == self.workspace_root {
            return Err(Error::InvalidPath {
                path: path.to_string_lossy().to_string(),
                reason: "agent workdir cannot be the workspace root".to_string(),
            });
        }
        let worktrees_base = self.default_agent_worktrees_base()?;
        if path == worktrees_base {
            return Err(Error::InvalidPath {
                path: path.to_string_lossy().to_string(),
                reason: "agent workdir must include an agent-specific directory".to_string(),
            });
        }
        if path.starts_with(&self.workspace_root) && !path.starts_with(&worktrees_base) {
            return Err(Error::InvalidPath {
                path: path.to_string_lossy().to_string(),
                reason: format!(
                    "agent workdirs inside the workspace must live under `{}`",
                    worktrees_base.display()
                ),
            });
        }
        if let Ok(metadata) = fs::symlink_metadata(path) {
            if metadata.file_type().is_symlink() {
                return Err(Error::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                    reason: "agent workdir cannot be a symlink".to_string(),
                });
            }
        }
        Ok(())
    }

    pub(crate) fn resolve_checkout_workdir_path(&self, workdir: &Path) -> Result<PathBuf> {
        let raw = if workdir.is_absolute() {
            workdir.to_path_buf()
        } else {
            self.workspace_root.join(workdir)
        };
        let normalized = normalize_workdir_path(&raw)?;
        let normalized = canonicalize_existing_workdir_prefix(&normalized)?;
        let workspace = self.workspace_root.canonicalize()?;
        if normalized == workspace {
            return Err(Error::InvalidPath {
                path: normalized.to_string_lossy().to_string(),
                reason: "checkout workdir cannot be the workspace root".to_string(),
            });
        }
        if normalized.starts_with(&workspace) {
            let db_dir = self.db_dir.canonicalize()?;
            if !normalized.starts_with(&db_dir) {
                return Err(Error::InvalidPath {
                    path: normalized.to_string_lossy().to_string(),
                    reason: format!(
                        "checkout workdir inside the workspace must live under `{}`",
                        db_dir.display()
                    ),
                });
            }
        }
        Ok(normalized)
    }
}
