use super::*;

impl CrabDb {
    pub fn init(
        workspace_root: impl AsRef<Path>,
        branch: impl Into<String>,
        mode: InitImportMode,
        force: bool,
    ) -> Result<InitReport> {
        Self::init_with_text_policy(workspace_root, branch, mode, force, None)
    }

    pub fn init_with_text_policy(
        workspace_root: impl AsRef<Path>,
        branch: impl Into<String>,
        mode: InitImportMode,
        force: bool,
        text_policy: Option<&str>,
    ) -> Result<InitReport> {
        let workspace_root = workspace_root.as_ref().canonicalize()?;
        let db_dir = workspace_root.join(".crabdb");
        if db_dir.exists() {
            if !force {
                return Err(Error::WorkspaceExists(db_dir));
            }
            fs::remove_dir_all(&db_dir)?;
        }

        fs::create_dir_all(db_dir.join("index"))?;
        fs::create_dir_all(db_dir.join("refs/branches"))?;
        fs::create_dir_all(db_dir.join("refs/agents"))?;
        fs::create_dir_all(db_dir.join("worktrees"))?;

        let branch = branch.into();
        let workspace_id = WorkspaceId::new(workspace_root.to_string_lossy().as_bytes());
        let mut config = CrabConfig::new(workspace_id.clone(), branch.clone());
        apply_text_policy(&mut config.text, text_policy)?;
        fs::write(db_dir.join(CONFIG_FILE), toml::to_string_pretty(&config)?)?;
        fs::write(db_dir.join(HEAD_FILE), format!("{branch}\n"))?;
        write_default_crabignore(&workspace_root)?;

        let db = Self::open_at(workspace_root, db_dir, config)?;
        db.init_schema()?;

        let actor = Actor::system();
        let change_id = db.allocate_change_id(&actor.id, "init")?;
        let disk_files = match mode {
            InitImportMode::Empty => Vec::new(),
            InitImportMode::GitTracked => db.scan_git_tracked_files()?,
            InitImportMode::WorkingTree => db.scan_worktree_files()?,
        };
        let built = db.build_root_from_disk_files(&disk_files, &change_id, None)?;
        let kind = if mode == InitImportMode::Empty {
            OperationKind::Init
        } else {
            OperationKind::GitImport
        };
        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind,
            parents: Vec::new(),
            before_root: None,
            after_root: built.root_id.clone(),
            branch: branch.clone(),
            actor,
            session_id: None,
            message: Some("Initialize CrabDB workspace".to_string()),
            changes: built
                .files
                .iter()
                .map(|(path, entry)| FileChange {
                    path: path.clone(),
                    old_path: None,
                    file_id: Some(entry.file_id.clone()),
                    kind: FileChangeKind::Added,
                    before_hash: None,
                    after_hash: Some(entry.content_hash.clone()),
                    line_changes: Vec::new(),
                })
                .collect(),
            created_at: now_ts(),
        };
        let operation_id = db.store_operation(&operation)?;
        db.set_ref(
            &branch_ref(&branch),
            &change_id,
            &built.root_id,
            &operation_id,
        )?;
        if mode == InitImportMode::GitTracked {
            db.insert_git_mapping("import", &branch, &change_id, &built.root_id)?;
        }

