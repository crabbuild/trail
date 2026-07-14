use super::*;

const AUTO_MINIMAL_FILES_THRESHOLD: usize = 10_000;
const AUTO_MINIMAL_BYTES_THRESHOLD: u64 = 128 * 1024 * 1024;
const DETAILED_INIT_CHANGES_FILE_THRESHOLD: usize = 10_000;

impl Trail {
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
        Self::init_with_options(workspace_root, branch, mode, force, text_policy, None)
    }

    pub fn init_with_text_policy_and_prolly_backend(
        workspace_root: impl AsRef<Path>,
        branch: impl Into<String>,
        mode: InitImportMode,
        force: bool,
        text_policy: Option<&str>,
        prolly_backend: Option<&str>,
    ) -> Result<InitReport> {
        Self::init_with_options(
            workspace_root,
            branch,
            mode,
            force,
            text_policy,
            prolly_backend,
        )
    }

    fn init_with_options(
        workspace_root: impl AsRef<Path>,
        branch: impl Into<String>,
        mode: InitImportMode,
        force: bool,
        text_policy: Option<&str>,
        prolly_backend: Option<&str>,
    ) -> Result<InitReport> {
        let workspace_root = canonicalize_lossless(workspace_root.as_ref())?;
        super::backup::recover_restore_publication(&workspace_root)?;
        let db_dir = workspace_root.join(".trail");
        if db_dir.exists() {
            if !force {
                return Err(Error::WorkspaceExists(db_dir));
            }
            fs::remove_dir_all(&db_dir)?;
        }

        fs::create_dir_all(db_dir.join("index"))?;
        fs::create_dir_all(db_dir.join("refs/branches"))?;
        fs::create_dir_all(db_dir.join("refs/lanes"))?;
        fs::create_dir_all(db_dir.join("worktrees"))?;

        let branch = branch.into();
        let workspace_id = WorkspaceId::new(workspace_root.to_string_lossy().as_bytes());
        let mut config = TrailConfig::new(workspace_id.clone(), branch.clone());
        if let Some(prolly_backend) = prolly_backend {
            match prolly_backend {
                "sqlite" | "slatedb" => {
                    config.storage.prolly_backend = prolly_backend.to_string();
                    if prolly_backend == "slatedb" {
                        config.storage.slatedb_path =
                            format!("trail/workspaces/{}/prolly", workspace_id.0);
                    }
                }
                other => {
                    return Err(Error::InvalidInput(format!(
                        "storage.prolly_backend must be sqlite or slatedb, got `{other}`"
                    )));
                }
            }
        }
        let explicit_text_policy = text_policy.is_some();
        apply_text_policy(&mut config.text, text_policy)?;
        if !explicit_text_policy
            && mode == InitImportMode::GitTracked
            && git_tracked_import_is_large(&workspace_root)?
        {
            apply_text_policy(&mut config.text, Some("minimal"))?;
        }
        fs::write(db_dir.join(CONFIG_FILE), toml::to_string_pretty(&config)?)?;
        fs::write(db_dir.join(HEAD_FILE), format!("{branch}\n"))?;
        write_default_trailignore(&workspace_root)?;

        let mut db = Self::open_at(workspace_root, db_dir, config, SchemaOpenMode::FreshCreate)?;

        let actor = Actor::system();
        let change_id = db.allocate_change_id(&actor.id, "init")?;
        let built = match mode {
            InitImportMode::Empty => {
                let disk_files = Vec::new();
                let built = db.build_root_from_disk_files(&disk_files, &change_id, None)?;
                db.update_worktree_index_from_disk_files_and_manifest(
                    &disk_files,
                    &built.disk_manifest,
                )?;
                built
            }
            InitImportMode::GitTracked => {
                if let Some(paths) = db.scan_git_tracked_paths_impl(false)? {
                    let built = db.build_root_from_git_tracked_paths(&paths, &change_id)?;
                    let imported_paths = built.disk_manifest.keys().cloned().collect::<Vec<_>>();
                    db.update_worktree_index_from_paths_and_manifest(
                        &imported_paths,
                        &built.disk_manifest,
                    )?;
                    built
                } else {
                    let scan = db.scan_worktree_file_paths()?;
                    apply_minimal_text_policy_for_large_worktree_scan(
                        &mut db,
                        explicit_text_policy,
                        &scan,
                    )?;
                    let built = db.build_root_from_worktree_paths(&scan.paths, &change_id)?;
                    let imported_paths = built.disk_manifest.keys().cloned().collect::<Vec<_>>();
                    db.update_worktree_index_from_paths_and_manifest(
                        &imported_paths,
                        &built.disk_manifest,
                    )?;
                    built
                }
            }
            InitImportMode::WorkingTree => {
                let scan = db.scan_worktree_file_paths()?;
                apply_minimal_text_policy_for_large_worktree_scan(
                    &mut db,
                    explicit_text_policy,
                    &scan,
                )?;
                let built = db.build_root_from_worktree_paths(&scan.paths, &change_id)?;
                let imported_paths = built.disk_manifest.keys().cloned().collect::<Vec<_>>();
                db.update_worktree_index_from_paths_and_manifest(
                    &imported_paths,
                    &built.disk_manifest,
                )?;
                built
            }
        };
        let kind = if mode == InitImportMode::Empty {
            OperationKind::Init
        } else {
            OperationKind::GitImport
        };
        let changes = if built.files.len() <= DETAILED_INIT_CHANGES_FILE_THRESHOLD {
            built
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
                .collect()
        } else {
            Vec::new()
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
            message: Some("Initialize Trail workspace".to_string()),
            changes,
            created_at: now_ts(),
        };
        let operation_id = db.store_operation(&operation)?;
        db.set_ref(
            &branch_ref(&branch),
            &change_id,
            &built.root_id,
            &operation_id,
        )?;
        db.set_worktree_index_baseline(&built.root_id)?;
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
        let mut current = canonicalize_lossless(start.as_ref())?;
        loop {
            super::backup::recover_restore_publication(&current)?;
            let db_dir = current.join(".trail");
            if db_dir.is_dir() {
                let config = read_config(&db_dir)?;
                return Self::open_at(current, db_dir, config, SchemaOpenMode::Existing);
            }
            if !current.pop() {
                return Err(Error::WorkspaceNotFound(start.as_ref().to_path_buf()));
            }
        }
    }

    pub fn open(workspace_root: impl AsRef<Path>) -> Result<Self> {
        let workspace_root = canonicalize_lossless(workspace_root.as_ref())?;
        super::backup::recover_restore_publication(&workspace_root)?;
        let db_dir = workspace_root.join(".trail");
        if !db_dir.is_dir() {
            return Err(Error::WorkspaceNotFound(workspace_root));
        }
        let config = read_config(&db_dir)?;
        Self::open_at(workspace_root, db_dir, config, SchemaOpenMode::Existing)
    }

    pub fn open_with_db_dir(
        workspace_root: impl AsRef<Path>,
        db_dir: impl AsRef<Path>,
    ) -> Result<Self> {
        let workspace_root = canonicalize_lossless(workspace_root.as_ref())?;
        super::backup::recover_restore_publication(&workspace_root)?;
        let db_dir = canonicalize_lossless(db_dir.as_ref())?;
        if !db_dir.is_dir() {
            return Err(Error::WorkspaceNotFound(db_dir));
        }
        let config = read_config(&db_dir)?;
        Self::open_at(workspace_root, db_dir, config, SchemaOpenMode::Existing)
    }

    pub(crate) fn open_at(
        workspace_root: PathBuf,
        db_dir: PathBuf,
        config: TrailConfig,
        schema_mode: SchemaOpenMode,
    ) -> Result<Self> {
        let db = Self::open_at_without_recovery(workspace_root, db_dir, config, schema_mode)?;
        db.recover_after_open()?;
        Ok(db)
    }

    pub(crate) fn open_without_recovering_derived_paths(
        workspace_root: impl AsRef<Path>,
        db_dir: impl AsRef<Path>,
    ) -> Result<Self> {
        let workspace_root = canonicalize_lossless(workspace_root.as_ref())?;
        let db_dir = canonicalize_lossless(db_dir.as_ref())?;
        if !db_dir.is_dir() {
            return Err(Error::WorkspaceNotFound(db_dir));
        }
        let config = read_config(&db_dir)?;
        Self::open_at_without_recovery(workspace_root, db_dir, config, SchemaOpenMode::Existing)
    }

    fn open_at_without_recovery(
        workspace_root: PathBuf,
        db_dir: PathBuf,
        config: TrailConfig,
        schema_mode: SchemaOpenMode,
    ) -> Result<Self> {
        let sqlite_path = db_dir.join(DB_RELATIVE_PATH);
        match schema_mode {
            SchemaOpenMode::FreshCreate => fs::create_dir_all(db_dir.join("index"))?,
            SchemaOpenMode::Existing => {
                preflight_existing_schema(&sqlite_path, &config.storage.prolly_backend)?
            }
        }
        register_sqlite_vec_extension()?;
        let operation_metrics =
            operation_metrics_are_enabled().then(|| Arc::new(OperationMetricsState::default()));
        let store = open_prolly_store(
            &config,
            &sqlite_path,
            operation_metrics.clone(),
            schema_mode,
        )?;
        let conn = Connection::open(&sqlite_path)?;
        apply_sqlite_pragmas(&conn)?;
        let prolly = Prolly::new(store.clone(), prolly_config());
        let root_prolly = Prolly::new(store.clone(), root_map_prolly_config());
        let db = Self {
            workspace_root,
            db_dir,
            sqlite_path,
            conn,
            store,
            prolly,
            root_prolly,
            config,
            object_cache: Mutex::new(ObjectCache::default()),
            daemon_worktree_cache: None,
            git_handoff_metrics: Cell::new(GitHandoffMetrics::default()),
            case_fold_index_metrics: Cell::new(CaseFoldIndexMetrics::default()),
            operation_metrics,
        };
        if schema_mode == SchemaOpenMode::FreshCreate {
            db.create_schema_v18()?;
        }
        Ok(db)
    }

    pub(crate) fn recover_after_open(&self) -> Result<()> {
        if self.has_pending_path_index_derived_repairs()? {
            let _lock = self.acquire_write_lock()?;
            if self.has_pending_path_index_derived_repairs()? {
                self.drain_pending_path_index_derived_repairs()?;
            }
        }
        self.recover_materialization_stages()?;
        self.recover_workspace_views()?;
        self.recover_workspace_environment_sync_attempts()?;
        self.recover_workspace_runtime_leases()?;
        Ok(())
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn db_dir(&self) -> &Path {
        &self.db_dir
    }

    pub(crate) fn changed_path_ledger(&self) -> super::change_ledger::ChangedPathLedger<'_> {
        super::change_ledger::ChangedPathLedger::new_at(&self.conn, &self.sqlite_path)
    }

    pub fn config(&self) -> &TrailConfig {
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

    pub(crate) fn with_write_lock_wait<T>(
        timeout: Duration,
        f: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let previous = WRITE_LOCK_WAIT_DEADLINE
            .with(|deadline| deadline.replace(Some(Instant::now() + timeout)));
        let _guard = WriteLockWaitGuard { previous };
        f()
    }

    pub(crate) fn acquire_write_lock(&self) -> Result<WorkspaceLock> {
        let path = self.db_dir.join("lock");
        let mut delay = Duration::from_millis(2);
        let mut file = loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => break file,
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    let holder =
                        fs::read_to_string(&path).unwrap_or_else(|_| "unknown writer".to_string());
                    if is_stale_lock_holder(&holder)
                        && fs::read_to_string(&path).unwrap_or_default() == holder
                    {
                        match fs::remove_file(&path) {
                            Ok(()) => continue,
                            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                            Err(err) => return Err(Error::Io(err)),
                        }
                    }
                    let should_wait = WRITE_LOCK_WAIT_DEADLINE
                        .with(|deadline| deadline.get())
                        .is_some_and(|deadline| Instant::now() < deadline);
                    if should_wait {
                        std::thread::sleep(delay);
                        delay = (delay * 2).min(Duration::from_millis(50));
                        continue;
                    }
                    return Err(Error::WorkspaceLocked(holder.trim().to_string()));
                }
                Err(err) => return Err(Error::Io(err)),
            }
        };
        writeln!(file, "pid={} created_at={}", std::process::id(), now_ts())?;
        Ok(WorkspaceLock { path })
    }
}

