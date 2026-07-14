use super::*;

const AUTO_MINIMAL_FILES_THRESHOLD: usize = 10_000;
const AUTO_MINIMAL_BYTES_THRESHOLD: u64 = 128 * 1024 * 1024;
const DETAILED_INIT_CHANGES_FILE_THRESHOLD: usize = 10_000;

#[cfg(test)]
thread_local! {
    static SCHEMA_HANDOFF_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce(&Path)>>> =
        const { std::cell::RefCell::new(None) };
    static SCHEMA_PRIMARY_OPEN_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce(&Path)>>> =
        const { std::cell::RefCell::new(None) };
    static SCHEMA_PROLLY_OPEN_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce(&Path)>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn install_schema_primary_open_hook(hook: impl FnOnce(&Path) + 'static) {
    SCHEMA_PRIMARY_OPEN_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(test)]
fn run_schema_primary_open_hook(db_dir: &Path) {
    SCHEMA_PRIMARY_OPEN_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook(db_dir);
        }
    });
}

#[cfg(test)]
fn install_schema_prolly_open_hook(hook: impl FnOnce(&Path) + 'static) {
    SCHEMA_PROLLY_OPEN_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(test)]
fn run_schema_prolly_open_hook(db_dir: &Path) {
    SCHEMA_PROLLY_OPEN_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook(db_dir);
        }
    });
}

#[cfg(test)]
fn install_schema_handoff_hook(hook: impl FnOnce(&Path) + 'static) {
    SCHEMA_HANDOFF_HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}