        Ok(InitReport {
            workspace_id,
            branch,
            operation: change_id,
            root_id: built.root_id,
            imported: built.stats,
        })
    }

    pub fn discover(start: impl AsRef<Path>) -> Result<Self> {
        let mut current = start.as_ref().canonicalize()?;
        loop {
            let db_dir = current.join(".crabdb");
            if db_dir.is_dir() {
                let config = read_config(&db_dir)?;
                return Self::open_at(current, db_dir, config);
            }
            if !current.pop() {
                return Err(Error::WorkspaceNotFound(start.as_ref().to_path_buf()));
            }
        }
    }

    pub fn open(workspace_root: impl AsRef<Path>) -> Result<Self> {
        let workspace_root = workspace_root.as_ref().canonicalize()?;
        let db_dir = workspace_root.join(".crabdb");
        if !db_dir.is_dir() {
            return Err(Error::WorkspaceNotFound(workspace_root));
        }
        let config = read_config(&db_dir)?;
        Self::open_at(workspace_root, db_dir, config)
    }

    pub fn open_with_db_dir(
        workspace_root: impl AsRef<Path>,
        db_dir: impl AsRef<Path>,
    ) -> Result<Self> {
        let workspace_root = workspace_root.as_ref().canonicalize()?;
        let db_dir = db_dir.as_ref().canonicalize()?;
        if !db_dir.is_dir() {
            return Err(Error::WorkspaceNotFound(db_dir));
        }
        let config = read_config(&db_dir)?;
        Self::open_at(workspace_root, db_dir, config)
    }

    pub(crate) fn open_at(
        workspace_root: PathBuf,
        db_dir: PathBuf,
        config: CrabConfig,
    ) -> Result<Self> {
        fs::create_dir_all(db_dir.join("index"))?;
        let sqlite_path = db_dir.join(DB_RELATIVE_PATH);
        let store = Arc::new(SqliteStore::open(&sqlite_path)?);
        let conn = Connection::open(&sqlite_path)?;
        apply_sqlite_pragmas(&conn)?;
        let prolly = Prolly::new(store.clone(), prolly_config());
        let db = Self {
            workspace_root,
            db_dir,
            conn,
            store,
            prolly,
            config,
        };
        db.init_schema()?;
        Ok(db)
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn db_dir(&self) -> &Path {
        &self.db_dir
    }

    pub fn config(&self) -> &CrabConfig {
        &self.config
    }

    pub fn config_entries(&self) -> Vec<ConfigEntry> {
        config_entries_from(&self.config)
    }

    pub fn config_get(&self, key: &str) -> Result<ConfigEntry> {
        config_entry_from(&self.config, key)
            .ok_or_else(|| Error::InvalidInput(format!("unknown config key `{key}`")))
    }

    pub fn config_set(&mut self, key: &str, value: &str) -> Result<ConfigSetReport> {
        let _lock = self.acquire_write_lock()?;
        let old = self.config_get(key)?;
        if old.read_only {
            return Err(Error::InvalidInput(format!(
                "config key `{key}` is read-only"
            )));
        }

        let mut next = self.config.clone();
        set_config_value(self, &mut next, key, value)?;
        let new_value = config_entry_from(&next, key)
            .ok_or_else(|| Error::InvalidInput(format!("unknown config key `{key}`")))?
            .value;
        write_config(&self.db_dir, &next)?;
        self.config = next;

        Ok(ConfigSetReport {
            key: key.to_string(),
            old_value: old.value,
            new_value,
        })
    }

    pub fn current_branch(&self) -> Result<String> {
        let head = self.db_dir.join(HEAD_FILE);
        let branch = fs::read_to_string(head)
            .unwrap_or_else(|_| self.config.workspace.default_branch.clone())
            .trim()
            .to_string();
        if branch.is_empty() {
            Ok(self.config.workspace.default_branch.clone())
        } else {
            Ok(branch)
        }
    }

    pub(crate) fn acquire_write_lock(&self) -> Result<WorkspaceLock> {
        let path = self.db_dir.join("lock");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|err| {
                if err.kind() == std::io::ErrorKind::AlreadyExists {
                    let holder =
                        fs::read_to_string(&path).unwrap_or_else(|_| "unknown writer".to_string());
                    Error::WorkspaceLocked(holder.trim().to_string())
                } else {
                    Error::Io(err)
                }
            })?;
        writeln!(file, "pid={} created_at={}", std::process::id(), now_ts())?;
        Ok(WorkspaceLock { path })
    }
}