fn git_tracked_import_is_large(workspace_root: &Path) -> Result<bool> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_root)
        .arg("ls-files")
        .arg("-z")
        .output()
        .map_err(|err| Error::Git(err.to_string()))?;
    if !output.status.success() {
        return Ok(false);
    }

    let mut files = 0usize;
    let mut bytes = 0u64;
    for raw in output.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        let path = normalize_relative_path(&String::from_utf8_lossy(raw))?;
        if is_default_ignored(&path) {
            continue;
        }
        let abs = workspace_root.join(path_from_rel(&path));
        let metadata = match fs::symlink_metadata(&abs) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(Error::Io(err)),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            continue;
        }
        files += 1;
        bytes = bytes.saturating_add(metadata.len());
        if files > AUTO_MINIMAL_FILES_THRESHOLD || bytes > AUTO_MINIMAL_BYTES_THRESHOLD {
            return Ok(true);
        }
    }
    Ok(false)
}

fn apply_minimal_text_policy_for_large_worktree_scan(
    db: &mut Trail,
    explicit_text_policy: bool,
    scan: &WorktreePathScan,
) -> Result<()> {
    if explicit_text_policy || !worktree_path_scan_is_large(scan) {
        return Ok(());
    }
    apply_text_policy(&mut db.config.text, Some("minimal"))?;
    write_config(&db.db_dir, &db.config)
}

fn worktree_path_scan_is_large(scan: &WorktreePathScan) -> bool {
    scan.paths.len() > AUTO_MINIMAL_FILES_THRESHOLD
        || scan.total_bytes > AUTO_MINIMAL_BYTES_THRESHOLD
}

fn is_stale_lock_holder(holder: &str) -> bool {
    let Some(pid) = lock_holder_pid(holder) else {
        return false;
    };
    !process_is_alive(pid)
}

fn lock_holder_pid(holder: &str) -> Option<u32> {
    holder.split_whitespace().find_map(|part| {
        part.strip_prefix("pid=")
            .and_then(|value| value.parse::<u32>().ok())
    })
}