#[cfg(test)]
fn run_schema_handoff_hook(db_dir: &Path) {
    SCHEMA_HANDOFF_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook(db_dir);
        }
    });
}

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
        fs::write(db_dir.join("index").join(SCHEMA_EXCLUSION_FILE), [])?;
        fs::write(db_dir.join("index").join(SCHEMA_VALIDATION_LEADER_FILE), [])?;
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
        let db =
            Self::open_at_without_recovery(workspace_root, db_dir, config, schema_mode, false)?;
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
        Self::open_at_without_recovery(
            workspace_root,
            db_dir,
            config,
            SchemaOpenMode::Existing,
            false,
        )
    }

    pub(crate) fn open_without_recovering_derived_paths_under_write_lock(
        workspace_root: impl AsRef<Path>,
        db_dir: impl AsRef<Path>,
    ) -> Result<Self> {
        let workspace_root = canonicalize_lossless(workspace_root.as_ref())?;
        let db_dir = canonicalize_lossless(db_dir.as_ref())?;
        if !db_dir.is_dir() {
            return Err(Error::WorkspaceNotFound(db_dir));
        }
        let config = read_config(&db_dir)?;
        // The caller owns the workspace write lock and already holds a Trail handle whose
        // mutable handoff completed schema validation under that same exclusion.
        Self::open_at_without_recovery(
            workspace_root,
            db_dir,
            config,
            SchemaOpenMode::Existing,
            true,
        )
    }

    fn open_at_without_recovery(
        workspace_root: PathBuf,
        db_dir: PathBuf,
        config: TrailConfig,
        schema_mode: SchemaOpenMode,
        writer_exclusion_held: bool,
    ) -> Result<Self> {
        let sqlite_path = db_dir.join(DB_RELATIVE_PATH);
        let validated_schema = match schema_mode {
            SchemaOpenMode::FreshCreate => None,
            SchemaOpenMode::Existing if writer_exclusion_held => None,
            SchemaOpenMode::Existing => {
                Some(Self::with_write_lock_wait(Duration::from_secs(10), || {
                    preflight_existing_schema(&sqlite_path, &config.storage.prolly_backend)
                })?)
            }
        };
        if schema_mode == SchemaOpenMode::FreshCreate {
            fs::create_dir_all(db_dir.join("index"))?;
        }
        #[cfg(test)]
        if schema_mode == SchemaOpenMode::Existing && !writer_exclusion_held {
            run_schema_handoff_hook(&db_dir);
        }
        if let Some(validated) = &validated_schema {
            validated.verify_unchanged()?;
        }
        register_sqlite_vec_extension()?;
        let operation_metrics =
            operation_metrics_are_enabled().then(|| Arc::new(OperationMetricsState::default()));
        #[cfg(test)]
        if validated_schema.is_some() {
            run_schema_primary_open_hook(&db_dir);
        }
        let conn = match schema_mode {
            SchemaOpenMode::FreshCreate => Connection::open(&sqlite_path)?,
            SchemaOpenMode::Existing => Connection::open_with_flags(
                &sqlite_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                    | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?,
        };
        if let Some(validated) = &validated_schema {
            validated.verify_connection(&conn)?;
        }
        conn.set_db_config(
            rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
            true,
        )?;
        #[cfg(test)]
        if validated_schema.is_some() {
            run_schema_prolly_open_hook(&db_dir);
        }
        let store = open_prolly_store(
            &config,
            &sqlite_path,
            operation_metrics.clone(),
            schema_mode,
            validated_schema.as_ref(),
        )?;
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
        let lock_path = self.db_dir.join("lock");
        if lock_path.exists()
            && fs::read_to_string(&lock_path)
                .ok()
                .and_then(|holder| {
                    holder.split_whitespace().find_map(|part| {
                        part.strip_prefix("pid=")
                            .and_then(|value| value.parse::<u32>().ok())
                    })
                })
                .is_none()
        {
            // Legacy or manually managed writer locks have always allowed a read-only open.
            // Defer recovery until a later open after that writer releases its lock.
            return Ok(());
        }
        match Self::with_write_lock_wait(Duration::from_secs(30), || {
            let _lock = self.acquire_write_lock()?;
            self.recover_after_open_under_write_lock()
        }) {
            Err(Error::WorkspaceLocked(_)) => Ok(()),
            result => result,
        }
    }

    fn recover_after_open_under_write_lock(&self) -> Result<()> {
        if self.has_pending_path_index_derived_repairs()? {
            self.drain_pending_path_index_derived_repairs()?;
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
        acquire_workspace_lock(&self.db_dir)
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

#[cfg(test)]
mod schema_handoff_tests {
    use super::*;
    use std::io::Write;
    use std::process::{Child, Stdio};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Barrier,
    };

    #[cfg(unix)]
    fn assert_open_replacement_is_rejected_before_mutation(
        install: impl FnOnce(Box<dyn FnOnce(&Path)>),
    ) {
        let root = tempfile::tempdir().unwrap();
        Trail::init(root.path(), "main", InitImportMode::Empty, false).unwrap();
        let db_path = root.path().join(".trail").join(DB_RELATIVE_PATH);
        let original_bytes = fs::read(&db_path).unwrap();
        let replacement = db_path.with_extension("replacement.sqlite");
        fs::copy(&db_path, &replacement).unwrap();
        let replacement_bytes = fs::read(&replacement).unwrap();
        let retained = db_path.with_extension("validated.sqlite");
        let hook_db = db_path.clone();
        let hook_replacement = replacement.clone();
        let hook_retained = retained.clone();
        install(Box::new(move |_| {
            fs::rename(&hook_db, &hook_retained).unwrap();
            fs::rename(&hook_replacement, &hook_db).unwrap();
        }));
        let error = Trail::open(root.path()).err();
        assert!(
            matches!(error, Some(Error::SchemaReinitializeRequired { .. })),
            "replacement rejection returned {error:?}"
        );
        assert_eq!(fs::read(retained).unwrap(), original_bytes);
        assert_eq!(fs::read(db_path).unwrap(), replacement_bytes);
    }

    #[cfg(unix)]
    #[test]
    fn primary_sqlite_open_binds_the_validated_inode_before_any_statement() {
        assert_open_replacement_is_rejected_before_mutation(|hook| {
            install_schema_primary_open_hook(hook)
        });
    }

    #[cfg(unix)]
    #[test]
    fn prolly_sqlite_open_binds_the_validated_inode_before_any_pragma() {
        assert_open_replacement_is_rejected_before_mutation(|hook| {
            install_schema_prolly_open_hook(hook)
        });
    }

    const CROSS_PROCESS_TEST: &str =
        "db::core::init::schema_handoff_tests::cross_process_schema_validation_fanout";

    fn wait_for_path(path: &Path, deadline: Duration) {
        let started = Instant::now();
        while !path.exists() {
            assert!(
                started.elapsed() < deadline,
                "timed out waiting for {}",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn validation_count(path: &Path) -> usize {
        fs::read_to_string(path).unwrap_or_default().lines().count()
    }

    fn spawn_schema_children(
        root: &Path,
        counter: &Path,
        go: &Path,
        count: usize,
        mode: &str,
        configure: impl Fn(&mut Command),
    ) -> Vec<Child> {
        let ready_dir = go.with_extension("ready");
        fs::create_dir(&ready_dir).unwrap();
        let children = (0..count)
            .map(|index| {
                let ready = ready_dir.join(format!("{index}.ready"));
                let mut command = Command::new(std::env::current_exe().unwrap());
                command
                    .args(["--exact", CROSS_PROCESS_TEST, "--nocapture"])
                    .env("RUST_TEST_THREADS", "1")
                    .env("TRAIL_TEST_SCHEMA_CHILD", mode)
                    .env("TRAIL_TEST_SCHEMA_WORKSPACE", root)
                    .env("TRAIL_TEST_SCHEMA_GO", go)
                    .env("TRAIL_TEST_SCHEMA_READY", &ready)
                    .env("TRAIL_TEST_SCHEMA_VALIDATION_COUNTER", counter)
                    .stdout(Stdio::null())
                    .stderr(Stdio::inherit());
                configure(&mut command);
                command.spawn().unwrap()
            })
            .collect::<Vec<_>>();
        let started = Instant::now();
        while fs::read_dir(&ready_dir).unwrap().count() != count {
            assert!(started.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(5));
        }
        children
    }

    fn release_and_wait(mut children: Vec<Child>, go: &Path) {
        fs::write(go, []).unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        for child in &mut children {
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert!(status.success(), "schema validation child {status}");
                    break;
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("schema validation child timed out");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    fn run_schema_child() -> bool {
        let Ok(mode) = std::env::var("TRAIL_TEST_SCHEMA_CHILD") else {
            return false;
        };
        let root = PathBuf::from(std::env::var_os("TRAIL_TEST_SCHEMA_WORKSPACE").unwrap());
        let go = PathBuf::from(std::env::var_os("TRAIL_TEST_SCHEMA_GO").unwrap());
        fs::write(std::env::var_os("TRAIL_TEST_SCHEMA_READY").unwrap(), []).unwrap();
        wait_for_path(&go, Duration::from_secs(10));
        let result = Trail::open(root);
        match mode.as_str() {
            "success" | "crash" => result.unwrap(),
            "failure" => {
                let error = match result {
                    Ok(_) => panic!("injected schema failure opened"),
                    Err(error) => error,
                };
                assert!(matches!(error, Error::SchemaReinitializeRequired { .. }));
                assert!(error.to_string().contains("cross-process injected failure"));
                return true;
            }
            "schema-failure" => {
                assert!(matches!(
                    result,
                    Err(Error::SchemaReinitializeRequired { .. })
                ));
                return true;
            }
            other => panic!("unknown schema child mode {other}"),
        };
        true
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn cross_process_schema_validation_fanout() {
        if run_schema_child() {
            return;
        }

        let root = tempfile::tempdir().unwrap();
        Trail::init(root.path(), "main", InitImportMode::Empty, false).unwrap();
        let writer = Connection::open(root.path().join(".trail").join(DB_RELATIVE_PATH)).unwrap();
        writer
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
            .unwrap();
        writer
            .execute(
                "UPDATE schema_meta SET value='success-wave' WHERE key='app.version'",
                [],
            )
            .unwrap();
        let counter = root.path().join("success-count");
        let go = root.path().join("success-go");
        let started_marker = root.path().join("success-started");
        let started = Instant::now();
        let children =
            spawn_schema_children(root.path(), &counter, &go, 16, "success", |command| {
                command
                    .env("TRAIL_TEST_SCHEMA_VALIDATION_DELAY_MS", "1000")
                    .env("TRAIL_TEST_SCHEMA_VALIDATION_STARTED", &started_marker);
            });
        fs::write(&go, []).unwrap();
        wait_for_path(&started_marker, Duration::from_secs(5));
        let exclusion =
            File::open(root.path().join(".trail/index").join(SCHEMA_EXCLUSION_FILE)).unwrap();
        assert_eq!(
            rustix::fs::flock(
                &exclusion,
                rustix::fs::FlockOperation::NonBlockingLockExclusive,
            )
            .unwrap_err(),
            rustix::io::Errno::AGAIN,
            "external schema writer was not excluded during validation"
        );
        release_and_wait(children, &go);
        assert_eq!(validation_count(&counter), 1);
        assert!(started.elapsed() < Duration::from_secs(3));

        writer
            .execute(
                "UPDATE schema_meta SET value='failure-wave' WHERE key='app.version'",
                [],
            )
            .unwrap();
        let failure_counter = root.path().join("failure-count");
        let failure_go = root.path().join("failure-go");
        let children = spawn_schema_children(
            root.path(),
            &failure_counter,
            &failure_go,
            16,
            "failure",
            |command| {
                command
                    .env(
                        "TRAIL_TEST_SCHEMA_VALIDATION_FAIL",
                        "cross-process injected failure",
                    )
                    .env("TRAIL_TEST_SCHEMA_VALIDATION_DELAY_MS", "500");
            },
        );
        release_and_wait(children, &failure_go);
        assert_eq!(validation_count(&failure_counter), 1);

        writer
            .execute(
                "UPDATE schema_meta SET value='crash-wave' WHERE key='app.version'",
                [],
            )
            .unwrap();
        let crash_counter = root.path().join("crash-count");
        let crash_go = root.path().join("crash-go");
        let crash_once = root.path().join("crash-once");
        let mut children = spawn_schema_children(
            root.path(),
            &crash_counter,
            &crash_go,
            16,
            "crash",
            |command| {
                command.env("TRAIL_TEST_SCHEMA_VALIDATION_CRASH_ONCE", &crash_once);
            },
        );
        fs::write(&crash_go, []).unwrap();
        wait_for_path(&crash_once, Duration::from_secs(5));
        let leader_pid = fs::read_to_string(&crash_once)
            .unwrap()
            .trim()
            .parse::<u32>()
            .unwrap();
        let leader = children
            .iter_mut()
            .find(|child| child.id() == leader_pid)
            .unwrap();
        leader.kill().unwrap();
        let _ = leader.wait().unwrap();
        children.retain(|child| child.id() != leader_pid);
        release_and_wait(children, &crash_go);
        assert_eq!(
            validation_count(&crash_counter),
            2,
            "crash validation leaders: {}",
            fs::read_to_string(&crash_counter).unwrap_or_default()
        );

        writer
            .execute(
                "UPDATE schema_meta SET value='later-generation' WHERE key='app.version'",
                [],
            )
            .unwrap();
        let later_counter = root.path().join("later-count");
        let later_go = root.path().join("later-go");
        let children = spawn_schema_children(
            root.path(),
            &later_counter,
            &later_go,
            16,
            "success",
            |command| {
                command.env("TRAIL_TEST_SCHEMA_VALIDATION_DELAY_MS", "500");
            },
        );
        release_and_wait(children, &later_go);
        assert_eq!(validation_count(&later_counter), 1);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn ipc_result_is_bound_to_exact_main_wal_shm_generation_and_backend() {
        let mut generation = SchemaGeneration(
            ["", "-wal", "-shm", "-journal"]
                .into_iter()
                .enumerate()
                .map(|(index, suffix)| SchemaFileGeneration {
                    suffix,
                    present: true,
                    device: 10,
                    inode: 20 + index as u64,
                    length: 30 + index as u64,
                    modified_seconds: 40,
                    modified_nanoseconds: 50,
                    changed_seconds: 60,
                    changed_nanoseconds: 70,
                })
                .collect(),
        );
        let original_key = schema_generation_key(&generation);
        let response = schema_validation_wire_result(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            &original_key,
            "sqlite",
            &CrossProcessSchemaValidationOutcome::Success,
        );
        for suffix in ["", "-wal", "-shm"] {
            let index = generation
                .0
                .iter()
                .position(|file| file.suffix == suffix)
                .unwrap();
            generation.0[index].inode += 1;
            let changed_key = schema_generation_key(&generation);
            assert!(parse_schema_validation_wire_result(
                &response,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                &changed_key,
                "sqlite",
            )
            .is_none());
            generation.0[index].inode -= 1;
        }
        assert!(parse_schema_validation_wire_result(
            &response,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            &original_key,
            "slatedb",
        )
        .is_none());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn ipc_rejects_spoofed_announcement_and_nonleader_peer() {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::UnixListener;

        let root = tempfile::tempdir().unwrap();
        let announcement = root.path().join("spoof.announce");
        fs::write(
            &announcement,
            format!(
                "trail-schema-validation-announce-v1\n{}\nwrong-namespace\n{}\n{}\n",
                std::process::id(),
                "a".repeat(64),
                hex::encode(
                    root.path()
                        .join("spoof.socket")
                        .as_os_str()
                        .as_encoded_bytes()
                ),
            ),
        )
        .unwrap();
        fs::set_permissions(&announcement, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(read_schema_validation_announcement(
            &announcement,
            std::process::id(),
            "expected-namespace",
        )
        .is_none());

        let socket = root.path().join("peer.socket");
        let listener = UnixListener::bind(&socket).unwrap();
        let accept = std::thread::spawn(move || listener.accept().unwrap());
        assert!(request_schema_validation_result(
            &socket,
            &"b".repeat(64),
            &"c".repeat(64),
            "sqlite",
            std::process::id().wrapping_add(1),
        )
        .is_none());
        drop(accept.join().unwrap());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn real_main_wal_and_shm_changes_each_start_one_fresh_multiprocess_validation() {
        fn open_stable_writer(db_path: &Path, label: &str) -> Connection {
            let writer = Connection::open(db_path).unwrap();
            writer
                .execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
                .unwrap();
            writer
                .execute(
                    "UPDATE schema_meta SET value=?1 WHERE key='app.version'",
                    [label],
                )
                .unwrap();
            writer
        }

        for (index, suffix) in ["", "-wal", "-shm"].into_iter().enumerate() {
            let root = tempfile::tempdir().unwrap();
            Trail::init(root.path(), "main", InitImportMode::Empty, false).unwrap();
            let db_dir = root.path().join(".trail");
            let db_path = db_dir.join(DB_RELATIVE_PATH);
            let writer = open_stable_writer(&db_path, &format!("before-{index}"));
            let before = schema_generation(&db_path).unwrap();
            assert!(before
                .0
                .iter()
                .find(|file| file.suffix == suffix)
                .is_some_and(|file| file.present));

            let counter = root.path().join(format!("{index}-count"));
            let initial_go = root.path().join(format!("{index}-initial-go"));
            let children = spawn_schema_children(
                root.path(),
                &counter,
                &initial_go,
                16,
                "success",
                |command| {
                    command.env("TRAIL_TEST_SCHEMA_VALIDATION_DELAY_MS", "100");
                },
            );
            release_and_wait(children, &initial_go);
            assert_eq!(validation_count(&counter), 1);
            drop(writer);

            let lock = acquire_workspace_lock(&db_dir).unwrap();
            if suffix.is_empty() {
                let replacement = db_path.with_extension("validation-replacement");
                fs::copy(&db_path, &replacement).unwrap();
                fs::rename(&replacement, &db_path).unwrap();
            } else {
                let mut sidecar = db_path.as_os_str().to_os_string();
                sidecar.push(suffix);
                match fs::remove_file(PathBuf::from(sidecar)) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => panic!("failed to rotate {suffix}: {error}"),
                }
            }
            let writer = open_stable_writer(&db_path, &format!("after-{index}"));
            if suffix == "-shm" {
                let mut shm = db_path.as_os_str().to_os_string();
                shm.push("-shm");
                assert!(PathBuf::from(shm).exists());
            }
            let after = schema_generation(&db_path).unwrap();
            let before_file = before.0.iter().find(|file| file.suffix == suffix).unwrap();
            let after_file = after.0.iter().find(|file| file.suffix == suffix).unwrap();
            assert_ne!(
                before_file, after_file,
                "{suffix} generation did not change"
            );
            drop(lock);

            let fresh_go = root.path().join(format!("{index}-fresh-go"));
            let children =
                spawn_schema_children(root.path(), &counter, &fresh_go, 16, "success", |command| {
                    command.env("TRAIL_TEST_SCHEMA_VALIDATION_DELAY_MS", "100");
                });
            release_and_wait(children, &fresh_go);
            assert_eq!(
                validation_count(&counter),
                2,
                "{suffix} change did not start exactly one fresh validation"
            );
            drop(writer);
        }

        let root = tempfile::tempdir().unwrap();
        Trail::init(root.path(), "main", InitImportMode::Empty, false).unwrap();
        let db_dir = root.path().join(".trail");
        let db_path = db_dir.join(DB_RELATIVE_PATH);
        let writer = open_stable_writer(&db_path, "backend-before");
        let counter = root.path().join("backend-count");
        let sqlite_go = root.path().join("backend-sqlite-go");
        let children =
            spawn_schema_children(root.path(), &counter, &sqlite_go, 16, "success", |_| {});
        release_and_wait(children, &sqlite_go);
        assert_eq!(validation_count(&counter), 1);
        drop(writer);
        let lock = acquire_workspace_lock(&db_dir).unwrap();
        let config_path = db_dir.join(CONFIG_FILE);
        let config = fs::read_to_string(&config_path).unwrap().replace(
            "prolly_backend = \"sqlite\"",
            "prolly_backend = \"slatedb\"",
        );
        fs::write(&config_path, config).unwrap();
        drop(lock);
        let slatedb_go = root.path().join("backend-slatedb-go");
        let children = spawn_schema_children(
            root.path(),
            &counter,
            &slatedb_go,
            16,
            "schema-failure",
            |_| {},
        );
        release_and_wait(children, &slatedb_go);
        assert_eq!(
            validation_count(&counter),
            2,
            "backend change reused the sqlite validation result"
        );
    }

    #[test]
    fn schema_snapshot_exclusion_is_retained_through_mutable_handoff() {
        let root = tempfile::tempdir().unwrap();
        Trail::init(root.path(), "main", InitImportMode::Empty, false).unwrap();
        let observed = Arc::new(AtomicBool::new(false));
        let observed_in_hook = observed.clone();
        install_schema_handoff_hook(move |db_dir| {
            assert!(matches!(
                acquire_workspace_lock(db_dir),
                Err(Error::WorkspaceLocked(_))
            ));
            observed_in_hook.store(true, Ordering::SeqCst);
        });
        let reopened = Trail::open(root.path()).unwrap();
        drop(reopened);
        assert!(observed.load(Ordering::SeqCst));
    }

    #[test]
    fn one_hundred_ordinary_opens_share_one_unchanged_generation_validation() {
        let root = tempfile::tempdir().unwrap();
        Trail::init(root.path(), "main", InitImportMode::Empty, false).unwrap();
        let db_path =
            canonicalize_lossless(&root.path().join(".trail").join(DB_RELATIVE_PATH)).unwrap();
        let writer = Connection::open(&db_path).unwrap();
        writer
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
            .unwrap();
        writer
            .execute(
                "UPDATE schema_meta SET value='singleflight' WHERE key='app.version'",
                [],
            )
            .unwrap();
        let barrier = Arc::new(Barrier::new(100));
        let started = Instant::now();
        let handles = (0..100)
            .map(|_| {
                let root = root.path().to_path_buf();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    Trail::open(root).map(drop)
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap().unwrap();
        }
        assert_eq!(schema_validation_count(&db_path), 1);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "100 shared schema opens exceeded the five second budget"
        );
        drop(writer);
    }

    #[test]
    fn generation_replacement_or_sidecar_change_invalidates_mutable_handoff() {
        for changed_suffix in ["", "-wal", "-shm"] {
            let root = tempfile::tempdir().unwrap();
            Trail::init(root.path(), "main", InitImportMode::Empty, false).unwrap();
            let db_path = root.path().join(".trail").join(DB_RELATIVE_PATH);
            let writer = Connection::open(&db_path).unwrap();
            writer
                .execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
                .unwrap();
            writer
                .execute(
                    "UPDATE schema_meta SET value='toctou' WHERE key='app.version'",
                    [],
                )
                .unwrap();
            let mut changed_path = db_path.as_os_str().to_os_string();
            changed_path.push(changed_suffix);
            let changed_path = PathBuf::from(changed_path);
            assert!(changed_path.exists(), "missing sidecar {changed_suffix}");
            install_schema_handoff_hook(move |_| {
                if changed_suffix.is_empty() {
                    let replacement = changed_path.with_extension("replacement");
                    fs::copy(&changed_path, &replacement).unwrap();
                    fs::rename(replacement, &changed_path).unwrap();
                } else {
                    OpenOptions::new()
                        .append(true)
                        .open(&changed_path)
                        .unwrap()
                        .write_all(b"generation-change")
                        .unwrap();
                }
            });
            assert!(matches!(
                Trail::open(root.path()),
                Err(Error::SchemaReinitializeRequired { .. })
            ));
            drop(writer);
        }
    }

    #[test]
    fn schema_validation_leader_failure_propagates_and_next_open_retries() {
        let root = tempfile::tempdir().unwrap();
        Trail::init(root.path(), "main", InitImportMode::Empty, false).unwrap();
        let db_path =
            canonicalize_lossless(&root.path().join(".trail").join(DB_RELATIVE_PATH)).unwrap();
        fail_next_schema_validation(&db_path);
        let barrier = Arc::new(Barrier::new(16));
        let handles = (0..16)
            .map(|_| {
                let root = root.path().to_path_buf();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    Trail::open(root)
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            assert!(matches!(
                handle.join().unwrap(),
                Err(Error::SchemaReinitializeRequired { .. })
            ));
        }
        Trail::open(root.path()).unwrap();
        assert_eq!(schema_validation_count(&db_path), 2);
    }
}
