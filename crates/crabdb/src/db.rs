use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::fs::OpenOptions;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::{symlink as symlink_file, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ignore::WalkBuilder;
use prolly::{BatchBuilder, Cid, Config, Diff, Encoding, Prolly, SqliteStore, Tree};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use similar::{ChangeTag, TextDiff};

use crate::error::{cbor, from_cbor, Error, Result};
use crate::ids::{
    sha256_hex, AnchorId, ChangeId, FileId, LineId, MessageId, ObjectId, WorkspaceId,
};
use crate::model::*;

const CONFIG_FILE: &str = "config.toml";
const HEAD_FILE: &str = "HEAD";
const DB_RELATIVE_PATH: &str = "index/crabdb.sqlite";
const CRABDB_SCHEMA_VERSION: i64 = 1;
const SCHEMA_META_VERSION_KEY: &str = "schema.version";
const SCHEMA_META_APP_VERSION_KEY: &str = "app.version";
const MAIN_REF_PREFIX: &str = "refs/branches/";
const AGENT_REF_PREFIX: &str = "refs/agents/";
const ROOT_OBJECT_VERSION: u16 = 1;
const TEXT_OBJECT_VERSION: u16 = 1;
const OP_OBJECT_VERSION: u16 = 1;
const BLOB_OBJECT_VERSION: u16 = 1;
const MESSAGE_OBJECT_VERSION: u16 = 1;
const ANCHOR_OBJECT_VERSION: u16 = 1;
const ORDER_KEY_STEP: u64 = 1024;
const AGENT_TEST_OUTPUT_PREVIEW_BYTES: usize = 64 * 1024;
const DEFAULT_CRABIGNORE_PATTERNS: &[&str] = &[
    ".crabdb/",
    ".git/",
    ".env",
    ".env.*",
    "*.pem",
    "*.key",
    "*.p12",
    "*.pfx",
    "id_rsa",
    "id_ed25519",
    "node_modules/",
    "target/",
    "dist/",
    "build/",
    "coverage/",
];

pub struct CrabDb {
    workspace_root: PathBuf,
    db_dir: PathBuf,
    conn: Connection,
    store: Arc<SqliteStore>,
    prolly: Prolly<Arc<SqliteStore>>,
    config: CrabConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitImportMode {
    Empty,
    GitTracked,
    WorkingTree,
}

#[derive(Debug, Clone)]
struct DiskFile {
    path: String,
    bytes: Vec<u8>,
    executable: bool,
}

#[derive(Debug)]
struct RootBuildResult {
    root_id: ObjectId,
    files: BTreeMap<String, FileEntry>,
    stats: ImportStats,
}

#[derive(Debug)]
struct FileBuildResult {
    entry: FileEntry,
    line_changes: Vec<LineChange>,
}

#[derive(Debug)]
struct TextBuildResult {
    object_id: ObjectId,
    line_changes: Vec<LineChange>,
}

#[derive(Debug, Clone)]
struct RootDiff {
    changes: Vec<FileChange>,
    summaries: Vec<FileDiffSummary>,
}

#[derive(Debug)]
struct CommandRunResult {
    success: bool,
    exit_code: Option<i32>,
    timed_out: bool,
    duration_ms: u64,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

#[derive(Debug, Clone)]
struct AgentTraceSpanBuilder {
    span_id: String,
    trace_id: String,
    agent_id: String,
    session_id: Option<String>,
    turn_id: Option<String>,
    parent_span_id: Option<String>,
    span_type: String,
    name: String,
    started_event_id: String,
    started_at: i64,
    attributes: Option<serde_json::Value>,
    ended_event_id: Option<String>,
    ended_at: Option<i64>,
    status: Option<String>,
    result: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, serde::Deserialize)]
struct BackupManifest {
    format_version: u16,
    crabdb_version: String,
    created_at: i64,
    source_workspace: String,
    source_db_dir: String,
    workspace_id: WorkspaceId,
    branch: String,
    ref_count: u64,
    operation_count: u64,
    sqlite_bytes: u64,
    sqlite_sha256: String,
    worktree_bytes: u64,
}

#[derive(Debug)]
struct PendingLineMerge {
    path: String,
    target_entry: FileEntry,
    lines: Vec<LineEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct LineGap {
    previous: Option<String>,
    next: Option<String>,
}

#[derive(Debug, Clone)]
struct OperationObject {
    object_id: ObjectId,
    operation: Operation,
}

#[derive(Debug, Clone)]
struct DiskManifest {
    kind: FileKind,
    executable: bool,
    content_hash: String,
}

#[derive(Debug, Clone)]
struct MergeContext {
    base_change: ChangeId,
    left_change: ChangeId,
    right_change: ChangeId,
}

#[derive(Debug, Clone)]
struct PendingConflictMerge {
    merge_id: String,
    queue_id: Option<String>,
    source_ref: String,
    target_ref: String,
    base_change: ChangeId,
    left_change: ChangeId,
    right_change: ChangeId,
}

#[derive(Debug, Clone)]
struct GitState {
    head: Option<String>,
    dirty: bool,
}

#[derive(Debug, Default)]
struct GitTreeNode {
    blobs: BTreeMap<String, GitBlobEntry>,
    dirs: BTreeMap<String, GitTreeNode>,
}

#[derive(Debug)]
struct GitBlobEntry {
    mode: &'static str,
    oid: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConflictTake {
    Source,
    Target,
}

#[derive(Debug)]
enum ConflictResolution {
    Take(ConflictTake),
    Manual(ConflictManualResolution),
}

#[derive(Debug)]
struct WorkspaceLock {
    path: PathBuf,
}

impl Drop for WorkspaceLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

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

    fn open_at(workspace_root: PathBuf, db_dir: PathBuf, config: CrabConfig) -> Result<Self> {
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

    fn acquire_write_lock(&self) -> Result<WorkspaceLock> {
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

    pub fn ignore_list(&self) -> Result<IgnoreListReport> {
        let path = self.workspace_root.join(".crabignore");
        let patterns = read_ignore_patterns(&path)?;
        Ok(IgnoreListReport {
            path: path.to_string_lossy().to_string(),
            patterns,
        })
    }

    pub fn ignore_add(&mut self, pattern: &str) -> Result<IgnoreAddReport> {
        let _lock = self.acquire_write_lock()?;
        let pattern = normalize_ignore_pattern(pattern)?;
        write_default_crabignore(&self.workspace_root)?;
        let path = self.workspace_root.join(".crabignore");
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
        let path = self.workspace_root.join(".crabignore");
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
        let mut builder = ignore::gitignore::GitignoreBuilder::new(&self.workspace_root);
        let crabignore = self.workspace_root.join(".crabignore");
        if crabignore.exists() {
            if let Some(err) = builder.add(crabignore) {
                return Err(Error::InvalidInput(err.to_string()));
            }
        }
        let gitignore = self.workspace_root.join(".gitignore");
        if gitignore.exists() {
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

    pub fn guardrail_check(
        &self,
        agent: Option<&str>,
        action: &str,
        summary: Option<&str>,
        payload: Option<serde_json::Value>,
        paths: &[String],
    ) -> Result<GuardrailCheckReport> {
        let action = action.trim();
        if action.is_empty() {
            return Err(Error::InvalidInput(
                "guardrail action cannot be empty".to_string(),
            ));
        }
        let summary = summary
            .map(str::trim)
            .filter(|summary| !summary.is_empty())
            .map(redact_sensitive_text);
        let payload = payload.map(redact_sensitive_json);
        let agent_details = agent.map(|agent| self.agent_details(agent)).transpose()?;
        let agent_name = agent_details
            .as_ref()
            .map(|details| details.record.name.clone())
            .or_else(|| agent.map(str::to_string));
        let approvals = if let Some(agent) = agent {
            self.list_agent_approvals(Some(agent), None)?
        } else {
            Vec::new()
        };
        let pending_approvals = approvals
            .iter()
            .filter(|approval| approval.status == "pending")
            .cloned()
            .collect::<Vec<_>>();

        let mut reasons = Vec::new();
        let mut path_checks = Vec::new();
        for path in paths {
            let check = self.ignore_check(path)?;
            if check.ignored {
                match check.source.as_deref() {
                    Some("hardcoded") => reasons.push(guardrail_reason(
                        "blocked_path",
                        "blocked",
                        format!(
                            "`{}` is protected by CrabDB's hardcoded private path denylist",
                            check.path
                        ),
                        Some(serde_json::json!({ "path": check.path, "source": check.source })),
                    )),
                    _ => reasons.push(guardrail_reason(
                        "ignored_path",
                        "approval_required",
                        format!(
                            "`{}` is ignored by workspace policy and needs explicit approval or allow_ignored",
                            check.path
                        ),
                        Some(serde_json::json!({ "path": check.path, "source": check.source })),
                    )),
                }
            }
            path_checks.push(check);
        }

        let risk_text = guardrail_risk_text(action, summary.as_deref(), payload.as_ref());
        for reason in classify_guardrail_action(&risk_text) {
            reasons.push(reason);
        }
        apply_configured_guardrail_policy(
            &mut reasons,
            &self.config.guardrails.policy,
            action,
            &risk_text,
            &path_checks,
        )?;

        let matching_pending = pending_approvals
            .iter()
            .filter(|approval| approval.action == action)
            .map(|approval| approval.approval_id.clone())
            .collect::<Vec<_>>();
        if !matching_pending.is_empty() {
            reasons.push(guardrail_reason(
                "pending_approval",
                "approval_required",
                "matching human approval is already pending",
                Some(serde_json::json!({ "approval_ids": matching_pending })),
            ));
        }

        let latest_decided_matching_approval = approvals.iter().find(|approval| {
            approval.action == action && matches!(approval.status.as_str(), "approved" | "rejected")
        });
        let mut satisfied_approvals = Vec::new();
        if matching_pending.is_empty() {
            if let Some(approval) = latest_decided_matching_approval {
                match approval.status.as_str() {
                    "approved" => {
                        let approval_ids = vec![approval.approval_id.clone()];
                        for reason in reasons
                            .iter_mut()
                            .filter(|reason| reason.severity == "approval_required")
                        {
                            let original_details = reason.details.take();
                            reason.severity = "allowed".to_string();
                            reason.details = Some(serde_json::json!({
                                "approval_ids": approval_ids.clone(),
                                "original_severity": "approval_required",
                                "original_details": original_details
                            }));
                        }
                        reasons.push(guardrail_reason(
                            "approval_satisfied",
                            "allowed",
                            "matching approved human approval satisfies approval-required guardrails",
                            Some(serde_json::json!({ "approval_ids": approval_ids.clone() })),
                        ));
                        satisfied_approvals.push(approval.clone());
                    }
                    "rejected" => {
                        reasons.push(guardrail_reason(
                            "approval_rejected",
                            "blocked",
                            "matching human approval was rejected",
                            Some(serde_json::json!({
                                "approval_id": approval.approval_id.clone(),
                                "reviewer": approval.reviewer.clone(),
                                "note": approval.note.clone()
                            })),
                        ));
                    }
                    _ => {}
                }
            }
        }

        let decision = if reasons.iter().any(|reason| reason.severity == "blocked") {
            "blocked"
        } else if reasons
            .iter()
            .any(|reason| reason.severity == "approval_required")
        {
            "approval_required"
        } else {
            "allowed"
        }
        .to_string();

        let approval_request =
            (decision == "approval_required").then(|| GuardrailApprovalRequest {
                agent: agent_name,
                action: action.to_string(),
                summary: summary
                    .clone()
                    .unwrap_or_else(|| format!("Approve `{action}`")),
                payload: payload.clone(),
            });

        Ok(GuardrailCheckReport {
            agent: agent_details,
            action: action.to_string(),
            summary,
            decision,
            reasons,
            path_checks,
            pending_approvals,
            satisfied_approvals,
            approval_request,
        })
    }

    pub fn status(&self, branch: Option<&str>) -> Result<StatusReport> {
        let branch = branch.map(str::to_string).unwrap_or(self.current_branch()?);
        let head = self.resolve_branch_ref(&branch)?;
        let head_files = self.load_root_files(&head.root_id)?;
        let disk_files = self.scan_worktree_files()?;
        let disk_manifest = self.disk_manifest(&disk_files);
        let changed_paths = self.diff_file_maps_to_manifest(&head_files, &disk_manifest);
        let worktree_state = worktree_state_from_changes(&changed_paths);
        Ok(StatusReport {
            branch,
            head,
            worktree_state,
            changed_paths,
        })
    }

    pub fn doctor(&self) -> Result<DoctorReport> {
        let mut checks = Vec::new();

        let workspace_path = self.workspace_root.to_string_lossy().to_string();
        if self.workspace_root.is_dir() {
            checks.push(doctor_check(
                "workspace",
                "ok",
                format!("workspace root is available at {workspace_path}"),
                Some(serde_json::json!({ "path": workspace_path })),
            ));
        } else {
            checks.push(doctor_check(
                "workspace",
                "error",
                format!("workspace root is missing at {workspace_path}"),
                Some(serde_json::json!({ "path": workspace_path })),
            ));
        }

        let sqlite_path = self.db_dir.join(DB_RELATIVE_PATH);
        let db_path = self.db_dir.to_string_lossy().to_string();
        let sqlite_path_text = sqlite_path.to_string_lossy().to_string();
        if self.db_dir.is_dir() && sqlite_path.is_file() {
            checks.push(doctor_check(
                "database",
                "ok",
                "database directory and SQLite store are present",
                Some(serde_json::json!({
                    "db_dir": db_path,
                    "sqlite": sqlite_path_text
                })),
            ));
        } else {
            checks.push(doctor_check(
                "database",
                "error",
                "database directory or SQLite store is missing",
                Some(serde_json::json!({
                    "db_dir": db_path,
                    "db_dir_exists": self.db_dir.is_dir(),
                    "sqlite": sqlite_path_text,
                    "sqlite_exists": sqlite_path.is_file()
                })),
            ));
        }

        match (
            self.schema_user_version(),
            self.schema_meta_value(SCHEMA_META_VERSION_KEY),
        ) {
            (Ok(user_version), Ok(meta_version)) => {
                let meta_version_int = meta_version
                    .as_deref()
                    .and_then(|value| value.parse::<i64>().ok());
                let details = Some(serde_json::json!({
                    "supported_version": CRABDB_SCHEMA_VERSION,
                    "sqlite_user_version": user_version,
                    "metadata_version": meta_version,
                    "app_version": self.schema_meta_value(SCHEMA_META_APP_VERSION_KEY).ok().flatten()
                }));
                if user_version == CRABDB_SCHEMA_VERSION
                    && meta_version_int == Some(CRABDB_SCHEMA_VERSION)
                {
                    checks.push(doctor_check(
                        "schema_version",
                        "ok",
                        format!("schema version {CRABDB_SCHEMA_VERSION} is current"),
                        details,
                    ));
                } else if user_version > CRABDB_SCHEMA_VERSION
                    || meta_version_int.is_some_and(|version| version > CRABDB_SCHEMA_VERSION)
                {
                    checks.push(doctor_check(
                        "schema_version",
                        "error",
                        "workspace schema is newer than this CrabDB binary",
                        details,
                    ));
                } else {
                    checks.push(doctor_check(
                        "schema_version",
                        "warning",
                        "schema metadata is missing or older than the current version",
                        details,
                    ));
                }
            }
            (Err(err), _) | (_, Err(err)) => checks.push(doctor_check(
                "schema_version",
                "error",
                format!("failed to inspect schema version: {err}"),
                None,
            )),
        }

        match self.current_branch() {
            Ok(branch) => match self.resolve_branch_ref(&branch) {
                Ok(head) => checks.push(doctor_check(
                    "current_branch",
                    "ok",
                    format!("current branch `{branch}` resolves to {}", head.change_id.0),
                    Some(serde_json::json!({
                        "branch": branch,
                        "change_id": head.change_id.0,
                        "root_id": head.root_id.0
                    })),
                )),
                Err(err) => checks.push(doctor_check(
                    "current_branch",
                    "error",
                    format!("current branch `{branch}` does not resolve: {err}"),
                    Some(serde_json::json!({ "branch": branch })),
                )),
            },
            Err(err) => checks.push(doctor_check(
                "current_branch",
                "error",
                format!("could not read current branch: {err}"),
                None,
            )),
        }

        let crabignore_path = self.workspace_root.join(".crabignore");
        match read_ignore_patterns(&crabignore_path) {
            Ok(patterns) if crabignore_path.exists() => {
                let active: BTreeSet<&str> = patterns
                    .iter()
                    .map(|pattern| pattern.pattern.as_str())
                    .collect();
                let missing: Vec<&str> = DEFAULT_CRABIGNORE_PATTERNS
                    .iter()
                    .copied()
                    .filter(|pattern| !active.contains(pattern))
                    .collect();
                if missing.is_empty() {
                    checks.push(doctor_check(
                        "ignore_policy",
                        "ok",
                        ".crabignore includes CrabDB's default private and generated paths",
                        Some(serde_json::json!({
                            "path": crabignore_path.to_string_lossy(),
                            "patterns": patterns.len()
                        })),
                    ));
                } else {
                    checks.push(doctor_check(
                        "ignore_policy",
                        "warning",
                        ".crabignore is missing some default private or generated path rules",
                        Some(serde_json::json!({
                            "path": crabignore_path.to_string_lossy(),
                            "missing": missing
                        })),
                    ));
                }
            }
            Ok(_) => checks.push(doctor_check(
                "ignore_policy",
                "warning",
                ".crabignore is missing; agent patches still block CrabDB's hardcoded denylist",
                Some(serde_json::json!({ "path": crabignore_path.to_string_lossy() })),
            )),
            Err(err) => checks.push(doctor_check(
                "ignore_policy",
                "error",
                format!("could not read .crabignore: {err}"),
                Some(serde_json::json!({ "path": crabignore_path.to_string_lossy() })),
            )),
        }

        let lock_path = self.db_dir.join("lock");
        if lock_path.exists() {
            let holder = fs::read_to_string(&lock_path)
                .unwrap_or_else(|_| "unknown writer".to_string())
                .trim()
                .to_string();
            checks.push(doctor_check(
                "write_lock",
                "warning",
                "workspace write lock file is present",
                Some(serde_json::json!({
                    "path": lock_path.to_string_lossy(),
                    "holder": holder
                })),
            ));
        } else {
            checks.push(doctor_check(
                "write_lock",
                "ok",
                "no workspace write lock file is present",
                Some(serde_json::json!({ "path": lock_path.to_string_lossy() })),
            ));
        }

        let token_path = self.db_dir.join("daemon.token");
        if token_path.exists() {
            match fs::metadata(&token_path) {
                Ok(metadata) if metadata.len() == 0 => checks.push(doctor_check(
                    "daemon_token",
                    "error",
                    "daemon token file exists but is empty",
                    Some(serde_json::json!({ "path": token_path.to_string_lossy() })),
                )),
                Ok(metadata) => {
                    #[cfg(unix)]
                    {
                        let mode = metadata.permissions().mode() & 0o777;
                        if mode & 0o077 != 0 {
                            checks.push(doctor_check(
                                "daemon_token",
                                "warning",
                                format!("daemon token file permissions are {mode:o}; expected no group/other access"),
                                Some(serde_json::json!({
                                    "path": token_path.to_string_lossy(),
                                    "mode": format!("{mode:o}")
                                })),
                            ));
                        } else {
                            checks.push(doctor_check(
                                "daemon_token",
                                "ok",
                                "daemon token file exists with private permissions",
                                Some(serde_json::json!({
                                    "path": token_path.to_string_lossy(),
                                    "mode": format!("{mode:o}")
                                })),
                            ));
                        }
                    }
                    #[cfg(not(unix))]
                    {
                        checks.push(doctor_check(
                            "daemon_token",
                            "ok",
                            "daemon token file exists",
                            Some(serde_json::json!({ "path": token_path.to_string_lossy() })),
                        ));
                    }
                }
                Err(err) => checks.push(doctor_check(
                    "daemon_token",
                    "error",
                    format!("could not inspect daemon token file: {err}"),
                    Some(serde_json::json!({ "path": token_path.to_string_lossy() })),
                )),
            }
        } else {
            checks.push(doctor_check(
                "daemon_token",
                "ok",
                "daemon token has not been created yet; the daemon will create one when auth is enabled",
                Some(serde_json::json!({ "path": token_path.to_string_lossy() })),
            ));
        }

        match self.fsck() {
            Ok(report) if report.errors.is_empty() => checks.push(doctor_check(
                "fsck",
                "ok",
                "refs, roots, text objects, and indexes are internally consistent",
                Some(serde_json::json!({
                    "checked_refs": report.checked_refs,
                    "checked_roots": report.checked_roots,
                    "checked_texts": report.checked_texts
                })),
            )),
            Ok(report) => checks.push(doctor_check(
                "fsck",
                "error",
                format!("fsck found {} error(s)", report.errors.len()),
                Some(serde_json::json!({
                    "checked_refs": report.checked_refs,
                    "checked_roots": report.checked_roots,
                    "checked_texts": report.checked_texts,
                    "errors": report.errors
                })),
            )),
            Err(err) => checks.push(doctor_check(
                "fsck",
                "error",
                format!("fsck failed: {err}"),
                None,
            )),
        }

        match self.list_agent_approvals(None, Some("pending")) {
            Ok(approvals) if approvals.is_empty() => checks.push(doctor_check(
                "pending_approvals",
                "ok",
                "no pending human approval gates",
                Some(serde_json::json!({ "count": 0 })),
            )),
            Ok(approvals) => checks.push(doctor_check(
                "pending_approvals",
                "warning",
                format!("{} human approval gate(s) are pending", approvals.len()),
                Some(serde_json::json!({
                    "count": approvals.len(),
                    "approval_ids": approvals.iter().map(|approval| approval.approval_id.clone()).collect::<Vec<_>>()
                })),
            )),
            Err(err) => checks.push(doctor_check(
                "pending_approvals",
                "error",
                format!("could not list pending approvals: {err}"),
                None,
            )),
        }

        match self.list_leases(false) {
            Ok(leases) => checks.push(doctor_check(
                "active_leases",
                "ok",
                format!("{} active advisory lease(s)", leases.len()),
                Some(serde_json::json!({
                    "count": leases.len(),
                    "lease_ids": leases.iter().map(|lease| lease.lease_id.clone()).collect::<Vec<_>>()
                })),
            )),
            Err(err) => checks.push(doctor_check(
                "active_leases",
                "error",
                format!("could not list active leases: {err}"),
                None,
            )),
        }

        match self.list_merge_queue() {
            Ok(entries) => {
                let queued = entries
                    .iter()
                    .filter(|entry| entry.status == "queued")
                    .count();
                let running = entries
                    .iter()
                    .filter(|entry| entry.status == "running")
                    .count();
                let conflicted = entries
                    .iter()
                    .filter(|entry| entry.status == "conflicted")
                    .count();
                let failed = entries
                    .iter()
                    .filter(|entry| entry.status == "failed")
                    .count();
                let status = if conflicted > 0 || failed > 0 || queued > 0 || running > 0 {
                    "warning"
                } else {
                    "ok"
                };
                let message = if status == "ok" {
                    "merge queue has no pending attention".to_string()
                } else {
                    format!(
                        "merge queue has {queued} queued, {running} running, {conflicted} conflicted, and {failed} failed item(s)"
                    )
                };
                checks.push(doctor_check(
                    "merge_queue",
                    status,
                    message,
                    Some(serde_json::json!({
                        "total": entries.len(),
                        "queued": queued,
                        "running": running,
                        "conflicted": conflicted,
                        "failed": failed
                    })),
                ));
            }
            Err(err) => checks.push(doctor_check(
                "merge_queue",
                "error",
                format!("could not list merge queue: {err}"),
                None,
            )),
        }

        match self.list_conflicts() {
            Ok(conflicts) => {
                let open: Vec<String> = conflicts
                    .iter()
                    .filter(|conflict| conflict.status != "resolved")
                    .map(|conflict| conflict.conflict_set_id.clone())
                    .collect();
                if open.is_empty() {
                    checks.push(doctor_check(
                        "conflicts",
                        "ok",
                        "no open conflict sets",
                        Some(serde_json::json!({ "open": 0 })),
                    ));
                } else {
                    checks.push(doctor_check(
                        "conflicts",
                        "warning",
                        format!("{} conflict set(s) are still open", open.len()),
                        Some(serde_json::json!({
                            "open": open.len(),
                            "conflict_set_ids": open
                        })),
                    ));
                }
            }
            Err(err) => checks.push(doctor_check(
                "conflicts",
                "error",
                format!("could not list conflict sets: {err}"),
                None,
            )),
        }

        match self.list_agents() {
            Ok(agents) => {
                let mut dirty_agents = Vec::new();
                let mut missing_workdirs = Vec::new();
                let mut inspect_errors = Vec::new();
                for agent in &agents {
                    if agent.branch.workdir.is_none() {
                        continue;
                    }
                    match self.agent_status(&agent.branch.agent_id) {
                        Ok(status) if !status.workdir_changed_paths.is_empty() => {
                            dirty_agents.push(agent.record.name.clone());
                        }
                        Ok(_) => {}
                        Err(Error::WorkspaceNotFound(path)) => {
                            missing_workdirs.push(path.to_string_lossy().to_string());
                        }
                        Err(err) => inspect_errors.push(format!("{}: {err}", agent.record.name)),
                    }
                }
                let check_status = if !inspect_errors.is_empty() {
                    "error"
                } else if !dirty_agents.is_empty() || !missing_workdirs.is_empty() {
                    "warning"
                } else {
                    "ok"
                };
                let message = match check_status {
                    "ok" => format!("{} agent branch(es) inspected", agents.len()),
                    "warning" => format!(
                        "{} dirty agent workdir(s), {} missing agent workdir(s)",
                        dirty_agents.len(),
                        missing_workdirs.len()
                    ),
                    _ => format!(
                        "{} agent branch(es) could not be inspected",
                        inspect_errors.len()
                    ),
                };
                checks.push(doctor_check(
                    "agents",
                    check_status,
                    message,
                    Some(serde_json::json!({
                        "count": agents.len(),
                        "dirty_agents": dirty_agents,
                        "missing_workdirs": missing_workdirs,
                        "errors": inspect_errors
                    })),
                ));
            }
            Err(err) => checks.push(doctor_check(
                "agents",
                "error",
                format!("could not list agents: {err}"),
                None,
            )),
        }

        Ok(doctor_report(checks))
    }

    pub fn create_backup(
        &self,
        output: impl AsRef<Path>,
        overwrite: bool,
    ) -> Result<BackupCreateReport> {
        let _lock = self.acquire_write_lock()?;
        let output = absolute_path(output.as_ref())?;
        if output.starts_with(&self.db_dir) {
            return Err(Error::InvalidInput(
                "backup output cannot be inside .crabdb".to_string(),
            ));
        }
        if output.exists() {
            if !overwrite {
                return Err(Error::WorkspaceExists(output));
            }
            if output.is_dir() {
                fs::remove_dir_all(&output)?;
            } else {
                fs::remove_file(&output)?;
            }
        }

        let result = self.create_backup_inner(&output);
        if result.is_err() {
            let _ = fs::remove_dir_all(&output);
        }
        result
    }

    fn create_backup_inner(&self, output: &Path) -> Result<BackupCreateReport> {
        fs::create_dir_all(output.join("index"))?;
        fs::create_dir_all(output.join("refs/branches"))?;
        fs::create_dir_all(output.join("refs/agents"))?;

        fs::copy(self.db_dir.join(CONFIG_FILE), output.join(CONFIG_FILE))?;
        fs::copy(self.db_dir.join(HEAD_FILE), output.join(HEAD_FILE))?;
        let crabignore = self.workspace_root.join(".crabignore");
        if crabignore.exists() {
            fs::copy(crabignore, output.join(".crabignore"))?;
        }

        let sqlite_path = output.join(DB_RELATIVE_PATH);
        let sqlite_path_text = sqlite_path.to_string_lossy().to_string();
        self.conn
            .execute("VACUUM main INTO ?1", params![sqlite_path_text])?;
        let (sqlite_bytes, sqlite_sha256) = file_digest(&sqlite_path)?;

        let worktree_bytes =
            copy_dir_recursive(&self.db_dir.join("worktrees"), &output.join("worktrees"))?;

        let fsck = self.fsck()?;
        let branch = self.current_branch()?;
        let ref_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM refs", [], |row| row.get(0))?;
        let operation_count: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM operations", [], |row| row.get(0))?;

        let manifest = BackupManifest {
            format_version: 1,
            crabdb_version: env!("CARGO_PKG_VERSION").to_string(),
            created_at: now_ts(),
            source_workspace: self.workspace_root.to_string_lossy().to_string(),
            source_db_dir: self.db_dir.to_string_lossy().to_string(),
            workspace_id: self.config.workspace.id.clone(),
            branch: branch.clone(),
            ref_count: ref_count as u64,
            operation_count: operation_count as u64,
            sqlite_bytes,
            sqlite_sha256: sqlite_sha256.clone(),
            worktree_bytes,
        };
        let manifest_path = backup_manifest_path(output);
        fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;

        Ok(BackupCreateReport {
            path: output.to_string_lossy().to_string(),
            manifest_path: manifest_path.to_string_lossy().to_string(),
            sqlite_path: sqlite_path.to_string_lossy().to_string(),
            workspace_id: manifest.workspace_id,
            branch,
            ref_count: ref_count as u64,
            operation_count: operation_count as u64,
            sqlite_bytes,
            sqlite_sha256,
            worktree_bytes,
            fsck_errors: fsck.errors,
        })
    }

    pub fn verify_backup(path: impl AsRef<Path>) -> Result<BackupVerifyReport> {
        let path = absolute_path(path.as_ref())?;
        if !path.is_dir() {
            return Err(Error::WorkspaceNotFound(path));
        }
        let mut errors = Vec::new();
        let mut workspace_id = None;
        let mut branch = None;
        let mut checked_refs = 0;
        let mut checked_roots = 0;
        let mut checked_texts = 0;
        let mut sqlite_bytes = None;
        let mut sqlite_sha256 = None;

        let manifest = match read_backup_manifest(&path) {
            Ok(manifest) => {
                if manifest.format_version != 1 {
                    errors.push(format!(
                        "unsupported backup format version {}",
                        manifest.format_version
                    ));
                }
                workspace_id = Some(manifest.workspace_id.clone());
                branch = Some(manifest.branch.clone());
                Some(manifest)
            }
            Err(err) => {
                errors.push(format!("manifest invalid: {err}"));
                None
            }
        };

        for required in [CONFIG_FILE, HEAD_FILE] {
            if !path.join(required).is_file() {
                errors.push(format!("missing required file `{required}`"));
            }
        }

        let sqlite_path = backup_sqlite_path(&path);
        if sqlite_path.is_file() {
            match file_digest(&sqlite_path) {
                Ok((bytes, sha256)) => {
                    if let Some(manifest) = &manifest {
                        if manifest.sqlite_bytes != bytes {
                            errors.push(format!(
                                "SQLite byte size mismatch: manifest {}, actual {bytes}",
                                manifest.sqlite_bytes
                            ));
                        }
                        if manifest.sqlite_sha256 != sha256 {
                            errors.push("SQLite SHA-256 mismatch".to_string());
                        }
                    }
                    sqlite_bytes = Some(bytes);
                    sqlite_sha256 = Some(sha256);
                }
                Err(err) => errors.push(format!("could not hash SQLite store: {err}")),
            }
        } else {
            errors.push(format!("missing SQLite store `{}`", DB_RELATIVE_PATH));
        }

        if path.join(CONFIG_FILE).is_file()
            && path.join(HEAD_FILE).is_file()
            && sqlite_path.is_file()
        {
            let verify_dir = std::env::temp_dir().join(format!(
                "crabdb-backup-verify-{}-{}",
                std::process::id(),
                now_nanos()
            ));
            let verify_open = (|| -> Result<CrabDb> {
                fs::create_dir_all(verify_dir.join("index"))?;
                fs::copy(path.join(CONFIG_FILE), verify_dir.join(CONFIG_FILE))?;
                fs::copy(path.join(HEAD_FILE), verify_dir.join(HEAD_FILE))?;
                fs::copy(&sqlite_path, verify_dir.join(DB_RELATIVE_PATH))?;
                CrabDb::open_with_db_dir(&verify_dir, &verify_dir)
            })();
            match verify_open {
                Ok(db) => match db.fsck() {
                    Ok(fsck) => {
                        checked_refs = fsck.checked_refs;
                        checked_roots = fsck.checked_roots;
                        checked_texts = fsck.checked_texts;
                        errors.extend(fsck.errors);
                        workspace_id.get_or_insert_with(|| db.config.workspace.id.clone());
                        branch.get_or_insert(db.current_branch()?);
                    }
                    Err(err) => errors.push(format!("fsck failed: {err}")),
                },
                Err(err) => errors.push(format!("could not open backup store: {err}")),
            }
            let _ = fs::remove_dir_all(&verify_dir);
        }

        Ok(BackupVerifyReport {
            path: path.to_string_lossy().to_string(),
            valid: errors.is_empty(),
            workspace_id,
            branch,
            checked_refs,
            checked_roots,
            checked_texts,
            sqlite_bytes,
            sqlite_sha256,
            errors,
        })
    }

    pub fn restore_backup(
        workspace_root: impl AsRef<Path>,
        backup_path: impl AsRef<Path>,
        force: bool,
    ) -> Result<BackupRestoreReport> {
        fs::create_dir_all(workspace_root.as_ref())?;
        let workspace_root = workspace_root.as_ref().canonicalize()?;
        let backup_path = absolute_path(backup_path.as_ref())?;
        let verification = Self::verify_backup(&backup_path)?;
        if !verification.valid {
            return Err(Error::Corrupt(format!(
                "backup verification failed: {}",
                verification.errors.join("; ")
            )));
        }
        let manifest = read_backup_manifest(&backup_path)?;
        let db_dir = workspace_root.join(".crabdb");
        let replaced_existing = db_dir.exists();
        if replaced_existing {
            if db_dir.join("lock").exists() {
                let holder = fs::read_to_string(db_dir.join("lock"))
                    .unwrap_or_else(|_| "unknown writer".to_string());
                return Err(Error::WorkspaceLocked(holder.trim().to_string()));
            }
            if !force {
                return Err(Error::WorkspaceExists(db_dir));
            }
        }

        let temp_dir = workspace_root.join(format!(".crabdb.restore-{}", now_nanos()));
        if temp_dir.exists() {
            fs::remove_dir_all(&temp_dir)?;
        }

        let restore_result = (|| -> Result<()> {
            fs::create_dir_all(temp_dir.join("index"))?;
            fs::create_dir_all(temp_dir.join("refs/branches"))?;
            fs::create_dir_all(temp_dir.join("refs/agents"))?;
            fs::copy(backup_path.join(CONFIG_FILE), temp_dir.join(CONFIG_FILE))?;
            fs::copy(backup_path.join(HEAD_FILE), temp_dir.join(HEAD_FILE))?;
            fs::copy(
                backup_sqlite_path(&backup_path),
                temp_dir.join(DB_RELATIVE_PATH),
            )?;
            copy_dir_recursive(&backup_path.join("worktrees"), &temp_dir.join("worktrees"))?;
            Ok(())
        })();
        if let Err(err) = restore_result {
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(err);
        }

        if replaced_existing {
            fs::remove_dir_all(&db_dir)?;
        }
        if let Err(err) = fs::rename(&temp_dir, &db_dir) {
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(Error::Io(err));
        }

        let backup_crabignore = backup_path.join(".crabignore");
        let workspace_crabignore = workspace_root.join(".crabignore");
        let restored_crabignore =
            if backup_crabignore.is_file() && (force || !workspace_crabignore.exists()) {
                fs::copy(&backup_crabignore, &workspace_crabignore)?;
                true
            } else {
                if !workspace_crabignore.exists() {
                    write_default_crabignore(&workspace_root)?;
                }
                false
            };

        let mut db = CrabDb::open(&workspace_root)?;
        let rewritten_workdirs = db.rewrite_restored_agent_workdir_paths()?;
        let fsck = db.fsck()?;
        if !fsck.errors.is_empty() {
            return Err(Error::Corrupt(format!(
                "restored backup failed fsck: {}",
                fsck.errors.join("; ")
            )));
        }

        Ok(BackupRestoreReport {
            workspace: workspace_root.to_string_lossy().to_string(),
            db_dir: db_dir.to_string_lossy().to_string(),
            backup_path: backup_path.to_string_lossy().to_string(),
            workspace_id: manifest.workspace_id,
            branch: manifest.branch,
            replaced_existing,
            restored_crabignore,
            rewritten_workdirs,
            checked_refs: fsck.checked_refs,
            checked_roots: fsck.checked_roots,
            checked_texts: fsck.checked_texts,
        })
    }

    pub fn record(
        &mut self,
        branch: Option<&str>,
        message: Option<String>,
        actor: Actor,
        watch: bool,
    ) -> Result<RecordReport> {
        self.record_with_options(
            branch,
            message,
            actor,
            RecordOptions {
                kind: Some(if watch {
                    OperationKind::WatchRecord
                } else {
                    OperationKind::ManualRecord
                }),
                ..RecordOptions::default()
            },
        )
    }

    pub fn record_with_options(
        &mut self,
        branch: Option<&str>,
        message: Option<String>,
        actor: Actor,
        options: RecordOptions,
    ) -> Result<RecordReport> {
        let _lock = self.acquire_write_lock()?;
        self.record_with_options_unlocked(branch, message, actor, options)
    }

    fn record_with_options_unlocked(
        &mut self,
        branch: Option<&str>,
        message: Option<String>,
        actor: Actor,
        options: RecordOptions,
    ) -> Result<RecordReport> {
        let branch = branch.map(str::to_string).unwrap_or(self.current_branch()?);
        let ref_name = branch_ref(&branch);
        let head = self.get_ref(&ref_name)?;
        let previous_files = self.load_root_files(&head.root_id)?;
        let disk_files = self.scan_worktree_files()?;
        let selected_paths = normalize_record_paths(&options.paths)?;
        let session_id = options
            .session_id
            .map(|session_id| {
                validate_session_id(&session_id)?;
                self.agent_session(&session_id)?;
                Ok::<String, Error>(session_id)
            })
            .transpose()?;
        let change_id = self.allocate_change_id(&actor.id, "record")?;
        let built = if selected_paths.is_empty() {
            self.build_root_from_disk_files(&disk_files, &change_id, Some(&previous_files))?
        } else {
            self.build_root_for_selected_record(
                &previous_files,
                &disk_files,
                &selected_paths,
                options.allow_ignored,
                &change_id,
            )?
        };
        let diff = self.diff_file_maps(&previous_files, &built.files)?;

        if diff.changes.is_empty() {
            return Ok(RecordReport {
                branch,
                operation: None,
                root_id: head.root_id,
                changed_paths: Vec::new(),
            });
        }

        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind: options.kind.unwrap_or(OperationKind::ManualRecord),
            parents: vec![head.change_id.clone()],
            before_root: Some(head.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: branch.clone(),
            actor,
            session_id,
            message: message.map(|message| redact_sensitive_text(&message)),
            changes: diff.changes,
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&head, &change_id, &built.root_id, &operation_id)?;
        Ok(RecordReport {
            branch,
            operation: Some(change_id),
            root_id: built.root_id,
            changed_paths: diff.summaries,
        })
    }

    pub fn git_import_update(
        &mut self,
        branch: Option<&str>,
        message: Option<String>,
    ) -> Result<GitImportReport> {
        let _lock = self.acquire_write_lock()?;
        let branch = branch.map(str::to_string).unwrap_or(self.current_branch()?);
        let ref_name = branch_ref(&branch);
        let head = self.get_ref(&ref_name)?;
        let previous_files = self.load_root_files(&head.root_id)?;
        let disk_files = self.scan_git_tracked_files_required()?;
        let actor = Actor::system();
        let change_id = self.allocate_change_id(&actor.id, "git-import-update")?;
        let built =
            self.build_root_from_disk_files(&disk_files, &change_id, Some(&previous_files))?;
        let diff = self.diff_file_maps(&previous_files, &built.files)?;

        if diff.changes.is_empty() {
            return Ok(GitImportReport {
                branch,
                operation: None,
                root_id: head.root_id,
                imported: built.stats,
                changed_paths: Vec::new(),
                mapping: None,
            });
        }

        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind: OperationKind::GitImport,
            parents: vec![head.change_id.clone()],
            before_root: Some(head.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: branch.clone(),
            actor,
            session_id: None,
            message: message
                .map(|message| redact_sensitive_text(&message))
                .or_else(|| Some("Import Git-tracked workspace update".to_string())),
            changes: diff.changes,
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&head, &change_id, &built.root_id, &operation_id)?;
        let mapping = self.insert_git_mapping("import", &branch, &change_id, &built.root_id)?;

        Ok(GitImportReport {
            branch,
            operation: Some(change_id),
            root_id: built.root_id,
            imported: built.stats,
            changed_paths: diff.summaries,
            mapping,
        })
    }

    pub fn git_mappings(&self, limit: usize) -> Result<Vec<GitMapping>> {
        let mut stmt = self.conn.prepare(
            "SELECT mapping_id, direction, branch, git_head, git_dirty, crab_change, crab_root, created_at \
             FROM git_mappings ORDER BY created_at DESC, rowid DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], git_mapping_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub fn timeline(&self, branch: Option<&str>, limit: usize) -> Result<Vec<TimelineEntry>> {
        let mut sql = String::from(
            "SELECT change_id, kind, branch, actor_id, message, created_at, path_count \
             FROM operations",
        );
        if let Some(branch) = branch {
            let (branch_ref, bare_branch) = self.timeline_branch_terms(branch)?;
            if let Some(bare_branch) = bare_branch {
                sql.push_str(" WHERE branch = ?1 OR branch = ?2");
                sql.push_str(" ORDER BY created_at DESC, rowid DESC LIMIT ?3");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows =
                    stmt.query_map(params![branch_ref, bare_branch, limit as i64], timeline_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            } else {
                sql.push_str(" WHERE branch = ?1");
                sql.push_str(" ORDER BY created_at DESC, rowid DESC LIMIT ?2");
                let mut stmt = self.conn.prepare(&sql)?;
                let rows = stmt.query_map(params![branch_ref, limit as i64], timeline_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
        } else {
            sql.push_str(" ORDER BY created_at DESC, rowid DESC LIMIT ?1");
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(params![limit as i64], timeline_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Error::from)
        }
    }

    pub fn timeline_query(
        &self,
        branch: Option<&str>,
        session: Option<&str>,
        agent: Option<&str>,
        limit: usize,
    ) -> Result<Vec<TimelineEntry>> {
        let scoped = [branch.is_some(), session.is_some(), agent.is_some()]
            .into_iter()
            .filter(|set| *set)
            .count();
        if scoped > 1 {
            return Err(Error::InvalidInput(
                "timeline accepts only one of branch, session, or agent".to_string(),
            ));
        }
        if let Some(session_id) = session {
            return self.session_timeline(session_id, limit);
        }
        if let Some(agent) = agent {
            return self.agent_timeline(agent, limit);
        }
        self.timeline(branch, limit)
    }

    fn timeline_branch_terms(&self, branch: &str) -> Result<(String, Option<String>)> {
        let record = self.resolve_refish(branch)?;
        if record.name.starts_with(MAIN_REF_PREFIX) {
            let bare_branch = record
                .name
                .strip_prefix(MAIN_REF_PREFIX)
                .map(str::to_string);
            Ok((record.name, bare_branch))
        } else if record.name.starts_with(AGENT_REF_PREFIX) {
            Ok((record.name, None))
        } else {
            Err(Error::InvalidInput(format!(
                "timeline --branch expects a branch or agent ref, got `{branch}`"
            )))
        }
    }

    pub fn session_timeline(&self, session_id: &str, limit: usize) -> Result<Vec<TimelineEntry>> {
        self.agent_session(session_id)?;
        let mut stmt = self.conn.prepare(
            "SELECT change_id, kind, branch, actor_id, message, created_at, path_count \
             FROM operations WHERE session_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![session_id, limit as i64], timeline_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub fn show(&self, selector: &str) -> Result<ShowResult> {
        if let Some(agent) = selector.strip_prefix("agent:") {
            return Ok(ShowResult::Agent {
                value: self.agent_branch(agent)?,
            });
        }
        if selector.starts_with("ch_") {
            let operation = self.operation(&ChangeId(selector.to_string()))?;
            return Ok(ShowResult::Operation {
                value: OperationShow {
                    changed_paths: summarize_file_changes(&operation.changes),
                    messages: self.messages_for_change(&operation.change_id)?,
                    operation,
                },
            });
        }
        if selector.starts_with("msg_") {
            return Ok(ShowResult::Message {
                value: self.message(selector)?,
            });
        }
        if selector.starts_with("obj_") {
            return Ok(ShowResult::Object {
                value: self.object_info(selector)?,
            });
        }
        if let Ok(agent) = self.agent_branch(selector) {
            return Ok(ShowResult::Agent { value: agent });
        }
        if let Ok(ref_record) = self.resolve_refish(selector) {
            return Ok(ShowResult::Ref { value: ref_record });
        }
        Err(Error::InvalidInput(format!("cannot show `{selector}`")))
    }

    pub fn inspect_object(&self, object_id: &str) -> Result<ObjectInspectReport> {
        let info = self.object_info(object_id)?;
        let id = ObjectId(object_id.to_string());
        let summary = match info.kind.as_str() {
            WORKTREE_ROOT_KIND => {
                let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, &id)?;
                serde_json::json!({
                    "file_count": root.file_count,
                    "total_text_bytes": root.total_text_bytes,
                    "created_by": root.created_by,
                    "path_map_root": root.path_map_root,
                    "file_index_map_root": root.file_index_map_root,
                })
            }
            TEXT_CONTENT_KIND => {
                let text: TextContent = self.get_object(TEXT_CONTENT_KIND, &id)?;
                serde_json::json!({
                    "content_hash": text.content_hash,
                    "line_count": text.line_count,
                    "byte_count": text.byte_count,
                    "representation": text.representation,
                    "order_map_root": text.order_map_root,
                    "line_index_map_root": text.line_index_map_root,
                })
            }
            OPERATION_KIND => {
                let operation: Operation = self.get_object(OPERATION_KIND, &id)?;
                serde_json::json!({
                    "change_id": operation.change_id,
                    "kind": operation.kind,
                    "branch": operation.branch,
                    "actor": operation.actor,
                    "parent_count": operation.parents.len(),
                    "changed_path_count": operation.changes.len(),
                    "before_root": operation.before_root,
                    "after_root": operation.after_root,
                    "message": operation.message,
                    "created_at": operation.created_at,
                })
            }
            BLOB_KIND => {
                let blob: Blob = self.get_object(BLOB_KIND, &id)?;
                serde_json::json!({
                    "content_hash": blob.content_hash,
                    "byte_count": blob.bytes.len(),
                })
            }
            MESSAGE_KIND => {
                let message: Message = self.get_object(MESSAGE_KIND, &id)?;
                serde_json::json!({
                    "message_id": message.id,
                    "role": message.role,
                    "agent_id": message.agent_id,
                    "session_id": message.session_id,
                    "change_id": message.change_id,
                    "body_bytes": message.body.len(),
                    "created_at": message.created_at,
                })
            }
            ANCHOR_KIND => {
                let anchor: Anchor = self.get_object(ANCHOR_KIND, &id)?;
                serde_json::json!({
                    "anchor_id": anchor.id,
                    "label": anchor.label,
                    "file_id": file_id_key(&anchor.file_id),
                    "line_id": line_id_key_value(&anchor.line_id),
                    "created_path": anchor.created_path,
                    "created_line": anchor.created_line,
                    "created_change": anchor.created_change,
                    "created_at": anchor.created_at,
                })
            }
            _ => serde_json::json!({}),
        };
        Ok(ObjectInspectReport { info, summary })
    }

    pub fn inspect_root(&self, root_id: &str) -> Result<RootInspectReport> {
        let root_id = ObjectId(root_id.to_string());
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, &root_id)?;
        let files = self
            .load_root_files(&root_id)?
            .into_iter()
            .map(|(path, entry)| RootFileInspect {
                path,
                file_id: file_id_key(&entry.file_id),
                kind: entry.kind,
                mode: entry.mode,
                executable: entry.executable,
                size_bytes: entry.size_bytes,
                content_hash: entry.content_hash,
                content_object: content_object_id(&entry.content).clone(),
            })
            .collect();
        Ok(RootInspectReport {
            root_id,
            root,
            files,
        })
    }

    pub fn inspect_text(&self, text_id: &str, limit: usize) -> Result<TextInspectReport> {
        let text_id = ObjectId(text_id.to_string());
        let content: TextContent = self.get_object(TEXT_CONTENT_KIND, &text_id)?;
        let loaded_lines = self.load_text_lines(&text_id)?;
        let truncated = limit > 0 && loaded_lines.len() > limit;
        let lines = loaded_lines
            .into_iter()
            .take(if limit == 0 { usize::MAX } else { limit })
            .enumerate()
            .map(|(idx, line)| TextLineInspect {
                line_number: idx as u64 + 1,
                line_id: line.line_id_key(),
                text_hash: line.text_hash,
                text: String::from_utf8_lossy(&line.text).into_owned(),
                newline: line.newline,
                introduced_by: line.introduced_by,
                last_content_change: line.last_content_change,
                last_move_change: line.last_move_change,
            })
            .collect();
        Ok(TextInspectReport {
            text_id,
            content,
            lines,
            truncated,
        })
    }

    pub fn inspect_map_range(
        &self,
        map_id: &str,
        map_type: &str,
        start: Option<&str>,
        end: Option<&str>,
        limit: usize,
    ) -> Result<MapRangeReport> {
        let map_type = parse_map_inspect_type(map_type)?;
        let start_bytes = start
            .map(parse_map_key_spec)
            .transpose()?
            .unwrap_or_default();
        let end_bytes = end.map(parse_map_key_spec).transpose()?;
        let tree = tree_from_root_hex(Some(map_id))?;
        let iter = self
            .prolly
            .range(&tree, &start_bytes, end_bytes.as_deref())?;
        let mut entries = Vec::new();
        let mut truncated = false;
        for item in iter {
            let (key, value) = item?;
            if limit > 0 && entries.len() >= limit {
                truncated = true;
                break;
            }
            entries.push(MapEntryInspect {
                key: inspect_map_key(map_type, &key),
                value: inspect_map_value(map_type, &value),
            });
        }
        Ok(MapRangeReport {
            map_id: map_id.to_string(),
            map_type: map_type.as_str().to_string(),
            start: start.map(str::to_string),
            end: end.map(str::to_string),
            entries,
            truncated,
        })
    }

    pub fn inspect_map_diff(
        &self,
        left_map_id: &str,
        right_map_id: &str,
        map_type: &str,
        start: Option<&str>,
        end: Option<&str>,
        limit: usize,
    ) -> Result<MapDiffReport> {
        let map_type = parse_map_inspect_type(map_type)?;
        let start_bytes = start
            .map(parse_map_key_spec)
            .transpose()?
            .unwrap_or_default();
        let end_bytes = end.map(parse_map_key_spec).transpose()?;
        let left = tree_from_root_hex(Some(left_map_id))?;
        let right = tree_from_root_hex(Some(right_map_id))?;
        let diffs = self
            .prolly
            .range_diff(&left, &right, &start_bytes, end_bytes.as_deref())?;
        let mut changes = Vec::new();
        let mut truncated = false;
        for diff in diffs {
            if limit > 0 && changes.len() >= limit {
                truncated = true;
                break;
            }
            changes.push(inspect_map_diff_entry(map_type, diff));
        }
        Ok(MapDiffReport {
            left_map_id: left_map_id.to_string(),
            right_map_id: right_map_id.to_string(),
            map_type: map_type.as_str().to_string(),
            start: start.map(str::to_string),
            end: end.map(str::to_string),
            changes,
            truncated,
        })
    }

    pub fn history_for_path(&self, path: &str) -> Result<HistoryResult> {
        let path = normalize_relative_path(path)?;
        Ok(HistoryResult {
            selector: path.clone(),
            file_history: self.file_history_by_path(&path)?,
            line_history: Vec::new(),
        })
    }

    pub fn history_for_file_id(&self, file_id: &str) -> Result<HistoryResult> {
        Ok(HistoryResult {
            selector: file_id.to_string(),
            file_history: self.file_history_by_file_id(file_id)?,
            line_history: Vec::new(),
        })
    }

    pub fn history_for_line_id(&self, line_id: &str) -> Result<HistoryResult> {
        Ok(HistoryResult {
            selector: line_id.to_string(),
            file_history: Vec::new(),
            line_history: self.line_history_by_line_id(line_id)?,
        })
    }

    pub fn code_from(&self, selector: &str) -> Result<CodeFromResult> {
        let mut changes = Vec::new();
        if let Some(agent) = selector.strip_prefix("agent:") {
            changes.extend(self.agent_change_ids(agent)?);
        } else if selector.starts_with("msg_") {
            let change_id: Option<String> = self
                .conn
                .query_row(
                    "SELECT change_id FROM messages WHERE message_id = ?1",
                    params![selector],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(change_id) = change_id else {
                return Err(Error::InvalidInput(format!(
                    "message `{selector}` not found"
                )));
            };
            changes.push(ChangeId(change_id));
        } else if selector.starts_with("ch_") {
            changes.push(ChangeId(selector.to_string()));
        } else if selector.starts_with("session_") {
            changes.extend(self.session_change_ids(selector)?);
        } else if let Ok(agent) = self.agent_branch(selector) {
            changes.extend(self.agent_change_ids(&agent.agent_id)?);
        } else {
            changes.extend(self.session_change_ids(selector)?);
        }

        let mut operations = Vec::new();
        for change in changes {
            let operation = self.operation(&change)?;
            operations.push(CodeFromOperation {
                change_id: operation.change_id.clone(),
                kind: operation.kind.clone(),
                branch: operation.branch.clone(),
                actor_id: operation.actor.id.clone(),
                session_id: operation.session_id.clone(),
                message: operation.message.clone(),
                changed_paths: summarize_file_changes(&operation.changes),
                created_at: operation.created_at,
            });
        }
        Ok(CodeFromResult {
            selector: selector.to_string(),
            operations,
        })
    }

    pub fn diff_range(&self, spec: &str, patches: bool) -> Result<DiffSummary> {
        self.diff_range_with_options(spec, patches, false)
    }

    pub fn diff_range_with_options(
        &self,
        spec: &str,
        patches: bool,
        line_changes: bool,
    ) -> Result<DiffSummary> {
        let (left, right) = parse_range(spec)?;
        self.diff_refs_with_options(left, right, patches, line_changes)
    }

    pub fn diff_refs(&self, left: &str, right: &str, patches: bool) -> Result<DiffSummary> {
        self.diff_refs_with_options(left, right, patches, false)
    }

    pub fn diff_refs_with_options(
        &self,
        left: &str,
        right: &str,
        patches: bool,
        line_changes: bool,
    ) -> Result<DiffSummary> {
        let left_ref = self.resolve_refish(left)?;
        let right_ref = self.resolve_refish(right)?;
        let left_files = self.load_root_files(&left_ref.root_id)?;
        let right_files = self.load_root_files(&right_ref.root_id)?;
        self.diff_files(
            left.to_string(),
            right.to_string(),
            &left_files,
            &right_files,
            patches,
            line_changes,
        )
    }

    pub fn diff_roots(&self, spec: &str, patches: bool, line_changes: bool) -> Result<DiffSummary> {
        let (left, right) = parse_range(spec)?;
        let left_id = ObjectId(left.to_string());
        let right_id = ObjectId(right.to_string());
        let left_files = self.load_root_files(&left_id)?;
        let right_files = self.load_root_files(&right_id)?;
        self.diff_files(
            left.to_string(),
            right.to_string(),
            &left_files,
            &right_files,
            patches,
            line_changes,
        )
    }

    pub fn diff_dirty(&mut self, patches: bool, line_changes: bool) -> Result<DiffSummary> {
        let _lock = self.acquire_write_lock()?;
        let branch = self.current_branch()?;
        let head = self.resolve_branch_ref(&branch)?;
        let previous_files = self.load_root_files(&head.root_id)?;
        let disk_files = self.scan_worktree_files()?;
        let change_id = self.allocate_change_id("crabdb", "dirty-diff")?;
        let built =
            self.build_root_from_disk_files(&disk_files, &change_id, Some(&previous_files))?;
        self.diff_files(
            branch,
            "dirty".to_string(),
            &previous_files,
            &built.files,
            patches,
            line_changes,
        )
    }

    pub fn checkout(&mut self, change_or_ref: &str, force: bool) -> Result<CheckoutReport> {
        self.checkout_with_options(change_or_ref, force, false, None, false)
    }

    pub fn checkout_with_options(
        &mut self,
        change_or_ref: &str,
        force: bool,
        dry_run: bool,
        workdir: Option<&Path>,
        record_dirty: bool,
    ) -> Result<CheckoutReport> {
        let _lock = self.acquire_write_lock()?;
        if dry_run && record_dirty {
            return Err(Error::InvalidInput(
                "checkout --record-dirty cannot be combined with --dry-run".to_string(),
            ));
        }
        let mut recorded_dirty = None;
        if record_dirty {
            let current_branch = self.current_branch()?;
            let report = self.record_with_options_unlocked(
                Some(&current_branch),
                Some(format!(
                    "Record dirty worktree before checkout `{change_or_ref}`"
                )),
                Actor::human(),
                RecordOptions {
                    kind: Some(OperationKind::Checkout),
                    ..RecordOptions::default()
                },
            )?;
            recorded_dirty = report.operation;
        }
        let current = self.resolve_branch_ref(&self.current_branch()?)?;
        if !dry_run && workdir.is_none() && !force && !record_dirty {
            let status = self.status(None)?;
            if status.worktree_state != WorktreeState::Clean {
                return Err(Error::DirtyWorktree);
            }
        }
        let target = self.resolve_refish(change_or_ref)?;
        let current_files = self.load_root_files(&current.root_id)?;
        let target_files = self.load_root_files(&target.root_id)?;
        let diff = self.diff_file_maps(&current_files, &target_files)?;
        let output_root = workdir
            .map(|path| self.resolve_checkout_workdir_path(path))
            .transpose()?;
        if !dry_run {
            if let Some(output_root) = &output_root {
                prepare_checkout_workdir(output_root)?;
                materialize_into(
                    &self.workspace_root,
                    output_root,
                    &BTreeMap::new(),
                    &target_files,
                    |entry| self.materialize_entry_bytes(entry),
                )?;
            } else {
                self.materialize_files(&current_files, &target_files)?;
            }
        }
        Ok(CheckoutReport {
            change_id: target.change_id,
            root_id: target.root_id,
            written_files: if dry_run {
                0
            } else {
                target_files.len() as u64
            },
            dry_run,
            recorded_dirty,
            output_root: output_root.map(|path| path.to_string_lossy().to_string()),
            changed_paths: diff.summaries,
        })
    }

    pub fn create_branch(&mut self, name: &str, from: Option<&str>) -> Result<BranchReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(name)?;
        let source = match from {
            Some(refish) => self.resolve_refish(refish)?,
            None => self.resolve_branch_ref(&self.current_branch()?)?,
        };
        let ref_name = branch_ref(name);
        if self.try_get_ref(&ref_name)?.is_some() {
            return Err(Error::InvalidInput(format!(
                "branch `{name}` already exists"
            )));
        }
        self.set_ref(
            &ref_name,
            &source.change_id,
            &source.root_id,
            &source.operation_id,
        )?;
        Ok(BranchReport {
            name: name.to_string(),
            from: source.change_id,
            root_id: source.root_id,
        })
    }

    pub fn list_branches(&self) -> Result<Vec<BranchListEntry>> {
        let current = self.current_branch()?;
        let mut stmt = self.conn.prepare(
            "SELECT name, change_id, root_id, operation_id, generation, updated_at \
             FROM refs WHERE name LIKE 'refs/branches/%' ORDER BY name",
        )?;
        let rows = stmt.query_map([], ref_row)?;
        let refs = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        Ok(refs
            .into_iter()
            .map(|record| {
                let name = record
                    .name
                    .strip_prefix(MAIN_REF_PREFIX)
                    .unwrap_or(&record.name)
                    .to_string();
                BranchListEntry {
                    is_current: name == current || record.name == current,
                    name,
                    ref_name: record.name,
                    change_id: record.change_id,
                    root_id: record.root_id,
                    generation: record.generation,
                }
            })
            .collect())
    }

    pub fn delete_branch(&mut self, name: &str) -> Result<BranchDeleteReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(name)?;
        let current = self.current_branch()?;
        let ref_name = branch_ref(name);
        let short_name = ref_name.strip_prefix(MAIN_REF_PREFIX).unwrap_or(name);
        if short_name == current || ref_name == current {
            return Err(Error::InvalidInput(format!(
                "cannot delete current branch `{short_name}`"
            )));
        }
        self.get_ref(&ref_name)?;
        self.conn
            .execute("DELETE FROM refs WHERE name = ?1", params![ref_name])?;
        remove_ref_file(&self.db_dir, &ref_name)?;
        Ok(BranchDeleteReport {
            name: short_name.to_string(),
            ref_name,
        })
    }

    pub fn rename_branch(&mut self, old_name: &str, new_name: &str) -> Result<BranchRenameReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(old_name)?;
        validate_ref_segment(new_name)?;
        let old_ref = branch_ref(old_name);
        let new_ref = branch_ref(new_name);
        let record = self.get_ref(&old_ref)?;
        if self.try_get_ref(&new_ref)?.is_some() {
            return Err(Error::InvalidInput(format!(
                "branch `{new_name}` already exists"
            )));
        }
        self.conn.execute(
            "UPDATE refs SET name = ?1, updated_at = ?2 WHERE name = ?3",
            params![new_ref, now_ts(), old_ref],
        )?;
        remove_ref_file(&self.db_dir, &old_ref)?;
        write_ref_file(
            &self.db_dir,
            &new_ref,
            &record.change_id,
            &record.root_id,
            &record.operation_id,
            record.generation,
        )?;
        let current = self.current_branch()?;
        let old_short = old_ref.strip_prefix(MAIN_REF_PREFIX).unwrap_or(old_name);
        let new_short = new_ref.strip_prefix(MAIN_REF_PREFIX).unwrap_or(new_name);
        if current == old_short || current == old_ref {
            fs::write(self.db_dir.join(HEAD_FILE), format!("{new_short}\n"))?;
        }
        Ok(BranchRenameReport {
            old_name: old_short.to_string(),
            new_name: new_short.to_string(),
            change_id: record.change_id,
            root_id: record.root_id,
        })
    }

    pub fn why(&self, path_line: &str, branch: Option<&str>) -> Result<WhyResult> {
        let (path, line_number) = parse_path_line(path_line)?;
        let head = self.resolve_why_ref(branch)?;
        let files = self.load_root_files(&head.root_id)?;
        let entry = files
            .get(&path)
            .ok_or_else(|| Error::InvalidInput(format!("path `{path}` is not tracked")))?;
        let FileContentRef::Text(text_id) = &entry.content else {
            return Err(Error::InvalidInput(format!(
                "path `{path}` is not line-tracked text"
            )));
        };
        let lines = self.load_text_lines(text_id)?;
        let Some(line) = lines.get(line_number.saturating_sub(1) as usize) else {
            return Err(Error::InvalidInput(format!(
                "line {line_number} is outside `{path}`"
            )));
        };
        self.why_from_line(path, line_number, entry, line)
    }

    pub fn why_line_id(&self, line_id: &str, branch: Option<&str>) -> Result<WhyResult> {
        let parsed = parse_line_id_key(line_id)?;
        let line_id_key = line_id_key_value(&parsed);
        let head = self.resolve_why_ref(branch)?;
        let files = self.load_root_files(&head.root_id)?;
        for (path, entry) in &files {
            let FileContentRef::Text(text_id) = &entry.content else {
                continue;
            };
            let lines = self.load_text_lines(text_id)?;
            for (index, line) in lines.iter().enumerate() {
                if line.line_id_key() == line_id_key {
                    return self.why_from_line(path.clone(), index as u64 + 1, entry, line);
                }
            }
        }
        Err(Error::InvalidInput(format!(
            "line id `{line_id}` is not present in the selected root"
        )))
    }

    fn resolve_why_ref(&self, refish: Option<&str>) -> Result<RefRecord> {
        match refish {
            Some(refish) => self.resolve_refish(refish),
            None => self.resolve_branch_ref(&self.current_branch()?),
        }
    }

    fn why_from_line(
        &self,
        path: String,
        line_number: u64,
        entry: &FileEntry,
        line: &LineEntry,
    ) -> Result<WhyResult> {
        let mut stmt = self.conn.prepare(
            "SELECT change_id, path, line_number, kind, text_hash, created_at \
             FROM line_history WHERE line_id = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![line.line_id_key()], |row| {
            Ok(LineHistoryEntry {
                change_id: ChangeId(row.get(0)?),
                path: row.get(1)?,
                line_number: row.get::<_, Option<i64>>(2)?.map(|n| n as u64),
                kind: parse_line_change_kind(&row.get::<_, String>(3)?),
                text_hash: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?;
        let history = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        Ok(WhyResult {
            path,
            line_number,
            file_id: entry.file_id.clone(),
            line_id: line.line_id.clone(),
            current_text: String::from_utf8_lossy(&line.text).into_owned(),
            introduced_by: line.introduced_by.clone(),
            last_content_change: line.last_content_change.clone(),
            last_move_change: line.last_move_change.clone(),
            history,
        })
    }

    pub fn create_anchor(
        &mut self,
        path_line: &str,
        label: impl Into<String>,
        branch: Option<&str>,
    ) -> Result<AnchorCreateReport> {
        let _lock = self.acquire_write_lock()?;
        let label = label.into();
        if label.trim().is_empty() {
            return Err(Error::InvalidInput(
                "anchor label cannot be empty".to_string(),
            ));
        }
        let why = self.why(path_line, branch)?;
        let anchor = Anchor {
            version: ANCHOR_OBJECT_VERSION,
            id: AnchorId::new(&why.file_id, &why.line_id, &label),
            label,
            file_id: why.file_id,
            line_id: why.line_id,
            created_path: why.path,
            created_line: why.line_number,
            created_change: why.last_content_change,
            created_at: now_ts(),
        };
        let object_id = self.put_object(ANCHOR_KIND, ANCHOR_OBJECT_VERSION, &anchor)?;
        self.index_anchor(&anchor, &object_id)?;
        Ok(AnchorCreateReport { anchor, object_id })
    }

    pub fn list_anchors(&self) -> Result<Vec<Anchor>> {
        let mut stmt = self
            .conn
            .prepare("SELECT object_id FROM anchors ORDER BY created_at ASC, anchor_id ASC")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let object_ids = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        object_ids
            .into_iter()
            .map(|object_id| self.get_object(ANCHOR_KIND, &ObjectId(object_id)))
            .collect()
    }

    pub fn resolve_anchor(
        &self,
        anchor_id: &str,
        branch: Option<&str>,
    ) -> Result<AnchorResolveReport> {
        let anchor = self.anchor(anchor_id)?;
        let branch = branch.map(str::to_string).unwrap_or(self.current_branch()?);
        let head = self.resolve_refish(&branch)?;
        let files = self.load_root_files(&head.root_id)?;
        let Some((path, entry)) = files
            .iter()
            .find(|(_, entry)| entry.file_id == anchor.file_id)
        else {
            return Ok(AnchorResolveReport {
                anchor,
                branch,
                status: "missing_file".to_string(),
                path: None,
                line_number: None,
                text: None,
            });
        };
        let FileContentRef::Text(text_id) = &entry.content else {
            return Ok(AnchorResolveReport {
                anchor,
                branch,
                status: "non_text".to_string(),
                path: Some(path.clone()),
                line_number: None,
                text: None,
            });
        };
        let lines = self.load_text_lines(text_id)?;
        for (idx, line) in lines.iter().enumerate() {
            if line.line_id == anchor.line_id {
                return Ok(AnchorResolveReport {
                    anchor,
                    branch,
                    status: "found".to_string(),
                    path: Some(path.clone()),
                    line_number: Some(idx as u64 + 1),
                    text: Some(String::from_utf8_lossy(&line.text).into_owned()),
                });
            }
        }
        Ok(AnchorResolveReport {
            anchor,
            branch,
            status: "missing_line".to_string(),
            path: Some(path.clone()),
            line_number: None,
            text: None,
        })
    }

    pub fn delete_anchor(&mut self, anchor_id: &str) -> Result<AnchorDeleteReport> {
        let _lock = self.acquire_write_lock()?;
        let anchor = self.anchor(anchor_id)?;
        self.conn.execute(
            "DELETE FROM anchors WHERE anchor_id = ?1",
            params![anchor.id.0],
        )?;
        Ok(AnchorDeleteReport {
            anchor_id: anchor.id,
        })
    }

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

    fn materialize_agent_workdir(
        &self,
        name: &str,
        root_id: &ObjectId,
        custom_workdir: Option<&Path>,
    ) -> Result<PathBuf> {
        let dir = self.resolve_agent_workdir_path(name, custom_workdir)?;
        self.materialize_agent_workdir_at(root_id, &dir, custom_workdir.is_some())?;
        Ok(dir)
    }

    fn materialize_agent_workdir_at(
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

    fn resolve_agent_workdir_path(
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

    fn default_agent_workdir_path(&self, name: &str) -> Result<PathBuf> {
        Ok(self.default_agent_worktrees_base()?.join(name))
    }

    fn default_agent_worktrees_base(&self) -> Result<PathBuf> {
        let rel = normalize_relative_path(&self.config.agent.worktrees_dir)?;
        normalize_workdir_path(&self.workspace_root.join(path_from_rel(&rel)))
    }

    fn validate_agent_workdir_path(&self, path: &Path) -> Result<()> {
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

    fn resolve_checkout_workdir_path(&self, workdir: &Path) -> Result<PathBuf> {
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

    pub fn list_agents(&self) -> Result<Vec<AgentDetails>> {
        let mut stmt = self.conn.prepare(
            "SELECT a.agent_id, a.name, a.kind, a.provider, a.model, a.created_at, a.metadata_json, \
                    b.ref_name, b.base_change, b.head_change, b.base_root, b.head_root, b.session_id, b.workdir, b.status, b.created_at, b.updated_at \
             FROM agents a JOIN agent_branches b ON a.agent_id = b.agent_id \
             ORDER BY a.created_at ASC, a.name ASC",
        )?;
        let rows = stmt.query_map([], agent_details_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    fn rewrite_restored_agent_workdir_paths(&mut self) -> Result<u64> {
        let rows = {
            let mut stmt = self.conn.prepare(
                "SELECT b.agent_id, a.name \
                 FROM agent_branches b JOIN agents a ON a.agent_id = b.agent_id \
                 WHERE b.workdir IS NOT NULL",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };

        let mut rewritten = 0;
        for (agent_id, name) in rows {
            let workdir = self.default_agent_workdir_path(&name)?;
            self.conn.execute(
                "UPDATE agent_branches SET workdir = ?1, updated_at = ?2 WHERE agent_id = ?3",
                params![workdir.to_string_lossy(), now_ts(), agent_id],
            )?;
            rewritten += 1;
        }
        Ok(rewritten)
    }

    pub fn agent_details(&self, agent: &str) -> Result<AgentDetails> {
        let branch = self.agent_branch(agent)?;
        let record = self.agent_record(&branch.agent_id)?;
        Ok(AgentDetails { record, branch })
    }

    pub fn resolve_agent_handle(&self, handle: &str) -> Result<String> {
        if validate_ref_segment(handle).is_ok() && self.try_get_ref(&agent_ref(handle))?.is_some() {
            return Ok(handle.to_string());
        }
        if handle.starts_with("agent_") {
            let name = self
                .conn
                .query_row(
                    "SELECT name FROM agents WHERE agent_id = ?1",
                    params![handle],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(name) = name {
                return Ok(name);
            }
        }
        Err(Error::RefNotFound(handle.to_string()))
    }

    pub fn agent_status(&self, agent: &str) -> Result<AgentStatusReport> {
        let details = self.agent_details(agent)?;
        let source = self.get_ref(&details.branch.ref_name)?;
        let base = self.ref_from_change(&details.branch.base_change)?;
        let base_files = self.load_root_files(&base.root_id)?;
        let source_files = self.load_root_files(&source.root_id)?;
        let diff = self.diff_file_maps(&base_files, &source_files)?;
        let workdir_changed_paths = self
            .agent_workdir_changed_paths(&details.branch, &source)?
            .unwrap_or_default();
        let workdir_state = details
            .branch
            .workdir
            .as_ref()
            .map(|_| worktree_state_from_changes(&workdir_changed_paths));
        let queued_merges: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM merge_queue WHERE source_ref = ?1 AND status IN ('queued', 'running')",
            params![details.branch.ref_name],
            |row| row.get(0),
        )?;
        Ok(AgentStatusReport {
            latest_test: self.latest_agent_test(&details.branch.agent_id)?,
            latest_eval: self.latest_agent_gate(&details.branch.agent_id, "eval")?,
            agent: details,
            changed_paths: diff.summaries,
            queued_merges: queued_merges as u64,
            workdir_state,
            workdir_changed_paths,
        })
    }

    pub fn agent_contribution(&self, agent: &str, limit: usize) -> Result<AgentContributionReport> {
        let limit = normalize_query_limit(limit, 1000)?;
        let status = self.agent_status(agent)?;
        let operations = self.agent_timeline(agent, limit)?;
        let sessions = self.list_agent_sessions(Some(agent))?;
        let recent_events = self.list_agent_events(Some(agent), None, None, None, limit)?;
        let approvals = self.list_agent_approvals(Some(agent), None)?;
        Ok(AgentContributionReport {
            status,
            operations,
            sessions,
            recent_events,
            approvals,
        })
    }

    pub fn agent_gate_history(
        &self,
        agent: &str,
        kind: Option<&str>,
        limit: usize,
    ) -> Result<AgentGateHistoryReport> {
        let limit = normalize_query_limit(limit, 1000)?;
        let details = self.agent_details(agent)?;
        let kind_filter = normalize_agent_gate_filter(kind)?;
        let gates = self.agent_gate_history_for_id(&details.branch.agent_id, kind_filter, limit)?;
        Ok(AgentGateHistoryReport {
            agent: details,
            kind: kind_filter.unwrap_or("all").to_string(),
            limit,
            gates,
        })
    }

    pub fn agent_readiness(&self, agent: &str) -> Result<AgentReadinessReport> {
        let status = self.agent_status(agent)?;
        let agent_ref = status.agent.branch.ref_name.clone();
        let pending_approvals = self.list_agent_approvals(Some(agent), Some("pending"))?;
        let conflicts = self
            .list_conflicts()?
            .into_iter()
            .filter(|conflict| {
                conflict.status != "resolved"
                    && (conflict.source_ref.as_deref() == Some(agent_ref.as_str())
                        || conflict.target_ref.as_deref() == Some(agent_ref.as_str()))
            })
            .collect::<Vec<_>>();

        let mut blockers = Vec::new();
        let mut warnings = Vec::new();
        if status.agent.branch.status == "removed" {
            blockers.push(readiness_issue(
                "agent_removed",
                "agent branch has already been removed",
                Some(serde_json::json!({ "status": status.agent.branch.status })),
            ));
        }

        let workdir_state = status.workdir_state.clone();
        if workdir_state
            .as_ref()
            .is_some_and(|state| state != &WorktreeState::Clean)
        {
            let paths = status
                .workdir_changed_paths
                .iter()
                .map(|path| path.path.clone())
                .collect::<Vec<_>>();
            blockers.push(readiness_issue(
                "dirty_workdir",
                "materialized agent workdir has unrecorded changes",
                Some(serde_json::json!({
                    "state": workdir_state.clone(),
                    "paths": paths
                })),
            ));
        }

        if !pending_approvals.is_empty() {
            let approval_ids = pending_approvals
                .iter()
                .map(|approval| approval.approval_id.clone())
                .collect::<Vec<_>>();
            blockers.push(readiness_issue(
                "pending_approvals",
                format!(
                    "{} human approval request(s) are still pending",
                    pending_approvals.len()
                ),
                Some(serde_json::json!({ "approval_ids": approval_ids })),
            ));
        }

        if !conflicts.is_empty() {
            let conflict_ids = conflicts
                .iter()
                .map(|conflict| conflict.conflict_set_id.clone())
                .collect::<Vec<_>>();
            blockers.push(readiness_issue(
                "open_conflicts",
                format!("{} merge conflict set(s) are still open", conflicts.len()),
                Some(serde_json::json!({ "conflict_set_ids": conflict_ids })),
            ));
        }

        match &status.latest_test {
            Some(test) if !test.success => blockers.push(readiness_issue(
                "latest_test_failed",
                "latest recorded test gate did not pass",
                Some(serde_json::json!({
                    "event_id": test.event_id,
                    "status": test.status,
                    "exit_code": test.exit_code,
                    "command": test.command,
                    "suite": test.suite,
                    "score": test.score,
                    "threshold": test.threshold
                })),
            )),
            Some(_) => {}
            None => {
                let issue = readiness_issue(
                    "missing_latest_test",
                    "no test gate has been recorded for this agent",
                    None,
                );
                if self.config.agent.require_test_gate {
                    blockers.push(issue);
                } else {
                    warnings.push(issue);
                }
            }
        }

        match &status.latest_eval {
            Some(eval) if !eval.success => blockers.push(readiness_issue(
                "latest_eval_failed",
                "latest recorded eval gate did not pass",
                Some(serde_json::json!({
                    "event_id": eval.event_id,
                    "status": eval.status,
                    "exit_code": eval.exit_code,
                    "command": eval.command,
                    "suite": eval.suite,
                    "score": eval.score,
                    "threshold": eval.threshold
                })),
            )),
            Some(_) => {}
            None => {
                let issue = readiness_issue(
                    "missing_latest_eval",
                    "no eval gate has been recorded for this agent",
                    None,
                );
                if self.config.agent.require_eval_gate {
                    blockers.push(issue);
                } else {
                    warnings.push(issue);
                }
            }
        }

        blockers.extend(self.required_gate_suite_issues(
            &status.agent.branch.agent_id,
            "test",
            &self.config.agent.required_test_suites,
        )?);
        blockers.extend(self.required_gate_suite_issues(
            &status.agent.branch.agent_id,
            "eval",
            &self.config.agent.required_eval_suites,
        )?);

        if status.changed_paths.is_empty() {
            warnings.push(readiness_issue(
                "no_changed_paths",
                "agent branch does not currently differ from its base",
                None,
            ));
        }
        if status.queued_merges > 0 {
            warnings.push(readiness_issue(
                "queued_merge",
                "agent already has a queued or running merge",
                Some(serde_json::json!({ "queued_merges": status.queued_merges })),
            ));
        }

        let ready = blockers.is_empty();
        Ok(AgentReadinessReport {
            agent: status.agent,
            ready,
            status: if ready { "ready" } else { "blocked" }.to_string(),
            blockers,
            warnings,
            changed_paths: status.changed_paths,
            workdir_state,
            workdir_changed_paths: status.workdir_changed_paths,
            queued_merges: status.queued_merges,
            pending_approvals,
            conflicts,
            latest_test: status.latest_test,
            latest_eval: status.latest_eval,
        })
    }

    pub fn agent_handoff(&self, agent: &str, limit: usize) -> Result<AgentHandoffReport> {
        let limit = normalize_query_limit(limit, 1000)?;
        let readiness = self.agent_readiness(agent)?;
        let agent_details = readiness.agent.clone();
        let current_session = agent_details
            .branch
            .session_id
            .as_deref()
            .map(|session_id| self.show_agent_session(session_id))
            .transpose()?;
        let recent_sessions = self
            .list_agent_sessions(Some(agent))?
            .into_iter()
            .take(limit)
            .collect::<Vec<_>>();
        let recent_events = self.list_agent_events(Some(agent), None, None, None, limit)?;
        let recent_spans = self.list_agent_trace_spans(Some(agent), None, None, None, limit)?;
        let recent_operations = self.agent_timeline(agent, limit)?;
        let next_steps = handoff_next_steps(&readiness, current_session.as_ref());
        Ok(AgentHandoffReport {
            agent: agent_details,
            readiness,
            current_session,
            recent_sessions,
            recent_events,
            recent_spans,
            recent_operations,
            next_steps,
        })
    }

    pub fn add_agent_message(
        &mut self,
        agent: &str,
        role: &str,
        text: &str,
        session_id: Option<String>,
    ) -> Result<AgentMessageReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;
        if role.trim().is_empty() {
            return Err(Error::InvalidInput(
                "message role cannot be empty".to_string(),
            ));
        }
        if text.is_empty() {
            return Err(Error::InvalidInput(
                "message text cannot be empty".to_string(),
            ));
        }
        let branch = self.agent_branch(agent)?;
        let session_id = session_id.or(branch.session_id.clone());
        if let Some(session_id) = &session_id {
            self.ensure_agent_session(&branch.agent_id, session_id, None)?;
            self.conn.execute(
                "UPDATE agent_branches SET session_id = ?1, updated_at = ?2 WHERE agent_id = ?3",
                params![session_id, now_ts(), branch.agent_id],
            )?;
        }
        let turn_id = self.open_agent_turn(
            &branch.agent_id,
            session_id.as_deref(),
            &branch.base_change,
            &branch.head_change,
            Some(&serde_json::json!({
                "kind": "message",
                "role": role
            })),
        )?;
        let created_at = now_ts();
        let message_id = self.store_message(
            role,
            text,
            Some(&branch.agent_id),
            session_id.as_deref(),
            None,
            created_at,
        )?;
        self.insert_agent_event_with_context(
            &branch.agent_id,
            session_id.as_deref(),
            Some(&turn_id),
            "message_added",
            None,
            Some(&message_id),
            &serde_json::json!({
                "role": role,
                "session_id": session_id.clone()
            }),
        )?;
        self.finish_agent_turn(&turn_id, "completed", None)?;
        Ok(AgentMessageReport {
            agent_id: branch.agent_id,
            message_id,
            role: role.to_string(),
            session_id,
        })
    }

    pub fn begin_agent_turn(
        &mut self,
        agent: &str,
        from: Option<&str>,
        session_title: Option<String>,
        base_change: Option<&str>,
    ) -> Result<AgentTurnStartReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;

        let branch = match self.agent_branch(agent) {
            Ok(branch) => branch,
            Err(Error::RefNotFound(_)) => {
                let source_selector = match base_change.or(from) {
                    Some(selector) => selector.to_string(),
                    None => self.current_branch()?,
                };
                let source = self.resolve_refish(&source_selector)?;
                let agent_id = format!("agent_{}", crate::ids::short_hash(agent.as_bytes(), 8));
                let ref_name = agent_ref(agent);
                if self.try_get_ref(&ref_name)?.is_some() {
                    return Err(Error::InvalidInput(format!(
                        "agent `{agent}` already exists"
                    )));
                }
                let workdir = if self.config.agent.default_materialize {
                    let dir = self.materialize_agent_workdir(agent, &source.root_id, None)?;
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
                        agent,
                        "coding-agent",
                        Option::<String>::None,
                        Option::<String>::None,
                        now,
                        Option::<String>::None
                    ],
                )?;
                self.conn.execute(
                    "INSERT INTO agent_branches \
                     (agent_id, ref_name, base_change, head_change, base_root, head_root, session_id, workdir, status, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, 'active', ?8, ?8)",
                    params![
                        agent_id,
                        ref_name,
                        source.change_id.0,
                        source.change_id.0,
                        source.root_id.0,
                        source.root_id.0,
                        workdir.clone(),
                        now
                    ],
                )?;
                self.insert_agent_event(
                    &format!("agent_{}", crate::ids::short_hash(agent.as_bytes(), 8)),
                    "agent_spawned",
                    Some(&source.change_id),
                    None,
                    &serde_json::json!({
                        "ref_name": agent_ref(agent),
                        "base_root": source.root_id.0.clone(),
                        "workdir": workdir.clone(),
                        "source": "api"
                    }),
                )?;
                self.agent_branch(agent)?
            }
            Err(err) => return Err(err),
        };

        if let Some(expected_base) = base_change {
            if branch.head_change.0 != expected_base {
                return Err(Error::StaleBranch(branch.ref_name));
            }
        }

        let session_id = self.allocate_session_id(&branch.agent_id, session_title.as_deref());
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO agent_sessions \
             (session_id, agent_id, title, status, started_at, ended_at, metadata_json) \
             VALUES (?1, ?2, ?3, 'active', ?4, NULL, NULL)",
            params![session_id, branch.agent_id, session_title, now],
        )?;
        self.conn.execute(
            "UPDATE agent_branches SET session_id = ?1, updated_at = ?2 WHERE agent_id = ?3",
            params![session_id, now, branch.agent_id],
        )?;
        self.insert_agent_event_with_context(
            &branch.agent_id,
            Some(&session_id),
            None,
            "session_started",
            Some(&branch.head_change),
            None,
            &serde_json::json!({
                "session_id": session_id.clone(),
                "title": session_title.clone(),
                "source": "api"
            }),
        )?;
        let turn_id = self.open_agent_turn(
            &branch.agent_id,
            Some(&session_id),
            &branch.base_change,
            &branch.head_change,
            Some(&serde_json::json!({
                "kind": "api_turn",
                "from": from,
                "base_change": base_change
            })),
        )?;
        self.insert_agent_event_with_context(
            &branch.agent_id,
            Some(&session_id),
            Some(&turn_id),
            "turn_started",
            None,
            None,
            &serde_json::json!({
                "turn_id": turn_id.clone()
            }),
        )?;
        Ok(AgentTurnStartReport {
            turn: self.agent_turn(&turn_id)?,
            session: self.agent_session(&session_id)?,
            base_root: branch.head_root,
        })
    }

    pub fn add_agent_turn_message(
        &mut self,
        turn_id: &str,
        role: &str,
        text: &str,
    ) -> Result<AgentMessageReport> {
        let _lock = self.acquire_write_lock()?;
        if role.trim().is_empty() {
            return Err(Error::InvalidInput(
                "message role cannot be empty".to_string(),
            ));
        }
        if text.is_empty() {
            return Err(Error::InvalidInput(
                "message text cannot be empty".to_string(),
            ));
        }
        let turn = self.agent_turn(turn_id)?;
        if turn.ended_at.is_some() {
            return Err(Error::InvalidInput(format!(
                "turn `{turn_id}` is already ended"
            )));
        }
        let created_at = now_ts();
        let message_id = self.store_message(
            role,
            text,
            Some(&turn.agent_id),
            turn.session_id.as_deref(),
            None,
            created_at,
        )?;
        self.insert_agent_event_with_context(
            &turn.agent_id,
            turn.session_id.as_deref(),
            Some(turn_id),
            "message_added",
            None,
            Some(&message_id),
            &serde_json::json!({
                "role": role,
                "session_id": turn.session_id
            }),
        )?;
        Ok(AgentMessageReport {
            agent_id: turn.agent_id,
            message_id,
            role: role.to_string(),
            session_id: turn.session_id,
        })
    }

    pub fn add_agent_turn_event(
        &mut self,
        turn_id: &str,
        event_type: &str,
        payload: Option<serde_json::Value>,
        change_id: Option<&str>,
        message_id: Option<&str>,
    ) -> Result<AgentTurnEventReport> {
        let _lock = self.acquire_write_lock()?;
        let event_type = event_type.trim();
        if event_type.is_empty() {
            return Err(Error::InvalidInput(
                "event type cannot be empty".to_string(),
            ));
        }
        let turn = self.agent_turn(turn_id)?;
        if turn.ended_at.is_some() {
            return Err(Error::InvalidInput(format!(
                "turn `{turn_id}` is already ended"
            )));
        }
        let change_id = change_id
            .map(|change_id| {
                let change = ChangeId(change_id.to_string());
                self.operation(&change).map(|_| change)
            })
            .transpose()?;
        let message_id = message_id
            .map(|message_id| {
                self.message(message_id)
                    .map(|_| MessageId(message_id.to_string()))
            })
            .transpose()?;
        let event_id = self.insert_agent_event_with_context(
            &turn.agent_id,
            turn.session_id.as_deref(),
            Some(turn_id),
            event_type,
            change_id.as_ref(),
            message_id.as_ref(),
            &payload.unwrap_or(serde_json::Value::Null),
        )?;
        Ok(AgentTurnEventReport {
            event: self.agent_event(&event_id)?,
        })
    }

    pub fn show_agent_turn(&self, turn_id: &str) -> Result<AgentTurnDetails> {
        let turn = self.agent_turn(turn_id)?;
        let session = turn
            .session_id
            .as_deref()
            .map(|session_id| self.agent_session(session_id))
            .transpose()?;
        Ok(AgentTurnDetails {
            messages: self.agent_turn_messages(turn_id)?,
            events: self.agent_turn_events(turn_id)?,
            operations: self.agent_turn_operations(turn_id)?,
            turn,
            session,
        })
    }

    pub fn request_agent_approval(
        &mut self,
        agent: &str,
        action: &str,
        summary: &str,
        payload: Option<serde_json::Value>,
        session_id: Option<&str>,
        turn_id: Option<&str>,
    ) -> Result<AgentApprovalRequestReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;
        let action = action.trim();
        if action.is_empty() {
            return Err(Error::InvalidInput(
                "approval action cannot be empty".to_string(),
            ));
        }
        let summary = summary.trim();
        if summary.is_empty() {
            return Err(Error::InvalidInput(
                "approval summary cannot be empty".to_string(),
            ));
        }
        let branch = self.agent_branch(agent)?;
        let turn = turn_id
            .map(|turn_id| self.agent_turn(turn_id))
            .transpose()?;
        if let Some(turn) = &turn {
            if turn.agent_id != branch.agent_id {
                return Err(Error::InvalidInput(format!(
                    "turn `{}` does not belong to agent `{}`",
                    turn.turn_id, branch.agent_id
                )));
            }
            if turn.ended_at.is_some() {
                return Err(Error::InvalidInput(format!(
                    "turn `{}` is already ended",
                    turn.turn_id
                )));
            }
        }
        let approval_session_id = session_id
            .map(str::to_string)
            .or_else(|| turn.as_ref().and_then(|turn| turn.session_id.clone()))
            .or_else(|| branch.session_id.clone());
        if let Some(session_id) = approval_session_id.as_deref() {
            let session = self.agent_session(session_id)?;
            if session.agent_id != branch.agent_id {
                return Err(Error::InvalidInput(format!(
                    "session `{session_id}` does not belong to agent `{}`",
                    branch.agent_id
                )));
            }
        }

        let requested_at = now_ts();
        let redacted_action = redact_sensitive_text(action);
        let redacted_summary = redact_sensitive_text(summary);
        let redacted_payload = payload.map(redact_sensitive_json);
        let seed = format!(
            "{}:{}:{}:{}:{}",
            branch.agent_id,
            approval_session_id.as_deref().unwrap_or("none"),
            turn_id.unwrap_or("none"),
            redacted_action,
            now_nanos()
        );
        let approval_id = format!("approval_{}", crate::ids::short_hash(seed.as_bytes(), 16));
        let payload_json = redacted_payload
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        self.conn.execute(
            "INSERT INTO agent_approvals \
             (approval_id, agent_id, session_id, turn_id, action, summary, payload_json, status, requested_at, decided_at, reviewer, note) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8, NULL, NULL, NULL)",
            params![
                approval_id,
                branch.agent_id,
                approval_session_id,
                turn_id,
                redacted_action.clone(),
                redacted_summary.clone(),
                payload_json,
                requested_at
            ],
        )?;
        self.insert_agent_event_with_context(
            &branch.agent_id,
            approval_session_id.as_deref(),
            turn_id,
            "approval_requested",
            None,
            None,
            &serde_json::json!({
                "approval_id": approval_id,
                "action": redacted_action,
                "summary": redacted_summary
            }),
        )?;
        let approval = self.agent_approval(&approval_id)?;
        let run_state = self.insert_agent_run_state(
            &approval.agent_id,
            approval.session_id.as_deref(),
            approval.turn_id.as_deref(),
            Some(&approval.approval_id),
            "approval_required",
            &approval.summary,
            Some(serde_json::json!({
                "agent_id": approval.agent_id.clone(),
                "session_id": approval.session_id.clone(),
                "turn_id": approval.turn_id.clone(),
                "approval_id": approval.approval_id.clone(),
                "action": approval.action.clone(),
                "summary": approval.summary.clone(),
                "payload": approval.payload.clone()
            })),
            Some(serde_json::json!({
                "type": "approval_required",
                "approval_id": approval.approval_id.clone(),
                "action": approval.action.clone(),
                "summary": approval.summary.clone()
            })),
        )?;
        Ok(AgentApprovalRequestReport {
            approval,
            run_state: Some(run_state),
        })
    }

    pub fn list_agent_approvals(
        &self,
        agent: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<AgentApproval>> {
        let status = status
            .map(parse_approval_status_filter)
            .transpose()?
            .flatten();
        match (agent, status) {
            (Some(agent), Some(status)) => {
                let branch = self.agent_branch(agent)?;
                let mut stmt = self.conn.prepare(
                    "SELECT approval_id, agent_id, session_id, turn_id, action, summary, payload_json, status, requested_at, decided_at, reviewer, note \
                     FROM agent_approvals WHERE agent_id = ?1 AND status = ?2 ORDER BY requested_at DESC, approval_id DESC",
                )?;
                let rows = stmt.query_map(params![branch.agent_id, status], agent_approval_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
            (Some(agent), None) => {
                let branch = self.agent_branch(agent)?;
                let mut stmt = self.conn.prepare(
                    "SELECT approval_id, agent_id, session_id, turn_id, action, summary, payload_json, status, requested_at, decided_at, reviewer, note \
                     FROM agent_approvals WHERE agent_id = ?1 ORDER BY requested_at DESC, approval_id DESC",
                )?;
                let rows = stmt.query_map(params![branch.agent_id], agent_approval_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
            (None, Some(status)) => {
                let mut stmt = self.conn.prepare(
                    "SELECT approval_id, agent_id, session_id, turn_id, action, summary, payload_json, status, requested_at, decided_at, reviewer, note \
                     FROM agent_approvals WHERE status = ?1 ORDER BY requested_at DESC, approval_id DESC",
                )?;
                let rows = stmt.query_map(params![status], agent_approval_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
            (None, None) => {
                let mut stmt = self.conn.prepare(
                    "SELECT approval_id, agent_id, session_id, turn_id, action, summary, payload_json, status, requested_at, decided_at, reviewer, note \
                     FROM agent_approvals ORDER BY requested_at DESC, approval_id DESC",
                )?;
                let rows = stmt.query_map([], agent_approval_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
        }
    }

    pub fn list_agent_events(
        &self,
        agent: Option<&str>,
        session_id: Option<&str>,
        turn_id: Option<&str>,
        event_type: Option<&str>,
        limit: usize,
    ) -> Result<Vec<AgentEventRecord>> {
        let limit = normalize_query_limit(limit, 1000)?;
        let agent_id = agent
            .map(|agent| self.agent_branch(agent).map(|branch| branch.agent_id))
            .transpose()?;
        if let Some(session_id) = session_id {
            self.agent_session(session_id)?;
        }
        if let Some(turn_id) = turn_id {
            self.agent_turn(turn_id)?;
        }
        let event_type = event_type
            .map(str::trim)
            .map(|event_type| {
                if event_type.is_empty() {
                    Err(Error::InvalidInput(
                        "event type filter cannot be empty".to_string(),
                    ))
                } else {
                    Ok(event_type)
                }
            })
            .transpose()?;

        let mut stmt = self.conn.prepare(
            "SELECT event_id, agent_id, session_id, turn_id, event_type, change_id, message_id, payload_json, created_at \
             FROM agent_events \
             WHERE (?1 IS NULL OR agent_id = ?1) \
               AND (?2 IS NULL OR session_id = ?2) \
               AND (?3 IS NULL OR turn_id = ?3) \
               AND (?4 IS NULL OR event_type = ?4) \
             ORDER BY created_at DESC, rowid DESC LIMIT ?5",
        )?;
        let rows = stmt.query_map(
            params![agent_id, session_id, turn_id, event_type, limit as i64],
            agent_event_row,
        )?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub fn start_agent_trace_span(
        &mut self,
        turn_id: &str,
        span_type: &str,
        name: &str,
        parent_span_id: Option<&str>,
        trace_id: Option<&str>,
        attributes: Option<serde_json::Value>,
    ) -> Result<AgentTraceSpanStartReport> {
        let _lock = self.acquire_write_lock()?;
        let span_type = span_type.trim();
        if span_type.is_empty() {
            return Err(Error::InvalidInput("span type cannot be empty".to_string()));
        }
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::InvalidInput("span name cannot be empty".to_string()));
        }
        let turn = self.agent_turn(turn_id)?;
        if turn.ended_at.is_some() {
            return Err(Error::InvalidInput(format!(
                "turn `{turn_id}` is already ended"
            )));
        }

        let parent = parent_span_id
            .map(|span_id| self.show_agent_trace_span(span_id))
            .transpose()?;
        if let Some(parent) = &parent {
            if parent.agent_id != turn.agent_id
                || parent.turn_id.as_deref() != Some(turn_id)
                || parent.session_id != turn.session_id
            {
                return Err(Error::InvalidInput(format!(
                    "parent span `{}` does not belong to turn `{turn_id}`",
                    parent.span_id
                )));
            }
        }

        let trace_id = match (trace_id.map(str::trim), parent.as_ref()) {
            (Some(""), _) => {
                return Err(Error::InvalidInput("trace id cannot be empty".to_string()));
            }
            (Some(trace_id), Some(parent)) if trace_id != parent.trace_id => {
                return Err(Error::InvalidInput(format!(
                    "trace id `{trace_id}` does not match parent span trace `{}`",
                    parent.trace_id
                )));
            }
            (Some(trace_id), _) => trace_id.to_string(),
            (None, Some(parent)) => parent.trace_id.clone(),
            (None, None) => default_trace_id_for_turn(turn_id),
        };

        let seed = format!(
            "{}:{}:{}:{}:{}:{}",
            turn.agent_id,
            turn.session_id.as_deref().unwrap_or("none"),
            turn_id,
            trace_id,
            name,
            now_nanos()
        );
        let span_id = format!("span_{}", crate::ids::short_hash(seed.as_bytes(), 16));
        self.insert_agent_event_with_context(
            &turn.agent_id,
            turn.session_id.as_deref(),
            Some(turn_id),
            "span_started",
            None,
            None,
            &serde_json::json!({
                "span_id": span_id.clone(),
                "trace_id": trace_id,
                "parent_span_id": parent_span_id,
                "span_type": span_type,
                "name": name,
                "attributes": attributes.unwrap_or(serde_json::Value::Null)
            }),
        )?;
        Ok(AgentTraceSpanStartReport {
            span: self.show_agent_trace_span(&span_id)?,
        })
    }

    pub fn end_agent_trace_span(
        &mut self,
        span_id: &str,
        status: &str,
        result: Option<serde_json::Value>,
    ) -> Result<AgentTraceSpanEndReport> {
        let _lock = self.acquire_write_lock()?;
        let span = self.show_agent_trace_span(span_id)?;
        if span.ended_at.is_some() {
            return Err(Error::InvalidInput(format!(
                "span `{span_id}` is already ended"
            )));
        }
        let status = status.trim();
        if status.is_empty() {
            return Err(Error::InvalidInput(
                "span status cannot be empty".to_string(),
            ));
        }
        if let Some(turn_id) = span.turn_id.as_deref() {
            let turn = self.agent_turn(turn_id)?;
            if turn.ended_at.is_some() {
                return Err(Error::InvalidInput(format!(
                    "turn `{turn_id}` is already ended"
                )));
            }
        }
        self.insert_agent_event_with_context(
            &span.agent_id,
            span.session_id.as_deref(),
            span.turn_id.as_deref(),
            "span_ended",
            None,
            None,
            &serde_json::json!({
                "span_id": span.span_id,
                "trace_id": span.trace_id,
                "status": status,
                "result": result.unwrap_or(serde_json::Value::Null)
            }),
        )?;
        Ok(AgentTraceSpanEndReport {
            span: self.show_agent_trace_span(span_id)?,
        })
    }

    pub fn list_agent_trace_spans(
        &self,
        agent: Option<&str>,
        session_id: Option<&str>,
        turn_id: Option<&str>,
        trace_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<AgentTraceSpan>> {
        let limit = normalize_query_limit(limit, 1000)?;
        let trace_id = trace_id
            .map(str::trim)
            .map(|trace_id| {
                if trace_id.is_empty() {
                    Err(Error::InvalidInput(
                        "trace id filter cannot be empty".to_string(),
                    ))
                } else {
                    Ok(trace_id)
                }
            })
            .transpose()?;
        let events = self.list_agent_trace_span_events(agent, session_id, turn_id)?;
        let mut spans = build_agent_trace_spans(events);
        if let Some(trace_id) = trace_id {
            spans.retain(|span| span.trace_id == trace_id);
        }
        spans.sort_by(|left, right| {
            right
                .started_at
                .cmp(&left.started_at)
                .then_with(|| right.span_id.cmp(&left.span_id))
        });
        spans.truncate(limit);
        Ok(spans)
    }

    pub fn summarize_agent_trace_spans(
        &self,
        agent: Option<&str>,
        session_id: Option<&str>,
        turn_id: Option<&str>,
        trace_id: Option<&str>,
        slowest_limit: usize,
    ) -> Result<AgentTraceSummaryReport> {
        let slowest_limit = normalize_query_limit(slowest_limit, 50)?;
        let agent_id = agent
            .map(|agent| self.agent_branch(agent).map(|branch| branch.agent_id))
            .transpose()?;
        let trace_id = trace_id
            .map(str::trim)
            .map(|trace_id| {
                if trace_id.is_empty() {
                    Err(Error::InvalidInput(
                        "trace id filter cannot be empty".to_string(),
                    ))
                } else {
                    Ok(trace_id.to_string())
                }
            })
            .transpose()?;

        let events = self.list_agent_trace_span_events(agent, session_id, turn_id)?;
        let mut spans = build_agent_trace_spans(events);
        if let Some(trace_id) = trace_id.as_deref() {
            spans.retain(|span| span.trace_id == trace_id);
        }

        let mut status_counts = BTreeMap::new();
        let mut span_type_counts = BTreeMap::new();
        let mut trace_counts = BTreeMap::new();
        let mut open_spans = Vec::new();
        let mut slowest_spans = Vec::new();
        let mut total_duration_ms = 0u64;
        let mut max_duration_ms = 0u64;
        let mut duration_count = 0u64;
        let mut failed_span_count = 0u64;
        let mut ended_span_count = 0u64;

        for span in &spans {
            *status_counts.entry(span.status.clone()).or_insert(0) += 1;
            *span_type_counts.entry(span.span_type.clone()).or_insert(0) += 1;
            *trace_counts.entry(span.trace_id.clone()).or_insert(0) += 1;
            if span.ended_at.is_some() {
                ended_span_count += 1;
            } else {
                open_spans.push(span.clone());
            }
            if agent_trace_status_is_failed(&span.status) {
                failed_span_count += 1;
            }
            if let Some(duration_ms) = span.duration_ms {
                total_duration_ms = total_duration_ms.saturating_add(duration_ms);
                max_duration_ms = max_duration_ms.max(duration_ms);
                duration_count += 1;
                slowest_spans.push(span.clone());
            }
        }

        let open_span_count = open_spans.len() as u64;

        slowest_spans.sort_by(|left, right| {
            right
                .duration_ms
                .cmp(&left.duration_ms)
                .then_with(|| right.started_at.cmp(&left.started_at))
                .then_with(|| right.span_id.cmp(&left.span_id))
        });
        slowest_spans.truncate(slowest_limit);
        open_spans.sort_by(|left, right| {
            right
                .started_at
                .cmp(&left.started_at)
                .then_with(|| right.span_id.cmp(&left.span_id))
        });
        open_spans.truncate(slowest_limit);

        Ok(AgentTraceSummaryReport {
            agent_id,
            session_id: session_id.map(str::to_string),
            turn_id: turn_id.map(str::to_string),
            trace_id,
            span_count: spans.len() as u64,
            open_span_count,
            ended_span_count,
            failed_span_count,
            total_duration_ms,
            max_duration_ms,
            average_duration_ms: if duration_count == 0 {
                None
            } else {
                Some(total_duration_ms as f64 / duration_count as f64)
            },
            status_counts: named_counts(status_counts),
            span_type_counts: named_counts(span_type_counts),
            trace_counts: named_counts(trace_counts),
            slowest_spans,
            open_spans,
        })
    }

    pub fn show_agent_trace_span(&self, span_id: &str) -> Result<AgentTraceSpan> {
        let span_id = span_id.trim();
        if span_id.is_empty() {
            return Err(Error::InvalidInput("span id cannot be empty".to_string()));
        }
        build_agent_trace_spans(self.list_agent_trace_span_events(None, None, None)?)
            .into_iter()
            .find(|span| span.span_id == span_id)
            .ok_or_else(|| Error::InvalidInput(format!("span `{span_id}` not found")))
    }

    fn list_agent_trace_span_events(
        &self,
        agent: Option<&str>,
        session_id: Option<&str>,
        turn_id: Option<&str>,
    ) -> Result<Vec<AgentEventRecord>> {
        let agent_id = agent
            .map(|agent| self.agent_branch(agent).map(|branch| branch.agent_id))
            .transpose()?;
        if let Some(session_id) = session_id {
            self.agent_session(session_id)?;
        }
        if let Some(turn_id) = turn_id {
            self.agent_turn(turn_id)?;
        }

        let mut stmt = self.conn.prepare(
            "SELECT event_id, agent_id, session_id, turn_id, event_type, change_id, message_id, payload_json, created_at \
             FROM agent_events \
             WHERE (?1 IS NULL OR agent_id = ?1) \
               AND (?2 IS NULL OR session_id = ?2) \
               AND (?3 IS NULL OR turn_id = ?3) \
               AND event_type IN ('span_started', 'span_ended') \
             ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![agent_id, session_id, turn_id], agent_event_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub fn pause_agent_run(
        &mut self,
        agent: &str,
        reason: &str,
        summary: &str,
        state: Option<serde_json::Value>,
        interruption: Option<serde_json::Value>,
        session_id: Option<&str>,
        turn_id: Option<&str>,
    ) -> Result<AgentRunPauseReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;
        let branch = self.agent_branch(agent)?;
        let (session_id, turn_id) =
            self.validate_agent_run_context(&branch, session_id, turn_id)?;
        let run_state = self.insert_agent_run_state(
            &branch.agent_id,
            session_id.as_deref(),
            turn_id.as_deref(),
            None,
            reason,
            summary,
            state,
            interruption,
        )?;
        Ok(AgentRunPauseReport { run_state })
    }

    pub fn list_agent_run_states(
        &self,
        agent: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<AgentRunState>> {
        let status = status
            .map(parse_agent_run_status_filter)
            .transpose()?
            .flatten();
        match (agent, status) {
            (Some(agent), Some(status)) => {
                let branch = self.agent_branch(agent)?;
                let mut stmt = self.conn.prepare(
                    "SELECT run_id, agent_id, session_id, turn_id, approval_id, status, reason, summary, state_json, interruption_json, created_at, updated_at, resumed_at, reviewer, note \
                     FROM agent_run_states WHERE agent_id = ?1 AND status = ?2 ORDER BY updated_at DESC, run_id DESC",
                )?;
                let rows = stmt.query_map(params![branch.agent_id, status], agent_run_state_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
            (Some(agent), None) => {
                let branch = self.agent_branch(agent)?;
                let mut stmt = self.conn.prepare(
                    "SELECT run_id, agent_id, session_id, turn_id, approval_id, status, reason, summary, state_json, interruption_json, created_at, updated_at, resumed_at, reviewer, note \
                     FROM agent_run_states WHERE agent_id = ?1 ORDER BY updated_at DESC, run_id DESC",
                )?;
                let rows = stmt.query_map(params![branch.agent_id], agent_run_state_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
            (None, Some(status)) => {
                let mut stmt = self.conn.prepare(
                    "SELECT run_id, agent_id, session_id, turn_id, approval_id, status, reason, summary, state_json, interruption_json, created_at, updated_at, resumed_at, reviewer, note \
                     FROM agent_run_states WHERE status = ?1 ORDER BY updated_at DESC, run_id DESC",
                )?;
                let rows = stmt.query_map(params![status], agent_run_state_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
            (None, None) => {
                let mut stmt = self.conn.prepare(
                    "SELECT run_id, agent_id, session_id, turn_id, approval_id, status, reason, summary, state_json, interruption_json, created_at, updated_at, resumed_at, reviewer, note \
                     FROM agent_run_states ORDER BY updated_at DESC, run_id DESC",
                )?;
                let rows = stmt.query_map([], agent_run_state_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
        }
    }

    pub fn show_agent_run_state(&self, run_id: &str) -> Result<AgentRunState> {
        self.agent_run_state(run_id)
    }

    pub fn resume_agent_run(
        &mut self,
        run_id: &str,
        reviewer: Option<String>,
        note: Option<String>,
    ) -> Result<AgentRunResumeReport> {
        let _lock = self.acquire_write_lock()?;
        let run_state = self.agent_run_state(run_id)?;
        if run_state.status != "paused" {
            return Err(Error::InvalidInput(format!(
                "agent run `{}` is {} and cannot be resumed",
                run_state.run_id, run_state.status
            )));
        }
        if let Some(approval_id) = run_state.approval_id.as_deref() {
            let approval = self.agent_approval(approval_id)?;
            if approval.status != "approved" {
                return Err(Error::InvalidInput(format!(
                    "agent run `{}` is waiting on approval `{approval_id}` ({})",
                    run_state.run_id, approval.status
                )));
            }
        }
        let reviewer = reviewer.map(|reviewer| redact_sensitive_text(&reviewer));
        let note = note.map(|note| redact_sensitive_text(&note));
        let now = now_ts();
        self.conn.execute(
            "UPDATE agent_run_states SET status = 'resumed', updated_at = ?1, resumed_at = ?1, reviewer = ?2, note = ?3 WHERE run_id = ?4",
            params![now, reviewer.clone(), note.clone(), run_state.run_id],
        )?;
        self.insert_agent_event_with_context(
            &run_state.agent_id,
            run_state.session_id.as_deref(),
            run_state.turn_id.as_deref(),
            "run_resumed",
            None,
            None,
            &serde_json::json!({
                "run_id": run_state.run_id,
                "approval_id": run_state.approval_id,
                "reviewer": reviewer,
                "note": note
            }),
        )?;
        Ok(AgentRunResumeReport {
            run_state: self.agent_run_state(run_id)?,
        })
    }

    pub fn show_agent_approval(&self, approval_id: &str) -> Result<AgentApproval> {
        self.agent_approval(approval_id)
    }

    pub fn decide_agent_approval(
        &mut self,
        approval_id: &str,
        decision: &str,
        reviewer: Option<String>,
        note: Option<String>,
    ) -> Result<AgentApprovalDecisionReport> {
        let _lock = self.acquire_write_lock()?;
        let decision = parse_approval_decision(decision)?;
        let approval = self.agent_approval(approval_id)?;
        if approval.status != "pending" {
            return Err(Error::InvalidInput(format!(
                "approval `{approval_id}` is already {}",
                approval.status
            )));
        }
        let reviewer = reviewer.map(|reviewer| redact_sensitive_text(&reviewer));
        let note = note.map(|note| redact_sensitive_text(&note));
        let decided_at = now_ts();
        self.conn.execute(
            "UPDATE agent_approvals SET status = ?1, decided_at = ?2, reviewer = ?3, note = ?4 WHERE approval_id = ?5",
            params![decision, decided_at, reviewer.clone(), note.clone(), approval_id],
        )?;
        self.insert_agent_event_with_context(
            &approval.agent_id,
            approval.session_id.as_deref(),
            approval.turn_id.as_deref(),
            "approval_decided",
            None,
            None,
            &serde_json::json!({
                "approval_id": approval_id,
                "decision": decision,
                "reviewer": reviewer,
                "note": note
            }),
        )?;
        if matches!(decision, "rejected" | "cancelled") {
            let run_status = if decision == "rejected" {
                "blocked"
            } else {
                "cancelled"
            };
            self.conn.execute(
                "UPDATE agent_run_states SET status = ?1, updated_at = ?2, reviewer = ?3, note = ?4 WHERE approval_id = ?5 AND status = 'paused'",
                params![run_status, decided_at, reviewer.clone(), note.clone(), approval_id],
            )?;
        }
        Ok(AgentApprovalDecisionReport {
            approval: self.agent_approval(approval_id)?,
            decision: decision.to_string(),
            run_states: self.agent_run_states_for_approval(approval_id)?,
        })
    }

    pub fn start_agent_session(
        &mut self,
        agent: &str,
        title: Option<String>,
        requested_session_id: Option<String>,
    ) -> Result<AgentSessionStartReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;
        let branch = self.agent_branch(agent)?;
        let session_id = match requested_session_id {
            Some(session_id) => {
                validate_session_id(&session_id)?;
                session_id
            }
            None => self.allocate_session_id(&branch.agent_id, title.as_deref()),
        };
        if self.try_agent_session(&session_id)?.is_some() {
            return Err(Error::InvalidInput(format!(
                "session `{session_id}` already exists"
            )));
        }
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO agent_sessions \
             (session_id, agent_id, title, status, started_at, ended_at, metadata_json) \
             VALUES (?1, ?2, ?3, 'active', ?4, NULL, NULL)",
            params![session_id, branch.agent_id, title, now],
        )?;
        self.conn.execute(
            "UPDATE agent_branches SET session_id = ?1, updated_at = ?2 WHERE agent_id = ?3",
            params![session_id, now, branch.agent_id],
        )?;
        self.insert_agent_event_with_context(
            &branch.agent_id,
            Some(&session_id),
            None,
            "session_started",
            Some(&branch.head_change),
            None,
            &serde_json::json!({
                "session_id": session_id.clone(),
                "title": title.clone()
            }),
        )?;
        Ok(AgentSessionStartReport {
            session: self.agent_session(&session_id)?,
        })
    }

    pub fn list_agent_sessions(&self, agent: Option<&str>) -> Result<Vec<AgentSession>> {
        if let Some(agent) = agent {
            let branch = self.agent_branch(agent)?;
            let mut stmt = self.conn.prepare(
                "SELECT session_id, agent_id, title, status, started_at, ended_at, metadata_json \
                 FROM agent_sessions WHERE agent_id = ?1 ORDER BY started_at DESC, session_id DESC",
            )?;
            let rows = stmt.query_map(params![branch.agent_id], agent_session_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Error::from)
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT session_id, agent_id, title, status, started_at, ended_at, metadata_json \
                 FROM agent_sessions ORDER BY started_at DESC, session_id DESC",
            )?;
            let rows = stmt.query_map([], agent_session_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Error::from)
        }
    }

    pub fn current_agent_sessions(
        &self,
        agent: Option<&str>,
    ) -> Result<Vec<AgentSessionCurrentReport>> {
        if let Some(agent) = agent {
            let details = self.agent_details(agent)?;
            let session = details
                .branch
                .session_id
                .as_deref()
                .map(|session_id| self.agent_session(session_id))
                .transpose()?;
            return Ok(vec![AgentSessionCurrentReport {
                agent_id: details.record.agent_id,
                agent_name: details.record.name,
                ref_name: details.branch.ref_name,
                session,
            }]);
        }

        let mut reports = Vec::new();
        for details in self.list_agents()? {
            let Some(session_id) = details.branch.session_id.as_deref() else {
                continue;
            };
            reports.push(AgentSessionCurrentReport {
                agent_id: details.record.agent_id,
                agent_name: details.record.name,
                ref_name: details.branch.ref_name,
                session: Some(self.agent_session(session_id)?),
            });
        }
        Ok(reports)
    }

    pub fn show_agent_session(&self, session_id: &str) -> Result<AgentSessionDetails> {
        let session = self.agent_session(session_id)?;
        let turns = self.agent_session_turns(session_id)?;
        let messages = self.agent_session_messages(session_id)?;
        let events = self.agent_session_events(session_id)?;
        let operations = self.agent_session_operations(session_id)?;
        Ok(AgentSessionDetails {
            session,
            turns,
            messages,
            events,
            operations,
        })
    }

    pub fn agent_session_context(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<AgentSessionContextReport> {
        let limit = normalize_query_limit(limit, 200)?;
        let session = self.agent_session(session_id)?;
        let turns = self.agent_session_turns(session_id)?;
        let messages = self.agent_session_messages(session_id)?;
        let events = self.agent_session_events(session_id)?;
        let operations = self.agent_session_operations(session_id)?;
        Ok(AgentSessionContextReport {
            session,
            message_count: messages.len() as u64,
            event_count: events.len() as u64,
            turn_count: turns.len() as u64,
            operation_count: operations.len() as u64,
            recent_messages: tail_limited(&messages, limit),
            recent_events: tail_limited(&events, limit),
            recent_turns: tail_limited(&turns, limit),
            recent_operations: tail_limited(&operations, limit),
        })
    }

    pub fn end_agent_session(
        &mut self,
        session_id: &str,
        status: &str,
    ) -> Result<AgentSessionEndReport> {
        let _lock = self.acquire_write_lock()?;
        let status = parse_session_end_status(status)?;
        let session = self.agent_session(session_id)?;
        let now = now_ts();
        self.conn.execute(
            "UPDATE agent_sessions SET status = ?1, ended_at = ?2 WHERE session_id = ?3",
            params![status, now, session_id],
        )?;
        self.conn.execute(
            "UPDATE agent_branches SET session_id = NULL, updated_at = ?1 \
             WHERE agent_id = ?2 AND session_id = ?3",
            params![now, session.agent_id, session_id],
        )?;
        self.insert_agent_event_with_context(
            &session.agent_id,
            Some(session_id),
            None,
            "session_ended",
            None,
            None,
            &serde_json::json!({
                "session_id": session_id,
                "status": status
            }),
        )?;
        Ok(AgentSessionEndReport {
            session: self.agent_session(session_id)?,
        })
    }

    pub fn record_agent_workdir(
        &mut self,
        agent: &str,
        message: Option<String>,
    ) -> Result<AgentRecordReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;
        let branch = self.agent_branch(agent)?;
        let Some(workdir) = branch.workdir.clone() else {
            return Err(Error::InvalidInput(format!(
                "agent `{agent}` does not have a materialized workdir"
            )));
        };
        let workdir_path = PathBuf::from(&workdir);
        if !workdir_path.is_dir() {
            return Err(Error::WorkspaceNotFound(workdir_path));
        }
        let head = self.get_ref(&branch.ref_name)?;
        let previous_files = self.load_root_files(&head.root_id)?;
        let disk_files = self.scan_files_under(&workdir_path)?;
        let actor = Actor::agent(agent);
        let change_id = self.allocate_change_id(&actor.id, "agent_record")?;
        let built =
            self.build_root_from_disk_files(&disk_files, &change_id, Some(&previous_files))?;
        let diff = self.diff_file_maps(&previous_files, &built.files)?;
        if diff.changes.is_empty() {
            return Ok(AgentRecordReport {
                agent_id: branch.agent_id,
                operation: None,
                root_id: head.root_id,
                changed_paths: Vec::new(),
            });
        }
        if let Some(session_id) = &branch.session_id {
            self.ensure_agent_session(&branch.agent_id, session_id, None)?;
        }
        let turn_id = self.open_agent_turn(
            &branch.agent_id,
            branch.session_id.as_deref(),
            &branch.base_change,
            &head.change_id,
            Some(&serde_json::json!({
                "kind": "workdir_record",
                "path_count": diff.summaries.len()
            })),
        )?;

        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind: OperationKind::AgentRecord,
            parents: vec![head.change_id.clone()],
            before_root: Some(head.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: branch.ref_name.clone(),
            actor,
            session_id: branch.session_id.clone(),
            message: message.as_deref().map(redact_sensitive_text),
            changes: diff.changes,
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&head, &change_id, &built.root_id, &operation_id)?;
        let message_id = if let Some(message) = message {
            Some(self.store_message(
                "agent",
                &message,
                Some(&branch.agent_id),
                branch.session_id.as_deref(),
                Some(&change_id),
                operation.created_at,
            )?)
        } else {
            None
        };
        self.conn.execute(
            "UPDATE agent_branches SET head_change = ?1, head_root = ?2, updated_at = ?3 WHERE agent_id = ?4",
            params![change_id.0, built.root_id.0, now_ts(), branch.agent_id],
        )?;
        self.insert_agent_event_with_context(
            &branch.agent_id,
            branch.session_id.as_deref(),
            Some(&turn_id),
            "workdir_recorded",
            Some(&change_id),
            message_id.as_ref(),
            &serde_json::json!({
                "workdir": workdir,
                "root_id": built.root_id.0.clone(),
                "session_id": branch.session_id.clone(),
                "changed_paths": diff.summaries.iter().map(|item| item.path.clone()).collect::<Vec<_>>()
            }),
        )?;
        self.finish_agent_turn(&turn_id, "completed", Some(&change_id))?;
        Ok(AgentRecordReport {
            agent_id: branch.agent_id,
            operation: Some(change_id),
            root_id: built.root_id,
            changed_paths: diff.summaries,
        })
    }

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

    pub fn watch_agent_workdir(
        &mut self,
        agent: &str,
        message: Option<String>,
        interval: Duration,
        iterations: Option<u64>,
    ) -> Result<AgentWatchReport> {
        validate_ref_segment(agent)?;
        let branch = self.agent_branch(agent)?;
        if branch.workdir.is_none() {
            return Err(Error::InvalidInput(format!(
                "agent `{agent}` does not have a materialized workdir"
            )));
        }
        let mut report = AgentWatchReport {
            agent_id: branch.agent_id,
            iterations: 0,
            recorded_operations: Vec::new(),
            changed_paths: Vec::new(),
        };
        loop {
            let record = self.record_agent_workdir(agent, message.clone())?;
            report.iterations += 1;
            if let Some(operation) = record.operation {
                report.recorded_operations.push(operation);
                report.changed_paths.extend(record.changed_paths);
            }
            if iterations.is_some_and(|limit| report.iterations >= limit) {
                break;
            }
            std::thread::sleep(interval);
        }
        Ok(report)
    }

    pub fn run_agent_test(
        &mut self,
        agent: &str,
        command: Vec<String>,
        turn_id: Option<&str>,
        timeout_secs: u64,
    ) -> Result<AgentTestReport> {
        self.run_agent_test_with_options(
            agent,
            command,
            turn_id,
            timeout_secs,
            AgentGateOptions::default(),
        )
    }

    pub fn run_agent_test_with_options(
        &mut self,
        agent: &str,
        command: Vec<String>,
        turn_id: Option<&str>,
        timeout_secs: u64,
        options: AgentGateOptions,
    ) -> Result<AgentTestReport> {
        self.run_agent_gate("test", agent, command, turn_id, timeout_secs, options)
    }

    pub fn run_agent_eval(
        &mut self,
        agent: &str,
        command: Vec<String>,
        turn_id: Option<&str>,
        timeout_secs: u64,
    ) -> Result<AgentTestReport> {
        self.run_agent_eval_with_options(
            agent,
            command,
            turn_id,
            timeout_secs,
            AgentGateOptions::default(),
        )
    }

    pub fn run_agent_eval_with_options(
        &mut self,
        agent: &str,
        command: Vec<String>,
        turn_id: Option<&str>,
        timeout_secs: u64,
        options: AgentGateOptions,
    ) -> Result<AgentTestReport> {
        self.run_agent_gate("eval", agent, command, turn_id, timeout_secs, options)
    }

    fn run_agent_gate(
        &mut self,
        kind: &str,
        agent: &str,
        command: Vec<String>,
        turn_id: Option<&str>,
        timeout_secs: u64,
        options: AgentGateOptions,
    ) -> Result<AgentTestReport> {
        let (started_event_type, finished_event_type, run_kind, passed_status, failed_status) =
            match kind {
                "test" => (
                    "test_started",
                    "test_finished",
                    "test_run",
                    "test_passed",
                    "test_failed",
                ),
                "eval" => (
                    "eval_started",
                    "eval_finished",
                    "eval_run",
                    "eval_passed",
                    "eval_failed",
                ),
                other => {
                    return Err(Error::InvalidInput(format!(
                        "agent gate kind must be test or eval, got `{other}`"
                    )));
                }
            };
        validate_ref_segment(agent)?;
        if command.is_empty() {
            return Err(Error::InvalidInput(format!(
                "agent {kind} requires a command after `--`"
            )));
        }
        if timeout_secs == 0 {
            return Err(Error::InvalidInput(format!(
                "agent {kind} timeout must be greater than zero"
            )));
        }
        let options = normalize_agent_gate_options(kind, options)?;
        let suite = options.suite.clone();
        let score = options.score;
        let threshold = options.threshold;

        let (agent_id, session_id, workdir, turn_id, head_change, started_event_id) = {
            let _lock = self.acquire_write_lock()?;
            let branch = self.agent_branch(agent)?;
            let Some(workdir) = branch.workdir.clone() else {
                return Err(Error::InvalidInput(format!(
                    "agent `{agent}` does not have a materialized workdir"
                )));
            };
            let workdir_path = PathBuf::from(&workdir);
            if !workdir_path.is_dir() {
                return Err(Error::WorkspaceNotFound(workdir_path));
            }
            let head = self.get_ref(&branch.ref_name)?;
            let (turn_id, session_id) = if let Some(turn_id) = turn_id {
                let turn = self.agent_turn(turn_id)?;
                if turn.agent_id != branch.agent_id {
                    return Err(Error::InvalidInput(format!(
                        "turn `{turn_id}` does not belong to agent `{agent}`"
                    )));
                }
                if turn.ended_at.is_some() {
                    return Err(Error::InvalidInput(format!(
                        "turn `{turn_id}` is already ended"
                    )));
                }
                (turn.turn_id, turn.session_id)
            } else {
                let turn_id = self.open_agent_turn(
                    &branch.agent_id,
                    branch.session_id.as_deref(),
                    &branch.base_change,
                    &head.change_id,
                    Some(&serde_json::json!({
                        "kind": run_kind,
                        "command": command.clone(),
                        "suite": suite.clone(),
                        "score": score,
                        "threshold": threshold
                    })),
                )?;
                (turn_id, branch.session_id.clone())
            };
            let started_event_id = self.insert_agent_event_with_context(
                &branch.agent_id,
                session_id.as_deref(),
                Some(&turn_id),
                started_event_type,
                Some(&head.change_id),
                None,
                &serde_json::json!({
                    "kind": kind,
                    "command": command.clone(),
                    "suite": suite.clone(),
                    "score": score,
                    "threshold": threshold,
                    "workdir": workdir.clone(),
                    "timeout_secs": timeout_secs,
                    "head_change": head.change_id.0.clone()
                }),
            )?;
            (
                branch.agent_id,
                session_id,
                workdir,
                turn_id,
                head.change_id,
                started_event_id,
            )
        };

        let run = run_command_with_timeout(
            &command,
            Path::new(&workdir),
            Duration::from_secs(timeout_secs),
        )?;
        let threshold_met = score
            .zip(threshold)
            .map(|(score, threshold)| score >= threshold);
        let gate_success = run.success && threshold_met.unwrap_or(true);
        let status = if gate_success {
            passed_status
        } else {
            failed_status
        }
        .to_string();
        let stdout_bytes = run.stdout.len() as u64;
        let stderr_bytes = run.stderr.len() as u64;
        let stdout_hash = sha256_hex(&run.stdout);
        let stderr_hash = sha256_hex(&run.stderr);
        let (stdout_preview, stdout_truncated) = output_preview(&run.stdout);
        let (stderr_preview, stderr_truncated) = output_preview(&run.stderr);

        let (stdout_object, stderr_object, finished_event_id) = {
            let _lock = self.acquire_write_lock()?;
            let stdout_object = self.put_blob(run.stdout.clone())?;
            let stderr_object = self.put_blob(run.stderr.clone())?;
            let finished_event_id = self.insert_agent_event_with_context(
                &agent_id,
                session_id.as_deref(),
                Some(&turn_id),
                finished_event_type,
                Some(&head_change),
                None,
                &serde_json::json!({
                    "kind": kind,
                    "command": command.clone(),
                    "suite": suite.clone(),
                    "score": score,
                    "threshold": threshold,
                    "threshold_met": threshold_met,
                    "status": status.clone(),
                    "success": gate_success,
                    "process_success": run.success,
                    "exit_code": run.exit_code,
                    "timed_out": run.timed_out,
                    "duration_ms": run.duration_ms,
                    "stdout_object": stdout_object.0.clone(),
                    "stderr_object": stderr_object.0.clone(),
                    "stdout_bytes": stdout_bytes,
                    "stderr_bytes": stderr_bytes,
                    "stdout_hash": stdout_hash,
                    "stderr_hash": stderr_hash,
                    "stdout_preview": stdout_preview.clone(),
                    "stderr_preview": stderr_preview.clone(),
                    "stdout_truncated": stdout_truncated,
                    "stderr_truncated": stderr_truncated
                }),
            )?;
            self.finish_agent_turn(&turn_id, &status, Some(&head_change))?;
            (stdout_object, stderr_object, finished_event_id)
        };

        Ok(AgentTestReport {
            agent_id,
            turn_id,
            session_id,
            workdir,
            command,
            kind: kind.to_string(),
            suite,
            score,
            threshold,
            status,
            success: gate_success,
            exit_code: run.exit_code,
            timed_out: run.timed_out,
            duration_ms: run.duration_ms,
            stdout_object,
            stderr_object,
            stdout_bytes,
            stderr_bytes,
            stdout_preview,
            stderr_preview,
            stdout_truncated,
            stderr_truncated,
            started_event_id,
            finished_event_id,
        })
    }

    pub fn agent_timeline(&self, agent: &str, limit: usize) -> Result<Vec<TimelineEntry>> {
        let branch = self.agent_branch(agent)?;
        let mut stmt = self.conn.prepare(
            "SELECT change_id, kind, branch, actor_id, message, created_at, path_count \
             FROM operations WHERE branch = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![branch.ref_name, limit as i64], timeline_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub fn checkout_agent(&mut self, agent: &str, force: bool) -> Result<CheckoutReport> {
        self.checkout_agent_with_options(agent, force, false, None)
    }

    pub fn checkout_agent_with_options(
        &mut self,
        agent: &str,
        force: bool,
        dry_run: bool,
        workdir: Option<&Path>,
    ) -> Result<CheckoutReport> {
        let ref_name = self.agent_branch(agent)?.ref_name;
        self.checkout_with_options(&ref_name, force, dry_run, workdir, false)
    }

    pub fn remove_agent(&mut self, agent: &str, force: bool) -> Result<AgentRemoveReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;
        let branch = self.agent_branch(agent)?;
        if branch.status != "merged" && branch.head_change != branch.base_change && !force {
            return Err(Error::InvalidInput(format!(
                "agent `{agent}` has unmerged changes; pass --force to remove"
            )));
        }
        remove_ref_file(&self.db_dir, &branch.ref_name)?;
        self.conn
            .execute("DELETE FROM refs WHERE name = ?1", params![branch.ref_name])?;
        if let Some(workdir) = &branch.workdir {
            let path = PathBuf::from(workdir);
            if path.exists() {
                fs::remove_dir_all(&path)?;
            }
        }
        self.conn.execute(
            "UPDATE agent_branches SET status = 'removed', updated_at = ?1 WHERE agent_id = ?2",
            params![now_ts(), branch.agent_id],
        )?;
        self.insert_agent_event(
            &branch.agent_id,
            "agent_removed",
            Some(&branch.head_change),
            None,
            &serde_json::json!({
                "ref_name": branch.ref_name.clone(),
                "forced": force
            }),
        )?;
        Ok(AgentRemoveReport {
            agent_id: branch.agent_id,
            ref_name: branch.ref_name,
            removed_workdir: branch.workdir,
            forced: force,
        })
    }

    pub fn acquire_lease(
        &mut self,
        agent: &str,
        path: Option<&str>,
        mode: &str,
        ttl_secs: u64,
    ) -> Result<LeaseAcquireReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;
        let mode = parse_lease_mode(mode)?;
        if ttl_secs == 0 {
            return Err(Error::InvalidInput(
                "lease ttl must be greater than zero".to_string(),
            ));
        }
        let branch = self.agent_branch(agent)?;
        let path = path.map(normalize_relative_path).transpose()?;
        let file_id = if let Some(path) = &path {
            let ref_record = self.get_ref(&branch.ref_name)?;
            let files = self.load_root_files(&ref_record.root_id)?;
            files.get(path).map(|entry| file_id_key(&entry.file_id))
        } else {
            None
        };
        let now = now_ts();
        if let Some(existing) =
            self.existing_active_lease(&branch.agent_id, path.as_deref(), mode)?
        {
            return Ok(LeaseAcquireReport { lease: existing });
        }
        let conflicts = self.conflicting_active_leases(&branch.agent_id, path.as_deref(), mode)?;
        if !conflicts.is_empty() {
            let holders = conflicts
                .iter()
                .map(|lease| format!("{} {}", lease.agent_id, lease.lease_id))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(Error::Conflict(format!(
                "active lease conflict on {} held by {holders}",
                path.as_deref().unwrap_or("<workspace>")
            )));
        }

        let expires_at = now + ttl_secs as i64;
        let seed = format!(
            "{}:{}:{}:{}:{}:{}",
            branch.agent_id,
            branch.ref_name,
            path.as_deref().unwrap_or("workspace"),
            mode,
            expires_at,
            now_nanos()
        );
        let lease_id = format!("lease_{}", crate::ids::short_hash(seed.as_bytes(), 16));
        self.conn.execute(
            "INSERT INTO leases \
             (lease_id, agent_id, ref_name, path, file_id, mode, expires_at, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                lease_id,
                branch.agent_id,
                branch.ref_name,
                path,
                file_id,
                mode,
                expires_at,
                now
            ],
        )?;
        let lease = self.lease(&lease_id)?;
        self.insert_agent_event(
            &branch.agent_id,
            "lease_acquired",
            Some(&branch.head_change),
            None,
            &serde_json::json!({
                "lease_id": lease.lease_id,
                "path": lease.path,
                "mode": lease.mode,
                "expires_at": lease.expires_at
            }),
        )?;
        Ok(LeaseAcquireReport { lease })
    }

    pub fn claim_agent_path(
        &mut self,
        agent: &str,
        path: &str,
        ttl_secs: u64,
    ) -> Result<AgentClaimReport> {
        let _lock = self.acquire_write_lock()?;
        validate_ref_segment(agent)?;
        if ttl_secs == 0 {
            return Err(Error::InvalidInput(
                "agent claim ttl must be greater than zero".to_string(),
            ));
        }
        let branch = self.agent_branch(agent)?;
        let path = normalize_relative_path(path)?;
        let mode = "write";
        if let Some(existing) = self.existing_active_lease(&branch.agent_id, Some(&path), mode)? {
            return Ok(AgentClaimReport {
                agent_id: branch.agent_id,
                ref_name: branch.ref_name,
                path,
                mode: mode.to_string(),
                ttl_secs,
                claimed: true,
                lease: Some(existing),
                conflicts: Vec::new(),
                warning: None,
            });
        }

        let conflicts = self.conflicting_active_leases(&branch.agent_id, Some(&path), mode)?;
        if !conflicts.is_empty() {
            let holders = conflicts
                .iter()
                .map(|lease| format!("{} {}", lease.agent_id, lease.lease_id))
                .collect::<Vec<_>>()
                .join(", ");
            let warning = format!("`{path}` is already claimed by {holders}");
            self.insert_agent_event(
                &branch.agent_id,
                "claim_conflicted",
                Some(&branch.head_change),
                None,
                &serde_json::json!({
                    "path": &path,
                    "mode": mode,
                    "conflicts": &conflicts,
                    "warning": &warning
                }),
            )?;
            return Ok(AgentClaimReport {
                agent_id: branch.agent_id,
                ref_name: branch.ref_name,
                path,
                mode: mode.to_string(),
                ttl_secs,
                claimed: false,
                lease: None,
                conflicts,
                warning: Some(warning),
            });
        }

        let file_id = {
            let ref_record = self.get_ref(&branch.ref_name)?;
            let files = self.load_root_files(&ref_record.root_id)?;
            files.get(&path).map(|entry| file_id_key(&entry.file_id))
        };
        let now = now_ts();
        let expires_at = now + ttl_secs as i64;
        let seed = format!(
            "{}:{}:{}:{}:{}:{}",
            branch.agent_id,
            branch.ref_name,
            path,
            mode,
            expires_at,
            now_nanos()
        );
        let lease_id = format!("lease_{}", crate::ids::short_hash(seed.as_bytes(), 16));
        self.conn.execute(
            "INSERT INTO leases \
             (lease_id, agent_id, ref_name, path, file_id, mode, expires_at, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                lease_id,
                branch.agent_id,
                branch.ref_name,
                path,
                file_id,
                mode,
                expires_at,
                now
            ],
        )?;
        let lease = self.lease(&lease_id)?;
        self.insert_agent_event(
            &lease.agent_id,
            "agent_claimed_path",
            Some(&branch.head_change),
            None,
            &serde_json::json!({
                "lease_id": &lease.lease_id,
                "path": &lease.path,
                "mode": &lease.mode,
                "expires_at": lease.expires_at
            }),
        )?;
        Ok(AgentClaimReport {
            agent_id: lease.agent_id.clone(),
            ref_name: lease.ref_name.clone(),
            path: lease.path.clone().unwrap_or_else(|| path.to_string()),
            mode: lease.mode.clone(),
            ttl_secs,
            claimed: true,
            lease: Some(lease),
            conflicts: Vec::new(),
            warning: None,
        })
    }

    pub fn list_leases(&self, include_expired: bool) -> Result<Vec<LeaseRecord>> {
        if include_expired {
            let mut stmt = self.conn.prepare(
                "SELECT lease_id, agent_id, ref_name, path, file_id, mode, expires_at, created_at \
                 FROM leases ORDER BY expires_at ASC, created_at ASC",
            )?;
            let rows = stmt.query_map([], lease_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Error::from)
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT lease_id, agent_id, ref_name, path, file_id, mode, expires_at, created_at \
                 FROM leases WHERE expires_at > ?1 ORDER BY expires_at ASC, created_at ASC",
            )?;
            let rows = stmt.query_map(params![now_ts()], lease_row)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Error::from)
        }
    }

    pub fn release_lease(&mut self, lease_id: &str) -> Result<LeaseReleaseReport> {
        let _lock = self.acquire_write_lock()?;
        let lease = self.lease(lease_id)?;
        let deleted = self
            .conn
            .execute("DELETE FROM leases WHERE lease_id = ?1", params![lease_id])?;
        if deleted > 0 {
            self.insert_agent_event(
                &lease.agent_id,
                "lease_released",
                None,
                None,
                &serde_json::json!({
                    "lease_id": lease.lease_id,
                    "path": lease.path,
                    "mode": lease.mode
                }),
            )?;
        }
        Ok(LeaseReleaseReport {
            lease_id: lease_id.to_string(),
            released: deleted > 0,
        })
    }

    pub fn apply_agent_patch(
        &mut self,
        agent: &str,
        patch: PatchDocument,
    ) -> Result<AgentPatchReport> {
        let _lock = self.acquire_write_lock()?;
        self.apply_agent_patch_locked(agent, patch, None)
    }

    pub fn apply_agent_turn_patch(
        &mut self,
        turn_id: &str,
        patch: PatchDocument,
    ) -> Result<AgentPatchReport> {
        let _lock = self.acquire_write_lock()?;
        let turn = self.agent_turn(turn_id)?;
        if turn.ended_at.is_some() {
            return Err(Error::InvalidInput(format!(
                "turn `{turn_id}` is already ended"
            )));
        }
        self.apply_agent_patch_locked(&turn.agent_id, patch, Some(&turn))
    }

    pub fn end_agent_turn(&mut self, turn_id: &str, status: &str) -> Result<AgentTurnEndReport> {
        let _lock = self.acquire_write_lock()?;
        let status = parse_session_end_status(status)?;
        let turn = self.agent_turn(turn_id)?;
        if turn.ended_at.is_some() {
            return Ok(AgentTurnEndReport { turn });
        }
        let after_change = turn
            .after_change
            .as_ref()
            .unwrap_or(&turn.before_change)
            .clone();
        self.finish_agent_turn(turn_id, status, Some(&after_change))?;
        self.insert_agent_event_with_context(
            &turn.agent_id,
            turn.session_id.as_deref(),
            Some(turn_id),
            "turn_ended",
            Some(&after_change),
            None,
            &serde_json::json!({
                "turn_id": turn_id,
                "status": status
            }),
        )?;
        Ok(AgentTurnEndReport {
            turn: self.agent_turn(turn_id)?,
        })
    }

    fn apply_agent_patch_locked(
        &mut self,
        agent: &str,
        patch: PatchDocument,
        api_turn: Option<&AgentTurn>,
    ) -> Result<AgentPatchReport> {
        validate_ref_segment(agent)?;
        let agent_row = self.agent_branch(agent)?;
        let ref_name = agent_row.ref_name.clone();
        let head = self.get_ref(&ref_name)?;
        if let Some(turn) = api_turn {
            if turn.agent_id != agent_row.agent_id {
                return Err(Error::InvalidInput(format!(
                    "turn `{}` belongs to another agent",
                    turn.turn_id
                )));
            }
            if turn.before_change != head.change_id {
                return Err(Error::StaleBranch(ref_name));
            }
        }
        if let Some(base_change) = &patch.base_change {
            if base_change != &head.change_id.0 {
                return Err(Error::PatchRejected(format!(
                    "patch base {base_change} does not match agent head {}",
                    head.change_id.0
                )));
            }
        }
        for edit in &patch.edits {
            self.ensure_patch_edit_allowed(edit, patch.allow_ignored)?;
        }

        let previous_files = self.load_root_files(&head.root_id)?;
        let actor = Actor::agent(agent);
        let change_id = self.allocate_change_id(&actor.id, "agent_patch")?;
        let mut files = previous_files.clone();
        let mut manual_line_changes = Vec::new();
        let mut file_seq = 1;
        let mut line_seq = 1;

        for edit in patch.edits {
            match edit {
                PatchEdit::Write {
                    path,
                    content,
                    executable,
                } => {
                    let path = normalize_relative_path(&path)?;
                    let previous = files.get(&path);
                    let built = self.build_file_entry(
                        &path,
                        content.into_bytes(),
                        executable,
                        &change_id,
                        previous,
                        &mut file_seq,
                        &mut line_seq,
                    )?;
                    manual_line_changes.extend(
                        built
                            .line_changes
                            .iter()
                            .map(|line| (path.clone(), built.entry.file_id.clone(), line.clone())),
                    );
                    files.insert(path, built.entry);
                }
                PatchEdit::WriteBytes {
                    path,
                    bytes_hex,
                    executable,
                } => {
                    let path = normalize_relative_path(&path)?;
                    let bytes = hex::decode(bytes_hex).map_err(|err| {
                        Error::PatchRejected(format!("invalid bytes_hex for `{path}`: {err}"))
                    })?;
                    let previous = files.get(&path);
                    let built = self.build_file_entry(
                        &path,
                        bytes,
                        executable,
                        &change_id,
                        previous,
                        &mut file_seq,
                        &mut line_seq,
                    )?;
                    manual_line_changes.extend(
                        built
                            .line_changes
                            .iter()
                            .map(|line| (path.clone(), built.entry.file_id.clone(), line.clone())),
                    );
                    files.insert(path, built.entry);
                }
                PatchEdit::ReplaceLine {
                    path,
                    line_id,
                    expected_text,
                    new_text,
                } => {
                    let path = normalize_relative_path(&path)?;
                    let Some(entry) = files.get(&path).cloned() else {
                        return Err(Error::PatchRejected(format!(
                            "replace_line path `{path}` is absent"
                        )));
                    };
                    let FileContentRef::Text(text_id) = &entry.content else {
                        return Err(Error::PatchRejected(format!(
                            "replace_line path `{path}` is not text"
                        )));
                    };
                    let mut lines = self.load_text_lines(text_id)?;
                    let Some(line_idx) =
                        lines.iter().position(|line| line.line_id_key() == line_id)
                    else {
                        return Err(Error::PatchRejected(format!(
                            "replace_line line_id `{line_id}` not found in `{path}`"
                        )));
                    };
                    if let Some(expected_text) = expected_text {
                        let actual = String::from_utf8_lossy(&lines[line_idx].text);
                        if actual != expected_text {
                            return Err(Error::PatchRejected(format!(
                                "replace_line expected text mismatch for `{path}` {line_id}"
                            )));
                        }
                    }
                    let before_hash = lines[line_idx].text_hash.clone();
                    lines[line_idx].text = new_text.into_bytes();
                    lines[line_idx].text_hash = sha256_hex(&lines[line_idx].text);
                    lines[line_idx].last_content_change = change_id.clone();
                    let text_id = self.put_text_content_from_lines(&lines)?;
                    let bytes = materialize_lines(&lines);
                    let mut next_entry = entry.clone();
                    next_entry.content = FileContentRef::Text(text_id);
                    next_entry.size_bytes = bytes.len() as u64;
                    next_entry.content_hash = sha256_hex(&bytes);
                    next_entry.last_content_change = change_id.clone();
                    manual_line_changes.push((
                        path.clone(),
                        next_entry.file_id.clone(),
                        LineChange {
                            line_id: lines[line_idx].line_id.clone(),
                            kind: LineChangeKind::Modified,
                            old_line_number: Some(line_idx as u64 + 1),
                            new_line_number: Some(line_idx as u64 + 1),
                            before_hash: Some(before_hash),
                            after_hash: Some(lines[line_idx].text_hash.clone()),
                        },
                    ));
                    files.insert(path, next_entry);
                }
                PatchEdit::Delete { path } => {
                    let path = normalize_relative_path(&path)?;
                    if files.remove(&path).is_none() {
                        return Err(Error::PatchRejected(format!(
                            "delete path `{path}` is absent"
                        )));
                    }
                }
                PatchEdit::Rename { from, to } => {
                    let from = normalize_relative_path(&from)?;
                    let to = normalize_relative_path(&to)?;
                    if files.contains_key(&to) {
                        return Err(Error::PatchRejected(format!(
                            "rename destination `{to}` already exists"
                        )));
                    }
                    let Some(mut entry) = files.remove(&from) else {
                        return Err(Error::PatchRejected(format!(
                            "rename source `{from}` is absent"
                        )));
                    };
                    entry.last_path_change = Some(change_id.clone());
                    files.insert(to, entry);
                }
            }
        }

        let built = self.build_root_from_file_entries(files, &change_id)?;
        let mut diff = self.diff_file_maps(&previous_files, &built.files)?;
        for (path, file_id, line) in manual_line_changes {
            if let Some(change) = diff
                .changes
                .iter_mut()
                .find(|change| change.path == path && change.file_id.as_ref() == Some(&file_id))
            {
                if !change
                    .line_changes
                    .iter()
                    .any(|existing| existing.line_id == line.line_id)
                {
                    change.line_changes.push(line);
                }
            }
        }
        if diff.changes.is_empty() {
            return Err(Error::PatchRejected(
                "patch produced no changes".to_string(),
            ));
        }

        let patch_message = patch.message.as_deref().map(redact_sensitive_text);
        let patch_session_id = if let Some(turn) = api_turn {
            if patch.session_id.is_some() && patch.session_id != turn.session_id {
                return Err(Error::InvalidInput(format!(
                    "patch session does not match turn `{}`",
                    turn.turn_id
                )));
            }
            turn.session_id.clone()
        } else {
            patch.session_id.clone().or(agent_row.session_id.clone())
        };
        if let Some(session_id) = &patch_session_id {
            self.ensure_agent_session(&agent_row.agent_id, session_id, None)?;
        }
        let turn_id = if let Some(turn) = api_turn {
            turn.turn_id.clone()
        } else {
            self.open_agent_turn(
                &agent_row.agent_id,
                patch_session_id.as_deref(),
                &agent_row.base_change,
                &head.change_id,
                Some(&serde_json::json!({
                    "kind": "patch",
                    "path_count": diff.summaries.len()
                })),
            )?
        };
        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind: OperationKind::AgentPatch,
            parents: vec![head.change_id.clone()],
            before_root: Some(head.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: ref_name.clone(),
            actor,
            session_id: patch_session_id.clone(),
            message: patch_message.clone(),
            changes: diff.changes,
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&head, &change_id, &built.root_id, &operation_id)?;
        let message_id = if let Some(message) = patch_message {
            Some(self.store_message(
                "agent",
                &message,
                Some(&agent_row.agent_id),
                patch_session_id.as_deref(),
                Some(&change_id),
                operation.created_at,
            )?)
        } else {
            None
        };
        self.insert_agent_event_with_context(
                &agent_row.agent_id,
                patch_session_id.as_deref(),
                Some(&turn_id),
                "patch_applied",
            Some(&change_id),
            message_id.as_ref(),
                &serde_json::json!({
                    "ref_name": ref_name.clone(),
                    "root_id": built.root_id.0.clone(),
                    "session_id": patch_session_id.clone(),
                    "allow_ignored": patch.allow_ignored,
                    "changed_paths": diff.summaries.iter().map(|item| item.path.clone()).collect::<Vec<_>>()
                }),
            )?;
        self.conn.execute(
            "UPDATE agent_branches SET head_change = ?1, head_root = ?2, session_id = COALESCE(?3, session_id), updated_at = ?4 \
             WHERE agent_id = ?5",
            params![
                change_id.0,
                built.root_id.0,
                patch_session_id,
                now_ts(),
                agent_row.agent_id
            ],
        )?;
        if let Some(workdir) = agent_row.workdir {
            let previous = self.load_root_files(&head.root_id)?;
            materialize_into(
                &self.workspace_root,
                Path::new(&workdir),
                &previous,
                &built.files,
                |entry| self.materialize_entry_bytes(entry),
            )?;
        }
        if api_turn.is_some() {
            self.update_agent_turn_progress(&turn_id, "patch_applied", Some(&change_id))?;
        } else {
            self.finish_agent_turn(&turn_id, "patch_applied", Some(&change_id))?;
        }
        Ok(AgentPatchReport {
            agent_id: agent_row.agent_id,
            operation: change_id,
            root_id: built.root_id,
            changed_paths: diff.summaries,
        })
    }

    fn ensure_patch_edit_allowed(&self, edit: &PatchEdit, allow_ignored: bool) -> Result<()> {
        match edit {
            PatchEdit::Write { path, .. }
            | PatchEdit::WriteBytes { path, .. }
            | PatchEdit::ReplaceLine { path, .. }
            | PatchEdit::Delete { path } => {
                let path = normalize_relative_path(path)?;
                self.ensure_patch_path_allowed(&path, allow_ignored)
            }
            PatchEdit::Rename { from, to } => {
                let from = normalize_relative_path(from)?;
                let to = normalize_relative_path(to)?;
                self.ensure_patch_path_allowed(&from, allow_ignored)?;
                self.ensure_patch_path_allowed(&to, allow_ignored)
            }
        }
    }

    fn ensure_patch_path_allowed(&self, path: &str, allow_ignored: bool) -> Result<()> {
        if is_internal_path(path) {
            return Err(Error::IgnoredPath(path.to_string()));
        }
        if allow_ignored {
            return Ok(());
        }
        let report = self.ignore_check(path)?;
        if report.ignored {
            return Err(Error::IgnoredPath(path.to_string()));
        }
        Ok(())
    }

    pub fn diff_agent(&self, agent: &str, patches: bool) -> Result<DiffSummary> {
        self.diff_agent_with_options(agent, patches, false)
    }

    pub fn diff_agent_with_options(
        &self,
        agent: &str,
        patches: bool,
        line_changes: bool,
    ) -> Result<DiffSummary> {
        let agent_branch = self.agent_branch(agent)?;
        let source = self.get_ref(&agent_branch.ref_name)?;
        let base = self.ref_from_change(&agent_branch.base_change)?;
        let left_files = self.load_root_files(&base.root_id)?;
        let right_files = self.load_root_files(&source.root_id)?;
        self.diff_files(
            agent_branch.base_change.0,
            source.change_id.0,
            &left_files,
            &right_files,
            patches,
            line_changes,
        )
    }

    pub fn merge_agent(&mut self, agent: &str, into: &str) -> Result<MergeReport> {
        self.merge_agent_with_options(agent, into, false)
    }

    pub fn merge_agent_with_options(
        &mut self,
        agent: &str,
        into: &str,
        dry_run: bool,
    ) -> Result<MergeReport> {
        let _lock = self.acquire_write_lock()?;
        self.merge_agent_unlocked(agent, into, dry_run, true)
    }

    pub fn enqueue_merge(
        &mut self,
        source: &str,
        target: &str,
        priority: i64,
    ) -> Result<MergeQueueAddReport> {
        let _lock = self.acquire_write_lock()?;
        let source_ref = self.normalize_merge_queue_source_ref(source)?;
        let target_ref = self.normalize_merge_queue_target_ref(target)?;
        if let Some(entry) = self
            .conn
            .query_row(
                "SELECT queue_id, source_ref, target_ref, status, priority, created_at, updated_at \
                 FROM merge_queue \
                 WHERE source_ref = ?1 AND target_ref = ?2 AND status IN ('queued', 'running') \
                 ORDER BY created_at LIMIT 1",
                params![source_ref, target_ref],
                merge_queue_row,
            )
            .optional()?
        {
            return Ok(MergeQueueAddReport { entry });
        }

        let now = now_ts();
        let seed = format!("{source_ref}:{target_ref}:{priority}:{now}");
        let hash = sha256_hex(seed.as_bytes());
        let queue_id = format!("mq_{}", &hash[..16]);
        self.conn.execute(
            "INSERT INTO merge_queue \
             (queue_id, source_ref, target_ref, status, priority, created_at, updated_at) \
             VALUES (?1, ?2, ?3, 'queued', ?4, ?5, ?5)",
            params![queue_id, source_ref, target_ref, priority, now],
        )?;

        Ok(MergeQueueAddReport {
            entry: MergeQueueEntry {
                queue_id,
                source_ref,
                target_ref,
                status: "queued".to_string(),
                priority,
                created_at: now,
                updated_at: now,
            },
        })
    }

    pub fn list_merge_queue(&self) -> Result<Vec<MergeQueueEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT queue_id, source_ref, target_ref, status, priority, created_at, updated_at \
             FROM merge_queue ORDER BY status = 'queued' DESC, priority DESC, created_at ASC",
        )?;
        let rows = stmt.query_map([], merge_queue_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub fn remove_merge_queue(&mut self, selector: &str) -> Result<MergeQueueRemoveReport> {
        let _lock = self.acquire_write_lock()?;
        let agent_candidate = agent_ref(selector);
        let branch_candidate = branch_ref(selector);
        let entry = self
            .conn
            .query_row(
                "SELECT queue_id, source_ref, target_ref, status, priority, created_at, updated_at \
                 FROM merge_queue \
                 WHERE (queue_id = ?1 OR source_ref = ?1 OR source_ref = ?2 OR source_ref = ?3) \
                   AND status NOT IN ('merged', 'cancelled') \
                 ORDER BY priority DESC, created_at ASC LIMIT 1",
                params![selector, agent_candidate, branch_candidate],
                merge_queue_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("merge queue item `{selector}` not found")))?;
        let now = now_ts();
        self.conn.execute(
            "UPDATE merge_queue SET status = 'cancelled', updated_at = ?1 WHERE queue_id = ?2",
            params![now, entry.queue_id],
        )?;
        Ok(MergeQueueRemoveReport {
            entry: MergeQueueEntry {
                status: "cancelled".to_string(),
                updated_at: now,
                ..entry
            },
        })
    }

    pub fn run_merge_queue(&mut self, limit: Option<usize>) -> Result<MergeQueueRunReport> {
        let _lock = self.acquire_write_lock()?;
        let entries = self.queued_merge_entries(limit)?;
        let mut processed = Vec::new();
        let mut stopped_on_conflict = false;
        let mut stopped_on_failure = false;

        for entry in entries {
            self.set_merge_queue_status(&entry.queue_id, "running")?;
            let context = match self.merge_queue_context(&entry.source_ref, &entry.target_ref) {
                Ok(context) => context,
                Err(err) => {
                    self.set_merge_queue_status(&entry.queue_id, "failed")?;
                    processed.push(MergeQueueRunItem {
                        queue_id: entry.queue_id,
                        source_ref: entry.source_ref,
                        target_ref: entry.target_ref,
                        status: "failed".to_string(),
                        operation: None,
                        changed_paths: Vec::new(),
                        error: Some(err.to_string()),
                    });
                    stopped_on_failure = true;
                    break;
                }
            };

            match self.merge_queue_entry(&entry) {
                Ok(report) => {
                    self.set_merge_queue_status(&entry.queue_id, "merged")?;
                    self.insert_merge_result(
                        &entry,
                        &context,
                        Some(&report.operation),
                        "merged",
                        None,
                    )?;
                    processed.push(MergeQueueRunItem {
                        queue_id: entry.queue_id,
                        source_ref: report.source_ref,
                        target_ref: report.target_ref,
                        status: "merged".to_string(),
                        operation: Some(report.operation),
                        changed_paths: report.changed_paths,
                        error: None,
                    });
                }
                Err(err) => {
                    let is_conflict = matches!(err, Error::Conflict(_));
                    let status = if is_conflict { "conflicted" } else { "failed" };
                    let message = err.to_string();
                    self.set_merge_queue_status(&entry.queue_id, status)?;
                    self.insert_merge_result(
                        &entry,
                        &context,
                        None,
                        status,
                        is_conflict.then_some(message.as_str()),
                    )?;
                    processed.push(MergeQueueRunItem {
                        queue_id: entry.queue_id,
                        source_ref: entry.source_ref,
                        target_ref: entry.target_ref,
                        status: status.to_string(),
                        operation: None,
                        changed_paths: Vec::new(),
                        error: Some(message),
                    });
                    if is_conflict {
                        stopped_on_conflict = true;
                    } else {
                        stopped_on_failure = true;
                    }
                    break;
                }
            }
        }

        Ok(MergeQueueRunReport {
            processed,
            stopped_on_conflict,
            stopped_on_failure,
        })
    }

    pub fn list_conflicts(&self) -> Result<Vec<ConflictSetSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT conflict_set_id, merge_id, source_ref, target_ref, status, details_json, created_at \
             FROM conflict_sets ORDER BY created_at DESC, conflict_set_id DESC",
        )?;
        let rows = stmt.query_map([], conflict_set_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    pub fn show_conflict(&self, conflict_set_id: &str) -> Result<ConflictSetSummary> {
        self.conn
            .query_row(
                "SELECT conflict_set_id, merge_id, source_ref, target_ref, status, details_json, created_at \
                 FROM conflict_sets WHERE conflict_set_id = ?1",
                params![conflict_set_id],
                conflict_set_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("conflict set `{conflict_set_id}` not found")))
    }

    pub fn resolve_conflict(
        &mut self,
        conflict_set_id: &str,
        take: &str,
    ) -> Result<ConflictResolveReport> {
        let _lock = self.acquire_write_lock()?;
        let take = parse_conflict_take(take)?;
        self.resolve_conflict_unlocked(conflict_set_id, ConflictResolution::Take(take))
    }

    pub fn resolve_conflict_manual(
        &mut self,
        conflict_set_id: &str,
        manual: ConflictManualResolution,
    ) -> Result<ConflictResolveReport> {
        let _lock = self.acquire_write_lock()?;
        self.resolve_conflict_unlocked(conflict_set_id, ConflictResolution::Manual(manual))
    }

    fn resolve_conflict_unlocked(
        &mut self,
        conflict_set_id: &str,
        resolution: ConflictResolution,
    ) -> Result<ConflictResolveReport> {
        let summary = self.show_conflict(conflict_set_id)?;
        if summary.status != "open" {
            return Err(Error::InvalidInput(format!(
                "conflict set `{conflict_set_id}` is {}",
                summary.status
            )));
        }
        let pending = self.pending_conflict_merge(conflict_set_id)?;
        let source_ref = self.get_ref(&pending.source_ref)?;
        let target_ref = self.get_ref(&pending.target_ref)?;
        if source_ref.change_id != pending.right_change {
            return Err(Error::StaleBranch(pending.source_ref));
        }
        if target_ref.change_id != pending.left_change {
            return Err(Error::StaleBranch(pending.target_ref));
        }

        let conflict_paths = conflict_paths_from_details(&summary.details)?;
        let base_ref = self.ref_from_change(&pending.base_change)?;
        let base_files = self.load_root_files(&base_ref.root_id)?;
        let source_files = self.load_root_files(&source_ref.root_id)?;
        let target_files = self.load_root_files(&target_ref.root_id)?;
        let manual_files = match &resolution {
            ConflictResolution::Take(_) => None,
            ConflictResolution::Manual(manual) => Some(normalize_manual_conflict_files(
                manual.clone(),
                &conflict_paths,
            )?),
        };

        let actor = Actor::system();
        let change_id = self.allocate_change_id(&actor.id, "conflict_resolve")?;
        let (merged_files, resolution_label) = match resolution {
            ConflictResolution::Take(take) => {
                let merged_files = merge_files_with_resolution(
                    &base_files,
                    &target_files,
                    &source_files,
                    &conflict_paths,
                    take,
                )?;
                let resolution = match take {
                    ConflictTake::Source => "source",
                    ConflictTake::Target => "target",
                };
                (merged_files, resolution.to_string())
            }
            ConflictResolution::Manual(_) => {
                let mut merged_files = merge_files_with_resolution(
                    &base_files,
                    &target_files,
                    &source_files,
                    &conflict_paths,
                    ConflictTake::Target,
                )?;
                self.apply_manual_conflict_files(
                    &mut merged_files,
                    &base_files,
                    &target_files,
                    &source_files,
                    manual_files.unwrap_or_default(),
                    &change_id,
                )?;
                (merged_files, "manual".to_string())
            }
        };
        let built = self.build_root_from_file_entries(merged_files, &change_id)?;
        let diff = self.diff_file_maps(&target_files, &built.files)?;
        let (kind, session_id) =
            if let Some(agent) = pending.source_ref.strip_prefix(AGENT_REF_PREFIX) {
                let branch = self.agent_branch(agent)?;
                (OperationKind::AgentMerge, branch.session_id)
            } else {
                (OperationKind::Merge, None)
            };
        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind,
            parents: vec![target_ref.change_id.clone(), source_ref.change_id.clone()],
            before_root: Some(target_ref.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: pending.target_ref.clone(),
            actor,
            session_id,
            message: Some(format!(
                "Resolve conflict `{conflict_set_id}` with {resolution_label}"
            )),
            changes: diff.changes,
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&target_ref, &change_id, &built.root_id, &operation_id)?;
        self.conn.execute(
            "UPDATE merge_results SET status = 'resolved', result_change = ?1 WHERE merge_id = ?2",
            params![change_id.0, pending.merge_id],
        )?;
        self.conn.execute(
            "UPDATE conflict_sets SET status = 'resolved' WHERE conflict_set_id = ?1",
            params![conflict_set_id],
        )?;
        if let Some(queue_id) = pending.queue_id {
            self.conn.execute(
                "UPDATE merge_queue SET status = 'merged', updated_at = ?1 WHERE queue_id = ?2",
                params![now_ts(), queue_id],
            )?;
        }
        if let Some(agent) = pending.source_ref.strip_prefix(AGENT_REF_PREFIX) {
            self.conn.execute(
                "UPDATE agent_branches SET status = 'merged', updated_at = ?1 WHERE agent_id = ?2",
                params![now_ts(), self.agent_branch(agent)?.agent_id],
            )?;
        }
        Ok(ConflictResolveReport {
            conflict_set_id: conflict_set_id.to_string(),
            resolution: resolution_label,
            operation: change_id,
            target_ref: pending.target_ref,
            root_id: built.root_id,
            changed_paths: diff.summaries,
        })
    }

    fn apply_manual_conflict_files(
        &self,
        merged_files: &mut BTreeMap<String, FileEntry>,
        base_files: &BTreeMap<String, FileEntry>,
        target_files: &BTreeMap<String, FileEntry>,
        source_files: &BTreeMap<String, FileEntry>,
        manual_files: BTreeMap<String, ConflictManualFile>,
        change_id: &ChangeId,
    ) -> Result<()> {
        let mut file_seq = 1;
        let mut line_seq = 1;
        for (path, file) in manual_files {
            let previous = target_files
                .get(&path)
                .or_else(|| source_files.get(&path))
                .or_else(|| base_files.get(&path));
            let default_executable = previous.is_some_and(|entry| entry.executable);
            match manual_conflict_file_payload(file, default_executable)? {
                ManualConflictPayload::Delete => {
                    merged_files.remove(&path);
                }
                ManualConflictPayload::Text {
                    content,
                    executable,
                } => {
                    let built = self.build_file_entry(
                        &path,
                        content.into_bytes(),
                        executable,
                        change_id,
                        previous,
                        &mut file_seq,
                        &mut line_seq,
                    )?;
                    merged_files.insert(path, built.entry);
                }
            }
        }
        Ok(())
    }

    fn merge_agent_unlocked(
        &mut self,
        agent: &str,
        into: &str,
        dry_run: bool,
        persist_conflict: bool,
    ) -> Result<MergeReport> {
        validate_ref_segment(agent)?;
        let agent_branch = self.agent_branch(agent)?;
        let source_ref = self.get_ref(&agent_branch.ref_name)?;
        self.ensure_agent_workdir_clean(&agent_branch, &source_ref)?;
        if !dry_run {
            self.ensure_agent_merge_readiness(agent)?;
        }
        let target_ref_name = branch_ref(into);
        let target_ref = self.get_ref(&target_ref_name)?;
        let base_ref = self.ref_from_change(&agent_branch.base_change)?;

        let base_files = self.load_root_files(&base_ref.root_id)?;
        let source_files = self.load_root_files(&source_ref.root_id)?;
        let target_files = self.load_root_files(&target_ref.root_id)?;
        let actor = Actor::system();
        let change_id = self.allocate_change_id(&actor.id, "agent_merge")?;
        let (merged_files, conflicts) =
            self.merge_file_maps(&base_files, &target_files, &source_files, &change_id)?;
        if !conflicts.is_empty() {
            if dry_run {
                return Ok(MergeReport {
                    operation: change_id,
                    source_ref: agent_branch.ref_name,
                    target_ref: target_ref_name,
                    root_id: target_ref.root_id,
                    dry_run,
                    changed_paths: Vec::new(),
                    conflicts,
                });
            }
            let detail = conflicts.join("; ");
            let conflict_message = if persist_conflict {
                let context = MergeContext {
                    base_change: agent_branch.base_change.clone(),
                    left_change: target_ref.change_id.clone(),
                    right_change: source_ref.change_id.clone(),
                };
                let conflict_set_id = match self.existing_open_conflict_set(
                    &agent_branch.ref_name,
                    &target_ref_name,
                    &context,
                )? {
                    Some(conflict_set_id) => conflict_set_id,
                    None => self
                        .insert_merge_result_for_refs(
                            None,
                            &agent_branch.ref_name,
                            &target_ref_name,
                            &context,
                            None,
                            "conflicted",
                            Some(&detail),
                        )?
                        .ok_or_else(|| {
                            Error::Corrupt(
                                "conflicted merge result did not create a conflict set".to_string(),
                            )
                        })?,
                };
                format!("recorded {conflict_set_id}: {detail}")
            } else {
                detail
            };
            self.conn.execute(
                "UPDATE agent_branches SET status = 'conflicted', updated_at = ?1 WHERE agent_id = ?2",
                params![now_ts(), agent_branch.agent_id],
            )?;
            return Err(Error::Conflict(conflict_message));
        }

        let built = self.build_root_from_file_entries(merged_files, &change_id)?;
        let diff = self.diff_file_maps(&target_files, &built.files)?;
        if diff.changes.is_empty() {
            if !dry_run {
                self.conn.execute(
                    "UPDATE agent_branches SET status = 'merged', updated_at = ?1 WHERE agent_id = ?2",
                    params![now_ts(), agent_branch.agent_id],
                )?;
            }
            return Ok(MergeReport {
                operation: target_ref.change_id,
                source_ref: agent_branch.ref_name,
                target_ref: target_ref_name,
                root_id: target_ref.root_id,
                dry_run,
                changed_paths: Vec::new(),
                conflicts: Vec::new(),
            });
        }
        if dry_run {
            return Ok(MergeReport {
                operation: change_id,
                source_ref: agent_branch.ref_name,
                target_ref: target_ref_name,
                root_id: built.root_id,
                dry_run,
                changed_paths: diff.summaries,
                conflicts: Vec::new(),
            });
        }

        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind: OperationKind::AgentMerge,
            parents: vec![target_ref.change_id.clone(), source_ref.change_id.clone()],
            before_root: Some(target_ref.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: target_ref_name.clone(),
            actor,
            session_id: agent_branch.session_id,
            message: Some(format!("Merge agent `{agent}` into `{into}`")),
            changes: diff.changes,
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&target_ref, &change_id, &built.root_id, &operation_id)?;
        self.conn.execute(
            "UPDATE agent_branches SET status = 'merged', updated_at = ?1 WHERE agent_id = ?2",
            params![now_ts(), agent_branch.agent_id],
        )?;
        Ok(MergeReport {
            operation: change_id,
            source_ref: agent_branch.ref_name,
            target_ref: target_ref_name,
            root_id: built.root_id,
            dry_run,
            changed_paths: diff.summaries,
            conflicts: Vec::new(),
        })
    }

    fn ensure_agent_workdir_clean(&self, branch: &AgentBranch, head: &RefRecord) -> Result<()> {
        let Some(changed_paths) = self.agent_workdir_changed_paths(branch, head)? else {
            return Ok(());
        };
        if changed_paths.is_empty() {
            return Ok(());
        }
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
        let agent_label = branch
            .ref_name
            .strip_prefix(AGENT_REF_PREFIX)
            .unwrap_or(&branch.agent_id);
        Err(Error::DirtyWorktreeWithMessage(format!(
            "agent `{}` workdir has unrecorded changes; run `crabdb agent record {}` or discard them before merging: {}{}",
            agent_label, agent_label, preview, suffix
        )))
    }

    fn ensure_agent_merge_readiness(&self, agent: &str) -> Result<()> {
        let readiness = self.agent_readiness(agent)?;
        if readiness.ready {
            return Ok(());
        }
        let blockers = readiness
            .blockers
            .iter()
            .filter(|issue| issue.code != "open_conflicts" && issue.code != "dirty_workdir")
            .map(|issue| format!("{}: {}", issue.code, issue.message))
            .collect::<Vec<_>>();
        if blockers.is_empty() {
            return Ok(());
        }
        let blockers = blockers.join("; ");
        Err(Error::InvalidInput(format!(
            "agent `{}` is not merge-ready: {blockers}",
            readiness.agent.record.name
        )))
    }

    fn agent_workdir_changed_paths(
        &self,
        branch: &AgentBranch,
        head: &RefRecord,
    ) -> Result<Option<Vec<FileDiffSummary>>> {
        let Some(workdir) = &branch.workdir else {
            return Ok(None);
        };
        let workdir_path = PathBuf::from(workdir);
        if !workdir_path.is_dir() {
            return Err(Error::WorkspaceNotFound(workdir_path));
        }
        let head_files = self.load_root_files(&head.root_id)?;
        let disk_files = self.scan_files_under(&workdir_path)?;
        let disk_manifest = self.disk_manifest(&disk_files);
        Ok(Some(
            self.diff_file_maps_to_manifest(&head_files, &disk_manifest),
        ))
    }

    fn merge_file_maps(
        &self,
        base: &BTreeMap<String, FileEntry>,
        target: &BTreeMap<String, FileEntry>,
        source: &BTreeMap<String, FileEntry>,
        change_id: &ChangeId,
    ) -> Result<(BTreeMap<String, FileEntry>, Vec<String>)> {
        let mut merged = target.clone();
        let mut conflicts = Vec::new();
        let mut pending_text_merges = Vec::new();
        let mut paths = BTreeSet::new();
        paths.extend(base.keys().cloned());
        paths.extend(target.keys().cloned());
        paths.extend(source.keys().cloned());
        for path in paths {
            let base_entry = base.get(&path);
            let target_entry = target.get(&path);
            let source_entry = source.get(&path);
            let target_changed = entry_hash(base_entry) != entry_hash(target_entry);
            let source_changed = entry_hash(base_entry) != entry_hash(source_entry);
            match (target_changed, source_changed) {
                (false, true) => match source_entry {
                    Some(entry) => {
                        merged.insert(path.clone(), entry.clone());
                    }
                    None => {
                        merged.remove(&path);
                    }
                },
                (true, true) => {
                    if entry_hash(target_entry) == entry_hash(source_entry) {
                        continue;
                    }
                    match self.plan_line_merge(&path, base_entry, target_entry, source_entry)? {
                        Some(plan) => pending_text_merges.push(plan),
                        None => conflicts.push(format!("both changed `{path}` differently")),
                    }
                }
                _ => {}
            }
        }

        if conflicts.is_empty() {
            for plan in pending_text_merges {
                let entry =
                    self.file_entry_from_merged_lines(&plan.target_entry, &plan.lines, change_id)?;
                merged.insert(plan.path, entry);
            }
        }
        Ok((merged, conflicts))
    }

    fn plan_line_merge(
        &self,
        path: &str,
        base_entry: Option<&FileEntry>,
        target_entry: Option<&FileEntry>,
        source_entry: Option<&FileEntry>,
    ) -> Result<Option<PendingLineMerge>> {
        let (Some(base_entry), Some(target_entry), Some(source_entry)) =
            (base_entry, target_entry, source_entry)
        else {
            return Ok(None);
        };
        if base_entry.kind != FileKind::Text
            || target_entry.kind != FileKind::Text
            || source_entry.kind != FileKind::Text
            || base_entry.file_id != target_entry.file_id
            || base_entry.file_id != source_entry.file_id
            || target_entry.executable != source_entry.executable
            || target_entry.mode != source_entry.mode
        {
            return Ok(None);
        }
        let (
            FileContentRef::Text(base_text),
            FileContentRef::Text(target_text),
            FileContentRef::Text(source_text),
        ) = (
            &base_entry.content,
            &target_entry.content,
            &source_entry.content,
        )
        else {
            return Ok(None);
        };
        let base_lines = self.load_text_lines(base_text)?;
        let target_lines = self.load_text_lines(target_text)?;
        let source_lines = self.load_text_lines(source_text)?;
        let base_order = base_lines
            .iter()
            .map(LineEntryExt::line_id_key)
            .collect::<Vec<_>>();
        if !preserves_base_line_order(&base_order, &target_lines)
            || !preserves_base_line_order(&base_order, &source_lines)
        {
            return Ok(None);
        }

        let base_keys = base_order.iter().cloned().collect::<HashSet<_>>();
        let target_inserted_gaps = inserted_line_gaps(&target_lines, &base_keys);
        let source_inserted_groups = inserted_line_groups(&source_lines, &base_keys);
        if source_inserted_groups
            .iter()
            .any(|(gap, _)| target_inserted_gaps.contains(gap))
        {
            return Ok(None);
        }

        let base_by_id = line_map_by_id(&base_lines);
        let target_by_id = line_map_by_id(&target_lines);
        let source_by_id = line_map_by_id(&source_lines);
        let mut merged_lines = target_lines.clone();
        for line_id in &base_order {
            let base_line = base_by_id.get(line_id).copied();
            let target_line = target_by_id.get(line_id).copied();
            let source_line = source_by_id.get(line_id).copied();
            let target_changed = !line_content_equal(base_line, target_line);
            let source_changed = !line_content_equal(base_line, source_line);
            match (target_changed, source_changed) {
                (true, true) if !line_content_equal(target_line, source_line) => return Ok(None),
                (false, true) => match source_line {
                    Some(line) => replace_or_insert_line(&mut merged_lines, line_id, line.clone()),
                    None => remove_line(&mut merged_lines, line_id),
                },
                _ => {}
            }
        }
        for (gap, lines) in source_inserted_groups {
            insert_lines_at_gap(&mut merged_lines, &gap, lines);
        }
        Ok(Some(PendingLineMerge {
            path: path.to_string(),
            target_entry: target_entry.clone(),
            lines: merged_lines,
        }))
    }

    fn file_entry_from_merged_lines(
        &self,
        target_entry: &FileEntry,
        lines: &[LineEntry],
        change_id: &ChangeId,
    ) -> Result<FileEntry> {
        let bytes = materialize_lines(lines);
        let text_id = self.put_text_content_from_lines(lines)?;
        let mut entry = target_entry.clone();
        entry.content = FileContentRef::Text(text_id);
        entry.size_bytes = bytes.len() as u64;
        entry.content_hash = sha256_hex(&bytes);
        entry.last_content_change = change_id.clone();
        Ok(entry)
    }

    pub fn merge_branches(&mut self, source: &str, target: &str) -> Result<MergeReport> {
        self.merge_branches_with_options(source, target, false)
    }

    pub fn merge_branches_with_options(
        &mut self,
        source: &str,
        target: &str,
        dry_run: bool,
    ) -> Result<MergeReport> {
        let _lock = self.acquire_write_lock()?;
        self.merge_branches_unlocked(source, target, dry_run)
    }

    fn merge_branches_unlocked(
        &mut self,
        source: &str,
        target: &str,
        dry_run: bool,
    ) -> Result<MergeReport> {
        let source_ref_name = branch_ref(source);
        let target_ref_name = branch_ref(target);
        let source_ref = self.get_ref(&source_ref_name)?;
        let target_ref = self.get_ref(&target_ref_name)?;
        let base_change = self.common_parent_hint(&source_ref.change_id, &target_ref.change_id)?;
        let base_ref = self.ref_from_change(&base_change)?;
        let base_files = self.load_root_files(&base_ref.root_id)?;
        let source_files = self.load_root_files(&source_ref.root_id)?;
        let target_files = self.load_root_files(&target_ref.root_id)?;
        let actor = Actor::human();
        let change_id = self.allocate_change_id(&actor.id, "merge")?;
        let (merged_files, conflicts) =
            self.merge_file_maps(&base_files, &target_files, &source_files, &change_id)?;
        if !conflicts.is_empty() {
            if dry_run {
                return Ok(MergeReport {
                    operation: change_id,
                    source_ref: source_ref_name,
                    target_ref: target_ref_name,
                    root_id: target_ref.root_id,
                    dry_run,
                    changed_paths: Vec::new(),
                    conflicts,
                });
            }
            return Err(Error::Conflict(conflicts.join("; ")));
        }
        let built = self.build_root_from_file_entries(merged_files, &change_id)?;
        let diff = self.diff_file_maps(&target_files, &built.files)?;
        if dry_run {
            return Ok(MergeReport {
                operation: change_id,
                source_ref: source_ref_name,
                target_ref: target_ref_name,
                root_id: built.root_id,
                dry_run,
                changed_paths: diff.summaries,
                conflicts: Vec::new(),
            });
        }
        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind: OperationKind::Merge,
            parents: vec![target_ref.change_id.clone(), source_ref.change_id.clone()],
            before_root: Some(target_ref.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: target_ref_name.clone(),
            actor,
            session_id: None,
            message: Some(format!("Merge `{source}` into `{target}`")),
            changes: diff.changes,
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&target_ref, &change_id, &built.root_id, &operation_id)?;
        Ok(MergeReport {
            operation: change_id,
            source_ref: source_ref_name,
            target_ref: target_ref_name,
            root_id: built.root_id,
            dry_run,
            changed_paths: diff.summaries,
            conflicts: Vec::new(),
        })
    }

    fn normalize_merge_queue_source_ref(&self, source: &str) -> Result<String> {
        if source.starts_with("refs/") {
            self.get_ref(source)?;
            return Ok(source.to_string());
        }
        let agent_ref_name = agent_ref(source);
        if self.try_get_ref(&agent_ref_name)?.is_some() {
            return Ok(agent_ref_name);
        }
        let branch_ref_name = branch_ref(source);
        self.get_ref(&branch_ref_name)?;
        Ok(branch_ref_name)
    }

    fn normalize_merge_queue_target_ref(&self, target: &str) -> Result<String> {
        let target_ref_name = branch_ref(target);
        if !target_ref_name.starts_with(MAIN_REF_PREFIX) {
            return Err(Error::InvalidInput(
                "merge queue target must be a branch ref".to_string(),
            ));
        }
        self.get_ref(&target_ref_name)?;
        Ok(target_ref_name)
    }

    fn queued_merge_entries(&self, limit: Option<usize>) -> Result<Vec<MergeQueueEntry>> {
        let sql =
            "SELECT queue_id, source_ref, target_ref, status, priority, created_at, updated_at \
                   FROM merge_queue WHERE status = 'queued' \
                   ORDER BY priority DESC, created_at ASC";
        match limit {
            Some(limit) => {
                let mut stmt = self.conn.prepare(&format!("{sql} LIMIT ?1"))?;
                let rows = stmt.query_map(params![limit as i64], merge_queue_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
            None => {
                let mut stmt = self.conn.prepare(sql)?;
                let rows = stmt.query_map([], merge_queue_row)?;
                rows.collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(Error::from)
            }
        }
    }

    fn set_merge_queue_status(&self, queue_id: &str, status: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE merge_queue SET status = ?1, updated_at = ?2 WHERE queue_id = ?3",
            params![status, now_ts(), queue_id],
        )?;
        Ok(())
    }

    fn merge_queue_entry(&mut self, entry: &MergeQueueEntry) -> Result<MergeReport> {
        let target = entry
            .target_ref
            .strip_prefix(MAIN_REF_PREFIX)
            .unwrap_or(&entry.target_ref);
        if let Some(agent) = entry.source_ref.strip_prefix(AGENT_REF_PREFIX) {
            return self.merge_agent_unlocked(agent, target, false, false);
        }
        if let Some(source) = entry.source_ref.strip_prefix(MAIN_REF_PREFIX) {
            return self.merge_branches_unlocked(source, target, false);
        }
        Err(Error::InvalidInput(format!(
            "merge queue source `{}` must be an agent or branch ref",
            entry.source_ref
        )))
    }

    fn merge_queue_context(
        &self,
        source_ref_name: &str,
        target_ref_name: &str,
    ) -> Result<MergeContext> {
        let source_ref = self.get_ref(source_ref_name)?;
        let target_ref = self.get_ref(target_ref_name)?;
        let base_change = if let Some(agent) = source_ref_name.strip_prefix(AGENT_REF_PREFIX) {
            self.agent_branch(agent)?.base_change
        } else {
            self.common_parent_hint(&source_ref.change_id, &target_ref.change_id)?
        };
        Ok(MergeContext {
            base_change,
            left_change: target_ref.change_id,
            right_change: source_ref.change_id,
        })
    }

    fn pending_conflict_merge(&self, conflict_set_id: &str) -> Result<PendingConflictMerge> {
        self.conn
            .query_row(
                "SELECT merge_id, queue_id, source_ref, target_ref, base_change, left_change, right_change \
                 FROM merge_results WHERE conflict_set = ?1 ORDER BY created_at DESC LIMIT 1",
                params![conflict_set_id],
                |row| {
                    Ok(PendingConflictMerge {
                        merge_id: row.get(0)?,
                        queue_id: row.get(1)?,
                        source_ref: row.get(2)?,
                        target_ref: row.get(3)?,
                        base_change: ChangeId(row.get(4)?),
                        left_change: ChangeId(row.get(5)?),
                        right_change: ChangeId(row.get(6)?),
                    })
                },
            )
            .optional()?
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "conflict set `{conflict_set_id}` is not linked to a merge result"
                ))
            })
    }

    fn insert_merge_result(
        &self,
        entry: &MergeQueueEntry,
        context: &MergeContext,
        result_change: Option<&ChangeId>,
        status: &str,
        conflict_detail: Option<&str>,
    ) -> Result<()> {
        self.insert_merge_result_for_refs(
            Some(&entry.queue_id),
            &entry.source_ref,
            &entry.target_ref,
            context,
            result_change,
            status,
            conflict_detail,
        )?;
        Ok(())
    }

    fn insert_merge_result_for_refs(
        &self,
        queue_id: Option<&str>,
        source_ref: &str,
        target_ref: &str,
        context: &MergeContext,
        result_change: Option<&ChangeId>,
        status: &str,
        conflict_detail: Option<&str>,
    ) -> Result<Option<String>> {
        let created_at = now_ts();
        let seed = format!(
            "{}:{}:{}:{}:{}",
            queue_id.unwrap_or("direct"),
            source_ref,
            target_ref,
            status,
            created_at
        );
        let hash = sha256_hex(seed.as_bytes());
        let merge_id = format!("merge_{}", &hash[..16]);
        let conflict_set = conflict_detail.map(|detail| {
            let conflict_hash = sha256_hex(format!("{merge_id}:{detail}").as_bytes());
            format!("conflict_{}", &conflict_hash[..16])
        });
        let conflict_details_json = conflict_detail
            .map(|detail| {
                let details = detail
                    .split("; ")
                    .filter(|item| !item.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                serde_json::to_string(&details)
            })
            .transpose()?;
        let result_change = result_change.map(|change| change.0.clone());
        self.conn.execute(
            "INSERT INTO merge_results \
             (merge_id, queue_id, source_ref, target_ref, base_change, left_change, right_change, result_change, status, conflict_set, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                merge_id,
                queue_id,
                source_ref,
                target_ref,
                context.base_change.0,
                context.left_change.0,
                context.right_change.0,
                result_change,
                status,
                conflict_set,
                created_at
            ],
        )?;
        if let Some(conflict_set_id) = &conflict_set {
            self.conn.execute(
                "INSERT INTO conflict_sets \
                 (conflict_set_id, merge_id, source_ref, target_ref, status, details_json, created_at) \
                 VALUES (?1, ?2, ?3, ?4, 'open', ?5, ?6)",
                params![
                    conflict_set_id,
                    merge_id,
                    source_ref,
                    target_ref,
                    conflict_details_json,
                    created_at
                ],
            )?;
        }
        Ok(conflict_set)
    }

    fn existing_open_conflict_set(
        &self,
        source_ref: &str,
        target_ref: &str,
        context: &MergeContext,
    ) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT mr.conflict_set \
                 FROM merge_results mr \
                 JOIN conflict_sets cs ON cs.conflict_set_id = mr.conflict_set \
                 WHERE mr.source_ref = ?1 \
                   AND mr.target_ref = ?2 \
                   AND mr.base_change = ?3 \
                   AND mr.left_change = ?4 \
                   AND mr.right_change = ?5 \
                   AND mr.status = 'conflicted' \
                   AND cs.status = 'open' \
                 ORDER BY mr.created_at DESC LIMIT 1",
                params![
                    source_ref,
                    target_ref,
                    context.base_change.0,
                    context.left_change.0,
                    context.right_change.0
                ],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::from)
    }

    pub fn export_patch(&self, range: &str) -> Result<String> {
        let summary = self.diff_range(range, true)?;
        let mut out = String::new();
        for file in summary.files {
            if let Some(patch) = file.patch {
                out.push_str(&patch);
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
        }
        Ok(out)
    }

    pub fn git_export_commit(&mut self, range: &str, message: &str) -> Result<GitExportReport> {
        let _lock = self.acquire_write_lock()?;
        let message = message.trim();
        if message.is_empty() {
            return Err(Error::InvalidInput(
                "git export commit message cannot be empty".to_string(),
            ));
        }
        let Some(git_state) = self.current_git_state()? else {
            return Err(Error::Git(format!(
                "git export requires a Git working tree at {}",
                self.workspace_root.display()
            )));
        };
        let (left, right) = parse_range(range)?;
        let left_ref = self.resolve_refish(left)?;
        let right_ref = self.resolve_refish(right)?;
        if !self
            .ancestor_set(&right_ref.change_id)?
            .contains(&left_ref.change_id.0)
        {
            return Err(Error::InvalidInput(format!(
                "range `{range}` is not an ancestor range"
            )));
        }
        let files = self.load_root_files(&right_ref.root_id)?;
        let tree_oid = self.git_write_tree(&files)?;
        let commit = self.git_commit_tree(&tree_oid, git_state.head.as_deref(), message)?;
        let operation = self.operation(&right_ref.change_id)?;
        let branch = operation.branch.clone();
        let mapping = self.insert_git_mapping_for_state(
            "export",
            &branch,
            &right_ref.change_id,
            &right_ref.root_id,
            Some(commit.clone()),
            git_state.dirty,
        )?;
        Ok(GitExportReport {
            range: range.to_string(),
            branch,
            operation: right_ref.change_id,
            root_id: right_ref.root_id,
            commit,
            parent: git_state.head,
            mapping,
        })
    }

    pub fn fsck(&self) -> Result<FsckReport> {
        let mut report = FsckReport {
            checked_refs: 0,
            checked_roots: 0,
            checked_texts: 0,
            errors: Vec::new(),
        };
        let refs = self.all_refs()?;
        for reference in refs {
            report.checked_refs += 1;
            if self.operation(&reference.change_id).is_err() {
                report.errors.push(format!(
                    "ref {} points to missing operation {}",
                    reference.name, reference.change_id.0
                ));
            }
            match self.get_object::<WorktreeRoot>(WORKTREE_ROOT_KIND, &reference.root_id) {
                Ok(root) => {
                    report.checked_roots += 1;
                    if let Err(err) = self.validate_worktree_root(&root) {
                        report
                            .errors
                            .push(format!("root {} invalid: {err}", reference.root_id.0));
                    }
                    if let Ok(files) = self.load_root_files(&reference.root_id) {
                        for entry in files.values() {
                            if let FileContentRef::Text(text_id) = &entry.content {
                                report.checked_texts += 1;
                                if let Err(err) = self.validate_text_content(text_id) {
                                    report
                                        .errors
                                        .push(format!("text {} invalid: {err}", text_id.0));
                                }
                            }
                        }
                    }
                }
                Err(err) => report.errors.push(format!(
                    "ref {} points to missing root {}: {err}",
                    reference.name, reference.root_id.0
                )),
            }
        }
        Ok(report)
    }

    pub fn write_patch_to(&self, range: &str, output: &Path) -> Result<()> {
        let patch = self.export_patch(range)?;
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(output, patch)?;
        Ok(())
    }

    fn insert_git_mapping(
        &self,
        direction: &str,
        branch: &str,
        change_id: &ChangeId,
        root_id: &ObjectId,
    ) -> Result<Option<GitMapping>> {
        let Some(state) = self.current_git_state()? else {
            return Ok(None);
        };
        self.insert_git_mapping_for_state(
            direction,
            branch,
            change_id,
            root_id,
            state.head,
            state.dirty,
        )
    }

    fn insert_git_mapping_for_state(
        &self,
        direction: &str,
        branch: &str,
        change_id: &ChangeId,
        root_id: &ObjectId,
        git_head: Option<String>,
        git_dirty: bool,
    ) -> Result<Option<GitMapping>> {
        let created_at = now_ts();
        let seed = format!(
            "{direction}:{branch}:{:?}:{}:{}:{created_at}",
            git_head, change_id.0, root_id.0
        );
        let hash = sha256_hex(seed.as_bytes());
        let mapping_id = format!("gitmap_{}", &hash[..16]);
        self.conn.execute(
            "INSERT INTO git_mappings \
             (mapping_id, direction, branch, git_head, git_dirty, crab_change, crab_root, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                mapping_id,
                direction,
                branch,
                git_head.as_deref(),
                if git_dirty { 1_i64 } else { 0_i64 },
                change_id.0,
                root_id.0,
                created_at
            ],
        )?;
        Ok(Some(GitMapping {
            mapping_id,
            direction: direction.to_string(),
            branch: branch.to_string(),
            git_head,
            git_dirty,
            crab_change: change_id.clone(),
            crab_root: root_id.clone(),
            created_at,
        }))
    }

    pub fn rebuild_indexes(&mut self) -> Result<IndexRebuildReport> {
        let _lock = self.acquire_write_lock()?;
        self.rebuild_indexes_unlocked()
    }

    pub fn gc(&mut self, dry_run: bool) -> Result<GcReport> {
        let _lock = self.acquire_write_lock()?;
        let reachable = self.reachable_object_ids()?;
        let known_kinds = known_gc_object_kinds();
        let mut stmt = self
            .conn
            .prepare("SELECT object_id, kind FROM objects ORDER BY object_id")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut prunable = Vec::new();
        let mut total_known = 0;
        let mut preserved_unknown = 0;
        for row in rows {
            let (object_id, kind) = row?;
            if known_kinds.contains(kind.as_str()) {
                total_known += 1;
                if !reachable.contains(&object_id) {
                    prunable.push(object_id);
                }
            } else {
                preserved_unknown += 1;
            }
        }
        let mut report = GcReport {
            dry_run,
            total_known_objects: total_known,
            reachable_objects: reachable.len() as u64,
            prunable_objects: prunable.len() as u64,
            pruned_objects: 0,
            preserved_unknown_objects: preserved_unknown,
            errors: Vec::new(),
        };
        if !dry_run {
            for object_id in &prunable {
                self.conn.execute(
                    "DELETE FROM objects WHERE object_id = ?1",
                    params![object_id],
                )?;
                report.pruned_objects += 1;
            }
            let rebuild = self.rebuild_indexes_unlocked()?;
            report.errors.extend(rebuild.errors);
        }
        Ok(report)
    }

    fn rebuild_indexes_unlocked(&self) -> Result<IndexRebuildReport> {
        let (operation_objects, mut errors) = self.operation_objects()?;
        let reachable_changes =
            self.reachable_operation_changes(&operation_objects, &mut errors)?;
        self.conn.execute_batch(
            "\
            DELETE FROM operations;
            DELETE FROM operation_parents;
            DELETE FROM file_history;
            DELETE FROM line_history;
            DELETE FROM messages;
            ",
        )?;

        let mut by_change = operation_objects
            .into_iter()
            .map(|object| (object.operation.change_id.0.clone(), object))
            .collect::<HashMap<_, _>>();
        let mut changes = reachable_changes.into_iter().collect::<Vec<_>>();
        changes.sort();

        let mut report = IndexRebuildReport {
            errors,
            ..IndexRebuildReport::default()
        };
        for change_id in changes {
            let Some(object) = by_change.remove(&change_id) else {
                report.errors.push(format!(
                    "reachable operation missing from object map: {change_id}"
                ));
                continue;
            };
            report.operations += 1;
            report.operation_parents += object.operation.parents.len() as u64;
            for change in &object.operation.changes {
                if change.file_id.is_some() {
                    report.file_history_rows += 1;
                    report.line_history_rows += change.line_changes.len() as u64;
                }
            }
            self.index_operation(&object.operation, &object.object_id)?;
        }

        for (object_id, message) in self.message_objects(&mut report.errors)? {
            self.index_message(&message, &object_id)?;
            report.messages += 1;
        }

        Ok(report)
    }

    fn operation_objects(&self) -> Result<(Vec<OperationObject>, Vec<String>)> {
        let mut stmt = self
            .conn
            .prepare("SELECT object_id, bytes FROM objects WHERE kind = ?1 ORDER BY object_id")?;
        let rows = stmt.query_map(params![OPERATION_KIND], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        let mut objects = Vec::new();
        let mut errors = Vec::new();
        for row in rows {
            let (object_id, bytes) = row?;
            match from_cbor::<Operation>(&bytes) {
                Ok(operation) => objects.push(OperationObject {
                    object_id: ObjectId(object_id),
                    operation,
                }),
                Err(err) => errors.push(format!(
                    "failed to decode operation object {object_id}: {err}"
                )),
            }
        }
        Ok((objects, errors))
    }

    fn message_objects(&self, errors: &mut Vec<String>) -> Result<Vec<(ObjectId, Message)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT object_id, bytes FROM objects WHERE kind = ?1 ORDER BY object_id")?;
        let rows = stmt.query_map(params![MESSAGE_KIND], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        let mut messages = Vec::new();
        for row in rows {
            let (object_id, bytes) = row?;
            match from_cbor::<Message>(&bytes) {
                Ok(message) => messages.push((ObjectId(object_id), message)),
                Err(err) => errors.push(format!(
                    "failed to decode message object {object_id}: {err}"
                )),
            }
        }
        Ok(messages)
    }

    fn reachable_operation_changes(
        &self,
        operation_objects: &[OperationObject],
        errors: &mut Vec<String>,
    ) -> Result<HashSet<String>> {
        let by_change = operation_objects
            .iter()
            .map(|object| (object.operation.change_id.0.clone(), object))
            .collect::<HashMap<_, _>>();
        let by_object = operation_objects
            .iter()
            .map(|object| {
                (
                    object.object_id.0.clone(),
                    object.operation.change_id.0.clone(),
                )
            })
            .collect::<HashMap<_, _>>();

        let mut stack = Vec::new();
        for reference in self.all_refs()? {
            match by_object.get(&reference.operation_id.0) {
                Some(change_id) => stack.push(change_id.clone()),
                None => errors.push(format!(
                    "ref {} points to missing operation object {}",
                    reference.name, reference.operation_id.0
                )),
            }
        }

        let mut reachable = HashSet::new();
        while let Some(change_id) = stack.pop() {
            if !reachable.insert(change_id.clone()) {
                continue;
            }
            let Some(object) = by_change.get(&change_id) else {
                errors.push(format!(
                    "operation {change_id} is reachable but missing from object table"
                ));
                continue;
            };
            for parent in &object.operation.parents {
                stack.push(parent.0.clone());
            }
        }
        Ok(reachable)
    }

    fn reachable_object_ids(&self) -> Result<HashSet<String>> {
        let (operation_objects, mut errors) = self.operation_objects()?;
        let reachable_changes =
            self.reachable_operation_changes(&operation_objects, &mut errors)?;
        let by_change = operation_objects
            .iter()
            .map(|object| (object.operation.change_id.0.clone(), object))
            .collect::<HashMap<_, _>>();
        let mut reachable = HashSet::new();

        for reference in self.all_refs()? {
            reachable.insert(reference.root_id.0.clone());
            reachable.insert(reference.operation_id.0.clone());
            self.collect_root_reachable(&reference.root_id, &mut reachable, &mut errors);
        }

        for change_id in &reachable_changes {
            let Some(object) = by_change.get(change_id) else {
                continue;
            };
            reachable.insert(object.object_id.0.clone());
            if let Some(root_id) = &object.operation.before_root {
                self.collect_root_reachable(root_id, &mut reachable, &mut errors);
            }
            self.collect_root_reachable(&object.operation.after_root, &mut reachable, &mut errors);
        }

        for (object_id, _message) in self.message_objects(&mut errors)? {
            reachable.insert(object_id.0);
        }

        self.collect_agent_event_object_refs(&mut reachable, &mut errors)?;

        let mut stmt = self.conn.prepare("SELECT object_id FROM anchors")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            reachable.insert(row?);
        }

        if !errors.is_empty() {
            // GC should be conservative. Surface corruption to the caller rather
            // than deleting objects when reachability is uncertain.
            return Err(Error::Corrupt(errors.join("; ")));
        }
        Ok(reachable)
    }

    fn collect_agent_event_object_refs(
        &self,
        reachable: &mut HashSet<String>,
        errors: &mut Vec<String>,
    ) -> Result<()> {
        let mut stmt = self.conn.prepare(
            "SELECT event_id, payload_json FROM agent_events ORDER BY created_at, event_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (event_id, payload_json) = row?;
            let payload = match serde_json::from_str::<serde_json::Value>(&payload_json) {
                Ok(payload) => payload,
                Err(err) => {
                    errors.push(format!("failed to decode agent event {event_id}: {err}"));
                    continue;
                }
            };
            for key in ["stdout_object", "stderr_object"] {
                if let Some(object_id) = payload.get(key).and_then(|value| value.as_str()) {
                    reachable.insert(object_id.to_string());
                }
            }
        }
        Ok(())
    }

    fn collect_root_reachable(
        &self,
        root_id: &ObjectId,
        reachable: &mut HashSet<String>,
        errors: &mut Vec<String>,
    ) {
        reachable.insert(root_id.0.clone());
        match self.load_root_files(root_id) {
            Ok(files) => {
                for entry in files.values() {
                    match &entry.content {
                        FileContentRef::Text(text_id) => {
                            reachable.insert(text_id.0.clone());
                        }
                        FileContentRef::Opaque(blob_id) | FileContentRef::Binary(blob_id) => {
                            reachable.insert(blob_id.0.clone());
                        }
                    }
                }
            }
            Err(err) => errors.push(format!("failed to walk root {}: {err}", root_id.0)),
        }
    }

    fn init_schema(&self) -> Result<()> {
        let user_version = self.schema_user_version()?;
        if user_version > CRABDB_SCHEMA_VERSION {
            return Err(Error::InvalidInput(format!(
                "CrabDB schema version {user_version} is newer than supported version {CRABDB_SCHEMA_VERSION}; upgrade this binary before opening the workspace"
            )));
        }
        self.conn.execute_batch(
            "\
            CREATE TABLE IF NOT EXISTS schema_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS objects (
                object_id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                version INTEGER NOT NULL,
                codec TEXT NOT NULL,
                hash_alg TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                bytes BLOB NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS refs (
                name TEXT PRIMARY KEY,
                change_id TEXT NOT NULL,
                root_id TEXT NOT NULL,
                operation_id TEXT NOT NULL,
                generation INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS operations (
                change_id TEXT PRIMARY KEY,
                operation_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                branch TEXT NOT NULL,
                before_root TEXT,
                after_root TEXT NOT NULL,
                actor_kind TEXT NOT NULL,
                actor_id TEXT NOT NULL,
                session_id TEXT,
                message TEXT,
                path_count INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS operations_branch_created_idx ON operations(branch, created_at);
            CREATE INDEX IF NOT EXISTS operations_session_created_idx ON operations(session_id, created_at);
            CREATE TABLE IF NOT EXISTS operation_parents (
                change_id TEXT NOT NULL,
                parent_change_id TEXT NOT NULL,
                position INTEGER NOT NULL,
                PRIMARY KEY (change_id, position)
            );
            CREATE TABLE IF NOT EXISTS file_history (
                file_id TEXT NOT NULL,
                change_id TEXT NOT NULL,
                path TEXT NOT NULL,
                old_path TEXT,
                kind TEXT NOT NULL,
                before_hash TEXT,
                after_hash TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS file_history_file_idx ON file_history(file_id, created_at);
            CREATE INDEX IF NOT EXISTS file_history_path_idx ON file_history(path, created_at);
            CREATE TABLE IF NOT EXISTS line_history (
                line_id TEXT NOT NULL,
                file_id TEXT NOT NULL,
                change_id TEXT NOT NULL,
                path TEXT NOT NULL,
                line_number INTEGER,
                kind TEXT NOT NULL,
                text_hash TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS line_history_line_idx ON line_history(line_id, created_at);
            CREATE TABLE IF NOT EXISTS messages (
                message_id TEXT PRIMARY KEY,
                role TEXT NOT NULL,
                body TEXT NOT NULL,
                agent_id TEXT,
                session_id TEXT,
                change_id TEXT,
                object_id TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS anchors (
                anchor_id TEXT PRIMARY KEY,
                label TEXT NOT NULL,
                file_id TEXT NOT NULL,
                line_id TEXT NOT NULL,
                object_id TEXT NOT NULL,
                created_path TEXT NOT NULL,
                created_line INTEGER NOT NULL,
                created_change TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS anchors_file_idx ON anchors(file_id, created_at);
            CREATE INDEX IF NOT EXISTS anchors_line_idx ON anchors(line_id, created_at);
            CREATE TABLE IF NOT EXISTS agents (
                agent_id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                kind TEXT,
                provider TEXT,
                model TEXT,
                created_at INTEGER NOT NULL,
                metadata_json TEXT
            );
            CREATE TABLE IF NOT EXISTS agent_branches (
                agent_id TEXT PRIMARY KEY,
                ref_name TEXT NOT NULL UNIQUE,
                base_change TEXT NOT NULL,
                head_change TEXT NOT NULL,
                base_root TEXT NOT NULL,
                head_root TEXT NOT NULL,
                session_id TEXT,
                workdir TEXT,
                status TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS agent_sessions (
                session_id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                title TEXT,
                status TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                ended_at INTEGER,
                metadata_json TEXT
            );
            CREATE TABLE IF NOT EXISTS agent_turns (
                turn_id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                session_id TEXT,
                base_change TEXT NOT NULL,
                before_change TEXT NOT NULL,
                after_change TEXT,
                status TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                ended_at INTEGER,
                metadata_json TEXT
            );
            CREATE TABLE IF NOT EXISTS agent_events (
                event_id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                turn_id TEXT,
                session_id TEXT,
                event_type TEXT NOT NULL,
                change_id TEXT,
                message_id TEXT,
                payload_json TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS agent_approvals (
                approval_id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                session_id TEXT,
                turn_id TEXT,
                action TEXT NOT NULL,
                summary TEXT NOT NULL,
                payload_json TEXT,
                status TEXT NOT NULL,
                requested_at INTEGER NOT NULL,
                decided_at INTEGER,
                reviewer TEXT,
                note TEXT
            );
            CREATE INDEX IF NOT EXISTS agent_approvals_status_idx ON agent_approvals(status, requested_at);
            CREATE INDEX IF NOT EXISTS agent_approvals_agent_idx ON agent_approvals(agent_id, requested_at);
            CREATE TABLE IF NOT EXISTS agent_run_states (
                run_id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                session_id TEXT,
                turn_id TEXT,
                approval_id TEXT,
                status TEXT NOT NULL,
                reason TEXT NOT NULL,
                summary TEXT NOT NULL,
                state_json TEXT NOT NULL,
                interruption_json TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                resumed_at INTEGER,
                reviewer TEXT,
                note TEXT
            );
            CREATE INDEX IF NOT EXISTS agent_run_states_agent_idx ON agent_run_states(agent_id, updated_at);
            CREATE INDEX IF NOT EXISTS agent_run_states_status_idx ON agent_run_states(status, updated_at);
            CREATE INDEX IF NOT EXISTS agent_run_states_approval_idx ON agent_run_states(approval_id);
            CREATE TABLE IF NOT EXISTS leases (
                lease_id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                ref_name TEXT NOT NULL,
                path TEXT,
                file_id TEXT,
                mode TEXT NOT NULL,
                expires_at INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS merge_queue (
                queue_id TEXT PRIMARY KEY,
                source_ref TEXT NOT NULL,
                target_ref TEXT NOT NULL,
                status TEXT NOT NULL,
                priority INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS merge_results (
                merge_id TEXT PRIMARY KEY,
                queue_id TEXT,
                source_ref TEXT NOT NULL,
                target_ref TEXT NOT NULL,
                base_change TEXT NOT NULL,
                left_change TEXT NOT NULL,
                right_change TEXT NOT NULL,
                result_change TEXT,
                status TEXT NOT NULL,
                conflict_set TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS conflict_sets (
                conflict_set_id TEXT PRIMARY KEY,
                merge_id TEXT,
                source_ref TEXT,
                target_ref TEXT,
                status TEXT NOT NULL,
                details_json TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS git_mappings (
                mapping_id TEXT PRIMARY KEY,
                direction TEXT NOT NULL,
                branch TEXT NOT NULL,
                git_head TEXT,
                git_dirty INTEGER NOT NULL,
                crab_change TEXT NOT NULL,
                crab_root TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS git_mappings_change_idx ON git_mappings(crab_change);
            CREATE INDEX IF NOT EXISTS git_mappings_head_idx ON git_mappings(git_head);
            ",
        )?;
        ensure_column(&self.conn, "conflict_sets", "details_json", "TEXT")?;
        ensure_column(&self.conn, "agent_events", "session_id", "TEXT")?;
        self.record_schema_version()?;
        Ok(())
    }

    fn schema_user_version(&self) -> Result<i64> {
        self.conn
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .map_err(Error::from)
    }

    fn set_schema_user_version(&self, version: i64) -> Result<()> {
        self.conn
            .execute_batch(&format!("PRAGMA user_version = {version};"))?;
        Ok(())
    }

    fn record_schema_version(&self) -> Result<()> {
        self.set_schema_user_version(CRABDB_SCHEMA_VERSION)?;
        let now = now_ts();
        for (key, value) in [
            (SCHEMA_META_VERSION_KEY, CRABDB_SCHEMA_VERSION.to_string()),
            (
                SCHEMA_META_APP_VERSION_KEY,
                env!("CARGO_PKG_VERSION").to_string(),
            ),
        ] {
            self.conn.execute(
                "INSERT OR REPLACE INTO schema_meta (key, value, updated_at) VALUES (?1, ?2, ?3)",
                params![key, value, now],
            )?;
        }
        Ok(())
    }

    fn schema_meta_value(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM schema_meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::from)
    }

    fn allocate_change_id(&self, actor_id: &str, hint: &str) -> Result<ChangeId> {
        let lamport = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(generation), 0) + 1 FROM refs",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(1);
        Ok(ChangeId::allocate(
            &self.config.workspace.id,
            actor_id,
            lamport,
            hint,
        ))
    }

    fn scan_git_tracked_files(&self) -> Result<Vec<DiskFile>> {
        self.scan_git_tracked_files_impl(false)
    }

    fn current_git_state(&self) -> Result<Option<GitState>> {
        let inside = Command::new("git")
            .arg("-C")
            .arg(&self.workspace_root)
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .map_err(|err| Error::Git(err.to_string()))?;
        if !inside.status.success() {
            return Ok(None);
        }

        let head_output = Command::new("git")
            .arg("-C")
            .arg(&self.workspace_root)
            .args(["rev-parse", "--verify", "HEAD"])
            .output()
            .map_err(|err| Error::Git(err.to_string()))?;
        let head = if head_output.status.success() {
            Some(
                String::from_utf8_lossy(&head_output.stdout)
                    .trim()
                    .to_string(),
            )
            .filter(|head| !head.is_empty())
        } else {
            None
        };

        let status = Command::new("git")
            .arg("-C")
            .arg(&self.workspace_root)
            .args(["status", "--porcelain", "--untracked-files=no"])
            .output()
            .map_err(|err| Error::Git(err.to_string()))?;
        if !status.status.success() {
            let stderr = String::from_utf8_lossy(&status.stderr);
            return Err(Error::Git(format!(
                "git status failed in {}: {}",
                self.workspace_root.display(),
                stderr.trim()
            )));
        }

        Ok(Some(GitState {
            head,
            dirty: !status.stdout.is_empty(),
        }))
    }

    fn git_write_tree(&self, files: &BTreeMap<String, FileEntry>) -> Result<String> {
        let mut root = GitTreeNode::default();
        for (path, entry) in files {
            let bytes = self.materialize_entry_bytes(entry)?;
            let oid = self.git_output_with_input(&["hash-object", "-w", "--stdin"], &bytes)?;
            let blob = GitBlobEntry {
                mode: if entry.executable { "100755" } else { "100644" },
                oid,
            };
            Self::git_insert_tree_path(&mut root, path, blob)?;
        }
        self.git_write_tree_node(&root)
    }

    fn git_insert_tree_path(root: &mut GitTreeNode, path: &str, blob: GitBlobEntry) -> Result<()> {
        let mut parts = path.split('/').collect::<Vec<_>>();
        if parts.is_empty()
            || parts
                .iter()
                .any(|part| part.is_empty() || *part == "." || *part == "..")
        {
            return Err(Error::InvalidPath {
                path: path.to_string(),
                reason: "path cannot be represented in a Git tree".to_string(),
            });
        }
        let name = parts.pop().unwrap();
        let mut node = root;
        for part in parts {
            if node.blobs.contains_key(part) {
                return Err(Error::InvalidPath {
                    path: path.to_string(),
                    reason: "path conflicts with a file in the Git tree".to_string(),
                });
            }
            node = node.dirs.entry(part.to_string()).or_default();
        }
        if node.dirs.contains_key(name) || node.blobs.insert(name.to_string(), blob).is_some() {
            return Err(Error::InvalidPath {
                path: path.to_string(),
                reason: "duplicate path in Git tree export".to_string(),
            });
        }
        Ok(())
    }

    fn git_write_tree_node(&self, node: &GitTreeNode) -> Result<String> {
        let mut entries = Vec::new();
        for (name, blob) in &node.blobs {
            entries.push((
                name.clone(),
                format!("{} blob {}\t{}\n", blob.mode, blob.oid, name),
            ));
        }
        for (name, child) in &node.dirs {
            let oid = self.git_write_tree_node(child)?;
            entries.push((name.clone(), format!("040000 tree {}\t{}\n", oid, name)));
        }
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        let input = entries
            .into_iter()
            .map(|(_, line)| line)
            .collect::<String>();
        self.git_output_with_input(&["mktree"], input.as_bytes())
    }

    fn git_commit_tree(
        &self,
        tree_oid: &str,
        parent: Option<&str>,
        message: &str,
    ) -> Result<String> {
        let mut args = vec!["commit-tree".to_string(), tree_oid.to_string()];
        if let Some(parent) = parent {
            args.push("-p".to_string());
            args.push(parent.to_string());
        }
        args.push("-m".to_string());
        args.push(message.to_string());
        self.git_output(&args)
    }

    fn git_output(&self, args: &[String]) -> Result<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.workspace_root)
            .args(args)
            .output()
            .map_err(|err| Error::Git(err.to_string()))?;
        self.git_checked_output(args, output)
    }

    fn git_output_with_input(&self, args: &[&str], input: &[u8]) -> Result<String> {
        let mut child = Command::new("git")
            .arg("-C")
            .arg(&self.workspace_root)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|err| Error::Git(err.to_string()))?;
        {
            let stdin = child
                .stdin
                .as_mut()
                .ok_or_else(|| Error::Git("failed to open git stdin".to_string()))?;
            stdin.write_all(input)?;
        }
        let output = child
            .wait_with_output()
            .map_err(|err| Error::Git(err.to_string()))?;
        let args = args
            .iter()
            .map(|arg| (*arg).to_string())
            .collect::<Vec<_>>();
        self.git_checked_output(&args, output)
    }

    fn git_checked_output(&self, args: &[String], output: std::process::Output) -> Result<String> {
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Git(format!(
                "git {} failed in {}: {}",
                args.join(" "),
                self.workspace_root.display(),
                stderr.trim()
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn scan_git_tracked_files_required(&self) -> Result<Vec<DiskFile>> {
        self.scan_git_tracked_files_impl(true)
    }

    fn scan_git_tracked_files_impl(&self, required: bool) -> Result<Vec<DiskFile>> {
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

    fn scan_worktree_files(&self) -> Result<Vec<DiskFile>> {
        self.scan_files_under(&self.workspace_root)
    }

    fn scan_files_under(&self, root: &Path) -> Result<Vec<DiskFile>> {
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
            if entry.file_type().is_some_and(|kind| kind.is_dir()) {
                if is_default_ignored(&rel) {
                    continue;
                }
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

    fn disk_manifest(&self, disk_files: &[DiskFile]) -> BTreeMap<String, DiskManifest> {
        disk_files
            .iter()
            .map(|file| {
                (
                    file.path.clone(),
                    DiskManifest {
                        kind: classify_file_kind(&file.bytes, &self.config.text),
                        executable: file.executable,
                        content_hash: sha256_hex(&file.bytes),
                    },
                )
            })
            .collect()
    }

    fn diff_file_maps_to_manifest(
        &self,
        left: &BTreeMap<String, FileEntry>,
        right: &BTreeMap<String, DiskManifest>,
    ) -> Vec<FileDiffSummary> {
        let mut paths = BTreeSet::new();
        paths.extend(left.keys().cloned());
        paths.extend(right.keys().cloned());
        let mut summaries = Vec::new();
        for path in paths {
            match (left.get(&path), right.get(&path)) {
                (None, Some(new_entry)) => summaries.push(FileDiffSummary {
                    path,
                    old_path: None,
                    kind: FileChangeKind::Added,
                    before_hash: None,
                    after_hash: Some(new_entry.content_hash.clone()),
                    additions: 0,
                    deletions: 0,
                    line_changes: Vec::new(),
                    patch: None,
                }),
                (Some(old_entry), None) => summaries.push(FileDiffSummary {
                    path,
                    old_path: None,
                    kind: FileChangeKind::Deleted,
                    before_hash: Some(old_entry.content_hash.clone()),
                    after_hash: None,
                    additions: 0,
                    deletions: 0,
                    line_changes: Vec::new(),
                    patch: None,
                }),
                (Some(old_entry), Some(new_entry)) => {
                    if old_entry.content_hash == new_entry.content_hash
                        && old_entry.executable == new_entry.executable
                        && old_entry.kind == new_entry.kind
                    {
                        continue;
                    }
                    summaries.push(FileDiffSummary {
                        path,
                        old_path: None,
                        kind: if old_entry.kind == new_entry.kind {
                            FileChangeKind::Modified
                        } else {
                            FileChangeKind::TypeChanged
                        },
                        before_hash: Some(old_entry.content_hash.clone()),
                        after_hash: Some(new_entry.content_hash.clone()),
                        additions: 0,
                        deletions: 0,
                        line_changes: Vec::new(),
                        patch: None,
                    });
                }
                (None, None) => {}
            }
        }
        summaries
    }

    fn build_root_from_disk_files(
        &self,
        disk_files: &[DiskFile],
        change_id: &ChangeId,
        previous: Option<&BTreeMap<String, FileEntry>>,
    ) -> Result<RootBuildResult> {
        let mut files = BTreeMap::new();
        let mut file_seq = 1;
        let mut line_seq = 1;
        let new_paths = disk_files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<HashSet<_>>();
        let mut previous_by_hash: HashMap<String, Vec<(String, FileEntry)>> = HashMap::new();
        if let Some(previous) = previous {
            for (path, entry) in previous {
                if new_paths.contains(path.as_str()) {
                    continue;
                }
                previous_by_hash
                    .entry(entry.content_hash.clone())
                    .or_default()
                    .push((path.clone(), entry.clone()));
            }
        }
        for disk_file in disk_files {
            let previous_entry = previous.and_then(|entries| entries.get(&disk_file.path));
            let previous_entry = if previous_entry.is_none() {
                previous_by_hash
                    .get(&sha256_hex(&disk_file.bytes))
                    .and_then(|matches| matches.first().map(|(_, entry)| entry))
            } else {
                previous_entry
            };
            let built = self.build_file_entry(
                &disk_file.path,
                disk_file.bytes.clone(),
                disk_file.executable,
                change_id,
                previous_entry,
                &mut file_seq,
                &mut line_seq,
            )?;
            files.insert(disk_file.path.clone(), built.entry);
        }
        self.build_root_from_file_entries(files, change_id)
    }

    fn build_root_for_selected_record(
        &self,
        previous: &BTreeMap<String, FileEntry>,
        disk_files: &[DiskFile],
        selected_paths: &[String],
        allow_ignored: bool,
        change_id: &ChangeId,
    ) -> Result<RootBuildResult> {
        let selected_disk_files =
            self.selected_record_disk_files(disk_files, selected_paths, allow_ignored)?;
        let mut files = previous.clone();
        let mut removed_entries = Vec::new();
        for selected in selected_paths {
            let removed_paths = files
                .keys()
                .filter(|path| path_matches_selection(path, selected))
                .cloned()
                .collect::<Vec<_>>();
            for path in removed_paths {
                if let Some(entry) = files.remove(&path) {
                    removed_entries.push((path, entry));
                }
            }
        }

        let mut previous_by_hash: HashMap<String, Vec<(String, FileEntry)>> = HashMap::new();
        for (path, entry) in removed_entries {
            previous_by_hash
                .entry(entry.content_hash.clone())
                .or_default()
                .push((path, entry));
        }

        let mut file_seq = 1;
        let mut line_seq = 1;
        for disk_file in selected_disk_files {
            let previous_entry = previous.get(&disk_file.path).or_else(|| {
                previous_by_hash
                    .get(&sha256_hex(&disk_file.bytes))
                    .and_then(|matches| matches.first().map(|(_, entry)| entry))
            });
            let built = self.build_file_entry(
                &disk_file.path,
                disk_file.bytes,
                disk_file.executable,
                change_id,
                previous_entry,
                &mut file_seq,
                &mut line_seq,
            )?;
            files.insert(disk_file.path, built.entry);
        }
        self.build_root_from_file_entries(files, change_id)
    }

    fn selected_record_disk_files(
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

    fn read_record_selection_unfiltered(&self, path: &str) -> Result<Vec<DiskFile>> {
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

    fn read_record_dir_unfiltered(
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

    fn build_root_from_file_entries(
        &self,
        files: BTreeMap<String, FileEntry>,
        change_id: &ChangeId,
    ) -> Result<RootBuildResult> {
        let mut path_builder = BatchBuilder::new(self.store.clone(), prolly_config());
        let mut file_index_builder = BatchBuilder::new(self.store.clone(), prolly_config());
        let mut stats = ImportStats::default();
        let mut total_text_bytes = 0;
        for (path, entry) in &files {
            path_builder.add(path.as_bytes().to_vec(), cbor(entry)?);
            file_index_builder.add(entry.file_id.encode_key(), path.as_bytes().to_vec());
            stats.files += 1;
            match entry.kind {
                FileKind::Text => {
                    stats.text += 1;
                    total_text_bytes += entry.size_bytes;
                }
                FileKind::OpaqueText => stats.opaque += 1,
                FileKind::Binary => stats.binary += 1,
            }
        }
        let path_tree = path_builder.build()?;
        let file_index_tree = file_index_builder.build()?;
        let root = WorktreeRoot {
            version: ROOT_OBJECT_VERSION,
            path_map_root: tree_root_hex(&path_tree),
            file_index_map_root: tree_root_hex(&file_index_tree),
            file_count: files.len() as u64,
            total_text_bytes,
            created_by: change_id.clone(),
        };
        let root_id = self.put_object(WORKTREE_ROOT_KIND, ROOT_OBJECT_VERSION, &root)?;
        Ok(RootBuildResult {
            root_id,
            files,
            stats,
        })
    }

    fn build_file_entry(
        &self,
        path: &str,
        bytes: Vec<u8>,
        executable: bool,
        change_id: &ChangeId,
        previous: Option<&FileEntry>,
        file_seq: &mut u64,
        line_seq: &mut u64,
    ) -> Result<FileBuildResult> {
        let content_hash = sha256_hex(&bytes);
        let file_id = previous
            .map(|entry| entry.file_id.clone())
            .unwrap_or_else(|| {
                let id = FileId::new(change_id.clone(), *file_seq);
                *file_seq += 1;
                id
            });
        let created_by = previous
            .map(|entry| entry.created_by.clone())
            .unwrap_or_else(|| change_id.clone());
        let previous_text = previous.and_then(|entry| match &entry.content {
            FileContentRef::Text(text_id) => self.load_text_lines(text_id).ok(),
            _ => None,
        });
        let (kind, content, line_changes) = if looks_binary(&bytes) {
            let blob_id = self.put_blob(bytes.clone())?;
            (
                FileKind::Binary,
                FileContentRef::Binary(blob_id),
                Vec::new(),
            )
        } else if std::str::from_utf8(&bytes).is_err() {
            let blob_id = self.put_blob(bytes.clone())?;
            (
                FileKind::OpaqueText,
                FileContentRef::Opaque(blob_id),
                Vec::new(),
            )
        } else if bytes.len() as u64 > self.config.text.opaque_text_max_bytes {
            let blob_id = self.put_blob(bytes.clone())?;
            (
                FileKind::OpaqueText,
                FileContentRef::Opaque(blob_id),
                Vec::new(),
            )
        } else if max_line_len(&bytes) as u64 > self.config.text.max_line_bytes {
            let blob_id = self.put_blob(bytes.clone())?;
            (
                FileKind::OpaqueText,
                FileContentRef::Opaque(blob_id),
                Vec::new(),
            )
        } else {
            let built_text = self.build_text_content(
                &bytes,
                change_id,
                previous_text.as_deref(),
                line_seq,
                self.config.text.preserve_similarity,
            )?;
            (
                FileKind::Text,
                FileContentRef::Text(built_text.object_id),
                built_text.line_changes,
            )
        };
        let last_content_change =
            if previous.is_some_and(|entry| entry.content_hash == content_hash) {
                previous
                    .map(|entry| entry.last_content_change.clone())
                    .unwrap_or_else(|| change_id.clone())
            } else {
                change_id.clone()
            };
        let entry = FileEntry {
            file_id,
            kind,
            mode: if executable { 0o755 } else { 0o644 },
            executable,
            content,
            size_bytes: bytes.len() as u64,
            content_hash,
            created_by,
            last_content_change,
            last_path_change: previous.and_then(|entry| entry.last_path_change.clone()),
        };
        let line_changes = line_changes.into_iter().map(|line| line).collect();
        let _ = path;
        Ok(FileBuildResult {
            entry,
            line_changes,
        })
    }

    fn build_text_content(
        &self,
        bytes: &[u8],
        change_id: &ChangeId,
        previous: Option<&[LineEntry]>,
        line_seq: &mut u64,
        similarity_threshold: f32,
    ) -> Result<TextBuildResult> {
        let new_lines = split_lines(bytes);
        let previous = previous.unwrap_or(&[]);
        let new_hashes = new_lines
            .iter()
            .map(|line| sha256_hex(&line.text))
            .collect::<Vec<_>>();
        let mut used_old = HashSet::new();
        let mut entries = Vec::with_capacity(new_lines.len());

        for (idx, line) in new_lines.iter().enumerate() {
            let text_hash = new_hashes[idx].clone();
            let mut matched_idx = None;
            if let Some(old) = previous.get(idx) {
                if old.text_hash == text_hash && !used_old.contains(&idx) {
                    matched_idx = Some(idx);
                }
            }
            if matched_idx.is_none() {
                matched_idx = previous
                    .iter()
                    .enumerate()
                    .find(|(old_idx, old)| {
                        !used_old.contains(old_idx) && old.text_hash == text_hash
                    })
                    .map(|(old_idx, _)| old_idx);
            }
            if matched_idx.is_none() {
                matched_idx = previous
                    .iter()
                    .enumerate()
                    .find(|(old_idx, old)| {
                        !used_old.contains(old_idx)
                            && line_similarity(&old.text, &line.text) >= similarity_threshold
                    })
                    .map(|(old_idx, _)| old_idx);
            }
            if matched_idx.is_none() {
                if let Some(old) = previous.get(idx) {
                    let old_has_future_match =
                        new_lines
                            .iter()
                            .enumerate()
                            .skip(idx + 1)
                            .any(|(future_idx, future)| {
                                old.text_hash == new_hashes[future_idx]
                                    || line_similarity(&old.text, &future.text)
                                        >= similarity_threshold
                            });
                    if !used_old.contains(&idx) && !old_has_future_match {
                        matched_idx = Some(idx);
                    }
                }
            }
            let entry = if let Some(old_idx) = matched_idx {
                used_old.insert(old_idx);
                let old = &previous[old_idx];
                LineEntry {
                    line_id: old.line_id.clone(),
                    text: line.text.clone(),
                    newline: line.newline,
                    text_hash,
                    introduced_by: old.introduced_by.clone(),
                    last_content_change: if old.text == line.text && old.newline == line.newline {
                        old.last_content_change.clone()
                    } else {
                        change_id.clone()
                    },
                    last_move_change: if old_idx == idx {
                        old.last_move_change.clone()
                    } else {
                        Some(change_id.clone())
                    },
                    flags: old.flags.clone(),
                }
            } else {
                let line_id = LineId::new(change_id.clone(), *line_seq);
                *line_seq += 1;
                LineEntry {
                    line_id,
                    text: line.text.clone(),
                    newline: line.newline,
                    text_hash,
                    introduced_by: change_id.clone(),
                    last_content_change: change_id.clone(),
                    last_move_change: None,
                    flags: LineFlags::default(),
                }
            };
            entries.push(entry);
        }

        let old_positions = previous
            .iter()
            .enumerate()
            .map(|(idx, line)| (line.line_id.clone(), (idx, line)))
            .collect::<HashMap<_, _>>();
        let new_positions = entries
            .iter()
            .enumerate()
            .map(|(idx, line)| (line.line_id.clone(), (idx, line)))
            .collect::<HashMap<_, _>>();
        let mut line_changes = Vec::new();
        for (line_id, (new_idx, new_line)) in &new_positions {
            if let Some((old_idx, old_line)) = old_positions.get(line_id) {
                if old_line.text_hash != new_line.text_hash || old_line.newline != new_line.newline
                {
                    line_changes.push(LineChange {
                        line_id: line_id.clone(),
                        kind: LineChangeKind::Modified,
                        old_line_number: Some(*old_idx as u64 + 1),
                        new_line_number: Some(*new_idx as u64 + 1),
                        before_hash: Some(old_line.text_hash.clone()),
                        after_hash: Some(new_line.text_hash.clone()),
                    });
                } else if old_idx != new_idx {
                    line_changes.push(LineChange {
                        line_id: line_id.clone(),
                        kind: LineChangeKind::Moved,
                        old_line_number: Some(*old_idx as u64 + 1),
                        new_line_number: Some(*new_idx as u64 + 1),
                        before_hash: Some(old_line.text_hash.clone()),
                        after_hash: Some(new_line.text_hash.clone()),
                    });
                }
            } else {
                line_changes.push(LineChange {
                    line_id: line_id.clone(),
                    kind: LineChangeKind::Added,
                    old_line_number: None,
                    new_line_number: Some(*new_idx as u64 + 1),
                    before_hash: None,
                    after_hash: Some(new_line.text_hash.clone()),
                });
            }
        }
        for (line_id, (old_idx, old_line)) in old_positions {
            if !new_positions.contains_key(&line_id) {
                line_changes.push(LineChange {
                    line_id,
                    kind: LineChangeKind::Deleted,
                    old_line_number: Some(old_idx as u64 + 1),
                    new_line_number: None,
                    before_hash: Some(old_line.text_hash.clone()),
                    after_hash: None,
                });
            }
        }
        line_changes.sort_by_key(|change| {
            (
                change
                    .new_line_number
                    .or(change.old_line_number)
                    .unwrap_or(u64::MAX),
                change.line_id.local_seq,
            )
        });

        let mut order_builder = BatchBuilder::new(self.store.clone(), prolly_config());
        let mut index_builder = BatchBuilder::new(self.store.clone(), prolly_config());
        for (idx, entry) in entries.iter().enumerate() {
            let key = order_key(idx as u64 + 1);
            order_builder.add(key.clone(), cbor(entry)?);
            index_builder.add(entry.line_id.encode_key(), key);
        }
        let order_tree = order_builder.build()?;
        let index_tree = index_builder.build()?;
        let content = TextContent {
            version: TEXT_OBJECT_VERSION,
            content_hash: sha256_hex(bytes),
            line_count: entries.len() as u64,
            byte_count: bytes.len() as u64,
            order_map_root: tree_root_hex(&order_tree),
            line_index_map_root: tree_root_hex(&index_tree),
            representation: TextRepresentation::TreeText,
        };
        let object_id = self.put_object(TEXT_CONTENT_KIND, TEXT_OBJECT_VERSION, &content)?;
        Ok(TextBuildResult {
            object_id,
            line_changes,
        })
    }

    fn put_text_content_from_lines(&self, lines: &[LineEntry]) -> Result<ObjectId> {
        let bytes = materialize_lines(lines);
        let mut order_builder = BatchBuilder::new(self.store.clone(), prolly_config());
        let mut index_builder = BatchBuilder::new(self.store.clone(), prolly_config());
        for (idx, entry) in lines.iter().enumerate() {
            let key = order_key(idx as u64 + 1);
            order_builder.add(key.clone(), cbor(entry)?);
            index_builder.add(entry.line_id.encode_key(), key);
        }
        let order_tree = order_builder.build()?;
        let index_tree = index_builder.build()?;
        let content = TextContent {
            version: TEXT_OBJECT_VERSION,
            content_hash: sha256_hex(&bytes),
            line_count: lines.len() as u64,
            byte_count: bytes.len() as u64,
            order_map_root: tree_root_hex(&order_tree),
            line_index_map_root: tree_root_hex(&index_tree),
            representation: TextRepresentation::TreeText,
        };
        self.put_object(TEXT_CONTENT_KIND, TEXT_OBJECT_VERSION, &content)
    }

    fn put_blob(&self, bytes: Vec<u8>) -> Result<ObjectId> {
        let blob = Blob {
            version: BLOB_OBJECT_VERSION,
            content_hash: sha256_hex(&bytes),
            bytes,
        };
        self.put_object(BLOB_KIND, BLOB_OBJECT_VERSION, &blob)
    }

    fn put_object<T: Serialize>(&self, kind: &str, version: u16, value: &T) -> Result<ObjectId> {
        let bytes = cbor(value)?;
        let object_id = ObjectId::for_bytes(kind, version, &bytes);
        self.conn.execute(
            "INSERT OR IGNORE INTO objects \
             (object_id, kind, version, codec, hash_alg, size_bytes, bytes, created_at) \
             VALUES (?1, ?2, ?3, 'cbor', 'sha256', ?4, ?5, ?6)",
            params![
                object_id.0,
                kind,
                version as i64,
                bytes.len() as i64,
                bytes,
                now_ts()
            ],
        )?;
        Ok(object_id)
    }

    fn get_object<T: serde::de::DeserializeOwned>(
        &self,
        kind: &'static str,
        object_id: &ObjectId,
    ) -> Result<T> {
        let row: Option<(String, Vec<u8>)> = self
            .conn
            .query_row(
                "SELECT kind, bytes FROM objects WHERE object_id = ?1",
                params![object_id.0],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((actual_kind, bytes)) = row else {
            return Err(Error::ObjectNotFound {
                kind,
                id: object_id.0.clone(),
            });
        };
        if actual_kind != kind {
            return Err(Error::Corrupt(format!(
                "object {} has kind {}, expected {}",
                object_id.0, actual_kind, kind
            )));
        }
        from_cbor(&bytes)
    }

    fn store_operation(&self, operation: &Operation) -> Result<ObjectId> {
        let operation_id = self.put_object(OPERATION_KIND, OP_OBJECT_VERSION, operation)?;
        self.index_operation(operation, &operation_id)?;
        Ok(operation_id)
    }

    fn index_operation(&self, operation: &Operation, operation_id: &ObjectId) -> Result<()> {
        self.conn.execute(
            "INSERT INTO operations \
             (change_id, operation_id, kind, branch, before_root, after_root, actor_kind, actor_id, session_id, message, path_count, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                operation.change_id.0,
                operation_id.0,
                format!("{:?}", operation.kind),
                operation.branch,
                operation.before_root.as_ref().map(|id| id.0.clone()),
                operation.after_root.0,
                format!("{:?}", operation.actor.kind),
                operation.actor.id,
                operation.session_id,
                operation.message,
                operation.changes.len() as i64,
                operation.created_at
            ],
        )?;
        for (idx, parent) in operation.parents.iter().enumerate() {
            self.conn.execute(
                "INSERT INTO operation_parents (change_id, parent_change_id, position) VALUES (?1, ?2, ?3)",
                params![operation.change_id.0, parent.0, idx as i64],
            )?;
        }
        for change in &operation.changes {
            if let Some(file_id) = &change.file_id {
                self.conn.execute(
                    "INSERT INTO file_history \
                     (file_id, change_id, path, old_path, kind, before_hash, after_hash, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        file_id_key(file_id),
                        operation.change_id.0,
                        change.path,
                        change.old_path,
                        format!("{:?}", change.kind),
                        change.before_hash,
                        change.after_hash,
                        operation.created_at
                    ],
                )?;
                for line in &change.line_changes {
                    self.conn.execute(
                        "INSERT INTO line_history \
                         (line_id, file_id, change_id, path, line_number, kind, text_hash, created_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        params![
                            line.line_id_key(),
                            file_id_key(file_id),
                            operation.change_id.0,
                            change.path,
                            line.new_line_number.or(line.old_line_number).map(|n| n as i64),
                            format!("{:?}", line.kind),
                            line.after_hash.clone().or_else(|| line.before_hash.clone()),
                            operation.created_at
                        ],
                    )?;
                }
            }
        }
        Ok(())
    }

    fn store_message(
        &self,
        role: &str,
        body: &str,
        agent_id: Option<&str>,
        session_id: Option<&str>,
        change_id: Option<&ChangeId>,
        created_at: i64,
    ) -> Result<MessageId> {
        let id_seed = change_id.cloned().unwrap_or_else(|| {
            let seed = format!(
                "{}:{}:{}:{}:{}",
                self.config.workspace.id.0,
                role,
                agent_id.unwrap_or("none"),
                created_at,
                now_nanos()
            );
            ChangeId(format!(
                "msg_seed_{}",
                crate::ids::short_hash(seed.as_bytes(), 16)
            ))
        });
        let body = redact_sensitive_text(body);
        let message_id = MessageId::new(&id_seed, role, &body);
        let message = Message {
            version: MESSAGE_OBJECT_VERSION,
            id: message_id.clone(),
            role: role.to_string(),
            body,
            agent_id: agent_id.map(str::to_string),
            session_id: session_id.map(str::to_string),
            change_id: change_id.cloned(),
            created_at,
        };
        let object_id = self.put_object(MESSAGE_KIND, MESSAGE_OBJECT_VERSION, &message)?;
        self.index_message(&message, &object_id)?;
        Ok(message_id)
    }

    fn index_anchor(&self, anchor: &Anchor, object_id: &ObjectId) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO anchors \
             (anchor_id, label, file_id, line_id, object_id, created_path, created_line, created_change, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                anchor.id.0.clone(),
                anchor.label.clone(),
                file_id_key(&anchor.file_id),
                line_id_key_value(&anchor.line_id),
                object_id.0.clone(),
                anchor.created_path.clone(),
                anchor.created_line as i64,
                anchor.created_change.0.clone(),
                anchor.created_at
            ],
        )?;
        Ok(())
    }

    fn anchor(&self, anchor_id: &str) -> Result<Anchor> {
        let object_id: Option<String> = self
            .conn
            .query_row(
                "SELECT object_id FROM anchors WHERE anchor_id = ?1",
                params![anchor_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(object_id) = object_id else {
            return Err(Error::InvalidInput(format!(
                "anchor `{anchor_id}` not found"
            )));
        };
        self.get_object(ANCHOR_KIND, &ObjectId(object_id))
    }

    fn index_message(&self, message: &Message, object_id: &ObjectId) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO messages \
             (message_id, role, body, agent_id, session_id, change_id, object_id, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                message.id.0.clone(),
                message.role.clone(),
                message.body.clone(),
                message.agent_id.clone(),
                message.session_id.clone(),
                message.change_id.as_ref().map(|id| id.0.clone()),
                object_id.0.clone(),
                message.created_at
            ],
        )?;
        Ok(())
    }

    fn allocate_session_id(&self, agent_id: &str, title: Option<&str>) -> String {
        let seed = format!(
            "{}:{}:{}:{}",
            self.config.workspace.id.0,
            agent_id,
            title.unwrap_or("session"),
            now_nanos()
        );
        format!("session_{}", crate::ids::short_hash(seed.as_bytes(), 16))
    }

    fn ensure_agent_session(
        &self,
        agent_id: &str,
        session_id: &str,
        title: Option<&str>,
    ) -> Result<()> {
        validate_session_id(session_id)?;
        if let Some(existing) = self.try_agent_session(session_id)? {
            if existing.agent_id != agent_id {
                return Err(Error::InvalidInput(format!(
                    "session `{session_id}` belongs to another agent"
                )));
            }
            return Ok(());
        }
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO agent_sessions \
             (session_id, agent_id, title, status, started_at, ended_at, metadata_json) \
             VALUES (?1, ?2, ?3, 'active', ?4, NULL, NULL)",
            params![session_id, agent_id, title, now],
        )?;
        Ok(())
    }

    fn try_agent_session(&self, session_id: &str) -> Result<Option<AgentSession>> {
        self.conn
            .query_row(
                "SELECT session_id, agent_id, title, status, started_at, ended_at, metadata_json \
                 FROM agent_sessions WHERE session_id = ?1",
                params![session_id],
                agent_session_row,
            )
            .optional()
            .map_err(Error::from)
    }

    fn agent_session(&self, session_id: &str) -> Result<AgentSession> {
        self.try_agent_session(session_id)?
            .ok_or_else(|| Error::InvalidInput(format!("session `{session_id}` not found")))
    }

    fn open_agent_turn(
        &self,
        agent_id: &str,
        session_id: Option<&str>,
        base_change: &ChangeId,
        before_change: &ChangeId,
        metadata_json: Option<&serde_json::Value>,
    ) -> Result<String> {
        if let Some(session_id) = session_id {
            self.ensure_agent_session(agent_id, session_id, None)?;
        }
        let seed = format!(
            "{}:{}:{}:{}:{}",
            agent_id,
            session_id.unwrap_or("none"),
            base_change.0,
            before_change.0,
            now_nanos()
        );
        let turn_id = format!("turn_{}", crate::ids::short_hash(seed.as_bytes(), 16));
        self.conn.execute(
            "INSERT INTO agent_turns \
             (turn_id, agent_id, session_id, base_change, before_change, after_change, status, started_at, ended_at, metadata_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, 'started', ?6, NULL, ?7)",
            params![
                turn_id,
                agent_id,
                session_id,
                base_change.0,
                before_change.0,
                now_ts(),
                metadata_json.map(serde_json::to_string).transpose()?
            ],
        )?;
        Ok(turn_id)
    }

    fn finish_agent_turn(
        &self,
        turn_id: &str,
        status: &str,
        after_change: Option<&ChangeId>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE agent_turns SET status = ?1, after_change = ?2, ended_at = ?3 WHERE turn_id = ?4",
            params![
                status,
                after_change.map(|change_id| change_id.0.clone()),
                now_ts(),
                turn_id
            ],
        )?;
        Ok(())
    }

    fn update_agent_turn_progress(
        &self,
        turn_id: &str,
        status: &str,
        after_change: Option<&ChangeId>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE agent_turns SET status = ?1, after_change = ?2 WHERE turn_id = ?3",
            params![
                status,
                after_change.map(|change_id| change_id.0.clone()),
                turn_id
            ],
        )?;
        Ok(())
    }

    fn agent_turn(&self, turn_id: &str) -> Result<AgentTurn> {
        self.conn
            .query_row(
                "SELECT turn_id, agent_id, session_id, base_change, before_change, after_change, status, started_at, ended_at, metadata_json \
                 FROM agent_turns WHERE turn_id = ?1",
                params![turn_id],
                agent_turn_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("turn `{turn_id}` not found")))
    }

    fn agent_session_turns(&self, session_id: &str) -> Result<Vec<AgentTurn>> {
        let mut stmt = self.conn.prepare(
            "SELECT turn_id, agent_id, session_id, base_change, before_change, after_change, status, started_at, ended_at, metadata_json \
             FROM agent_turns WHERE session_id = ?1 ORDER BY started_at ASC, turn_id ASC",
        )?;
        let rows = stmt.query_map(params![session_id], agent_turn_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    fn agent_session_messages(&self, session_id: &str) -> Result<Vec<Message>> {
        let mut stmt = self.conn.prepare(
            "SELECT object_id FROM messages WHERE session_id = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| row.get::<_, String>(0))?;
        let object_ids = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        object_ids
            .into_iter()
            .map(|object_id| self.get_object(MESSAGE_KIND, &ObjectId(object_id)))
            .collect()
    }

    fn agent_session_events(&self, session_id: &str) -> Result<Vec<AgentEventRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT event_id, agent_id, session_id, turn_id, event_type, change_id, message_id, payload_json, created_at \
             FROM agent_events WHERE session_id = ?1 ORDER BY created_at ASC, event_id ASC",
        )?;
        let rows = stmt.query_map(params![session_id], agent_event_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    fn agent_session_operations(&self, session_id: &str) -> Result<Vec<TimelineEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT change_id, kind, branch, actor_id, message, created_at, path_count \
             FROM operations WHERE session_id = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![session_id], timeline_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    fn agent_turn_messages(&self, turn_id: &str) -> Result<Vec<Message>> {
        let mut stmt = self.conn.prepare(
            "SELECT object_id FROM messages \
             WHERE message_id IN ( \
                 SELECT message_id FROM agent_events \
                 WHERE turn_id = ?1 AND message_id IS NOT NULL \
             ) \
             ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![turn_id], |row| row.get::<_, String>(0))?;
        let object_ids = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        object_ids
            .into_iter()
            .map(|object_id| self.get_object(MESSAGE_KIND, &ObjectId(object_id)))
            .collect()
    }

    fn agent_turn_events(&self, turn_id: &str) -> Result<Vec<AgentEventRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT event_id, agent_id, session_id, turn_id, event_type, change_id, message_id, payload_json, created_at \
             FROM agent_events WHERE turn_id = ?1 ORDER BY created_at ASC, event_id ASC",
        )?;
        let rows = stmt.query_map(params![turn_id], agent_event_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    fn agent_turn_operations(&self, turn_id: &str) -> Result<Vec<TimelineEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT o.change_id, o.kind, o.branch, o.actor_id, o.message, o.created_at, o.path_count \
             FROM operations o \
             JOIN agent_events e ON e.change_id = o.change_id \
             WHERE e.turn_id = ?1 \
             ORDER BY o.created_at ASC, o.change_id ASC",
        )?;
        let rows = stmt.query_map(params![turn_id], timeline_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    fn agent_event(&self, event_id: &str) -> Result<AgentEventRecord> {
        self.conn
            .query_row(
                "SELECT event_id, agent_id, session_id, turn_id, event_type, change_id, message_id, payload_json, created_at \
                 FROM agent_events WHERE event_id = ?1",
                params![event_id],
                agent_event_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("event `{event_id}` not found")))
    }

    fn validate_agent_run_context(
        &self,
        branch: &AgentBranch,
        session_id: Option<&str>,
        turn_id: Option<&str>,
    ) -> Result<(Option<String>, Option<String>)> {
        let turn = turn_id
            .map(|turn_id| self.agent_turn(turn_id))
            .transpose()?;
        if let Some(turn) = &turn {
            if turn.agent_id != branch.agent_id {
                return Err(Error::InvalidInput(format!(
                    "turn `{}` does not belong to agent `{}`",
                    turn.turn_id, branch.agent_id
                )));
            }
            if turn.ended_at.is_some() {
                return Err(Error::InvalidInput(format!(
                    "turn `{}` is already ended",
                    turn.turn_id
                )));
            }
        }
        let resolved_session_id = session_id
            .map(str::to_string)
            .or_else(|| turn.as_ref().and_then(|turn| turn.session_id.clone()))
            .or_else(|| branch.session_id.clone());
        if let Some(session_id) = resolved_session_id.as_deref() {
            let session = self.agent_session(session_id)?;
            if session.agent_id != branch.agent_id {
                return Err(Error::InvalidInput(format!(
                    "session `{session_id}` does not belong to agent `{}`",
                    branch.agent_id
                )));
            }
        }
        Ok((resolved_session_id, turn_id.map(str::to_string)))
    }

    fn insert_agent_run_state(
        &self,
        agent_id: &str,
        session_id: Option<&str>,
        turn_id: Option<&str>,
        approval_id: Option<&str>,
        reason: &str,
        summary: &str,
        state: Option<serde_json::Value>,
        interruption: Option<serde_json::Value>,
    ) -> Result<AgentRunState> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(Error::InvalidInput(
                "agent run pause reason cannot be empty".to_string(),
            ));
        }
        let summary = summary.trim();
        if summary.is_empty() {
            return Err(Error::InvalidInput(
                "agent run pause summary cannot be empty".to_string(),
            ));
        }
        let redacted_reason = redact_sensitive_text(reason);
        let redacted_summary = redact_sensitive_text(summary);
        let redacted_state = redact_sensitive_json(state.unwrap_or_else(|| serde_json::json!({})));
        let redacted_interruption = interruption.map(redact_sensitive_json);
        let seed = format!(
            "{}:{}:{}:{}:{}:{}",
            agent_id,
            session_id.unwrap_or("none"),
            turn_id.unwrap_or("none"),
            approval_id.unwrap_or("none"),
            redacted_reason,
            now_nanos()
        );
        let run_id = format!("run_{}", crate::ids::short_hash(seed.as_bytes(), 16));
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO agent_run_states \
             (run_id, agent_id, session_id, turn_id, approval_id, status, reason, summary, state_json, interruption_json, created_at, updated_at, resumed_at, reviewer, note) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'paused', ?6, ?7, ?8, ?9, ?10, ?10, NULL, NULL, NULL)",
            params![
                run_id,
                agent_id,
                session_id,
                turn_id,
                approval_id,
                redacted_reason,
                redacted_summary,
                serde_json::to_string(&redacted_state)?,
                redacted_interruption
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?,
                now
            ],
        )?;
        self.insert_agent_event_with_context(
            agent_id,
            session_id,
            turn_id,
            "run_paused",
            None,
            None,
            &serde_json::json!({
                "run_id": run_id,
                "approval_id": approval_id,
                "reason": redacted_reason,
                "summary": redacted_summary
            }),
        )?;
        self.agent_run_state(&run_id)
    }

    fn agent_run_state(&self, run_id: &str) -> Result<AgentRunState> {
        let run_id = run_id.trim();
        if run_id.is_empty() {
            return Err(Error::InvalidInput(
                "agent run id cannot be empty".to_string(),
            ));
        }
        self.conn
            .query_row(
                "SELECT run_id, agent_id, session_id, turn_id, approval_id, status, reason, summary, state_json, interruption_json, created_at, updated_at, resumed_at, reviewer, note \
                 FROM agent_run_states WHERE run_id = ?1",
                params![run_id],
                agent_run_state_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("agent run `{run_id}` not found")))
    }

    fn agent_run_states_for_approval(&self, approval_id: &str) -> Result<Vec<AgentRunState>> {
        let mut stmt = self.conn.prepare(
            "SELECT run_id, agent_id, session_id, turn_id, approval_id, status, reason, summary, state_json, interruption_json, created_at, updated_at, resumed_at, reviewer, note \
             FROM agent_run_states WHERE approval_id = ?1 ORDER BY updated_at DESC, run_id DESC",
        )?;
        let rows = stmt.query_map(params![approval_id], agent_run_state_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    fn agent_approval(&self, approval_id: &str) -> Result<AgentApproval> {
        self.conn
            .query_row(
                "SELECT approval_id, agent_id, session_id, turn_id, action, summary, payload_json, status, requested_at, decided_at, reviewer, note \
                 FROM agent_approvals WHERE approval_id = ?1",
                params![approval_id],
                agent_approval_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("approval `{approval_id}` not found")))
    }

    fn latest_agent_test(&self, agent_id: &str) -> Result<Option<AgentTestSummary>> {
        self.latest_agent_gate(agent_id, "test")
    }

    fn latest_agent_gate(&self, agent_id: &str, kind: &str) -> Result<Option<AgentTestSummary>> {
        let event_type = agent_gate_event_type(kind)?;
        let row = self
            .conn
            .query_row(
                "SELECT event_id, turn_id, payload_json, created_at \
                 FROM agent_events \
                 WHERE agent_id = ?1 AND event_type = ?2 \
                 ORDER BY rowid DESC LIMIT 1",
                params![agent_id, event_type],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((event_id, turn_id, payload_json, created_at)) = row else {
            return Ok(None);
        };
        parse_agent_gate_summary(&event_id, turn_id, kind, &payload_json, created_at).map(Some)
    }

    fn latest_agent_gate_for_suite(
        &self,
        agent_id: &str,
        kind: &str,
        suite: &str,
    ) -> Result<Option<AgentTestSummary>> {
        let event_type = agent_gate_event_type(kind)?;
        let mut stmt = self.conn.prepare(
            "SELECT event_id, turn_id, payload_json, created_at \
             FROM agent_events \
             WHERE agent_id = ?1 AND event_type = ?2 \
             ORDER BY rowid DESC",
        )?;
        let rows = stmt.query_map(params![agent_id, event_type], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        for row in rows {
            let (event_id, turn_id, payload_json, created_at) = row?;
            let summary =
                parse_agent_gate_summary(&event_id, turn_id, kind, &payload_json, created_at)?;
            if summary.suite.as_deref() == Some(suite) {
                return Ok(Some(summary));
            }
        }
        Ok(None)
    }

    fn required_gate_suite_issues(
        &self,
        agent_id: &str,
        kind: &str,
        suites: &[String],
    ) -> Result<Vec<AgentReadinessIssue>> {
        let mut issues = Vec::new();
        for suite in suites {
            match self.latest_agent_gate_for_suite(agent_id, kind, suite)? {
                Some(gate) if !gate.success => {
                    issues.push(readiness_issue(
                        format!("required_{kind}_suite_failed"),
                        format!("required {kind} suite `{suite}` did not pass"),
                        Some(serde_json::json!({
                            "suite": suite,
                            "event_id": gate.event_id,
                            "status": gate.status,
                            "exit_code": gate.exit_code,
                            "command": gate.command,
                            "score": gate.score,
                            "threshold": gate.threshold
                        })),
                    ));
                }
                Some(_) => {}
                None => issues.push(readiness_issue(
                    format!("missing_required_{kind}_suite"),
                    format!("required {kind} suite `{suite}` has not been recorded"),
                    Some(serde_json::json!({ "suite": suite })),
                )),
            }
        }
        Ok(issues)
    }

    fn agent_gate_history_for_id(
        &self,
        agent_id: &str,
        kind: Option<&str>,
        limit: usize,
    ) -> Result<Vec<AgentTestSummary>> {
        let rows = if let Some(kind) = kind {
            let event_type = agent_gate_event_type(kind)?;
            let mut stmt = self.conn.prepare(
                "SELECT event_id, turn_id, event_type, payload_json, created_at \
                 FROM agent_events \
                 WHERE agent_id = ?1 AND event_type = ?2 \
                 ORDER BY rowid DESC LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![agent_id, event_type, limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT event_id, turn_id, event_type, payload_json, created_at \
                 FROM agent_events \
                 WHERE agent_id = ?1 AND event_type IN ('test_finished', 'eval_finished') \
                 ORDER BY rowid DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![agent_id, limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };

        rows.into_iter()
            .map(
                |(event_id, turn_id, event_type, payload_json, created_at)| {
                    let kind = agent_gate_kind_from_event_type(&event_type)?;
                    parse_agent_gate_summary(&event_id, turn_id, kind, &payload_json, created_at)
                },
            )
            .collect()
    }

    fn insert_agent_event(
        &self,
        agent_id: &str,
        event_type: &str,
        change_id: Option<&ChangeId>,
        message_id: Option<&MessageId>,
        payload: &serde_json::Value,
    ) -> Result<String> {
        self.insert_agent_event_with_context(
            agent_id, None, None, event_type, change_id, message_id, payload,
        )
    }

    fn insert_agent_event_with_context(
        &self,
        agent_id: &str,
        session_id: Option<&str>,
        turn_id: Option<&str>,
        event_type: &str,
        change_id: Option<&ChangeId>,
        message_id: Option<&MessageId>,
        payload: &serde_json::Value,
    ) -> Result<String> {
        let event_seed = format!(
            "{}:{}:{}:{}:{}:{}:{}",
            agent_id,
            session_id.unwrap_or("none"),
            turn_id.unwrap_or("none"),
            event_type,
            change_id.map(|id| id.0.as_str()).unwrap_or("none"),
            message_id.map(|id| id.0.as_str()).unwrap_or("none"),
            now_nanos()
        );
        let event_id = format!("evt_{}", crate::ids::short_hash(event_seed.as_bytes(), 16));
        let payload = redact_sensitive_json(payload.clone());
        self.conn.execute(
            "INSERT INTO agent_events \
             (event_id, agent_id, turn_id, session_id, event_type, change_id, message_id, payload_json, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                event_id,
                agent_id,
                turn_id,
                session_id,
                event_type,
                change_id.map(|id| id.0.clone()),
                message_id.map(|id| id.0.clone()),
                serde_json::to_string(&payload)?,
                now_ts()
            ],
        )?;
        Ok(event_id)
    }

    fn messages_for_change(&self, change_id: &ChangeId) -> Result<Vec<Message>> {
        let mut stmt = self.conn.prepare(
            "SELECT object_id FROM messages WHERE change_id = ?1 ORDER BY created_at, rowid",
        )?;
        let rows = stmt.query_map(params![change_id.0], |row| row.get::<_, String>(0))?;
        let object_ids = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        object_ids
            .into_iter()
            .map(|object_id| self.get_object(MESSAGE_KIND, &ObjectId(object_id)))
            .collect()
    }

    fn message(&self, message_id: &str) -> Result<Message> {
        let object_id: Option<String> = self
            .conn
            .query_row(
                "SELECT object_id FROM messages WHERE message_id = ?1",
                params![message_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(object_id) = object_id else {
            return Err(Error::InvalidInput(format!(
                "message `{message_id}` not found"
            )));
        };
        self.get_object(MESSAGE_KIND, &ObjectId(object_id))
    }

    fn object_info(&self, object_id: &str) -> Result<ObjectInfo> {
        self.conn
            .query_row(
                "SELECT object_id, kind, version, size_bytes, created_at FROM objects WHERE object_id = ?1",
                params![object_id],
                |row| {
                    Ok(ObjectInfo {
                        object_id: ObjectId(row.get(0)?),
                        kind: row.get(1)?,
                        version: row.get::<_, i64>(2)? as u16,
                        size_bytes: row.get::<_, i64>(3)? as u64,
                        created_at: row.get(4)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("object `{object_id}` not found")))
    }

    fn file_history_by_path(&self, path: &str) -> Result<Vec<FileHistoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT file_id, change_id, path, old_path, kind, before_hash, after_hash, created_at \
             FROM file_history WHERE path = ?1 OR old_path = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![path], file_history_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    fn file_history_by_file_id(&self, file_id: &str) -> Result<Vec<FileHistoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT file_id, change_id, path, old_path, kind, before_hash, after_hash, created_at \
             FROM file_history WHERE file_id = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![file_id], file_history_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    fn line_history_by_line_id(&self, line_id: &str) -> Result<Vec<LineHistoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT change_id, path, line_number, kind, text_hash, created_at \
             FROM line_history WHERE line_id = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![line_id], line_history_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    fn session_change_ids(&self, session_id: &str) -> Result<Vec<ChangeId>> {
        let mut stmt = self.conn.prepare(
            "SELECT change_id FROM operations WHERE session_id = ?1 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| Ok(ChangeId(row.get(0)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    fn agent_change_ids(&self, agent: &str) -> Result<Vec<ChangeId>> {
        let branch = self.agent_branch(agent)?;
        let mut stmt = self.conn.prepare(
            "SELECT change_id FROM operations \
             WHERE branch = ?1 OR actor_id = ?2 ORDER BY created_at ASC, rowid ASC",
        )?;
        let rows = stmt.query_map(params![branch.ref_name, branch.agent_id], |row| {
            Ok(ChangeId(row.get(0)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    fn operation(&self, change_id: &ChangeId) -> Result<Operation> {
        let object_id: Option<String> = self
            .conn
            .query_row(
                "SELECT operation_id FROM operations WHERE change_id = ?1",
                params![change_id.0],
                |row| row.get(0),
            )
            .optional()?;
        let Some(object_id) = object_id else {
            return Err(Error::OperationNotFound(change_id.0.clone()));
        };
        self.get_object(OPERATION_KIND, &ObjectId(object_id))
    }

    fn set_ref(
        &self,
        name: &str,
        change_id: &ChangeId,
        root_id: &ObjectId,
        operation_id: &ObjectId,
    ) -> Result<()> {
        let now = now_ts();
        let generation = self
            .try_get_ref(name)?
            .map(|record| record.generation + 1)
            .unwrap_or(1);
        self.conn.execute(
            "INSERT INTO refs (name, change_id, root_id, operation_id, generation, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(name) DO UPDATE SET \
                change_id = excluded.change_id, root_id = excluded.root_id, \
                operation_id = excluded.operation_id, generation = excluded.generation, \
                updated_at = excluded.updated_at",
            params![
                name,
                change_id.0,
                root_id.0,
                operation_id.0,
                generation,
                now
            ],
        )?;
        write_ref_file(
            &self.db_dir,
            name,
            change_id,
            root_id,
            operation_id,
            generation,
        )?;
        Ok(())
    }

    fn advance_ref_cas(
        &self,
        expected: &RefRecord,
        change_id: &ChangeId,
        root_id: &ObjectId,
        operation_id: &ObjectId,
    ) -> Result<()> {
        let generation = expected.generation + 1;
        let now = now_ts();
        let updated = self.conn.execute(
            "UPDATE refs SET change_id = ?1, root_id = ?2, operation_id = ?3, generation = ?4, updated_at = ?5 \
             WHERE name = ?6 AND generation = ?7 AND change_id = ?8",
            params![
                change_id.0,
                root_id.0,
                operation_id.0,
                generation,
                now,
                expected.name.clone(),
                expected.generation,
                expected.change_id.0.clone()
            ],
        )?;
        if updated != 1 {
            return Err(Error::StaleBranch(expected.name.clone()));
        }
        write_ref_file(
            &self.db_dir,
            &expected.name,
            change_id,
            root_id,
            operation_id,
            generation,
        )?;
        Ok(())
    }

    fn get_ref(&self, name: &str) -> Result<RefRecord> {
        self.try_get_ref(name)?
            .ok_or_else(|| Error::RefNotFound(name.to_string()))
    }

    fn try_get_ref(&self, name: &str) -> Result<Option<RefRecord>> {
        self.conn
            .query_row(
                "SELECT name, change_id, root_id, operation_id, generation, updated_at FROM refs WHERE name = ?1",
                params![name],
                ref_row,
            )
            .optional()
            .map_err(Error::from)
    }

    fn all_refs(&self) -> Result<Vec<RefRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, change_id, root_id, operation_id, generation, updated_at FROM refs ORDER BY name",
        )?;
        let rows = stmt.query_map([], ref_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    fn resolve_branch_ref(&self, branch: &str) -> Result<RefRecord> {
        if branch.starts_with("refs/") {
            self.get_ref(branch)
        } else {
            self.get_ref(&branch_ref(branch))
        }
    }

    fn resolve_refish(&self, refish: &str) -> Result<RefRecord> {
        if let Some(branch) = refish.strip_prefix("branch:") {
            return self.resolve_branch_ref(branch);
        }
        if let Some(agent) = refish.strip_prefix("agent:") {
            return self.get_ref(&agent_ref(agent));
        }
        if let Some(root_id) = refish.strip_prefix("root:") {
            return self.ref_from_root(&ObjectId(root_id.to_string()));
        }
        if refish.starts_with("ch_") {
            return self.ref_from_change(&ChangeId(refish.to_string()));
        }
        if refish.starts_with("refs/") {
            return self.get_ref(refish);
        }
        if let Ok(record) = self.get_ref(&branch_ref(refish)) {
            return Ok(record);
        }
        if let Ok(record) = self.get_ref(&agent_ref(refish)) {
            return Ok(record);
        }
        if refish.starts_with("obj_") {
            return self.ref_from_root(&ObjectId(refish.to_string()));
        }
        Err(Error::RefNotFound(refish.to_string()))
    }

    fn ref_from_change(&self, change_id: &ChangeId) -> Result<RefRecord> {
        let op = self.operation(change_id)?;
        let operation_id: String = self.conn.query_row(
            "SELECT operation_id FROM operations WHERE change_id = ?1",
            params![change_id.0],
            |row| row.get(0),
        )?;
        Ok(RefRecord {
            name: format!("changes/{}", change_id.0),
            change_id: change_id.clone(),
            root_id: op.after_root,
            operation_id: ObjectId(operation_id),
            generation: 0,
            updated_at: op.created_at,
        })
    }

    fn ref_from_root(&self, root_id: &ObjectId) -> Result<RefRecord> {
        let _: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        let row: Option<(String, String, i64)> = self
            .conn
            .query_row(
                "SELECT change_id, operation_id, created_at \
                 FROM operations WHERE after_root = ?1 \
                 ORDER BY created_at DESC, rowid DESC LIMIT 1",
                params![root_id.0],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((change_id, operation_id, created_at)) = row else {
            return Err(Error::InvalidInput(format!(
                "root `{}` is not associated with a recorded operation",
                root_id.0
            )));
        };
        Ok(RefRecord {
            name: format!("roots/{}", root_id.0),
            change_id: ChangeId(change_id),
            root_id: root_id.clone(),
            operation_id: ObjectId(operation_id),
            generation: 0,
            updated_at: created_at,
        })
    }

    fn common_parent_hint(&self, source: &ChangeId, target: &ChangeId) -> Result<ChangeId> {
        let source_ancestors = self.ancestor_set(source)?;
        let mut cursor = Some(target.clone());
        while let Some(change) = cursor {
            if source_ancestors.contains(&change.0) {
                return Ok(change);
            }
            cursor = self.first_parent(&change)?;
        }
        Err(Error::Conflict(
            "branches do not have a recorded common ancestor".to_string(),
        ))
    }

    fn ancestor_set(&self, change_id: &ChangeId) -> Result<HashSet<String>> {
        let mut out = HashSet::new();
        let mut stack = vec![change_id.clone()];
        while let Some(change) = stack.pop() {
            if !out.insert(change.0.clone()) {
                continue;
            }
            let parents = self.parents(&change)?;
            stack.extend(parents);
        }
        Ok(out)
    }

    fn first_parent(&self, change_id: &ChangeId) -> Result<Option<ChangeId>> {
        self.conn
            .query_row(
                "SELECT parent_change_id FROM operation_parents WHERE change_id = ?1 ORDER BY position LIMIT 1",
                params![change_id.0],
                |row| Ok(ChangeId(row.get(0)?)),
            )
            .optional()
            .map_err(Error::from)
    }

    fn parents(&self, change_id: &ChangeId) -> Result<Vec<ChangeId>> {
        let mut stmt = self.conn.prepare(
            "SELECT parent_change_id FROM operation_parents WHERE change_id = ?1 ORDER BY position",
        )?;
        let rows = stmt.query_map(params![change_id.0], |row| Ok(ChangeId(row.get(0)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    fn load_root_files(&self, root_id: &ObjectId) -> Result<BTreeMap<String, FileEntry>> {
        let root: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, root_id)?;
        let tree = tree_from_root_hex(root.path_map_root.as_deref())?;
        let iter = self.prolly.range(&tree, &[], None)?;
        let mut out = BTreeMap::new();
        for item in iter {
            let (key, value) = item?;
            let path = String::from_utf8(key)
                .map_err(|err| Error::Corrupt(format!("non UTF-8 path key: {err}")))?;
            let entry: FileEntry = from_cbor(&value)?;
            out.insert(path, entry);
        }
        Ok(out)
    }

    fn load_text_lines(&self, text_id: &ObjectId) -> Result<Vec<LineEntry>> {
        let content: TextContent = self.get_object(TEXT_CONTENT_KIND, text_id)?;
        let tree = tree_from_root_hex(content.order_map_root.as_deref())?;
        let iter = self.prolly.range(&tree, &[], None)?;
        let mut out = Vec::new();
        for item in iter {
            let (_, value) = item?;
            out.push(from_cbor(&value)?);
        }
        Ok(out)
    }

    fn materialize_entry_bytes(&self, entry: &FileEntry) -> Result<Vec<u8>> {
        match &entry.content {
            FileContentRef::Text(text_id) => {
                let lines = self.load_text_lines(text_id)?;
                Ok(materialize_lines(&lines))
            }
            FileContentRef::Opaque(blob_id) | FileContentRef::Binary(blob_id) => {
                let blob: Blob = self.get_object(BLOB_KIND, blob_id)?;
                Ok(blob.bytes)
            }
        }
    }

    fn materialize_files(
        &self,
        previous: &BTreeMap<String, FileEntry>,
        target: &BTreeMap<String, FileEntry>,
    ) -> Result<()> {
        materialize_into(
            &self.workspace_root,
            &self.workspace_root,
            previous,
            target,
            |entry| self.materialize_entry_bytes(entry),
        )
    }

    fn diff_files(
        &self,
        from: String,
        to: String,
        left: &BTreeMap<String, FileEntry>,
        right: &BTreeMap<String, FileEntry>,
        patches: bool,
        include_line_changes: bool,
    ) -> Result<DiffSummary> {
        let mut diff = self.diff_file_maps(left, right)?;
        if include_line_changes {
            attach_line_changes(&diff.changes, &mut diff.summaries);
        }
        if patches {
            self.attach_patches(left, right, &mut diff.summaries)?;
        }
        Ok(DiffSummary {
            from,
            to,
            files: diff.summaries,
        })
    }

    fn diff_file_maps(
        &self,
        left: &BTreeMap<String, FileEntry>,
        right: &BTreeMap<String, FileEntry>,
    ) -> Result<RootDiff> {
        let mut paths = BTreeSet::new();
        paths.extend(left.keys().cloned());
        paths.extend(right.keys().cloned());
        let mut changes = Vec::new();
        let mut summaries = Vec::new();
        let mut removed_by_hash: HashMap<String, Vec<(String, FileEntry)>> = HashMap::new();
        for (path, entry) in left {
            if !right.contains_key(path) {
                removed_by_hash
                    .entry(entry.content_hash.clone())
                    .or_default()
                    .push((path.clone(), entry.clone()));
            }
        }

        let mut handled_renames = HashSet::new();
        for path in paths {
            let old = left.get(&path);
            let new = right.get(&path);
            match (old, new) {
                (None, Some(new_entry)) => {
                    let rename = removed_by_hash
                        .get(&new_entry.content_hash)
                        .and_then(|candidates| candidates.first());
                    if let Some((old_path, old_entry)) = rename {
                        if old_entry.file_id == new_entry.file_id {
                            handled_renames.insert(old_path.clone());
                            let change = FileChange {
                                path: path.clone(),
                                old_path: Some(old_path.clone()),
                                file_id: Some(new_entry.file_id.clone()),
                                kind: FileChangeKind::Renamed,
                                before_hash: Some(old_entry.content_hash.clone()),
                                after_hash: Some(new_entry.content_hash.clone()),
                                line_changes: Vec::new(),
                            };
                            summaries.push(FileDiffSummary {
                                path: path.clone(),
                                old_path: Some(old_path.clone()),
                                kind: FileChangeKind::Renamed,
                                before_hash: Some(old_entry.content_hash.clone()),
                                after_hash: Some(new_entry.content_hash.clone()),
                                additions: 0,
                                deletions: 0,
                                line_changes: Vec::new(),
                                patch: None,
                            });
                            changes.push(change);
                            continue;
                        }
                    }
                    let line_changes = self.added_line_changes(&path, new_entry)?;
                    let (adds, dels) = count_line_delta(&line_changes);
                    changes.push(FileChange {
                        path: path.clone(),
                        old_path: None,
                        file_id: Some(new_entry.file_id.clone()),
                        kind: FileChangeKind::Added,
                        before_hash: None,
                        after_hash: Some(new_entry.content_hash.clone()),
                        line_changes,
                    });
                    summaries.push(FileDiffSummary {
                        path,
                        old_path: None,
                        kind: FileChangeKind::Added,
                        before_hash: None,
                        after_hash: Some(new_entry.content_hash.clone()),
                        additions: adds,
                        deletions: dels,
                        line_changes: Vec::new(),
                        patch: None,
                    });
                }
                (Some(old_entry), None) => {
                    if handled_renames.contains(&path) {
                        continue;
                    }
                    let line_changes = self.deleted_line_changes(&path, old_entry)?;
                    let (adds, dels) = count_line_delta(&line_changes);
                    changes.push(FileChange {
                        path: path.clone(),
                        old_path: None,
                        file_id: Some(old_entry.file_id.clone()),
                        kind: FileChangeKind::Deleted,
                        before_hash: Some(old_entry.content_hash.clone()),
                        after_hash: None,
                        line_changes,
                    });
                    summaries.push(FileDiffSummary {
                        path,
                        old_path: None,
                        kind: FileChangeKind::Deleted,
                        before_hash: Some(old_entry.content_hash.clone()),
                        after_hash: None,
                        additions: adds,
                        deletions: dels,
                        line_changes: Vec::new(),
                        patch: None,
                    });
                }
                (Some(old_entry), Some(new_entry)) => {
                    if old_entry.content_hash == new_entry.content_hash
                        && old_entry.executable == new_entry.executable
                        && old_entry.kind == new_entry.kind
                    {
                        continue;
                    }
                    let line_changes = self.modified_line_changes(old_entry, new_entry)?;
                    let (adds, dels) = count_line_delta(&line_changes);
                    let kind = if old_entry.kind != new_entry.kind {
                        FileChangeKind::TypeChanged
                    } else {
                        FileChangeKind::Modified
                    };
                    changes.push(FileChange {
                        path: path.clone(),
                        old_path: None,
                        file_id: Some(new_entry.file_id.clone()),
                        kind: kind.clone(),
                        before_hash: Some(old_entry.content_hash.clone()),
                        after_hash: Some(new_entry.content_hash.clone()),
                        line_changes,
                    });
                    summaries.push(FileDiffSummary {
                        path,
                        old_path: None,
                        kind,
                        before_hash: Some(old_entry.content_hash.clone()),
                        after_hash: Some(new_entry.content_hash.clone()),
                        additions: adds,
                        deletions: dels,
                        line_changes: Vec::new(),
                        patch: None,
                    });
                }
                (None, None) => {}
            }
        }
        Ok(RootDiff { changes, summaries })
    }

    fn added_line_changes(&self, _path: &str, entry: &FileEntry) -> Result<Vec<LineChange>> {
        let FileContentRef::Text(text_id) = &entry.content else {
            return Ok(Vec::new());
        };
        Ok(self
            .load_text_lines(text_id)?
            .into_iter()
            .enumerate()
            .map(|(idx, line)| LineChange {
                line_id: line.line_id,
                kind: LineChangeKind::Added,
                old_line_number: None,
                new_line_number: Some(idx as u64 + 1),
                before_hash: None,
                after_hash: Some(line.text_hash),
            })
            .collect())
    }

    fn deleted_line_changes(&self, _path: &str, entry: &FileEntry) -> Result<Vec<LineChange>> {
        let FileContentRef::Text(text_id) = &entry.content else {
            return Ok(Vec::new());
        };
        Ok(self
            .load_text_lines(text_id)?
            .into_iter()
            .enumerate()
            .map(|(idx, line)| LineChange {
                line_id: line.line_id,
                kind: LineChangeKind::Deleted,
                old_line_number: Some(idx as u64 + 1),
                new_line_number: None,
                before_hash: Some(line.text_hash),
                after_hash: None,
            })
            .collect())
    }

    fn modified_line_changes(
        &self,
        old_entry: &FileEntry,
        new_entry: &FileEntry,
    ) -> Result<Vec<LineChange>> {
        let (FileContentRef::Text(old_text), FileContentRef::Text(new_text)) =
            (&old_entry.content, &new_entry.content)
        else {
            return Ok(Vec::new());
        };
        let old_lines = self.load_text_lines(old_text)?;
        let new_lines = self.load_text_lines(new_text)?;
        let old_positions = old_lines
            .iter()
            .enumerate()
            .map(|(idx, line)| (line.line_id.clone(), (idx, line)))
            .collect::<HashMap<_, _>>();
        let new_positions = new_lines
            .iter()
            .enumerate()
            .map(|(idx, line)| (line.line_id.clone(), (idx, line)))
            .collect::<HashMap<_, _>>();
        let mut out = Vec::new();
        for (line_id, (new_idx, new_line)) in &new_positions {
            match old_positions.get(line_id) {
                Some((old_idx, old_line)) if old_line.text_hash != new_line.text_hash => {
                    out.push(LineChange {
                        line_id: line_id.clone(),
                        kind: LineChangeKind::Modified,
                        old_line_number: Some(*old_idx as u64 + 1),
                        new_line_number: Some(*new_idx as u64 + 1),
                        before_hash: Some(old_line.text_hash.clone()),
                        after_hash: Some(new_line.text_hash.clone()),
                    });
                }
                Some((old_idx, old_line)) if old_idx != new_idx => {
                    out.push(LineChange {
                        line_id: line_id.clone(),
                        kind: LineChangeKind::Moved,
                        old_line_number: Some(*old_idx as u64 + 1),
                        new_line_number: Some(*new_idx as u64 + 1),
                        before_hash: Some(old_line.text_hash.clone()),
                        after_hash: Some(new_line.text_hash.clone()),
                    });
                }
                Some(_) => {}
                None => out.push(LineChange {
                    line_id: line_id.clone(),
                    kind: LineChangeKind::Added,
                    old_line_number: None,
                    new_line_number: Some(*new_idx as u64 + 1),
                    before_hash: None,
                    after_hash: Some(new_line.text_hash.clone()),
                }),
            }
        }
        for (line_id, (old_idx, old_line)) in old_positions {
            if !new_positions.contains_key(&line_id) {
                out.push(LineChange {
                    line_id,
                    kind: LineChangeKind::Deleted,
                    old_line_number: Some(old_idx as u64 + 1),
                    new_line_number: None,
                    before_hash: Some(old_line.text_hash.clone()),
                    after_hash: None,
                });
            }
        }
        out.sort_by_key(|change| {
            (
                change
                    .new_line_number
                    .or(change.old_line_number)
                    .unwrap_or(u64::MAX),
                change.line_id.local_seq,
            )
        });
        Ok(out)
    }

    fn attach_patches(
        &self,
        left: &BTreeMap<String, FileEntry>,
        right: &BTreeMap<String, FileEntry>,
        summaries: &mut [FileDiffSummary],
    ) -> Result<()> {
        for summary in summaries {
            let old = summary
                .old_path
                .as_ref()
                .and_then(|path| left.get(path))
                .or_else(|| left.get(&summary.path));
            let new = right.get(&summary.path);
            let old_text = old
                .map(|entry| self.materialize_entry_bytes(entry))
                .transpose()?
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_default();
            let new_text = new
                .map(|entry| self.materialize_entry_bytes(entry))
                .transpose()?
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_default();
            summary.patch = Some(unified_patch(
                summary.old_path.as_deref().unwrap_or(&summary.path),
                &summary.path,
                &old_text,
                &new_text,
            ));
        }
        Ok(())
    }

    fn validate_worktree_root(&self, root: &WorktreeRoot) -> Result<()> {
        let path_tree = tree_from_root_hex(root.path_map_root.as_deref())?;
        let index_tree = tree_from_root_hex(root.file_index_map_root.as_deref())?;
        let path_entries = self
            .prolly
            .range(&path_tree, &[], None)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut count = 0;
        for (path, value) in path_entries {
            count += 1;
            let entry: FileEntry = from_cbor(&value)?;
            let indexed = self.prolly.get(&index_tree, &entry.file_id.encode_key())?;
            if indexed.as_deref() != Some(path.as_slice()) {
                return Err(Error::Corrupt(format!(
                    "file index mismatch for {}",
                    String::from_utf8_lossy(&path)
                )));
            }
        }
        if count != root.file_count {
            return Err(Error::Corrupt(format!(
                "root file_count {} but path map has {}",
                root.file_count, count
            )));
        }
        Ok(())
    }

    fn validate_text_content(&self, text_id: &ObjectId) -> Result<()> {
        let content: TextContent = self.get_object(TEXT_CONTENT_KIND, text_id)?;
        let order_tree = tree_from_root_hex(content.order_map_root.as_deref())?;
        let index_tree = tree_from_root_hex(content.line_index_map_root.as_deref())?;
        let mut count = 0;
        for item in self.prolly.range(&order_tree, &[], None)? {
            let (order_key, value) = item?;
            count += 1;
            let entry: LineEntry = from_cbor(&value)?;
            let indexed = self.prolly.get(&index_tree, &entry.line_id.encode_key())?;
            if indexed.as_deref() != Some(order_key.as_slice()) {
                return Err(Error::Corrupt(format!(
                    "line index mismatch for {}",
                    entry.line_id.local_seq
                )));
            }
        }
        if count != content.line_count {
            return Err(Error::Corrupt(format!(
                "text line_count {} but order map has {}",
                content.line_count, count
            )));
        }
        Ok(())
    }

    fn agent_branch(&self, agent: &str) -> Result<AgentBranch> {
        self.conn
            .query_row(
                "SELECT agent_id, ref_name, base_change, head_change, base_root, head_root, session_id, workdir, status, created_at, updated_at \
                 FROM agent_branches WHERE agent_id = ?1 OR ref_name = ?2 OR agent_id IN (SELECT agent_id FROM agents WHERE name = ?1)",
                params![agent, agent_ref(agent)],
                |row| {
                    Ok(AgentBranch {
                        agent_id: row.get(0)?,
                        ref_name: row.get(1)?,
                        base_change: ChangeId(row.get(2)?),
                        head_change: ChangeId(row.get(3)?),
                        base_root: ObjectId(row.get(4)?),
                        head_root: ObjectId(row.get(5)?),
                        session_id: row.get(6)?,
                        workdir: row.get(7)?,
                        status: row.get(8)?,
                        created_at: row.get(9)?,
                        updated_at: row.get(10)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::RefNotFound(agent_ref(agent)))
    }

    fn agent_record(&self, agent_id: &str) -> Result<AgentRecord> {
        self.conn
            .query_row(
                "SELECT agent_id, name, kind, provider, model, created_at, metadata_json \
                 FROM agents WHERE agent_id = ?1 OR name = ?1",
                params![agent_id],
                |row| {
                    Ok(AgentRecord {
                        agent_id: row.get(0)?,
                        name: row.get(1)?,
                        kind: row.get(2)?,
                        provider: row.get(3)?,
                        model: row.get(4)?,
                        created_at: row.get(5)?,
                        metadata_json: row.get(6)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::RefNotFound(agent_id.to_string()))
    }

    fn lease(&self, lease_id: &str) -> Result<LeaseRecord> {
        self.conn
            .query_row(
                "SELECT lease_id, agent_id, ref_name, path, file_id, mode, expires_at, created_at \
                 FROM leases WHERE lease_id = ?1",
                params![lease_id],
                lease_row,
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("lease `{lease_id}` not found")))
    }

    fn existing_active_lease(
        &self,
        agent_id: &str,
        path: Option<&str>,
        mode: &str,
    ) -> Result<Option<LeaseRecord>> {
        self.conn
            .query_row(
                "SELECT lease_id, agent_id, ref_name, path, file_id, mode, expires_at, created_at \
                 FROM leases WHERE agent_id = ?1 AND COALESCE(path, '') = COALESCE(?2, '') \
                   AND mode = ?3 AND expires_at > ?4 ORDER BY expires_at DESC LIMIT 1",
                params![agent_id, path, mode, now_ts()],
                lease_row,
            )
            .optional()
            .map_err(Error::from)
    }

    fn conflicting_active_leases(
        &self,
        agent_id: &str,
        path: Option<&str>,
        mode: &str,
    ) -> Result<Vec<LeaseRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT lease_id, agent_id, ref_name, path, file_id, mode, expires_at, created_at \
             FROM leases WHERE agent_id != ?1 AND COALESCE(path, '') = COALESCE(?2, '') \
               AND expires_at > ?3 ORDER BY expires_at ASC, created_at ASC",
        )?;
        let rows = stmt.query_map(params![agent_id, path, now_ts()], lease_row)?;
        let leases = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        Ok(leases
            .into_iter()
            .filter(|lease| mode == "write" || lease.mode == "write")
            .collect())
    }
}

fn apply_sqlite_pragmas(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(())
}

fn ensure_column(
    conn: &Connection,
    table: &'static str,
    column: &'static str,
    definition: &'static str,
) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if !columns.iter().any(|existing| existing == column) {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
            [],
        )?;
    }
    Ok(())
}

fn config_entries_from(config: &CrabConfig) -> Vec<ConfigEntry> {
    vec![
        config_entry("workspace.id", &config.workspace.id.0, "string", true),
        config_entry(
            "workspace.default_branch",
            &config.workspace.default_branch,
            "string",
            false,
        ),
        config_entry("recording.mode", &config.recording.mode, "string", false),
        config_entry(
            "recording.debounce_ms",
            config.recording.debounce_ms,
            "u64",
            false,
        ),
        config_entry(
            "recording.ignore_gitignored",
            config.recording.ignore_gitignored,
            "bool",
            false,
        ),
        config_entry(
            "text.small_text_max_bytes",
            config.text.small_text_max_bytes,
            "u64",
            false,
        ),
        config_entry(
            "text.tree_text_min_bytes",
            config.text.tree_text_min_bytes,
            "u64",
            false,
        ),
        config_entry(
            "text.opaque_text_max_bytes",
            config.text.opaque_text_max_bytes,
            "u64",
            false,
        ),
        config_entry(
            "text.max_line_bytes",
            config.text.max_line_bytes,
            "u64",
            false,
        ),
        config_entry(
            "text.preserve_similarity",
            config.text.preserve_similarity,
            "f32",
            false,
        ),
        config_entry(
            "agent.default_materialize",
            config.agent.default_materialize,
            "bool",
            false,
        ),
        config_entry(
            "agent.require_test_gate",
            config.agent.require_test_gate,
            "bool",
            false,
        ),
        config_entry(
            "agent.require_eval_gate",
            config.agent.require_eval_gate,
            "bool",
            false,
        ),
        config_entry(
            "agent.required_test_suites",
            format_config_list(&config.agent.required_test_suites),
            "list",
            false,
        ),
        config_entry(
            "agent.required_eval_suites",
            format_config_list(&config.agent.required_eval_suites),
            "list",
            false,
        ),
        config_entry(
            "agent.worktrees_dir",
            &config.agent.worktrees_dir,
            "path",
            false,
        ),
        config_entry(
            "agent.merge_strategy",
            &config.agent.merge_strategy,
            "string",
            false,
        ),
        config_entry(
            "git.export_trailers",
            config.git.export_trailers,
            "bool",
            false,
        ),
        config_entry(
            "guardrails.policy",
            &config.guardrails.policy,
            "policy",
            false,
        ),
    ]
}

fn config_entry(
    key: impl Into<String>,
    value: impl ToString,
    value_type: impl Into<String>,
    read_only: bool,
) -> ConfigEntry {
    ConfigEntry {
        key: key.into(),
        value: value.to_string(),
        value_type: value_type.into(),
        read_only,
    }
}

fn format_config_list(values: &[String]) -> String {
    values.join(",")
}

fn normalize_agent_gate_options(
    kind: &str,
    mut options: AgentGateOptions,
) -> Result<AgentGateOptions> {
    if let Some(suite) = options.suite.take() {
        let suite = suite.trim();
        if suite.is_empty() {
            return Err(Error::InvalidInput(format!(
                "agent {kind} suite cannot be empty"
            )));
        }
        options.suite = Some(suite.to_string());
    }
    if let Some(score) = options.score {
        if !score.is_finite() {
            return Err(Error::InvalidInput(format!(
                "agent {kind} score must be a finite number"
            )));
        }
    }
    if let Some(threshold) = options.threshold {
        if !threshold.is_finite() {
            return Err(Error::InvalidInput(format!(
                "agent {kind} threshold must be a finite number"
            )));
        }
        if options.score.is_none() {
            return Err(Error::InvalidInput(format!(
                "agent {kind} threshold requires a score"
            )));
        }
    }
    Ok(options)
}

fn normalize_agent_gate_filter(kind: Option<&str>) -> Result<Option<&'static str>> {
    let Some(kind) = kind.map(str::trim).filter(|kind| !kind.is_empty()) else {
        return Ok(None);
    };
    let normalized = kind.to_ascii_lowercase();
    match normalized.as_str() {
        "all" => Ok(None),
        "test" | "tests" => Ok(Some("test")),
        "eval" | "evals" => Ok(Some("eval")),
        other => Err(Error::InvalidInput(format!(
            "agent gate kind must be test, eval, or all, got `{other}`"
        ))),
    }
}

fn agent_gate_event_type(kind: &str) -> Result<&'static str> {
    match kind {
        "test" => Ok("test_finished"),
        "eval" => Ok("eval_finished"),
        other => Err(Error::InvalidInput(format!(
            "agent gate kind must be test or eval, got `{other}`"
        ))),
    }
}

fn agent_gate_kind_from_event_type(event_type: &str) -> Result<&'static str> {
    match event_type {
        "test_finished" => Ok("test"),
        "eval_finished" => Ok("eval"),
        other => Err(Error::Corrupt(format!(
            "unknown agent gate event type `{other}`"
        ))),
    }
}

fn parse_agent_gate_summary(
    event_id: &str,
    turn_id: Option<String>,
    kind: &str,
    payload_json: &str,
    created_at: i64,
) -> Result<AgentTestSummary> {
    let payload =
        serde_json::from_str::<serde_json::Value>(payload_json).unwrap_or(serde_json::Value::Null);
    let command = payload
        .get("command")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(AgentTestSummary {
        event_id: event_id.to_string(),
        turn_id,
        kind: kind.to_string(),
        suite: payload
            .get("suite")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        score: payload.get("score").and_then(|value| value.as_f64()),
        threshold: payload.get("threshold").and_then(|value| value.as_f64()),
        status: payload
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown")
            .to_string(),
        success: payload
            .get("success")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        exit_code: payload
            .get("exit_code")
            .and_then(|value| value.as_i64())
            .map(|value| value as i32),
        timed_out: payload
            .get("timed_out")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        duration_ms: payload
            .get("duration_ms")
            .and_then(|value| value.as_u64())
            .unwrap_or(0),
        command,
        created_at,
    })
}

fn apply_text_policy(config: &mut TextConfig, policy: Option<&str>) -> Result<()> {
    let Some(policy) = policy else {
        return Ok(());
    };
    match policy {
        "balanced" => Ok(()),
        "minimal" => {
            config.small_text_max_bytes = 4 * 1024;
            config.tree_text_min_bytes = 64 * 1024 + 1;
            config.opaque_text_max_bytes = 64 * 1024;
            config.max_line_bytes = 256 * 1024;
            config.preserve_similarity = 0.35;
            Ok(())
        }
        "full" => {
            config.small_text_max_bytes = 0;
            config.tree_text_min_bytes = 1;
            config.opaque_text_max_bytes = 64 * 1024 * 1024;
            config.max_line_bytes = 8 * 1024 * 1024;
            config.preserve_similarity = 0.65;
            Ok(())
        }
        other => Err(Error::InvalidInput(format!(
            "text policy must be minimal, balanced, or full, got `{other}`"
        ))),
    }
}

fn config_entry_from(config: &CrabConfig, key: &str) -> Option<ConfigEntry> {
    config_entries_from(config)
        .into_iter()
        .find(|entry| entry.key == key)
}

fn set_config_value(db: &CrabDb, config: &mut CrabConfig, key: &str, value: &str) -> Result<()> {
    match key {
        "workspace.id" => Err(Error::InvalidInput(
            "config key `workspace.id` is read-only".to_string(),
        )),
        "workspace.default_branch" => {
            validate_ref_segment(value)?;
            if db.try_get_ref(&branch_ref(value))?.is_none() {
                return Err(Error::InvalidInput(format!(
                    "default branch `{value}` does not exist"
                )));
            }
            config.workspace.default_branch = value.to_string();
            Ok(())
        }
        "recording.mode" => match value {
            "save" | "manual" | "watch" => {
                config.recording.mode = value.to_string();
                Ok(())
            }
            other => Err(Error::InvalidInput(format!(
                "recording.mode must be save, manual, or watch, got `{other}`"
            ))),
        },
        "recording.debounce_ms" => {
            config.recording.debounce_ms = parse_config_u64(key, value, true)?;
            Ok(())
        }
        "recording.ignore_gitignored" => {
            config.recording.ignore_gitignored = parse_config_bool(key, value)?;
            Ok(())
        }
        "text.small_text_max_bytes" => {
            config.text.small_text_max_bytes = parse_config_u64(key, value, false)?;
            Ok(())
        }
        "text.tree_text_min_bytes" => {
            config.text.tree_text_min_bytes = parse_config_u64(key, value, false)?;
            Ok(())
        }
        "text.opaque_text_max_bytes" => {
            config.text.opaque_text_max_bytes = parse_config_u64(key, value, false)?;
            Ok(())
        }
        "text.max_line_bytes" => {
            config.text.max_line_bytes = parse_config_u64(key, value, false)?;
            Ok(())
        }
        "text.preserve_similarity" => {
            let parsed = value.parse::<f32>().map_err(|_| {
                Error::InvalidInput(format!("config key `{key}` expects a floating point value"))
            })?;
            if !parsed.is_finite() || !(0.0..=1.0).contains(&parsed) {
                return Err(Error::InvalidInput(format!(
                    "config key `{key}` must be between 0.0 and 1.0"
                )));
            }
            config.text.preserve_similarity = parsed;
            Ok(())
        }
        "agent.default_materialize" => {
            config.agent.default_materialize = parse_config_bool(key, value)?;
            Ok(())
        }
        "agent.require_test_gate" => {
            config.agent.require_test_gate = parse_config_bool(key, value)?;
            Ok(())
        }
        "agent.require_eval_gate" => {
            config.agent.require_eval_gate = parse_config_bool(key, value)?;
            Ok(())
        }
        "agent.required_test_suites" => {
            config.agent.required_test_suites = parse_config_suite_list(key, value)?;
            Ok(())
        }
        "agent.required_eval_suites" => {
            config.agent.required_eval_suites = parse_config_suite_list(key, value)?;
            Ok(())
        }
        "agent.worktrees_dir" => {
            config.agent.worktrees_dir = normalize_relative_path(value)?;
            Ok(())
        }
        "agent.merge_strategy" => {
            if value != "conservative" {
                return Err(Error::InvalidInput(format!(
                    "agent.merge_strategy must be conservative, got `{value}`"
                )));
            }
            config.agent.merge_strategy = value.to_string();
            Ok(())
        }
        "git.export_trailers" => {
            config.git.export_trailers = parse_config_bool(key, value)?;
            Ok(())
        }
        "guardrails.policy" => {
            let _ = parse_guardrail_policy(value)?;
            config.guardrails.policy = value.to_string();
            Ok(())
        }
        _ => Err(Error::InvalidInput(format!("unknown config key `{key}`"))),
    }
}

fn parse_config_bool(key: &str, value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(Error::InvalidInput(format!(
            "config key `{key}` expects a boolean value"
        ))),
    }
}

fn parse_config_suite_list(key: &str, value: &str) -> Result<Vec<String>> {
    let mut suites = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in value.split([',', ';', '\n']) {
        let suite = raw.trim();
        if suite.is_empty() {
            continue;
        }
        if suite
            .chars()
            .any(|ch| matches!(ch, ',' | ';' | '\n' | '\r'))
        {
            return Err(Error::InvalidInput(format!(
                "config key `{key}` suite names cannot contain separators"
            )));
        }
        if seen.insert(suite.to_string()) {
            suites.push(suite.to_string());
        }
    }
    Ok(suites)
}

fn parse_config_u64(key: &str, value: &str, allow_zero: bool) -> Result<u64> {
    let parsed = value.parse::<u64>().map_err(|_| {
        Error::InvalidInput(format!("config key `{key}` expects an unsigned integer"))
    })?;
    if !allow_zero && parsed == 0 {
        return Err(Error::InvalidInput(format!(
            "config key `{key}` must be greater than zero"
        )));
    }
    Ok(parsed)
}

fn read_config(db_dir: &Path) -> Result<CrabConfig> {
    let text = fs::read_to_string(db_dir.join(CONFIG_FILE))?;
    Ok(toml::from_str(&text)?)
}

fn write_config(db_dir: &Path, config: &CrabConfig) -> Result<()> {
    let path = db_dir.join(CONFIG_FILE);
    let temp = db_dir.join(format!("{CONFIG_FILE}.tmp.{}", now_nanos()));
    fs::write(&temp, toml::to_string_pretty(config)?)?;
    if let Err(err) = fs::rename(&temp, &path) {
        let _ = fs::remove_file(&temp);
        return Err(Error::Io(err));
    }
    Ok(())
}

fn write_default_crabignore(workspace_root: &Path) -> Result<()> {
    let path = workspace_root.join(".crabignore");
    if path.exists() {
        return Ok(());
    }
    fs::write(
        path,
        format!("{}\n", DEFAULT_CRABIGNORE_PATTERNS.join("\n")),
    )?;
    Ok(())
}

fn read_ignore_patterns(path: &Path) -> Result<Vec<IgnorePattern>> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(Error::Io(err)),
    };
    Ok(content
        .lines()
        .enumerate()
        .filter_map(|(idx, line)| {
            let pattern = line.trim();
            if pattern.is_empty() || pattern.starts_with('#') {
                None
            } else {
                Some(IgnorePattern {
                    line: idx + 1,
                    pattern: pattern.to_string(),
                })
            }
        })
        .collect())
}

fn normalize_ignore_pattern(pattern: &str) -> Result<String> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return Err(Error::InvalidInput(
            "ignore pattern cannot be empty".to_string(),
        ));
    }
    if pattern.starts_with('#') {
        return Err(Error::InvalidInput(
            "ignore pattern cannot be a comment".to_string(),
        ));
    }
    if pattern.contains('\0') || pattern.contains('\n') || pattern.contains('\r') {
        return Err(Error::InvalidInput(
            "ignore pattern cannot contain control separators".to_string(),
        ));
    }
    Ok(pattern.to_string())
}

fn redact_sensitive_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    if is_sensitive_json_key(&key) {
                        (key, serde_json::Value::String("[REDACTED]".to_string()))
                    } else {
                        (key, redact_sensitive_json(value))
                    }
                })
                .collect(),
        ),
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(redact_sensitive_json).collect())
        }
        serde_json::Value::String(value) => {
            serde_json::Value::String(redact_sensitive_text(&value))
        }
        other => other,
    }
}

fn redact_sensitive_text(input: &str) -> String {
    if !may_contain_sensitive_text(input) {
        return input.to_string();
    }
    let mut output = String::with_capacity(input.len());
    for chunk in input.split_inclusive('\n') {
        if let Some(line) = chunk.strip_suffix('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            output.push_str(&redact_sensitive_line(line));
            if chunk.ends_with("\r\n") {
                output.push_str("\r\n");
            } else {
                output.push('\n');
            }
        } else {
            output.push_str(&redact_sensitive_line(chunk));
        }
    }
    if input.is_empty() {
        output.clear();
    }
    output
}

fn may_contain_sensitive_text(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    [
        "authorization",
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "api-key",
        "apikey",
        "private_key",
        "private-key",
        "bearer ",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn redact_sensitive_line(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    if let Some((separator_idx, value_start)) = sensitive_assignment_span(line, &lower) {
        let mut redacted = String::new();
        redacted.push_str(&line[..separator_idx + 1]);
        redacted.push_str(&line[separator_idx + 1..value_start]);
        redacted.push_str("[REDACTED]");
        return redacted;
    }
    if let Some(idx) = lower.find("bearer ") {
        let value_start = idx + "bearer ".len();
        let mut redacted = String::new();
        redacted.push_str(&line[..value_start]);
        redacted.push_str("[REDACTED]");
        return redacted;
    }
    line.to_string()
}

fn sensitive_assignment_span(line: &str, lower: &str) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    for key in SENSITIVE_TEXT_KEYS {
        let mut search_from = 0;
        while let Some(relative_idx) = lower[search_from..].find(key) {
            let key_idx = search_from + relative_idx;
            let rest_start = key_idx + key.len();
            let rest = &lower[rest_start..];
            let Some(separator_relative_idx) = rest.find(|ch| ch == ':' || ch == '=') else {
                search_from = rest_start;
                continue;
            };
            let between = &line[rest_start..rest_start + separator_relative_idx];
            if between.chars().all(is_secret_separator_padding) {
                let separator_idx = rest_start + separator_relative_idx;
                let value_start = line[separator_idx + 1..]
                    .char_indices()
                    .find(|(_, ch)| !ch.is_whitespace())
                    .map(|(idx, _)| separator_idx + 1 + idx)
                    .unwrap_or(line.len());
                let candidate = (separator_idx, value_start);
                if best
                    .map(|(best_idx, _)| separator_idx < best_idx)
                    .unwrap_or(true)
                {
                    best = Some(candidate);
                }
                break;
            }
            search_from = rest_start;
        }
    }
    best
}

fn is_secret_separator_padding(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '"' | '\'' | '`' | '_' | '-')
}

fn is_sensitive_json_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .map(|ch| match ch {
            '-' | ' ' => '_',
            other => other.to_ascii_lowercase(),
        })
        .collect::<String>();
    normalized == "authorization"
        || normalized == "password"
        || normalized == "passwd"
        || normalized == "secret"
        || normalized == "token"
        || normalized == "credential"
        || normalized.ends_with("password")
        || normalized.ends_with("secret")
        || normalized.ends_with("token")
        || normalized.ends_with("credential")
        || normalized.ends_with("_secret")
        || normalized.ends_with("_token")
        || normalized.ends_with("_credential")
        || normalized.contains("api_key")
        || normalized.contains("apikey")
        || normalized.contains("private_key")
}

const SENSITIVE_TEXT_KEYS: &[&str] = &[
    "authorization",
    "openai_api_key",
    "anthropic_api_key",
    "client_secret",
    "client-secret",
    "private_key",
    "private-key",
    "refresh_token",
    "refresh-token",
    "access_token",
    "access-token",
    "auth_token",
    "auth-token",
    "id_token",
    "id-token",
    "api_key",
    "api-key",
    "apikey",
    "password",
    "passwd",
    "secret",
    "token",
];

fn write_ref_file(
    db_dir: &Path,
    name: &str,
    change_id: &ChangeId,
    root_id: &ObjectId,
    operation_id: &ObjectId,
    generation: i64,
) -> Result<()> {
    let path = db_dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = serde_json::json!({
        "name": name,
        "change_id": change_id.0,
        "root_id": root_id.0,
        "operation_id": operation_id.0,
        "generation": generation,
        "updated_at": now_ts(),
    });
    fs::write(path, serde_json::to_vec_pretty(&body)?)?;
    Ok(())
}

fn remove_ref_file(db_dir: &Path, name: &str) -> Result<()> {
    let path = db_dir.join(name);
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(Error::Io(err)),
    }
}

fn prolly_config() -> Config {
    Config::builder()
        .min_chunk_size(4)
        .max_chunk_size(1024)
        .chunking_factor(128)
        .hash_seed(0xC0DB)
        .encoding(Encoding::Raw)
        .build()
}

#[derive(Clone, Copy, Debug)]
enum MapInspectType {
    Raw,
    Path,
    FileIndex,
    TextOrder,
    LineIndex,
}

impl MapInspectType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Path => "path",
            Self::FileIndex => "file-index",
            Self::TextOrder => "text-order",
            Self::LineIndex => "line-index",
        }
    }
}

fn parse_map_inspect_type(value: &str) -> Result<MapInspectType> {
    match value {
        "raw" => Ok(MapInspectType::Raw),
        "path" | "path-map" => Ok(MapInspectType::Path),
        "file-index" | "file_index" | "file-index-map" => Ok(MapInspectType::FileIndex),
        "text-order" | "text_order" | "order" | "order-map" => Ok(MapInspectType::TextOrder),
        "line-index" | "line_index" | "line-index-map" => Ok(MapInspectType::LineIndex),
        other => Err(Error::InvalidInput(format!(
            "map type must be raw, path, file-index, text-order, or line-index, got `{other}`"
        ))),
    }
}

fn parse_map_key_spec(spec: &str) -> Result<Vec<u8>> {
    if let Some(hex_value) = spec.strip_prefix("hex:") {
        return hex::decode(hex_value)
            .map_err(|err| Error::InvalidInput(format!("invalid hex map key: {err}")));
    }
    if let Some(text) = spec.strip_prefix("text:") {
        return Ok(text.as_bytes().to_vec());
    }
    if let Some(value) = spec.strip_prefix("u64:") {
        let value = value.parse::<u64>().map_err(|_| {
            Error::InvalidInput(format!("invalid unsigned integer map key `{value}`"))
        })?;
        return Ok(value.to_be_bytes().to_vec());
    }
    if let Some(line_number) = spec.strip_prefix("order:") {
        let line_number = line_number.parse::<u64>().map_err(|_| {
            Error::InvalidInput(format!("invalid order line number `{line_number}`"))
        })?;
        return Ok(order_key(line_number));
    }
    if let Some(id) = spec
        .strip_prefix("id:")
        .or_else(|| spec.strip_prefix("compound:"))
    {
        return parse_compound_map_key(id);
    }
    Ok(spec.as_bytes().to_vec())
}

fn parse_compound_map_key(spec: &str) -> Result<Vec<u8>> {
    let (change_id, local_seq) = spec.rsplit_once(':').ok_or_else(|| {
        Error::InvalidInput("compound map key must look like id:ch_...:<local_seq>".to_string())
    })?;
    if !change_id.starts_with("ch_") {
        return Err(Error::InvalidInput(
            "compound map key change id must start with ch_".to_string(),
        ));
    }
    let local_seq = local_seq.parse::<u64>().map_err(|_| {
        Error::InvalidInput(format!(
            "invalid compound map key local sequence `{local_seq}`"
        ))
    })?;
    Ok(FileId::new(ChangeId(change_id.to_string()), local_seq).encode_key())
}

fn inspect_map_diff_entry(map_type: MapInspectType, diff: Diff) -> MapDiffInspect {
    match diff {
        Diff::Added { key, val } => MapDiffInspect {
            kind: "added".to_string(),
            key: inspect_map_key(map_type, &key),
            old_value: None,
            new_value: Some(inspect_map_value(map_type, &val)),
        },
        Diff::Removed { key, val } => MapDiffInspect {
            kind: "removed".to_string(),
            key: inspect_map_key(map_type, &key),
            old_value: Some(inspect_map_value(map_type, &val)),
            new_value: None,
        },
        Diff::Changed { key, old, new } => MapDiffInspect {
            kind: "changed".to_string(),
            key: inspect_map_key(map_type, &key),
            old_value: Some(inspect_map_value(map_type, &old)),
            new_value: Some(inspect_map_value(map_type, &new)),
        },
    }
}

fn inspect_map_key(map_type: MapInspectType, key: &[u8]) -> MapKeyInspect {
    let text = utf8_full(key);
    let summary = match map_type {
        MapInspectType::Path => serde_json::json!({ "path": text.clone() }),
        MapInspectType::FileIndex => compound_key_summary(key, "file_id"),
        MapInspectType::TextOrder => order_key_summary(key),
        MapInspectType::LineIndex => compound_key_summary(key, "line_id"),
        MapInspectType::Raw => serde_json::json!({ "bytes": key.len() }),
    };
    MapKeyInspect {
        hex: hex::encode(key),
        text,
        summary,
    }
}

fn inspect_map_value(map_type: MapInspectType, value: &[u8]) -> MapValueInspect {
    let summary = match map_type {
        MapInspectType::Path => path_map_value_summary(value),
        MapInspectType::FileIndex => serde_json::json!({
            "path": utf8_full(value),
        }),
        MapInspectType::TextOrder => text_order_value_summary(value),
        MapInspectType::LineIndex => order_key_summary(value),
        MapInspectType::Raw => serde_json::json!({ "bytes": value.len() }),
    };
    let (hex_preview, truncated) = hex_preview(value, 256);
    MapValueInspect {
        bytes: value.len(),
        hex_preview,
        truncated,
        text: utf8_preview(value, 240),
        summary,
    }
}

fn path_map_value_summary(value: &[u8]) -> serde_json::Value {
    match decode_cbor_value::<FileEntry>(value) {
        Ok(entry) => serde_json::json!({
            "file_id": file_id_key(&entry.file_id),
            "kind": entry.kind,
            "mode": entry.mode,
            "executable": entry.executable,
            "size_bytes": entry.size_bytes,
            "content_hash": entry.content_hash,
            "content_object": content_object_id(&entry.content),
            "created_by": entry.created_by,
            "last_content_change": entry.last_content_change,
            "last_path_change": entry.last_path_change,
        }),
        Err(error) => decode_error_summary(error),
    }
}

fn text_order_value_summary(value: &[u8]) -> serde_json::Value {
    match decode_cbor_value::<LineEntry>(value) {
        Ok(entry) => serde_json::json!({
            "line_id": line_id_key_value(&entry.line_id),
            "text_hash": entry.text_hash,
            "text": utf8_preview(&entry.text, 240),
            "newline": entry.newline,
            "introduced_by": entry.introduced_by,
            "last_content_change": entry.last_content_change,
            "last_move_change": entry.last_move_change,
            "flags": entry.flags,
        }),
        Err(error) => decode_error_summary(error),
    }
}

fn decode_cbor_value<T>(value: &[u8]) -> std::result::Result<T, String>
where
    T: DeserializeOwned,
{
    from_cbor(value).map_err(|err| err.to_string())
}

fn decode_error_summary(error: String) -> serde_json::Value {
    serde_json::json!({ "decode_error": error })
}

fn compound_key_summary(key: &[u8], name: &str) -> serde_json::Value {
    if key.len() != 40 {
        return serde_json::json!({
            "bytes": key.len(),
            "expected": format!("{name} compound key"),
        });
    }
    let local_seq = u64::from_be_bytes(key[32..40].try_into().unwrap_or([0; 8]));
    serde_json::json!({
        "kind": name,
        "origin_change_digest": hex::encode(&key[..32]),
        "local_seq": local_seq,
    })
}

fn order_key_summary(key: &[u8]) -> serde_json::Value {
    if key.len() != 8 {
        return serde_json::json!({
            "bytes": key.len(),
            "expected": "8-byte big-endian order key",
        });
    }
    let order = u64::from_be_bytes(key.try_into().unwrap_or([0; 8]));
    let line_number_hint = if order % ORDER_KEY_STEP == 0 {
        Some(order / ORDER_KEY_STEP)
    } else {
        None
    };
    serde_json::json!({
        "order": order,
        "line_number_hint": line_number_hint,
    })
}

fn utf8_full(bytes: &[u8]) -> Option<String> {
    String::from_utf8(bytes.to_vec()).ok()
}

fn utf8_preview(bytes: &[u8], max_chars: usize) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    if text.chars().count() <= max_chars {
        return Some(text.to_string());
    }
    let mut preview = text.chars().take(max_chars).collect::<String>();
    preview.push_str("...");
    Some(preview)
}

fn hex_preview(bytes: &[u8], max_bytes: usize) -> (String, bool) {
    if bytes.len() <= max_bytes {
        (hex::encode(bytes), false)
    } else {
        (hex::encode(&bytes[..max_bytes]), true)
    }
}

fn tree_root_hex(tree: &Tree) -> Option<String> {
    tree.root.as_ref().map(|cid| hex::encode(cid.as_bytes()))
}

fn tree_from_root_hex(root: Option<&str>) -> Result<Tree> {
    let cid = match root {
        Some(hex_root) => {
            let bytes = hex::decode(hex_root)
                .map_err(|err| Error::Corrupt(format!("invalid tree root hex: {err}")))?;
            let bytes: [u8; 32] = bytes
                .as_slice()
                .try_into()
                .map_err(|_| Error::Corrupt("tree root CID must be 32 bytes".to_string()))?;
            Some(Cid(bytes))
        }
        None => None,
    };
    Ok(Tree {
        root: cid,
        config: prolly_config(),
    })
}

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn run_command_with_timeout(
    command: &[String],
    cwd: &Path,
    timeout: Duration,
) -> Result<CommandRunResult> {
    let started = Instant::now();
    let mut child = match Command::new(&command[0])
        .args(&command[1..])
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            return Ok(CommandRunResult {
                success: false,
                exit_code: None,
                timed_out: false,
                duration_ms: elapsed_ms(started.elapsed()),
                stdout: Vec::new(),
                stderr: err.to_string().into_bytes(),
            });
        }
    };

    loop {
        if child.try_wait()?.is_some() {
            let output = child.wait_with_output()?;
            return Ok(CommandRunResult {
                success: output.status.success(),
                exit_code: output.status.code(),
                timed_out: false,
                duration_ms: elapsed_ms(started.elapsed()),
                stdout: output.stdout,
                stderr: output.stderr,
            });
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let output = child.wait_with_output()?;
            return Ok(CommandRunResult {
                success: false,
                exit_code: output.status.code(),
                timed_out: true,
                duration_ms: elapsed_ms(started.elapsed()),
                stdout: output.stdout,
                stderr: output.stderr,
            });
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn output_preview(bytes: &[u8]) -> (String, bool) {
    let truncated = bytes.len() > AGENT_TEST_OUTPUT_PREVIEW_BYTES;
    let preview = if truncated {
        &bytes[..AGENT_TEST_OUTPUT_PREVIEW_BYTES]
    } else {
        bytes
    };
    (String::from_utf8_lossy(preview).into_owned(), truncated)
}

fn elapsed_ms(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

fn known_gc_object_kinds() -> HashSet<&'static str> {
    [
        WORKTREE_ROOT_KIND,
        TEXT_CONTENT_KIND,
        OPERATION_KIND,
        BLOB_KIND,
        MESSAGE_KIND,
        ANCHOR_KIND,
    ]
    .into_iter()
    .collect()
}

fn normalize_relative_path(path: &str) -> Result<String> {
    if path.as_bytes().contains(&0) {
        return Err(Error::InvalidPath {
            path: path.to_string(),
            reason: "NUL bytes are not allowed".to_string(),
        });
    }
    let path = path.replace('\\', "/");
    let mut parts = Vec::new();
    for component in Path::new(&path).components() {
        match component {
            Component::Normal(part) => {
                let part = part.to_string_lossy();
                if part.is_empty() {
                    continue;
                }
                parts.push(part.to_string());
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::InvalidPath {
                    path: path.to_string(),
                    reason: "path must stay inside the workspace".to_string(),
                });
            }
        }
    }
    if parts.is_empty() {
        return Err(Error::InvalidPath {
            path: path.to_string(),
            reason: "path cannot be empty".to_string(),
        });
    }
    Ok(parts.join("/"))
}

fn normalize_workdir_path(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(Error::InvalidPath {
            path: String::new(),
            reason: "path cannot be empty".to_string(),
        });
    }
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push(component.as_os_str()),
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(Error::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                    reason: "agent workdir cannot contain parent directory components".to_string(),
                });
            }
        }
    }
    if out.as_os_str().is_empty() {
        return Err(Error::InvalidPath {
            path: path.to_string_lossy().to_string(),
            reason: "path cannot be empty".to_string(),
        });
    }
    Ok(out)
}

fn canonicalize_existing_workdir_prefix(path: &Path) -> Result<PathBuf> {
    let mut existing = path;
    let mut missing = Vec::new();
    while !existing.exists() {
        let Some(name) = existing.file_name() else {
            break;
        };
        missing.push(name.to_os_string());
        existing = existing.parent().ok_or_else(|| Error::InvalidPath {
            path: path.to_string_lossy().to_string(),
            reason: "agent workdir has no existing ancestor".to_string(),
        })?;
    }
    let mut out = existing.canonicalize()?;
    for name in missing.iter().rev() {
        out.push(name);
    }
    normalize_workdir_path(&out)
}

fn prepare_agent_workdir(path: &Path, custom_workdir: bool) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(Error::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                    reason: "agent workdir cannot be a symlink".to_string(),
                });
            }
            if !metadata.is_dir() {
                return Err(Error::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                    reason: "agent workdir path exists but is not a directory".to_string(),
                });
            }
            if custom_workdir && fs::read_dir(path)?.next().is_some() {
                return Err(Error::InvalidInput(format!(
                    "custom agent workdir `{}` must be empty or absent",
                    path.display()
                )));
            }
            fs::remove_dir_all(path)?;
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(Error::Io(err)),
    }
    fs::create_dir_all(path)?;
    Ok(())
}

fn prepare_checkout_workdir(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(Error::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                    reason: "checkout workdir cannot be a symlink".to_string(),
                });
            }
            if !metadata.is_dir() {
                return Err(Error::InvalidPath {
                    path: path.to_string_lossy().to_string(),
                    reason: "checkout workdir path exists but is not a directory".to_string(),
                });
            }
            if fs::read_dir(path)?.next().is_some() {
                return Err(Error::InvalidInput(format!(
                    "checkout workdir `{}` must be empty or absent",
                    path.display()
                )));
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(Error::Io(err)),
    }
    fs::create_dir_all(path)?;
    Ok(())
}

fn normalize_record_paths(paths: &[String]) -> Result<Vec<String>> {
    let mut normalized = BTreeSet::new();
    for path in paths {
        normalized.insert(normalize_relative_path(path)?);
    }
    Ok(normalized.into_iter().collect())
}

fn path_matches_selection(path: &str, selected: &str) -> bool {
    path == selected
        || path
            .strip_prefix(selected)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn validate_ref_segment(name: &str) -> Result<()> {
    if name.is_empty()
        || name.contains("..")
        || name.starts_with('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err(Error::InvalidInput(format!("invalid ref segment `{name}`")));
    }
    Ok(())
}

fn path_from_rel(path: &str) -> PathBuf {
    path.split('/').collect()
}

fn is_internal_path(path: &str) -> bool {
    path.split('/')
        .any(|part| part == ".crabdb" || part == ".git")
}

fn is_default_ignored(path: &str) -> bool {
    let components = path.split('/').collect::<Vec<_>>();
    if components.iter().any(|part| {
        matches!(
            *part,
            ".crabdb" | ".git" | "node_modules" | "target" | "dist" | "build" | "coverage"
        )
    }) {
        return true;
    }
    let file_name = components.last().copied().unwrap_or_default();
    file_name == ".crabignore"
        || file_name == ".env"
        || file_name.starts_with(".env.")
        || file_name.ends_with(".pem")
        || file_name.ends_with(".key")
        || file_name.ends_with(".p12")
        || file_name.ends_with(".pfx")
        || file_name == "id_rsa"
        || file_name == "id_ed25519"
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|byte| *byte == 0)
}

fn classify_file_kind(bytes: &[u8], text_config: &TextConfig) -> FileKind {
    if looks_binary(bytes) {
        FileKind::Binary
    } else if std::str::from_utf8(bytes).is_err()
        || bytes.len() as u64 > text_config.opaque_text_max_bytes
        || max_line_len(bytes) as u64 > text_config.max_line_bytes
    {
        FileKind::OpaqueText
    } else {
        FileKind::Text
    }
}

fn max_line_len(bytes: &[u8]) -> usize {
    bytes
        .split(|byte| *byte == b'\n')
        .map(|line| line.len())
        .max()
        .unwrap_or(0)
}

#[derive(Clone)]
struct SplitLine {
    text: Vec<u8>,
    newline: NewlineKind,
}

fn split_lines(bytes: &[u8]) -> Vec<SplitLine> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = 0;
    for (idx, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            if idx > start && bytes[idx - 1] == b'\r' {
                out.push(SplitLine {
                    text: bytes[start..idx - 1].to_vec(),
                    newline: NewlineKind::Crlf,
                });
            } else {
                out.push(SplitLine {
                    text: bytes[start..idx].to_vec(),
                    newline: NewlineKind::Lf,
                });
            }
            start = idx + 1;
        }
    }
    if start < bytes.len() {
        out.push(SplitLine {
            text: bytes[start..].to_vec(),
            newline: NewlineKind::None,
        });
    }
    out
}

fn materialize_lines(lines: &[LineEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    for line in lines {
        out.extend_from_slice(&line.text);
        match line.newline {
            NewlineKind::None => {}
            NewlineKind::Lf => out.push(b'\n'),
            NewlineKind::Crlf => out.extend_from_slice(b"\r\n"),
        }
    }
    out
}

fn line_map_by_id(lines: &[LineEntry]) -> HashMap<String, &LineEntry> {
    lines
        .iter()
        .map(|line| (line.line_id_key(), line))
        .collect()
}

fn line_content_equal(left: Option<&LineEntry>, right: Option<&LineEntry>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            left.text_hash == right.text_hash
                && left.newline == right.newline
                && left.text == right.text
        }
        (None, None) => true,
        _ => false,
    }
}

fn preserves_base_line_order(base_order: &[String], lines: &[LineEntry]) -> bool {
    let positions = base_order
        .iter()
        .enumerate()
        .map(|(idx, line_id)| (line_id.as_str(), idx))
        .collect::<HashMap<_, _>>();
    let mut last = None;
    for line in lines {
        let line_id = line.line_id_key();
        let Some(position) = positions.get(line_id.as_str()).copied() else {
            continue;
        };
        if last.is_some_and(|last| position < last) {
            return false;
        }
        last = Some(position);
    }
    true
}

fn inserted_line_gaps(lines: &[LineEntry], base_keys: &HashSet<String>) -> BTreeSet<LineGap> {
    inserted_line_groups(lines, base_keys)
        .into_iter()
        .map(|(gap, _)| gap)
        .collect()
}

fn inserted_line_groups(
    lines: &[LineEntry],
    base_keys: &HashSet<String>,
) -> Vec<(LineGap, Vec<LineEntry>)> {
    let mut groups: Vec<(LineGap, Vec<LineEntry>)> = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let line_id = line.line_id_key();
        if base_keys.contains(&line_id) {
            continue;
        }
        let gap = line_gap_at(lines, idx, base_keys);
        if let Some((last_gap, last_lines)) = groups.last_mut() {
            if *last_gap == gap {
                last_lines.push(line.clone());
                continue;
            }
        }
        groups.push((gap, vec![line.clone()]));
    }
    groups
}

fn line_gap_at(lines: &[LineEntry], idx: usize, base_keys: &HashSet<String>) -> LineGap {
    let previous = lines[..idx]
        .iter()
        .rev()
        .map(LineEntryExt::line_id_key)
        .find(|line_id| base_keys.contains(line_id));
    let next = lines[idx + 1..]
        .iter()
        .map(LineEntryExt::line_id_key)
        .find(|line_id| base_keys.contains(line_id));
    LineGap { previous, next }
}

fn replace_or_insert_line(lines: &mut Vec<LineEntry>, line_id: &str, replacement: LineEntry) {
    if let Some(line) = lines
        .iter_mut()
        .find(|line| line.line_id_key().as_str() == line_id)
    {
        *line = replacement;
    } else {
        lines.push(replacement);
    }
}

fn remove_line(lines: &mut Vec<LineEntry>, line_id: &str) {
    if let Some(idx) = lines
        .iter()
        .position(|line| line.line_id_key().as_str() == line_id)
    {
        lines.remove(idx);
    }
}

fn insert_lines_at_gap(lines: &mut Vec<LineEntry>, gap: &LineGap, inserted: Vec<LineEntry>) {
    let mut idx = if let Some(next) = &gap.next {
        lines
            .iter()
            .position(|line| line.line_id_key() == *next)
            .unwrap_or(lines.len())
    } else if let Some(previous) = &gap.previous {
        lines
            .iter()
            .position(|line| line.line_id_key() == *previous)
            .map(|idx| idx + 1)
            .unwrap_or(lines.len())
    } else {
        lines.len()
    };
    for line in inserted {
        lines.insert(idx, line);
        idx += 1;
    }
}

fn order_key(line_number: u64) -> Vec<u8> {
    (line_number * ORDER_KEY_STEP).to_be_bytes().to_vec()
}

fn line_similarity(left: &[u8], right: &[u8]) -> f32 {
    if left == right {
        return 1.0;
    }
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let max = left.len().max(right.len()) as f32;
    let common = left
        .iter()
        .zip(right)
        .filter(|(left, right)| left == right)
        .count() as f32;
    common / max
}

fn count_line_delta(changes: &[LineChange]) -> (u64, u64) {
    let mut additions = 0;
    let mut deletions = 0;
    for change in changes {
        match change.kind {
            LineChangeKind::Added => additions += 1,
            LineChangeKind::Deleted => deletions += 1,
            LineChangeKind::Modified => {
                additions += 1;
                deletions += 1;
            }
            LineChangeKind::Moved => {}
        }
    }
    (additions, deletions)
}

fn summarize_file_changes(changes: &[FileChange]) -> Vec<FileDiffSummary> {
    changes
        .iter()
        .map(|change| {
            let (additions, deletions) = count_line_delta(&change.line_changes);
            FileDiffSummary {
                path: change.path.clone(),
                old_path: change.old_path.clone(),
                kind: change.kind.clone(),
                before_hash: change.before_hash.clone(),
                after_hash: change.after_hash.clone(),
                additions,
                deletions,
                line_changes: Vec::new(),
                patch: None,
            }
        })
        .collect()
}

fn attach_line_changes(changes: &[FileChange], summaries: &mut [FileDiffSummary]) {
    for summary in summaries {
        summary.line_changes = changes
            .iter()
            .find(|change| {
                change.path == summary.path
                    && change.old_path == summary.old_path
                    && change.kind == summary.kind
            })
            .map(|change| change.line_changes.clone())
            .unwrap_or_default();
    }
}

fn worktree_state_from_changes(changed_paths: &[FileDiffSummary]) -> WorktreeState {
    if changed_paths.is_empty() {
        WorktreeState::Clean
    } else if changed_paths
        .iter()
        .any(|summary| summary.kind == FileChangeKind::Added)
    {
        WorktreeState::DirtyUntracked
    } else {
        WorktreeState::DirtyTracked
    }
}

fn readiness_issue(
    code: impl Into<String>,
    message: impl Into<String>,
    details: Option<serde_json::Value>,
) -> AgentReadinessIssue {
    AgentReadinessIssue {
        code: code.into(),
        message: message.into(),
        details,
    }
}

fn guardrail_reason(
    code: impl Into<String>,
    severity: impl Into<String>,
    message: impl Into<String>,
    details: Option<serde_json::Value>,
) -> GuardrailReason {
    GuardrailReason {
        code: code.into(),
        severity: severity.into(),
        message: message.into(),
        details,
    }
}

fn guardrail_risk_text(
    action: &str,
    summary: Option<&str>,
    payload: Option<&serde_json::Value>,
) -> String {
    let mut text = action.to_ascii_lowercase();
    if let Some(summary) = summary {
        text.push(' ');
        text.push_str(&summary.to_ascii_lowercase());
    }
    if let Some(payload) = payload {
        text.push(' ');
        text.push_str(&payload.to_string().to_ascii_lowercase());
    }
    text
}

fn classify_guardrail_action(text: &str) -> Vec<GuardrailReason> {
    let mut reasons = Vec::new();
    if contains_any(
        text,
        &[
            "rm -rf /", "rm -rf ~", "mkfs", "dd if=", "shutdown", "reboot", ":(){",
        ],
    ) {
        reasons.push(guardrail_reason(
            "dangerous_command",
            "blocked",
            "action resembles a destructive host-level command",
            None,
        ));
    }
    if contains_any(
        text,
        &[
            "shell",
            "exec",
            "terminal",
            "command",
            "process",
            "subprocess",
        ],
    ) {
        reasons.push(guardrail_reason(
            "shell_action",
            "approval_required",
            "shell or process execution requires human approval",
            None,
        ));
    }
    if contains_any(
        text,
        &[
            "curl", "wget", "http://", "https://", "ssh", "scp", "rsync", "network", "external",
        ],
    ) {
        reasons.push(guardrail_reason(
            "network_action",
            "approval_required",
            "network or external-system access requires human approval",
            None,
        ));
    }
    if contains_any(
        text,
        &["deploy", "release", "publish", "production", "preview"],
    ) {
        reasons.push(guardrail_reason(
            "release_action",
            "approval_required",
            "deployment, release, or publishing actions require human approval",
            None,
        ));
    }
    if contains_any(
        text,
        &[
            "delete",
            "remove",
            "overwrite",
            "force",
            "reset",
            "clean",
            "truncate",
            "chmod",
            "chown",
        ],
    ) {
        reasons.push(guardrail_reason(
            "destructive_action",
            "approval_required",
            "destructive or forceful workspace changes require human approval",
            None,
        ));
    }
    if contains_any(text, &["ignore_add", "ignore_remove", ".crabignore"]) {
        reasons.push(guardrail_reason(
            "policy_change",
            "approval_required",
            "ignore or guardrail policy changes require human approval",
            None,
        ));
    }
    reasons
}

fn apply_configured_guardrail_policy(
    reasons: &mut Vec<GuardrailReason>,
    policy: &str,
    action: &str,
    risk_text: &str,
    path_checks: &[IgnoreCheckReport],
) -> Result<()> {
    let rules = parse_guardrail_policy(policy)?;
    let mut allow_matches = Vec::new();
    let mut approval_matches = Vec::new();
    let mut block_matches = Vec::new();
    for rule in rules {
        if !guardrail_policy_rule_matches(&rule, action, risk_text, path_checks) {
            continue;
        }
        match rule.decision.as_str() {
            "allow" => allow_matches.push(rule),
            "approval" => approval_matches.push(rule),
            "block" => block_matches.push(rule),
            _ => {}
        }
    }

    if !block_matches.is_empty() {
        reasons.push(guardrail_reason(
            "policy_block",
            "blocked",
            "workspace guardrail policy blocks this action",
            Some(serde_json::json!({ "rules": guardrail_policy_rule_details(&block_matches) })),
        ));
    }
    if !approval_matches.is_empty() {
        reasons.push(guardrail_reason(
            "policy_approval",
            "approval_required",
            "workspace guardrail policy requires approval for this action",
            Some(serde_json::json!({ "rules": guardrail_policy_rule_details(&approval_matches) })),
        ));
    }
    if !allow_matches.is_empty() && block_matches.is_empty() && approval_matches.is_empty() {
        reasons.retain(|reason| reason.severity != "approval_required");
        reasons.push(guardrail_reason(
            "policy_allow",
            "info",
            "workspace guardrail policy allows this action",
            Some(serde_json::json!({ "rules": guardrail_policy_rule_details(&allow_matches) })),
        ));
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct GuardrailPolicyRule {
    decision: String,
    scope: String,
    pattern: String,
}

fn parse_guardrail_policy(policy: &str) -> Result<Vec<GuardrailPolicyRule>> {
    let mut rules = Vec::new();
    for raw_rule in policy.split([';', '\n']) {
        let raw_rule = raw_rule.trim();
        if raw_rule.is_empty() {
            continue;
        }
        let parts = raw_rule.splitn(3, ':').collect::<Vec<_>>();
        if parts.len() != 3 {
            return Err(Error::InvalidInput(format!(
                "guardrails.policy rule `{raw_rule}` must be decision:scope:pattern"
            )));
        }
        let decision = parts[0].trim().to_ascii_lowercase();
        let scope = parts[1].trim().to_ascii_lowercase();
        let pattern = parts[2].trim().to_ascii_lowercase();
        if !matches!(decision.as_str(), "allow" | "approval" | "block") {
            return Err(Error::InvalidInput(format!(
                "guardrails.policy decision must be allow, approval, or block, got `{}`",
                parts[0].trim()
            )));
        }
        if !matches!(scope.as_str(), "action" | "keyword" | "path") {
            return Err(Error::InvalidInput(format!(
                "guardrails.policy scope must be action, keyword, or path, got `{}`",
                parts[1].trim()
            )));
        }
        if pattern.is_empty() {
            return Err(Error::InvalidInput(
                "guardrails.policy pattern cannot be empty".to_string(),
            ));
        }
        rules.push(GuardrailPolicyRule {
            decision,
            scope,
            pattern,
        });
    }
    Ok(rules)
}

fn guardrail_policy_rule_matches(
    rule: &GuardrailPolicyRule,
    action: &str,
    risk_text: &str,
    path_checks: &[IgnoreCheckReport],
) -> bool {
    match rule.scope.as_str() {
        "action" => action.to_ascii_lowercase().contains(&rule.pattern),
        "keyword" => risk_text.contains(&rule.pattern),
        "path" => path_checks
            .iter()
            .any(|check| check.path.to_ascii_lowercase().contains(&rule.pattern)),
        _ => false,
    }
}

fn guardrail_policy_rule_details(rules: &[GuardrailPolicyRule]) -> Vec<serde_json::Value> {
    rules
        .iter()
        .map(|rule| {
            serde_json::json!({
                "decision": rule.decision,
                "scope": rule.scope,
                "pattern": rule.pattern
            })
        })
        .collect()
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn handoff_next_steps(
    readiness: &AgentReadinessReport,
    current_session: Option<&AgentSessionDetails>,
) -> Vec<String> {
    let mut steps = Vec::new();
    for blocker in &readiness.blockers {
        match blocker.code.as_str() {
            "agent_removed" => steps
                .push("Restore or respawn the agent branch before continuing the handoff.".into()),
            "dirty_workdir" => steps.push(
                "Record or force-sync the materialized workdir before reviewing or merging.".into(),
            ),
            "pending_approvals" => {
                steps.push("Resolve pending human approvals before merge.".into())
            }
            "open_conflicts" => steps.push("Resolve open conflict sets before merge.".into()),
            "latest_test_failed" => steps.push("Fix and rerun the latest test gate.".into()),
            "latest_eval_failed" => steps.push("Fix and rerun the latest eval gate.".into()),
            "missing_required_test_suite" => {
                steps.push("Run the required named test suite before merge.".into())
            }
            "missing_required_eval_suite" => {
                steps.push("Run the required named eval suite before merge.".into())
            }
            "required_test_suite_failed" => {
                steps.push("Fix and rerun the failed required test suite.".into())
            }
            "required_eval_suite_failed" => {
                steps.push("Fix and rerun the failed required eval suite.".into())
            }
            _ => steps.push(blocker.message.clone()),
        }
    }

    if steps.is_empty() {
        steps.push("Review changed paths, recent operations, and provenance before merge.".into());
    }

    for warning in &readiness.warnings {
        match warning.code.as_str() {
            "missing_latest_test" => {
                steps.push("Run a test gate if this branch should be merged.".into())
            }
            "missing_latest_eval" => {
                steps.push("Run an eval gate when model or policy quality matters.".into())
            }
            "no_changed_paths" => steps
                .push("Confirm this is an audit-only handoff or record the intended work.".into()),
            "queued_merge" => steps.push(
                "Inspect the existing queued or running merge before queuing another.".into(),
            ),
            _ => steps.push(warning.message.clone()),
        }
    }

    match current_session {
        Some(details) if details.session.status == "active" => steps.push(format!(
            "Continue or close active session `{}` after the receiving agent catches up.",
            details.session.session_id
        )),
        Some(details) => steps.push(format!(
            "Use session `{}` as historical context for this handoff.",
            details.session.session_id
        )),
        None => steps
            .push("Start a new session or turn if the receiving agent will continue work.".into()),
    }

    steps.dedup();
    steps
}

fn doctor_check(
    name: impl Into<String>,
    status: impl Into<String>,
    message: impl Into<String>,
    details: Option<serde_json::Value>,
) -> DoctorCheck {
    DoctorCheck {
        name: name.into(),
        status: status.into(),
        message: message.into(),
        details,
    }
}

fn doctor_report(checks: Vec<DoctorCheck>) -> DoctorReport {
    let status = if checks.iter().any(|check| check.status == "error") {
        "error"
    } else if checks.iter().any(|check| check.status == "warning") {
        "warning"
    } else {
        "ok"
    };
    DoctorReport {
        status: status.to_string(),
        checks,
    }
}

fn normalize_query_limit(limit: usize, max: usize) -> Result<usize> {
    if limit == 0 {
        return Err(Error::InvalidInput(
            "limit must be greater than 0".to_string(),
        ));
    }
    Ok(limit.min(max))
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn backup_manifest_path(path: &Path) -> PathBuf {
    path.join("manifest.json")
}

fn backup_sqlite_path(path: &Path) -> PathBuf {
    path.join(DB_RELATIVE_PATH)
}

fn read_backup_manifest(path: &Path) -> Result<BackupManifest> {
    let bytes = fs::read(backup_manifest_path(path))?;
    serde_json::from_slice(&bytes).map_err(Error::from)
}

fn file_digest(path: &Path) -> Result<(u64, String)> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut bytes = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok((bytes, hex::encode(hasher.finalize())))
}

fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<u64> {
    if !source.exists() {
        return Ok(0);
    }
    fs::create_dir_all(destination)?;
    let mut bytes = 0;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.file_type().is_symlink() {
            #[cfg(unix)]
            {
                let target = fs::read_link(&source_path)?;
                symlink_file(target, destination_path)?;
            }
            #[cfg(not(unix))]
            {
                return Err(Error::InvalidInput(format!(
                    "cannot copy symlink `{}` on this platform",
                    source_path.display()
                )));
            }
        } else if metadata.is_dir() {
            bytes += copy_dir_recursive(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            bytes += fs::copy(&source_path, &destination_path)?;
        }
    }
    Ok(bytes)
}

fn materialize_into<F>(
    workspace_root: &Path,
    output_root: &Path,
    previous: &BTreeMap<String, FileEntry>,
    target: &BTreeMap<String, FileEntry>,
    bytes_for: F,
) -> Result<()>
where
    F: Fn(&FileEntry) -> Result<Vec<u8>>,
{
    reject_case_insensitive_collisions(output_root, target)?;
    for path in previous.keys() {
        if !target.contains_key(path) {
            let abs = safe_join(output_root, path)?;
            if abs.exists() {
                fs::remove_file(abs)?;
            }
        }
    }
    for (path, entry) in target {
        let abs = safe_join(output_root, path)?;
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = bytes_for(entry)?;
        write_materialized_file(&abs, path, &bytes, entry.executable)?;
    }
    let _ = workspace_root;
    Ok(())
}

fn reject_case_insensitive_collisions(
    output_root: &Path,
    target: &BTreeMap<String, FileEntry>,
) -> Result<()> {
    if !is_case_insensitive_filesystem(output_root)? {
        return Ok(());
    }
    validate_no_case_fold_collisions(target.keys())
}

fn validate_no_case_fold_collisions<'a, I>(paths: I) -> Result<()>
where
    I: IntoIterator<Item = &'a String>,
{
    let mut seen = HashMap::new();
    for path in paths {
        let folded = path.to_lowercase();
        if let Some(previous) = seen.insert(folded, path.clone()) {
            if previous != *path {
                return Err(Error::InvalidPath {
                    path: path.clone(),
                    reason: format!("case-insensitive path collision with `{previous}`"),
                });
            }
        }
    }
    Ok(())
}

fn is_case_insensitive_filesystem(root: &Path) -> Result<bool> {
    let root = root.canonicalize()?;
    for _ in 0..16 {
        let lower_name = format!(".crabdb-case-probe-{}-a", now_nanos());
        let upper_name = lower_name.to_ascii_uppercase();
        let lower = root.join(&lower_name);
        let upper = root.join(&upper_name);
        match OpenOptions::new().write(true).create_new(true).open(&lower) {
            Ok(_) => {
                let insensitive = upper.exists();
                let _ = fs::remove_file(&lower);
                return Ok(insensitive);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(Error::from(err)),
        }
    }
    Err(Error::InvalidInput(
        "could not create filesystem case-sensitivity probe".to_string(),
    ))
}

fn safe_join(root: &Path, rel: &str) -> Result<PathBuf> {
    let normalized = normalize_relative_path(rel)?;
    let root_canon = root.canonicalize()?;
    let candidate = root_canon.join(path_from_rel(&normalized));
    ensure_no_symlink_ancestors(&root_canon, &normalized, rel)?;
    if let Some(parent) = candidate.parent() {
        let parent = if parent.exists() {
            parent.canonicalize()?
        } else {
            let mut existing = parent;
            while !existing.exists() {
                existing = existing.parent().ok_or_else(|| Error::InvalidPath {
                    path: rel.to_string(),
                    reason: "path has no existing ancestor".to_string(),
                })?;
            }
            existing.canonicalize()?
        };
        if !parent.starts_with(root_canon) {
            return Err(Error::InvalidPath {
                path: rel.to_string(),
                reason: "path escapes output root".to_string(),
            });
        }
    }
    Ok(candidate)
}

fn ensure_no_symlink_ancestors(root: &Path, normalized: &str, rel: &str) -> Result<()> {
    let mut current = root.to_path_buf();
    let rel_path = path_from_rel(normalized);
    let Some(parent) = rel_path.parent() else {
        return Ok(());
    };
    for component in parent.components() {
        let Component::Normal(part) = component else {
            continue;
        };
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(Error::InvalidPath {
                        path: rel.to_string(),
                        reason: "path uses a symlink ancestor".to_string(),
                    });
                }
                if !metadata.is_dir() {
                    return Err(Error::InvalidPath {
                        path: rel.to_string(),
                        reason: "path parent is not a directory".to_string(),
                    });
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => break,
            Err(err) => return Err(Error::from(err)),
        }
    }
    Ok(())
}

fn write_materialized_file(path: &Path, rel: &str, bytes: &[u8], executable: bool) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            return Err(Error::InvalidPath {
                path: rel.to_string(),
                reason: "refusing to follow symlink for write".to_string(),
            });
        }
        if !metadata.is_file() {
            return Err(Error::InvalidPath {
                path: rel.to_string(),
                reason: "path is not a regular file".to_string(),
            });
        }
    }
    let parent = path.parent().ok_or_else(|| Error::InvalidPath {
        path: rel.to_string(),
        reason: "path has no parent directory".to_string(),
    })?;
    fs::create_dir_all(parent)?;

    let (tmp, mut file) = create_materialize_temp_file(parent, path)?;
    let result = (|| -> Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        set_executable(&tmp, executable)?;
        fs::rename(&tmp, path)?;
        sync_directory(parent);
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn create_materialize_temp_file(parent: &Path, path: &Path) -> Result<(PathBuf, fs::File)> {
    let leaf = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "file".into());
    for _ in 0..16 {
        let tmp = parent.join(format!(".{leaf}.crabdb-tmp-{}", now_nanos()));
        match OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(file) => return Ok((tmp, file)),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(Error::from(err)),
        }
    }
    Err(Error::InvalidInput(
        "could not create materialization temp file".to_string(),
    ))
}

fn sync_directory(path: &Path) {
    if let Ok(dir) = OpenOptions::new().read(true).open(path) {
        let _ = dir.sync_all();
    }
}

fn merge_files_with_resolution(
    base: &BTreeMap<String, FileEntry>,
    target: &BTreeMap<String, FileEntry>,
    source: &BTreeMap<String, FileEntry>,
    conflict_paths: &BTreeSet<String>,
    take: ConflictTake,
) -> Result<BTreeMap<String, FileEntry>> {
    let mut merged = target.clone();
    let mut unresolved = Vec::new();
    let mut paths = BTreeSet::new();
    paths.extend(base.keys().cloned());
    paths.extend(target.keys().cloned());
    paths.extend(source.keys().cloned());
    for path in paths {
        let base_entry = base.get(&path);
        let target_entry = target.get(&path);
        let source_entry = source.get(&path);
        let target_changed = entry_hash(base_entry) != entry_hash(target_entry);
        let source_changed = entry_hash(base_entry) != entry_hash(source_entry);
        match (target_changed, source_changed) {
            (false, true) => match source_entry {
                Some(entry) => {
                    merged.insert(path.clone(), entry.clone());
                }
                None => {
                    merged.remove(&path);
                }
            },
            (true, true) => {
                if entry_hash(target_entry) != entry_hash(source_entry) {
                    if !conflict_paths.contains(&path) {
                        unresolved.push(format!("conflict path `{path}` was not recorded"));
                        continue;
                    }
                    if take == ConflictTake::Source {
                        match source_entry {
                            Some(entry) => {
                                merged.insert(path.clone(), entry.clone());
                            }
                            None => {
                                merged.remove(&path);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if !unresolved.is_empty() {
        return Err(Error::Conflict(unresolved.join("; ")));
    }
    Ok(merged)
}

fn entry_hash(entry: Option<&FileEntry>) -> Option<(&str, bool, &FileKind)> {
    entry.map(|entry| (entry.content_hash.as_str(), entry.executable, &entry.kind))
}

fn unified_patch(old_path: &str, new_path: &str, old_text: &str, new_text: &str) -> String {
    let diff = TextDiff::from_lines(old_text, new_text);
    let mut out = String::new();
    out.push_str(&format!("diff --crabdb a/{old_path} b/{new_path}\n"));
    out.push_str(&format!("--- a/{old_path}\n"));
    out.push_str(&format!("+++ b/{new_path}\n"));
    for group in diff.grouped_ops(3) {
        for op in group {
            for change in diff.iter_changes(&op) {
                let sign = match change.tag() {
                    ChangeTag::Delete => "-",
                    ChangeTag::Insert => "+",
                    ChangeTag::Equal => " ",
                };
                out.push_str(sign);
                out.push_str(change.value());
                if !change.value().ends_with('\n') {
                    out.push('\n');
                }
            }
        }
    }
    out
}

fn parse_range(spec: &str) -> Result<(&str, &str)> {
    let Some((left, right)) = spec.split_once("..") else {
        return Err(Error::InvalidInput(format!(
            "range `{spec}` must look like left..right"
        )));
    };
    if left.is_empty() || right.is_empty() {
        return Err(Error::InvalidInput(format!(
            "range `{spec}` must include both endpoints"
        )));
    }
    Ok((left, right))
}

fn parse_path_line(spec: &str) -> Result<(String, u64)> {
    let Some((path, line)) = spec.rsplit_once(':') else {
        return Err(Error::InvalidInput(format!(
            "`{spec}` must look like path:line"
        )));
    };
    let line_number = line
        .parse::<u64>()
        .map_err(|_| Error::InvalidInput(format!("invalid line number `{line}`")))?;
    if line_number == 0 {
        return Err(Error::InvalidInput("line numbers are 1-based".to_string()));
    }
    Ok((normalize_relative_path(path)?, line_number))
}

fn branch_ref(branch: &str) -> String {
    if branch.starts_with("refs/") {
        branch.to_string()
    } else {
        format!("{MAIN_REF_PREFIX}{branch}")
    }
}

fn agent_ref(agent: &str) -> String {
    if agent.starts_with("refs/") {
        agent.to_string()
    } else {
        format!("{AGENT_REF_PREFIX}{agent}")
    }
}

fn content_object_id(content: &FileContentRef) -> &ObjectId {
    match content {
        FileContentRef::Text(object_id)
        | FileContentRef::Opaque(object_id)
        | FileContentRef::Binary(object_id) => object_id,
    }
}

fn file_id_key(file_id: &FileId) -> String {
    format!("{}:{}", file_id.origin_change.0, file_id.local_seq)
}

fn line_id_key_value(line_id: &LineId) -> String {
    format!("{}:{}", line_id.origin_change.0, line_id.local_seq)
}

fn parse_line_id_key(value: &str) -> Result<LineId> {
    let (change_id, local_seq) = value.rsplit_once(':').ok_or_else(|| {
        Error::InvalidInput("line id must look like `ch_...:<local_seq>`".to_string())
    })?;
    if !change_id.starts_with("ch_") {
        return Err(Error::InvalidInput(format!(
            "line id change id must start with `ch_`, got `{change_id}`"
        )));
    }
    let local_seq = local_seq.parse::<u64>().map_err(|_| {
        Error::InvalidInput(format!("invalid line id local sequence `{local_seq}`"))
    })?;
    Ok(LineId::new(ChangeId(change_id.to_string()), local_seq))
}

trait LineChangeExt {
    fn line_id_key(&self) -> String;
}

impl LineChangeExt for LineChange {
    fn line_id_key(&self) -> String {
        line_id_key_value(&self.line_id)
    }
}

trait LineEntryExt {
    fn line_id_key(&self) -> String;
}

impl LineEntryExt for LineEntry {
    fn line_id_key(&self) -> String {
        line_id_key_value(&self.line_id)
    }
}

fn parse_line_change_kind(value: &str) -> LineChangeKind {
    match value {
        "Added" => LineChangeKind::Added,
        "Deleted" => LineChangeKind::Deleted,
        "Moved" => LineChangeKind::Moved,
        _ => LineChangeKind::Modified,
    }
}

fn parse_conflict_take(value: &str) -> Result<ConflictTake> {
    match value {
        "source" => Ok(ConflictTake::Source),
        "target" => Ok(ConflictTake::Target),
        other => Err(Error::InvalidInput(format!(
            "conflict resolution must take `source` or `target`, got `{other}`"
        ))),
    }
}

#[derive(Debug)]
enum ManualConflictPayload {
    Text { content: String, executable: bool },
    Delete,
}

fn normalize_manual_conflict_files(
    manual: ConflictManualResolution,
    conflict_paths: &BTreeSet<String>,
) -> Result<BTreeMap<String, ConflictManualFile>> {
    if manual.files.is_empty() {
        return Err(Error::InvalidInput(
            "manual conflict resolution must include at least one file".to_string(),
        ));
    }

    let mut normalized = BTreeMap::new();
    for (path, file) in manual.files {
        let normalized_path = normalize_relative_path(&path)?;
        if normalized.insert(normalized_path.clone(), file).is_some() {
            return Err(Error::InvalidInput(format!(
                "manual conflict resolution includes duplicate path `{normalized_path}`"
            )));
        }
    }

    let provided = normalized.keys().cloned().collect::<BTreeSet<_>>();
    let missing = conflict_paths
        .difference(&provided)
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(Error::InvalidInput(format!(
            "manual conflict resolution is missing conflicted path(s): {}",
            missing.join(", ")
        )));
    }

    let extra = provided
        .difference(conflict_paths)
        .cloned()
        .collect::<Vec<_>>();
    if !extra.is_empty() {
        return Err(Error::InvalidInput(format!(
            "manual conflict resolution includes non-conflicted path(s): {}",
            extra.join(", ")
        )));
    }

    Ok(normalized)
}

fn manual_conflict_file_payload(
    file: ConflictManualFile,
    default_executable: bool,
) -> Result<ManualConflictPayload> {
    match file {
        ConflictManualFile::Text(content) => Ok(ManualConflictPayload::Text {
            content,
            executable: default_executable,
        }),
        ConflictManualFile::Spec(spec) if spec.delete => {
            if spec.content.is_some() {
                return Err(Error::InvalidInput(
                    "manual conflict file cannot set both `delete` and `content`".to_string(),
                ));
            }
            Ok(ManualConflictPayload::Delete)
        }
        ConflictManualFile::Spec(spec) => {
            let Some(content) = spec.content else {
                return Err(Error::InvalidInput(
                    "manual conflict file must include `content` or set `delete` to true"
                        .to_string(),
                ));
            };
            Ok(ManualConflictPayload::Text {
                content,
                executable: spec.executable.unwrap_or(default_executable),
            })
        }
    }
}

fn parse_lease_mode(value: &str) -> Result<&'static str> {
    match value {
        "read" => Ok("read"),
        "write" => Ok("write"),
        other => Err(Error::InvalidInput(format!(
            "lease mode must be `read` or `write`, got `{other}`"
        ))),
    }
}

fn parse_session_end_status(value: &str) -> Result<&'static str> {
    match value {
        "completed" => Ok("completed"),
        "failed" => Ok("failed"),
        "cancelled" => Ok("cancelled"),
        "archived" => Ok("archived"),
        other => Err(Error::InvalidInput(format!(
            "session end status must be completed, failed, cancelled, or archived, got `{other}`"
        ))),
    }
}

fn parse_approval_status_filter(value: &str) -> Result<Option<&'static str>> {
    match value {
        "all" => Ok(None),
        "pending" => Ok(Some("pending")),
        "approved" => Ok(Some("approved")),
        "rejected" => Ok(Some("rejected")),
        "cancelled" => Ok(Some("cancelled")),
        other => Err(Error::InvalidInput(format!(
            "approval status must be pending, approved, rejected, cancelled, or all, got `{other}`"
        ))),
    }
}

fn parse_agent_run_status_filter(value: &str) -> Result<Option<&'static str>> {
    match value {
        "all" => Ok(None),
        "paused" => Ok(Some("paused")),
        "resumed" => Ok(Some("resumed")),
        "blocked" => Ok(Some("blocked")),
        "cancelled" | "canceled" => Ok(Some("cancelled")),
        other => Err(Error::InvalidInput(format!(
            "agent run status must be paused, resumed, blocked, cancelled, or all, got `{other}`"
        ))),
    }
}

fn parse_approval_decision(value: &str) -> Result<&'static str> {
    match value {
        "approved" | "approve" => Ok("approved"),
        "rejected" | "reject" => Ok("rejected"),
        "cancelled" | "cancel" => Ok("cancelled"),
        other => Err(Error::InvalidInput(format!(
            "approval decision must be approved, rejected, or cancelled, got `{other}`"
        ))),
    }
}

fn validate_session_id(session_id: &str) -> Result<()> {
    if session_id.trim().is_empty() {
        return Err(Error::InvalidInput(
            "session id cannot be empty".to_string(),
        ));
    }
    if !session_id.starts_with("session_") && !session_id.starts_with("session-") {
        return Err(Error::InvalidInput(format!(
            "session id `{session_id}` must start with `session_` or `session-`"
        )));
    }
    if !session_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(Error::InvalidInput(format!(
            "session id `{session_id}` contains invalid characters"
        )));
    }
    Ok(())
}

fn conflict_paths_from_details(details: &[String]) -> Result<BTreeSet<String>> {
    let mut paths = BTreeSet::new();
    for detail in details {
        let mut parts = detail.split('`');
        let _before = parts.next();
        if let Some(path) = parts.next() {
            paths.insert(normalize_relative_path(path)?);
        }
    }
    if paths.is_empty() {
        return Err(Error::InvalidInput(
            "conflict set does not include path details that can be resolved automatically"
                .to_string(),
        ));
    }
    Ok(paths)
}

fn build_agent_trace_spans(events: Vec<AgentEventRecord>) -> Vec<AgentTraceSpan> {
    let mut builders: BTreeMap<String, AgentTraceSpanBuilder> = BTreeMap::new();

    for event in events {
        let Some(payload) = event.payload.as_ref() else {
            continue;
        };
        let Some(span_id) = payload_string(payload, "span_id") else {
            continue;
        };

        match event.event_type.as_str() {
            "span_started" => {
                let trace_id = payload_string(payload, "trace_id").unwrap_or_else(|| {
                    event
                        .turn_id
                        .as_deref()
                        .map(default_trace_id_for_turn)
                        .unwrap_or_else(|| default_trace_id_for_turn(&event.event_id))
                });
                let builder = AgentTraceSpanBuilder {
                    span_id: span_id.clone(),
                    trace_id,
                    agent_id: event.agent_id.clone(),
                    session_id: event.session_id.clone(),
                    turn_id: event.turn_id.clone(),
                    parent_span_id: payload_string(payload, "parent_span_id"),
                    span_type: payload_string(payload, "span_type")
                        .unwrap_or_else(|| "custom".to_string()),
                    name: payload_string(payload, "name").unwrap_or_else(|| span_id.clone()),
                    started_event_id: event.event_id.clone(),
                    started_at: event.created_at,
                    attributes: payload_value(payload, "attributes"),
                    ended_event_id: None,
                    ended_at: None,
                    status: None,
                    result: None,
                };
                builders.entry(span_id).or_insert(builder);
            }
            "span_ended" => {
                if let Some(builder) = builders.get_mut(&span_id) {
                    builder.ended_event_id = Some(event.event_id.clone());
                    builder.ended_at = Some(event.created_at);
                    builder.status = payload_string(payload, "status");
                    builder.result = payload_value(payload, "result");
                }
            }
            _ => {}
        }
    }

    builders
        .into_values()
        .map(agent_trace_span_from_builder)
        .collect()
}

fn agent_trace_span_from_builder(builder: AgentTraceSpanBuilder) -> AgentTraceSpan {
    let duration_ms = builder
        .ended_at
        .and_then(|ended_at| ended_at.checked_sub(builder.started_at))
        .map(|seconds| seconds as u64 * 1000);
    AgentTraceSpan {
        span_id: builder.span_id,
        trace_id: builder.trace_id,
        agent_id: builder.agent_id,
        session_id: builder.session_id,
        turn_id: builder.turn_id,
        parent_span_id: builder.parent_span_id,
        span_type: builder.span_type,
        name: builder.name,
        status: builder.status.unwrap_or_else(|| {
            if builder.ended_at.is_some() {
                "completed".to_string()
            } else {
                "running".to_string()
            }
        }),
        started_event_id: builder.started_event_id,
        ended_event_id: builder.ended_event_id,
        started_at: builder.started_at,
        ended_at: builder.ended_at,
        duration_ms,
        attributes: builder.attributes,
        result: builder.result,
    }
}

fn named_counts(counts: BTreeMap<String, u64>) -> Vec<NamedCount> {
    counts
        .into_iter()
        .map(|(name, count)| NamedCount { name, count })
        .collect()
}

fn tail_limited<T: Clone>(values: &[T], limit: usize) -> Vec<T> {
    let start = values.len().saturating_sub(limit);
    values[start..].to_vec()
}

fn agent_trace_status_is_failed(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "failed" | "error" | "errored" | "cancelled" | "canceled" | "timeout" | "timed_out"
    )
}

fn payload_string(payload: &serde_json::Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

fn payload_value(payload: &serde_json::Value, key: &str) -> Option<serde_json::Value> {
    payload.get(key).filter(|value| !value.is_null()).cloned()
}

fn default_trace_id_for_turn(turn_id: &str) -> String {
    format!("trace_{}", crate::ids::short_hash(turn_id.as_bytes(), 16))
}

fn parse_file_change_kind(value: &str) -> FileChangeKind {
    match value {
        "Added" => FileChangeKind::Added,
        "Deleted" => FileChangeKind::Deleted,
        "Renamed" => FileChangeKind::Renamed,
        "TypeChanged" => FileChangeKind::TypeChanged,
        _ => FileChangeKind::Modified,
    }
}

fn ref_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RefRecord> {
    Ok(RefRecord {
        name: row.get(0)?,
        change_id: ChangeId(row.get(1)?),
        root_id: ObjectId(row.get(2)?),
        operation_id: ObjectId(row.get(3)?),
        generation: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

fn file_history_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FileHistoryEntry> {
    Ok(FileHistoryEntry {
        file_id: row.get(0)?,
        change_id: ChangeId(row.get(1)?),
        path: row.get(2)?,
        old_path: row.get(3)?,
        kind: parse_file_change_kind(&row.get::<_, String>(4)?),
        before_hash: row.get(5)?,
        after_hash: row.get(6)?,
        created_at: row.get(7)?,
    })
}

fn line_history_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LineHistoryEntry> {
    Ok(LineHistoryEntry {
        change_id: ChangeId(row.get(0)?),
        path: row.get(1)?,
        line_number: row.get::<_, Option<i64>>(2)?.map(|n| n as u64),
        kind: parse_line_change_kind(&row.get::<_, String>(3)?),
        text_hash: row.get(4)?,
        created_at: row.get(5)?,
    })
}

fn agent_details_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentDetails> {
    Ok(AgentDetails {
        record: AgentRecord {
            agent_id: row.get(0)?,
            name: row.get(1)?,
            kind: row.get(2)?,
            provider: row.get(3)?,
            model: row.get(4)?,
            created_at: row.get(5)?,
            metadata_json: row.get(6)?,
        },
        branch: AgentBranch {
            agent_id: row.get(0)?,
            ref_name: row.get(7)?,
            base_change: ChangeId(row.get(8)?),
            head_change: ChangeId(row.get(9)?),
            base_root: ObjectId(row.get(10)?),
            head_root: ObjectId(row.get(11)?),
            session_id: row.get(12)?,
            workdir: row.get(13)?,
            status: row.get(14)?,
            created_at: row.get(15)?,
            updated_at: row.get(16)?,
        },
    })
}

fn merge_queue_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MergeQueueEntry> {
    Ok(MergeQueueEntry {
        queue_id: row.get(0)?,
        source_ref: row.get(1)?,
        target_ref: row.get(2)?,
        status: row.get(3)?,
        priority: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

fn conflict_set_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConflictSetSummary> {
    let details_json: Option<String> = row.get(5)?;
    let details = details_json
        .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
        .unwrap_or_default();
    Ok(ConflictSetSummary {
        conflict_set_id: row.get(0)?,
        merge_id: row.get(1)?,
        source_ref: row.get(2)?,
        target_ref: row.get(3)?,
        status: row.get(4)?,
        details,
        created_at: row.get(6)?,
    })
}

fn lease_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LeaseRecord> {
    Ok(LeaseRecord {
        lease_id: row.get(0)?,
        agent_id: row.get(1)?,
        ref_name: row.get(2)?,
        path: row.get(3)?,
        file_id: row.get(4)?,
        mode: row.get(5)?,
        expires_at: row.get(6)?,
        created_at: row.get(7)?,
    })
}

fn git_mapping_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<GitMapping> {
    Ok(GitMapping {
        mapping_id: row.get(0)?,
        direction: row.get(1)?,
        branch: row.get(2)?,
        git_head: row.get(3)?,
        git_dirty: row.get::<_, i64>(4)? != 0,
        crab_change: ChangeId(row.get(5)?),
        crab_root: ObjectId(row.get(6)?),
        created_at: row.get(7)?,
    })
}

fn agent_session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentSession> {
    Ok(AgentSession {
        session_id: row.get(0)?,
        agent_id: row.get(1)?,
        title: row.get(2)?,
        status: row.get(3)?,
        started_at: row.get(4)?,
        ended_at: row.get(5)?,
        metadata_json: row.get(6)?,
    })
}

fn agent_turn_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentTurn> {
    Ok(AgentTurn {
        turn_id: row.get(0)?,
        agent_id: row.get(1)?,
        session_id: row.get(2)?,
        base_change: ChangeId(row.get(3)?),
        before_change: ChangeId(row.get(4)?),
        after_change: row.get::<_, Option<String>>(5)?.map(ChangeId),
        status: row.get(6)?,
        started_at: row.get(7)?,
        ended_at: row.get(8)?,
        metadata_json: row.get(9)?,
    })
}

fn agent_event_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentEventRecord> {
    let payload_json: Option<String> = row.get(7)?;
    let payload =
        payload_json.and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok());
    Ok(AgentEventRecord {
        event_id: row.get(0)?,
        agent_id: row.get(1)?,
        session_id: row.get(2)?,
        turn_id: row.get(3)?,
        event_type: row.get(4)?,
        change_id: row.get::<_, Option<String>>(5)?.map(ChangeId),
        message_id: row.get::<_, Option<String>>(6)?.map(MessageId),
        payload,
        created_at: row.get(8)?,
    })
}

fn agent_approval_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentApproval> {
    let payload_json: Option<String> = row.get(6)?;
    let payload =
        payload_json.and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok());
    Ok(AgentApproval {
        approval_id: row.get(0)?,
        agent_id: row.get(1)?,
        session_id: row.get(2)?,
        turn_id: row.get(3)?,
        action: row.get(4)?,
        summary: row.get(5)?,
        payload,
        status: row.get(7)?,
        requested_at: row.get(8)?,
        decided_at: row.get(9)?,
        reviewer: row.get(10)?,
        note: row.get(11)?,
    })
}

fn agent_run_state_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentRunState> {
    let state_json: String = row.get(8)?;
    let state =
        serde_json::from_str::<serde_json::Value>(&state_json).unwrap_or(serde_json::Value::Null);
    let interruption_json: Option<String> = row.get(9)?;
    let interruption =
        interruption_json.and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok());
    Ok(AgentRunState {
        run_id: row.get(0)?,
        agent_id: row.get(1)?,
        session_id: row.get(2)?,
        turn_id: row.get(3)?,
        approval_id: row.get(4)?,
        status: row.get(5)?,
        reason: row.get(6)?,
        summary: row.get(7)?,
        state,
        interruption,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        resumed_at: row.get(12)?,
        reviewer: row.get(13)?,
        note: row.get(14)?,
    })
}

fn timeline_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TimelineEntry> {
    Ok(TimelineEntry {
        change_id: ChangeId(row.get(0)?),
        kind: parse_operation_kind(&row.get::<_, String>(1)?),
        branch: row.get(2)?,
        actor_id: row.get(3)?,
        message: row.get(4)?,
        created_at: row.get(5)?,
        path_count: row.get::<_, i64>(6)? as u64,
    })
}

fn parse_operation_kind(value: &str) -> OperationKind {
    match value {
        "GitImport" => OperationKind::GitImport,
        "FileEdit" => OperationKind::FileEdit,
        "MultiFileEdit" => OperationKind::MultiFileEdit,
        "Format" => OperationKind::Format,
        "ManualCheckpoint" => OperationKind::ManualCheckpoint,
        "ManualRecord" => OperationKind::ManualRecord,
        "WatchRecord" => OperationKind::WatchRecord,
        "Checkout" => OperationKind::Checkout,
        "Branch" => OperationKind::Branch,
        "Merge" => OperationKind::Merge,
        "AgentSpawn" => OperationKind::AgentSpawn,
        "AgentPatch" => OperationKind::AgentPatch,
        "AgentRecord" => OperationKind::AgentRecord,
        "AgentMerge" => OperationKind::AgentMerge,
        "GitExport" => OperationKind::GitExport,
        _ => OperationKind::Init,
    }
}

#[cfg(unix)]
fn executable(path: &Path) -> Result<bool> {
    use std::os::unix::fs::PermissionsExt;
    Ok(fs::metadata(path)?.permissions().mode() & 0o111 != 0)
}

#[cfg(unix)]
fn executable_from_metadata(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_path: &Path) -> Result<bool> {
    Ok(false)
}

#[cfg(not(unix))]
fn executable_from_metadata(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn set_executable(path: &Path, executable: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)?.permissions();
    let mut mode = permissions.mode();
    if executable {
        mode |= 0o755;
    } else {
        mode &= !0o111;
    }
    permissions.set_mode(mode);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path, _executable: bool) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_fold_collision_validation_rejects_ambiguous_paths() {
        let paths = [
            "src/Foo.rs".to_string(),
            "src/foo.rs".to_string(),
            "src/bar.rs".to_string(),
        ];
        let err = validate_no_case_fold_collisions(paths.iter()).unwrap_err();
        match err {
            Error::InvalidPath { path, reason } => {
                assert_eq!(path, "src/foo.rs");
                assert!(reason.contains("src/Foo.rs"));
            }
            other => panic!("expected invalid path error, got {other:?}"),
        }
    }

    #[test]
    fn case_fold_collision_validation_allows_distinct_paths() {
        let paths = ["src/foo.rs".to_string(), "src/bar.rs".to_string()];
        validate_no_case_fold_collisions(paths.iter()).unwrap();
    }
}
