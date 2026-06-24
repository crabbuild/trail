use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crabdb::{
    Actor, AgentGateOptions, AgentMessageReport, AgentPatchReport, AgentTurnDetails,
    AgentTurnEndReport, AgentTurnEventReport, AgentTurnStartReport, ConflictManualFile,
    ConflictManualResolution, CrabDb, Error, InitImportMode, PatchDocument, ShowResult,
    WorktreeState,
};
use rusqlite::Connection;

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn run_git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn crabdb_bin() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_crabdb")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/crabdb")
        })
}

fn run_crabdb_json(workspace: &Path, args: &[&str]) -> serde_json::Value {
    let output = Command::new(crabdb_bin())
        .arg("--workspace")
        .arg(workspace)
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "crabdb {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn api_request(method: &str, path: &str, body: serde_json::Value) -> Vec<u8> {
    api_request_with_headers(method, path, &[], body)
}

fn conflicted_readme_workspace(
    agent_content: &str,
    human_content: &str,
) -> (tempfile::TempDir, CrabDb, String) {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("manual-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "agent edits readme",
        "edits": [
            {"op": "write", "path": "README.md", "content": agent_content}
        ]
    }))
    .unwrap();
    db.apply_agent_patch("manual-bot", patch).unwrap();

    fs::write(temp.path().join("README.md"), human_content).unwrap();
    db.record(
        Some("main"),
        Some("human edit".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();
    db.enqueue_merge("manual-bot", "main", 0).unwrap();
    let run = db.run_merge_queue(None).unwrap();
    assert!(run.stopped_on_conflict);
    let conflict_id = db.list_conflicts().unwrap()[0].conflict_set_id.clone();
    (temp, db, conflict_id)
}

fn api_request_with_headers(
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: serde_json::Value,
) -> Vec<u8> {
    let body = if body.is_null() {
        Vec::new()
    } else {
        serde_json::to_vec(&body).unwrap()
    };
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\n");
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str(&format!(
        "Content-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    ));
    [head.into_bytes(), body].concat()
}

#[test]
fn cli_json_errors_are_machine_readable() {
    let temp = tempfile::tempdir().unwrap();

    let parse_output = Command::new(crabdb_bin())
        .arg("--json")
        .arg("definitely-not-a-command")
        .output()
        .unwrap();
    assert!(!parse_output.status.success());
    assert_eq!(parse_output.status.code(), Some(2));
    assert!(parse_output.stdout.is_empty());
    let parse_stderr: serde_json::Value = serde_json::from_slice(&parse_output.stderr).unwrap();
    assert_eq!(parse_stderr["error"]["code"], "INVALID_INPUT");
    assert_eq!(parse_stderr["error"]["exit_code"], 2);
    assert!(parse_stderr["error"]["message"]
        .as_str()
        .unwrap()
        .contains("definitely-not-a-command"));

    let output = Command::new(crabdb_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("--json")
        .arg("status")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    let stderr: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(stderr["error"]["code"], "WORKSPACE_NOT_FOUND");
    assert_eq!(stderr["error"]["exit_code"], 3);
    assert!(stderr["error"]["message"]
        .as_str()
        .unwrap()
        .contains("workspace not found"));

    let format_output = Command::new(crabdb_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("--format")
        .arg("json")
        .arg("status")
        .output()
        .unwrap();
    assert_eq!(format_output.status.code(), Some(3));
    let format_stderr: serde_json::Value = serde_json::from_slice(&format_output.stderr).unwrap();
    assert_eq!(format_stderr["error"]["code"], "WORKSPACE_NOT_FOUND");

    let env_parse_output = Command::new(crabdb_bin())
        .env("CRABDB_FORMAT", "json")
        .arg("still-not-a-command")
        .output()
        .unwrap();
    assert!(!env_parse_output.status.success());
    assert_eq!(env_parse_output.status.code(), Some(2));
    let env_parse_stderr: serde_json::Value =
        serde_json::from_slice(&env_parse_output.stderr).unwrap();
    assert_eq!(env_parse_stderr["error"]["code"], "INVALID_INPUT");
    assert!(env_parse_stderr["error"]["message"]
        .as_str()
        .unwrap()
        .contains("still-not-a-command"));
}

#[test]
fn cli_env_defaults_select_workspace_db_branch_and_format() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = CrabDb::open(temp.path()).unwrap();
    db.create_branch("scratch", Some("main")).unwrap();
    drop(db);

    let workspace_output = Command::new(crabdb_bin())
        .env("CRABDB_WORKSPACE", temp.path())
        .env_remove("CRABDB_DIR")
        .env("CRABDB_FORMAT", "json")
        .env("CRABDB_BRANCH", "scratch")
        .arg("status")
        .output()
        .unwrap();
    assert!(
        workspace_output.status.success(),
        "status with env workspace failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&workspace_output.stdout),
        String::from_utf8_lossy(&workspace_output.stderr)
    );
    let workspace_status: serde_json::Value =
        serde_json::from_slice(&workspace_output.stdout).unwrap();
    assert_eq!(workspace_status["branch"], "scratch");

    let local_branch_status = run_crabdb_json(temp.path(), &["status", "--branch", "scratch"]);
    assert_eq!(local_branch_status["branch"], "scratch");

    let db_dir_output = Command::new(crabdb_bin())
        .env_remove("CRABDB_WORKSPACE")
        .env_remove("CRABDB_BRANCH")
        .env("CRABDB_DIR", temp.path().join(".crabdb"))
        .env("CRABDB_FORMAT", "json")
        .arg("status")
        .output()
        .unwrap();
    assert!(
        db_dir_output.status.success(),
        "status with env db dir failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&db_dir_output.stdout),
        String::from_utf8_lossy(&db_dir_output.stderr)
    );
    let db_dir_status: serde_json::Value = serde_json::from_slice(&db_dir_output.stdout).unwrap();
    assert_eq!(db_dir_status["branch"], "main");

    let invalid_format = Command::new(crabdb_bin())
        .env("CRABDB_WORKSPACE", temp.path())
        .env_remove("CRABDB_DIR")
        .env_remove("CRABDB_BRANCH")
        .env("CRABDB_FORMAT", "xml")
        .arg("status")
        .output()
        .unwrap();
    assert!(!invalid_format.status.success());
    assert!(String::from_utf8_lossy(&invalid_format.stderr)
        .contains("CRABDB_FORMAT must be human, json, or ndjson"));
}

#[test]
fn init_record_why_and_fsck_work() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();

    let init = CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    assert_eq!(init.imported.files, 1);

    fs::write(temp.path().join("README.md"), "hello\nCrabDB\n").unwrap();
    let mut db = CrabDb::open(temp.path()).unwrap();
    let record = db
        .record(
            Some("main"),
            Some("edit readme".to_string()),
            Actor::human(),
            false,
        )
        .unwrap();
    assert!(record.operation.is_some());
    assert_eq!(record.changed_paths.len(), 1);

    let why = db.why("README.md:2", Some("main")).unwrap();
    assert_eq!(why.current_text, "CrabDB");
    assert_eq!(why.history.len(), 1);

    let fsck = db.fsck().unwrap();
    assert!(fsck.errors.is_empty(), "{:?}", fsck.errors);
}

#[test]
fn doctor_reports_operational_health_across_cli_api_and_mcp() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    {
        let db = CrabDb::open(temp.path()).unwrap();
        let clean = db.doctor().unwrap();
        assert_eq!(clean.status, "ok");
        assert!(clean
            .checks
            .iter()
            .any(|check| check.name == "fsck" && check.status == "ok"));
        assert!(clean.checks.iter().any(|check| {
            check.name == "schema_version"
                && check.status == "ok"
                && check.details.as_ref().unwrap()["sqlite_user_version"] == 1
        }));
    }

    let cli = run_crabdb_json(temp.path(), &["doctor"]);
    assert_eq!(cli["status"], "ok");
    assert!(cli["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|check| check["name"] == "current_branch" && check["status"] == "ok"));

    let mut db = CrabDb::open(temp.path()).unwrap();
    let api = crabdb::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/doctor", serde_json::Value::Null),
    );
    assert_eq!(api.status, 200);
    let api: serde_json::Value = api.body_json().unwrap();
    assert_eq!(api["status"], "ok");

    let tools = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }),
    )
    .unwrap();
    let tool_list = tools["result"]["tools"].as_array().unwrap();
    assert!(tool_list.iter().any(|tool| tool["name"] == "crabdb.doctor"));

    let mcp = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "crabdb.doctor",
                "arguments": {}
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp["result"]["isError"], false);
    assert_eq!(mcp["result"]["structuredContent"]["status"], "ok");

    db.spawn_agent("doctor-bot", Some("main"), false, None, None)
        .unwrap();
    db.request_agent_approval(
        "doctor-bot",
        "shell.exec",
        "Run release smoke tests",
        None,
        None,
        None,
    )
    .unwrap();
    let warning = db.doctor().unwrap();
    assert_eq!(warning.status, "warning");
    let pending = warning
        .checks
        .iter()
        .find(|check| check.name == "pending_approvals")
        .unwrap();
    assert_eq!(pending.status, "warning");
    assert_eq!(pending.details.as_ref().unwrap()["count"], 1);
}

#[test]
fn crabdb_refuses_workspaces_with_newer_schema_versions() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let conn = Connection::open(temp.path().join(".crabdb/index/crabdb.sqlite")).unwrap();
    conn.execute_batch("PRAGMA user_version = 999;").unwrap();
    drop(conn);

    let err = match CrabDb::open(temp.path()) {
        Ok(_) => panic!("opening a future schema should fail"),
        Err(err) => err,
    };
    assert!(err
        .to_string()
        .contains("schema version 999 is newer than supported version"));
}

#[test]
fn init_text_policy_sets_text_tracking_thresholds() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();

    let initialized = run_crabdb_json(
        temp.path(),
        &["init", "--working-tree", "--text-policy", "full"],
    );
    assert_eq!(initialized["branch"], "main");

    let opaque_limit = run_crabdb_json(
        temp.path(),
        &["config", "get", "text.opaque_text_max_bytes"],
    );
    assert_eq!(opaque_limit["value"], "67108864");

    let line_limit = run_crabdb_json(temp.path(), &["config", "get", "text.max_line_bytes"]);
    assert_eq!(line_limit["value"], "8388608");
}

#[test]
fn backup_create_verify_and_restore_roundtrip() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    fs::write(temp.path().join("README.md"), "hello\nbackup\n").unwrap();
    let mut db = CrabDb::open(temp.path()).unwrap();
    db.record(
        Some("main"),
        Some("prepare backup".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();
    db.spawn_agent("backup-bot", Some("main"), true, None, None)
        .unwrap();
    drop(db);

    let backup_parent = tempfile::tempdir().unwrap();
    let backup_path = backup_parent.path().join("crabdb-backup");
    let created = run_crabdb_json(
        temp.path(),
        &["backup", "create", backup_path.to_str().unwrap()],
    );
    assert_eq!(created["branch"], "main");
    assert!(created["sqlite_bytes"].as_u64().unwrap() > 0);
    assert!(created["worktree_bytes"].as_u64().unwrap() > 0);

    let verified = run_crabdb_json(
        temp.path(),
        &["backup", "verify", backup_path.to_str().unwrap()],
    );
    assert_eq!(verified["valid"], true);
    assert_eq!(verified["branch"], "main");
    assert!(verified["checked_refs"].as_u64().unwrap() >= 2);

    let sdk_verified = CrabDb::verify_backup(&backup_path).unwrap();
    assert!(sdk_verified.valid, "{:?}", sdk_verified.errors);

    let restored = tempfile::tempdir().unwrap();
    let restored_report = run_crabdb_json(
        restored.path(),
        &["backup", "restore", backup_path.to_str().unwrap()],
    );
    assert_eq!(restored_report["branch"], "main");
    assert_eq!(restored_report["replaced_existing"], false);
    assert_eq!(restored_report["restored_crabignore"], true);
    assert_eq!(restored_report["rewritten_workdirs"], 1);

    let restored_db = CrabDb::open(restored.path()).unwrap();
    let why = restored_db.why("README.md:2", Some("main")).unwrap();
    assert_eq!(why.current_text, "backup");
    let fsck = restored_db.fsck().unwrap();
    assert!(fsck.errors.is_empty(), "{:?}", fsck.errors);

    let agent = restored_db.agent_details("backup-bot").unwrap();
    let workdir = agent.branch.workdir.as_ref().unwrap();
    let restored_db_dir = restored.path().canonicalize().unwrap().join(".crabdb");
    assert!(workdir.starts_with(&restored_db_dir.to_string_lossy().to_string()));
    assert!(PathBuf::from(workdir).is_dir());
    let status = restored_db.agent_status("backup-bot").unwrap();
    assert_eq!(status.workdir_state, Some(WorktreeState::Clean));
}

#[test]
fn record_paths_records_only_selected_changes() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), "a1\n").unwrap();
    fs::write(temp.path().join("b.txt"), "b1\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    fs::write(temp.path().join("a.txt"), "a2\n").unwrap();
    fs::write(temp.path().join("b.txt"), "b2\n").unwrap();

    let recorded = run_crabdb_json(
        temp.path(),
        &["record", "--paths", "a.txt", "-m", "record only a"],
    );
    assert!(recorded["operation"].as_str().is_some());
    assert_eq!(recorded["changed_paths"].as_array().unwrap().len(), 1);
    assert_eq!(recorded["changed_paths"][0]["path"], "a.txt");

    let db = CrabDb::open(temp.path()).unwrap();
    assert_eq!(db.why("a.txt:1", Some("main")).unwrap().current_text, "a2");
    assert_eq!(db.why("b.txt:1", Some("main")).unwrap().current_text, "b1");
    let status = db.status(Some("main")).unwrap();
    assert_eq!(status.changed_paths.len(), 1);
    assert_eq!(status.changed_paths[0].path, "b.txt");
}

#[test]
fn record_kind_session_and_allow_ignored_path_are_audited() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("auditor", Some("main"), false, None, None)
        .unwrap();
    db.start_agent_session(
        "auditor",
        Some("Record ignored fixture".to_string()),
        Some("session-record".to_string()),
    )
    .unwrap();
    drop(db);

    fs::write(temp.path().join(".env.local"), "SECRET=fixture\n").unwrap();
    let recorded = run_crabdb_json(
        temp.path(),
        &[
            "record",
            "--paths",
            ".env.local",
            "--allow-ignored",
            "--kind",
            "manual-checkpoint",
            "--session",
            "session-record",
            "-m",
            "capture ignored fixture",
        ],
    );
    let operation = recorded["operation"].as_str().unwrap();
    assert_eq!(recorded["changed_paths"][0]["path"], ".env.local");

    let db = CrabDb::open(temp.path()).unwrap();
    let shown = db.show(operation).unwrap();
    match shown {
        ShowResult::Operation { value } => {
            assert_eq!(
                value.operation.kind,
                crabdb::OperationKind::ManualCheckpoint
            );
            assert_eq!(
                value.operation.session_id.as_deref(),
                Some("session-record")
            );
            assert_eq!(
                value.operation.message.as_deref(),
                Some("capture ignored fixture")
            );
        }
        other => panic!("expected operation, got {other:?}"),
    }
    let root = db
        .inspect_root(recorded["root_id"].as_str().unwrap())
        .unwrap();
    assert!(root.files.iter().any(|file| file.path == ".env.local"));
}

#[test]
fn watch_cli_can_attach_recorded_operations_to_session() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("watch-bot", Some("main"), false, None, None)
        .unwrap();
    db.start_agent_session(
        "watch-bot",
        Some("Watch session".to_string()),
        Some("session-watch".to_string()),
    )
    .unwrap();
    drop(db);

    fs::write(temp.path().join("README.md"), "hello\nwatched\n").unwrap();
    let watched = run_crabdb_json(
        temp.path(),
        &[
            "watch",
            "--once",
            "--debounce",
            "10",
            "--include-untracked",
            "--session",
            "session-watch",
            "-m",
            "watch session edit",
        ],
    );
    let operation = watched["operation"].as_str().unwrap().to_string();
    assert_eq!(watched["changed_paths"][0]["path"], "README.md");

    let db = CrabDb::open(temp.path()).unwrap();
    let shown = db.show(&operation).unwrap();
    match shown {
        ShowResult::Operation { value } => {
            assert_eq!(value.operation.kind, crabdb::OperationKind::WatchRecord);
            assert_eq!(value.operation.session_id.as_deref(), Some("session-watch"));
            assert_eq!(
                value.operation.message.as_deref(),
                Some("watch session edit")
            );
        }
        other => panic!("expected operation, got {other:?}"),
    }
    let timeline = db.session_timeline("session-watch", 10).unwrap();
    assert_eq!(timeline.len(), 1);
    assert_eq!(timeline[0].change_id.0, operation);
}

#[test]
fn ignore_cli_manages_crabignore_and_status() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let listed = run_crabdb_json(temp.path(), &["ignore", "list"]);
    assert!(listed["patterns"]
        .as_array()
        .unwrap()
        .iter()
        .any(|pattern| pattern["pattern"] == "*.p12"));

    let added = run_crabdb_json(temp.path(), &["ignore", "add", "notes.secret"]);
    assert_eq!(added["added"], true);
    let added_again = run_crabdb_json(temp.path(), &["ignore", "add", "notes.secret"]);
    assert_eq!(added_again["added"], false);

    fs::write(temp.path().join("notes.secret"), "secret\n").unwrap();
    let checked = run_crabdb_json(temp.path(), &["ignore", "check", "notes.secret"]);
    assert_eq!(checked["ignored"], true);
    assert_eq!(checked["source"], "workspace");

    let db = CrabDb::open(temp.path()).unwrap();
    let status = db.status(Some("main")).unwrap();
    assert!(!status
        .changed_paths
        .iter()
        .any(|path| path.path == "notes.secret"));
    drop(db);

    let removed = run_crabdb_json(temp.path(), &["ignore", "remove", "notes.secret"]);
    assert_eq!(removed["removed"], true);
    let checked = run_crabdb_json(temp.path(), &["ignore", "check", "notes.secret"]);
    assert_eq!(checked["ignored"], false);

    let db = CrabDb::open(temp.path()).unwrap();
    let status = db.status(Some("main")).unwrap();
    assert!(status
        .changed_paths
        .iter()
        .any(|path| path.path == "notes.secret"));
}

#[test]
fn agent_patch_respects_ignore_policy_and_explicit_opt_in() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.ignore_add("ignored-agent-output.txt").unwrap();
    db.spawn_agent("privacy-bot", Some("main"), false, None, None)
        .unwrap();

    let blocked: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "try ignored write",
        "edits": [
            {
                "op": "write",
                "path": "ignored-agent-output.txt",
                "content": "secret-ish\n"
            }
        ]
    }))
    .unwrap();
    let err = db.apply_agent_patch("privacy-bot", blocked).unwrap_err();
    assert!(matches!(err, Error::IgnoredPath(path) if path == "ignored-agent-output.txt"));

    let allowed: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "explicit ignored fixture",
        "allow_ignored": true,
        "edits": [
            {
                "op": "write",
                "path": "ignored-agent-output.txt",
                "content": "intentional fixture\n"
            }
        ]
    }))
    .unwrap();
    let report = db.apply_agent_patch("privacy-bot", allowed).unwrap();
    assert!(report
        .changed_paths
        .iter()
        .any(|path| path.path == "ignored-agent-output.txt"));

    let internal: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "try internal write",
        "allow_ignored": true,
        "edits": [
            {
                "op": "write",
                "path": ".crabdb/leak.txt",
                "content": "nope\n"
            }
        ]
    }))
    .unwrap();
    let err = db.apply_agent_patch("privacy-bot", internal).unwrap_err();
    assert!(matches!(err, Error::IgnoredPath(path) if path == ".crabdb/leak.txt"));
}

#[test]
fn local_api_and_mcp_expose_ignore_controls() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let list_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/ignore", serde_json::Value::Null),
    );
    assert_eq!(list_response.status, 200);
    let listed: serde_json::Value = list_response.body_json().unwrap();
    assert!(listed["patterns"]
        .as_array()
        .unwrap()
        .iter()
        .any(|pattern| pattern["pattern"] == "*.p12"));

    let add_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/ignore/patterns",
            serde_json::json!({ "pattern": "*.agentlocal" }),
        ),
    );
    assert_eq!(add_response.status, 200);
    let added: serde_json::Value = add_response.body_json().unwrap();
    assert_eq!(added["added"], true);

    fs::write(temp.path().join("scratch.agentlocal"), "secret\n").unwrap();
    let check_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/ignore/check",
            serde_json::json!({ "path": "scratch.agentlocal" }),
        ),
    );
    assert_eq!(check_response.status, 200);
    let checked: serde_json::Value = check_response.body_json().unwrap();
    assert_eq!(checked["ignored"], true);
    assert_eq!(checked["source"], "workspace");

    let guardrail_ignored = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/guardrails/check",
            serde_json::json!({
                "action": "file.write",
                "summary": "write ignored scratch fixture",
                "paths": ["scratch.agentlocal"]
            }),
        ),
    );
    assert_eq!(guardrail_ignored.status, 200);
    let guardrail_ignored: serde_json::Value = guardrail_ignored.body_json().unwrap();
    assert_eq!(guardrail_ignored["decision"], "approval_required");
    assert!(guardrail_ignored["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|reason| reason["code"] == "ignored_path"));

    let guardrail_blocked = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/guardrails/check",
            serde_json::json!({
                "action": "file.write",
                "paths": [".env"]
            }),
        ),
    );
    assert_eq!(guardrail_blocked.status, 200);
    let guardrail_blocked: serde_json::Value = guardrail_blocked.body_json().unwrap();
    assert_eq!(guardrail_blocked["decision"], "blocked");
    assert!(guardrail_blocked["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|reason| reason["code"] == "blocked_path"));

    let remove_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "DELETE",
            "/v1/ignore/patterns",
            serde_json::json!({ "pattern": "*.agentlocal" }),
        ),
    );
    assert_eq!(remove_response.status, 200);
    let removed: serde_json::Value = remove_response.body_json().unwrap();
    assert_eq!(removed["removed"], true);

    let tools = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }),
    )
    .unwrap();
    let tool_list = tools["result"]["tools"].as_array().unwrap();
    assert!(tool_list
        .iter()
        .any(|tool| tool["name"] == "crabdb.ignore_list"));
    assert!(tool_list
        .iter()
        .any(|tool| tool["name"] == "crabdb.ignore_add"));
    assert!(tool_list
        .iter()
        .any(|tool| tool["name"] == "crabdb.ignore_remove"));
    assert!(tool_list
        .iter()
        .any(|tool| tool["name"] == "crabdb.ignore_check"));
    assert!(tool_list
        .iter()
        .any(|tool| tool["name"] == "crabdb.guardrail_check"));

    let mcp_add = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "crabdb.ignore_add",
                "arguments": { "pattern": "mcp-visible.fixture" }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_add["result"]["isError"], false);
    assert_eq!(mcp_add["result"]["structuredContent"]["added"], true);

    let mcp_check = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "crabdb.ignore_check",
                "arguments": { "path": "mcp-visible.fixture" }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_check["result"]["isError"], false);
    assert_eq!(mcp_check["result"]["structuredContent"]["ignored"], true);
    assert_eq!(
        mcp_check["result"]["structuredContent"]["source"],
        "workspace"
    );

    let mcp_guardrail = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "crabdb.guardrail_check",
                "arguments": {
                    "action": "file.write",
                    "paths": ["mcp-visible.fixture"]
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_guardrail["result"]["isError"], false);
    assert_eq!(
        mcp_guardrail["result"]["structuredContent"]["decision"],
        "approval_required"
    );

    let mcp_remove = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "crabdb.ignore_remove",
                "arguments": { "pattern": "mcp-visible.fixture" }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_remove["result"]["isError"], false);
    assert_eq!(mcp_remove["result"]["structuredContent"]["removed"], true);
}

#[test]
fn local_api_and_mcp_manage_agent_sessions() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("api-session-bot", Some("main"), false, None, None)
        .unwrap();

    let started = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/sessions",
            serde_json::json!({
                "agent": "api-session-bot",
                "title": "API session",
                "id": "session-api"
            }),
        ),
    );
    assert_eq!(started.status, 201);
    let started: serde_json::Value = started.body_json().unwrap();
    assert_eq!(started["session"]["session_id"], "session-api");

    let current = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/sessions/current?agent=api-session-bot",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(current.status, 200);
    let current: serde_json::Value = current.body_json().unwrap();
    assert_eq!(current[0]["agent_name"], "api-session-bot");
    assert_eq!(current[0]["session"]["session_id"], "session-api");

    let listed = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/sessions?agent=api-session-bot",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(listed.status, 200);
    let listed: serde_json::Value = listed.body_json().unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["session_id"], "session-api");

    let shown = crabdb::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/sessions/session-api", serde_json::Value::Null),
    );
    assert_eq!(shown.status, 200);
    let shown: serde_json::Value = shown.body_json().unwrap();
    assert_eq!(shown["session"]["title"], "API session");

    db.add_agent_message(
        "api-session-bot",
        "user",
        "Please improve the docs with a bounded context packet.",
        Some("session-api".to_string()),
    )
    .unwrap();
    db.add_agent_message(
        "api-session-bot",
        "assistant",
        "Context packet is ready for review.",
        Some("session-api".to_string()),
    )
    .unwrap();

    let context = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/sessions/session-api/context?limit=1",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(context.status, 200);
    let context: serde_json::Value = context.body_json().unwrap();
    assert_eq!(context["session"]["session_id"], "session-api");
    assert_eq!(context["message_count"], 2);
    assert_eq!(context["recent_messages"].as_array().unwrap().len(), 1);
    assert_eq!(context["recent_messages"][0]["role"], "assistant");
    assert!(context["turn_count"].as_u64().unwrap() >= 2);
    assert_eq!(context["recent_turns"].as_array().unwrap().len(), 1);

    let cli_context = run_crabdb_json(
        temp.path(),
        &["session", "context", "session-api", "--limit", "1"],
    );
    assert_eq!(cli_context["session"]["session_id"], "session-api");
    assert_eq!(cli_context["message_count"], 2);
    assert_eq!(cli_context["recent_messages"][0]["role"], "assistant");

    let ended = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/sessions/session-api/end",
            serde_json::json!({ "status": "completed" }),
        ),
    );
    assert_eq!(ended.status, 200);
    let ended: serde_json::Value = ended.body_json().unwrap();
    assert_eq!(ended["session"]["status"], "completed");

    let current = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/sessions/current?agent=api-session-bot",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(current.status, 200);
    let current: serde_json::Value = current.body_json().unwrap();
    assert!(current[0]["session"].is_null());

    db.spawn_agent("mcp-session-bot", Some("main"), false, None, None)
        .unwrap();
    let tools = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }),
    )
    .unwrap();
    let tool_list = tools["result"]["tools"].as_array().unwrap();
    assert!(tool_list
        .iter()
        .any(|tool| tool["name"] == "crabdb.session_start"));
    assert!(tool_list
        .iter()
        .any(|tool| tool["name"] == "crabdb.session_current"));
    assert!(tool_list
        .iter()
        .any(|tool| tool["name"] == "crabdb.session_context"));
    assert!(tool_list
        .iter()
        .any(|tool| tool["name"] == "crabdb.session_end"));

    let mcp_start = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "crabdb.session_start",
                "arguments": {
                    "agent": "mcp-session-bot",
                    "title": "MCP session",
                    "id": "session-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_start["result"]["isError"], false);
    assert_eq!(
        mcp_start["result"]["structuredContent"]["session"]["session_id"],
        "session-mcp"
    );

    let mcp_current = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "crabdb.session_current",
                "arguments": {
                    "agent": "mcp-session-bot"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_current["result"]["isError"], false);
    assert_eq!(
        mcp_current["result"]["structuredContent"][0]["session"]["session_id"],
        "session-mcp"
    );

    let mcp_show = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "crabdb.session_show",
                "arguments": {
                    "session_id": "session-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_show["result"]["isError"], false);
    assert_eq!(
        mcp_show["result"]["structuredContent"]["session"]["title"],
        "MCP session"
    );

    let mcp_context = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "crabdb.session_context",
                "arguments": {
                    "session_id": "session-api",
                    "limit": 1
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_context["result"]["isError"], false);
    assert_eq!(
        mcp_context["result"]["structuredContent"]["recent_messages"][0]["role"],
        "assistant"
    );

    let mcp_end = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "crabdb.session_end",
                "arguments": {
                    "session_id": "session-mcp",
                    "status": "failed"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_end["result"]["isError"], false);
    assert_eq!(
        mcp_end["result"]["structuredContent"]["session"]["status"],
        "failed"
    );
}

#[test]
fn local_api_and_mcp_manage_human_approval_gates() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("approval-bot", Some("main"), false, None, None)
        .unwrap();
    let turn = db
        .begin_agent_turn(
            "approval-bot",
            Some("main"),
            Some("Sensitive action".to_string()),
            None,
        )
        .unwrap();
    let turn_id = turn.turn.turn_id.clone();
    let session_id = turn.session.session_id.clone();

    let guardrail = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/guardrails/check",
            serde_json::json!({
                "agent": "approval-bot",
                "action": "shell.exec",
                "summary": "Run deployment smoke tests",
                "payload": {
                    "command": ["cargo", "test", "-p", "crabdb"],
                    "risk": "executes local process"
                },
                "paths": ["README.md"]
            }),
        ),
    );
    assert_eq!(guardrail.status, 200);
    let guardrail: serde_json::Value = guardrail.body_json().unwrap();
    assert_eq!(guardrail["decision"], "approval_required");
    assert_eq!(guardrail["agent"]["record"]["name"], "approval-bot");
    assert_eq!(guardrail["approval_request"]["action"], "shell.exec");
    assert!(guardrail["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|reason| reason["code"] == "shell_action"));

    let requested = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/approvals",
            serde_json::json!({
                "agent": "approval-bot",
                "turn_id": turn_id,
                "action": "shell.exec",
                "summary": "Run deployment smoke tests",
                "payload": {
                    "command": ["cargo", "test", "-p", "crabdb"],
                    "risk": "executes local process"
                }
            }),
        ),
    );
    assert_eq!(requested.status, 201);
    let requested: serde_json::Value = requested.body_json().unwrap();
    assert_eq!(requested["approval"]["status"], "pending");
    assert_eq!(requested["approval"]["session_id"], session_id);
    assert_eq!(requested["approval"]["turn_id"], turn_id);
    let approval_id = requested["approval"]["approval_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(requested["run_state"]["status"], "paused");
    assert_eq!(requested["run_state"]["reason"], "approval_required");
    assert_eq!(requested["run_state"]["approval_id"], approval_id);
    assert_eq!(requested["run_state"]["session_id"], session_id);
    assert_eq!(requested["run_state"]["turn_id"], turn_id);
    let run_id = requested["run_state"]["run_id"]
        .as_str()
        .unwrap()
        .to_string();

    let pending_resume = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/agent/runs/{run_id}/resume"),
            serde_json::json!({ "reviewer": "human-reviewer" }),
        ),
    );
    assert_eq!(pending_resume.status, 400);
    let pending_resume: serde_json::Value = pending_resume.body_json().unwrap();
    assert!(pending_resume["error"]["message"]
        .as_str()
        .unwrap()
        .contains("waiting on approval"));

    let run_list = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/agent/runs?agent=approval-bot&status=paused",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(run_list.status, 200);
    let run_list: serde_json::Value = run_list.body_json().unwrap();
    assert!(run_list
        .as_array()
        .unwrap()
        .iter()
        .any(|run_state| run_state["run_id"] == run_id));

    let cli_run_list = run_crabdb_json(
        temp.path(),
        &[
            "agent",
            "run",
            "list",
            "--agent",
            "approval-bot",
            "--status",
            "paused",
        ],
    );
    assert!(cli_run_list
        .as_array()
        .unwrap()
        .iter()
        .any(|run_state| run_state["run_id"] == run_id));

    let pending = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/approvals?agent=approval-bot&status=pending",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(pending.status, 200);
    let pending: serde_json::Value = pending.body_json().unwrap();
    assert_eq!(pending.as_array().unwrap().len(), 1);
    assert_eq!(pending[0]["approval_id"], approval_id);

    let pending_guardrail = run_crabdb_json(
        temp.path(),
        &[
            "guardrails",
            "check",
            "--agent",
            "approval-bot",
            "--action",
            "shell.exec",
            "--summary",
            "Run deployment smoke tests",
            "--path",
            "README.md",
        ],
    );
    assert_eq!(pending_guardrail["decision"], "approval_required");
    assert!(pending_guardrail["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|reason| reason["code"] == "pending_approval"));

    let readiness = db.agent_readiness("approval-bot").unwrap();
    assert!(!readiness.ready);
    assert_eq!(readiness.status, "blocked");
    assert_eq!(readiness.pending_approvals.len(), 1);
    assert!(readiness
        .blockers
        .iter()
        .any(|issue| issue.code == "pending_approvals"));

    let tools = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }),
    )
    .unwrap();
    let tool_list = tools["result"]["tools"].as_array().unwrap();
    for name in [
        "crabdb.approval_request",
        "crabdb.approval_list",
        "crabdb.approval_show",
        "crabdb.approval_decide",
        "crabdb.run_pause",
        "crabdb.run_list",
        "crabdb.run_show",
        "crabdb.run_resume",
    ] {
        assert!(tool_list.iter().any(|tool| tool["name"] == name), "{name}");
    }

    let mcp_run_show = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 20,
            "method": "tools/call",
            "params": {
                "name": "crabdb.run_show",
                "arguments": {
                    "run_id": run_id.clone()
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_run_show["result"]["isError"], false);
    assert_eq!(
        mcp_run_show["result"]["structuredContent"]["approval_id"],
        approval_id
    );

    let mcp_run_resource = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 21,
            "method": "resources/read",
            "params": {
                "uri": format!("crabdb://workspace/runs/{run_id}")
            }
        }),
    )
    .unwrap();
    let run_resource_text = mcp_run_resource["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    assert!(run_resource_text.contains(&run_id));

    let mcp_pause = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 22,
            "method": "tools/call",
            "params": {
                "name": "crabdb.run_pause",
                "arguments": {
                    "agent": "approval-bot",
                    "reason": "handoff",
                    "summary": "Pause for coordinator review",
                    "session_id": session_id.clone(),
                    "turn_id": turn_id.clone(),
                    "state": { "step": "review" },
                    "interruption": { "type": "handoff" }
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_pause["result"]["isError"], false);
    let manual_run_id = mcp_pause["result"]["structuredContent"]["run_state"]["run_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        mcp_pause["result"]["structuredContent"]["run_state"]["status"],
        "paused"
    );

    let mcp_run_list = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 23,
            "method": "tools/call",
            "params": {
                "name": "crabdb.run_list",
                "arguments": {
                    "agent": "approval-bot",
                    "status": "paused"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_run_list["result"]["isError"], false);
    assert!(mcp_run_list["result"]["structuredContent"]
        .as_array()
        .unwrap()
        .iter()
        .any(|run_state| run_state["run_id"] == manual_run_id));

    let mcp_run_resume = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 24,
            "method": "tools/call",
            "params": {
                "name": "crabdb.run_resume",
                "arguments": {
                    "run_id": manual_run_id,
                    "reviewer": "coordinator",
                    "note": "Handoff accepted"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_run_resume["result"]["isError"], false);
    assert_eq!(
        mcp_run_resume["result"]["structuredContent"]["run_state"]["status"],
        "resumed"
    );

    let mcp_show = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "crabdb.approval_show",
                "arguments": {
                    "approval_id": approval_id.clone()
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_show["result"]["isError"], false);
    assert_eq!(
        mcp_show["result"]["structuredContent"]["payload"]["risk"],
        "executes local process"
    );

    let mcp_decide = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "crabdb.approval_decide",
                "arguments": {
                    "approval_id": approval_id.clone(),
                    "decision": "approved",
                    "reviewer": "human-reviewer",
                    "note": "Smoke tests are allowed"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_decide["result"]["isError"], false);
    assert_eq!(
        mcp_decide["result"]["structuredContent"]["approval"]["status"],
        "approved"
    );
    assert_eq!(
        mcp_decide["result"]["structuredContent"]["run_states"][0]["run_id"],
        run_id
    );
    assert_eq!(
        mcp_decide["result"]["structuredContent"]["run_states"][0]["status"],
        "paused"
    );

    let resumed = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/agent/runs/{run_id}/resume"),
            serde_json::json!({
                "reviewer": "human-reviewer",
                "note": "Approval accepted; continue"
            }),
        ),
    );
    assert_eq!(resumed.status, 200);
    let resumed: serde_json::Value = resumed.body_json().unwrap();
    assert_eq!(resumed["run_state"]["run_id"], run_id);
    assert_eq!(resumed["run_state"]["status"], "resumed");
    assert_eq!(resumed["run_state"]["reviewer"], "human-reviewer");

    let shown_run = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            &format!("/v1/agent/runs/{run_id}"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(shown_run.status, 200);
    let shown_run: serde_json::Value = shown_run.body_json().unwrap();
    assert_eq!(shown_run["run_id"], run_id);
    assert_eq!(shown_run["status"], "resumed");

    let shown = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            &format!(
                "/v1/approvals/{}",
                mcp_decide["result"]["structuredContent"]["approval"]["approval_id"]
                    .as_str()
                    .unwrap()
            ),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(shown.status, 200);
    let shown: serde_json::Value = shown.body_json().unwrap();
    assert_eq!(shown["status"], "approved");
    assert_eq!(shown["reviewer"], "human-reviewer");

    let satisfied_guardrail = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/guardrails/check",
            serde_json::json!({
                "agent": "approval-bot",
                "action": "shell.exec",
                "summary": "Run deployment smoke tests",
                "payload": {
                    "command": ["cargo", "test", "-p", "crabdb"],
                    "risk": "executes local process"
                },
                "paths": ["README.md"]
            }),
        ),
    );
    assert_eq!(satisfied_guardrail.status, 200);
    let satisfied_guardrail: serde_json::Value = satisfied_guardrail.body_json().unwrap();
    assert_eq!(satisfied_guardrail["decision"], "allowed");
    assert_eq!(
        satisfied_guardrail["satisfied_approvals"][0]["approval_id"],
        approval_id
    );
    assert!(satisfied_guardrail["approval_request"].is_null());
    assert!(satisfied_guardrail["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|reason| reason["code"] == "approval_satisfied"));

    let cli_satisfied = run_crabdb_json(
        temp.path(),
        &[
            "guardrails",
            "check",
            "--agent",
            "approval-bot",
            "--action",
            "shell.exec",
            "--summary",
            "Run deployment smoke tests",
            "--path",
            "README.md",
        ],
    );
    assert_eq!(cli_satisfied["decision"], "allowed");
    assert_eq!(
        cli_satisfied["satisfied_approvals"][0]["approval_id"],
        approval_id
    );

    let rejected = db
        .request_agent_approval(
            "approval-bot",
            "deploy.preview",
            "Create preview deployment",
            Some(serde_json::json!({ "environment": "preview" })),
            Some(&session_id),
            Some(&turn_id),
        )
        .unwrap();
    let rejected_run_id = rejected.run_state.as_ref().unwrap().run_id.clone();
    let rejected_decision = db
        .decide_agent_approval(
            &rejected.approval.approval_id,
            "rejected",
            Some("human-reviewer".to_string()),
            Some("Preview deploy is not allowed".to_string()),
        )
        .unwrap();
    assert_eq!(rejected_decision.run_states[0].run_id, rejected_run_id);
    assert_eq!(rejected_decision.run_states[0].status, "blocked");
    let rejected_resume = db
        .resume_agent_run(&rejected_run_id, Some("human-reviewer".to_string()), None)
        .unwrap_err();
    assert!(rejected_resume.to_string().contains("cannot be resumed"));
    let rejected_guardrail = db
        .guardrail_check(
            Some("approval-bot"),
            "deploy.preview",
            Some("Create preview deployment"),
            Some(serde_json::json!({ "environment": "preview" })),
            &Vec::new(),
        )
        .unwrap();
    assert_eq!(rejected_guardrail.decision, "blocked");
    assert!(rejected_guardrail
        .reasons
        .iter()
        .any(|reason| reason.code == "approval_rejected"));

    let details = db.show_agent_turn(&turn_id).unwrap();
    assert!(details
        .events
        .iter()
        .any(|event| event.event_type == "approval_requested"));
    assert!(details
        .events
        .iter()
        .any(|event| event.event_type == "approval_decided"));
    assert!(details
        .events
        .iter()
        .any(|event| event.event_type == "run_paused"));
    assert!(details
        .events
        .iter()
        .any(|event| event.event_type == "run_resumed"));
}

#[test]
fn agent_trace_metadata_redacts_common_secrets() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let turn_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/agent/turns",
            serde_json::json!({
                "agent": "redaction-bot",
                "branch": "main",
                "session_title": "Redaction smoke"
            }),
        ),
    );
    assert_eq!(turn_response.status, 201);
    let turn: AgentTurnStartReport = turn_response.body_json().unwrap();

    let message_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/agent/turns/{}/messages", turn.turn.turn_id),
            serde_json::json!({
                "role": "user",
                "content": "Use password=hunter2 but keep token expiration logic visible."
            }),
        ),
    );
    assert_eq!(message_response.status, 201);

    let event_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/agent/turns/{}/events", turn.turn.turn_id),
            serde_json::json!({
                "type": "tool_call",
                "payload": {
                    "authorization": "Bearer secret-header",
                    "command": "OPENAI_API_KEY=sk-live-secret cargo test",
                    "safe": "token expiration logic"
                }
            }),
        ),
    );
    assert_eq!(event_response.status, 201);

    let approval_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/approvals",
            serde_json::json!({
                "agent": "redaction-bot",
                "turn_id": turn.turn.turn_id,
                "action": "shell.exec client_secret=action-secret",
                "summary": "Run command with api_key=summary-secret",
                "payload": {
                    "api_key": "payload-secret",
                    "args": ["--password=arg-secret"],
                    "safe": "token expiration logic"
                }
            }),
        ),
    );
    assert_eq!(approval_response.status, 201);
    let approval: serde_json::Value = approval_response.body_json().unwrap();
    let approval_id = approval["approval"]["approval_id"].as_str().unwrap();

    let decision_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/approvals/{approval_id}/decision"),
            serde_json::json!({
                "decision": "approved",
                "reviewer": "alice",
                "note": "Approved after checking token: decision-secret"
            }),
        ),
    );
    assert_eq!(decision_response.status, 200);

    let turn_details = db.show_agent_turn(&turn.turn.turn_id).unwrap();
    let approval = db.show_agent_approval(approval_id).unwrap();
    let serialized = serde_json::to_string(&(turn_details, approval)).unwrap();
    for secret in [
        "hunter2",
        "secret-header",
        "sk-live-secret",
        "action-secret",
        "summary-secret",
        "payload-secret",
        "arg-secret",
        "decision-secret",
    ] {
        assert!(
            !serialized.contains(secret),
            "serialized trace leaked {secret}: {serialized}"
        );
    }
    assert!(serialized.contains("[REDACTED]"));
    assert!(serialized.contains("token expiration logic"));
}

#[test]
fn agent_trace_events_are_queryable_across_cli_api_and_mcp() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let turn = db
        .begin_agent_turn(
            "trace-bot",
            Some("main"),
            Some("Trace inspection".to_string()),
            None,
        )
        .unwrap();
    let turn_id = turn.turn.turn_id.clone();
    let session_id = turn.session.session_id.clone();
    db.add_agent_turn_event(
        &turn_id,
        "tool_call",
        Some(serde_json::json!({
            "tool": "shell.exec",
            "command": ["cargo", "test", "-p", "crabdb"]
        })),
        None,
        None,
    )
    .unwrap();
    db.add_agent_turn_event(
        &turn_id,
        "guardrail",
        Some(serde_json::json!({
            "name": "private_path_check",
            "passed": true
        })),
        None,
        None,
    )
    .unwrap();

    let sdk_events = db
        .list_agent_events(Some("trace-bot"), None, None, Some("tool_call"), 10)
        .unwrap();
    assert_eq!(sdk_events.len(), 1);
    assert_eq!(sdk_events[0].turn_id.as_deref(), Some(turn_id.as_str()));
    assert_eq!(
        sdk_events[0].payload.as_ref().unwrap()["tool"],
        "shell.exec"
    );

    let api_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/agent/events?agent=trace-bot&type=guardrail&limit=10",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(api_response.status, 200);
    let api_events: serde_json::Value = api_response.body_json().unwrap();
    assert_eq!(api_events.as_array().unwrap().len(), 1);
    assert_eq!(api_events[0]["event_type"], "guardrail");
    assert_eq!(api_events[0]["payload"]["passed"], true);

    let tools = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }),
    )
    .unwrap();
    let tool_list = tools["result"]["tools"].as_array().unwrap();
    assert!(tool_list
        .iter()
        .any(|tool| tool["name"] == "crabdb.event_list"));

    let mcp_events = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "crabdb.event_list",
                "arguments": {
                    "turn_id": turn_id,
                    "limit": 10
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_events["result"]["isError"], false);
    let mcp_event_list = mcp_events["result"]["structuredContent"]
        .as_array()
        .unwrap();
    assert!(mcp_event_list
        .iter()
        .any(|event| event["event_type"] == "tool_call"));
    assert!(mcp_event_list
        .iter()
        .any(|event| event["event_type"] == "guardrail"));

    let cli_events = run_crabdb_json(
        temp.path(),
        &[
            "agent",
            "events",
            "--session",
            &session_id,
            "--type",
            "tool_call",
            "--limit",
            "5",
        ],
    );
    assert_eq!(cli_events.as_array().unwrap().len(), 1);
    assert_eq!(cli_events[0]["event_type"], "tool_call");
}

#[test]
fn agent_trace_spans_are_parentable_redacted_and_available_across_surfaces() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let turn = db
        .begin_agent_turn(
            "span-bot",
            Some("main"),
            Some("Trace span inspection".to_string()),
            None,
        )
        .unwrap();
    let turn_id = turn.turn.turn_id.clone();

    let root = db
        .start_agent_trace_span(
            &turn_id,
            "agent",
            "span-bot turn",
            None,
            None,
            Some(serde_json::json!({
                "goal": "inspect trace span surfaces",
                "authorization": "Bearer root-span-secret"
            })),
        )
        .unwrap();
    assert_eq!(root.span.status, "running");
    assert!(root.span.trace_id.starts_with("trace_"));
    assert!(root.span.parent_span_id.is_none());
    let root_span_id = root.span.span_id.clone();
    let trace_id = root.span.trace_id.clone();

    let http_start = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/agent/turns/{turn_id}/spans"),
            serde_json::json!({
                "type": "tool_call",
                "name": "shell.exec",
                "parent_span_id": root_span_id,
                "attributes": {
                    "command": "OPENAI_API_KEY=sk-child-span-secret cargo test",
                    "cwd": "."
                }
            }),
        ),
    );
    assert_eq!(http_start.status, 201);
    let http_start: serde_json::Value = http_start.body_json().unwrap();
    assert_eq!(http_start["span"]["trace_id"], trace_id);
    assert_eq!(http_start["span"]["parent_span_id"], root_span_id);
    let child_span_id = http_start["span"]["span_id"].as_str().unwrap().to_string();

    let http_end = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/agent/spans/{child_span_id}/end"),
            serde_json::json!({
                "status": "completed",
                "result": {
                    "api_key": "child-result-secret",
                    "exit_code": 0
                }
            }),
        ),
    );
    assert_eq!(http_end.status, 200);
    let http_end: serde_json::Value = http_end.body_json().unwrap();
    assert_eq!(http_end["span"]["status"], "completed");
    assert!(http_end["span"]["ended_at"].is_number());

    let tools = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }),
    )
    .unwrap();
    let tool_list = tools["result"]["tools"].as_array().unwrap();
    for name in [
        "crabdb.span_start",
        "crabdb.span_end",
        "crabdb.span_list",
        "crabdb.span_summary",
        "crabdb.span_show",
    ] {
        assert!(tool_list.iter().any(|tool| tool["name"] == name), "{name}");
    }

    let mcp_start = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "crabdb.span_start",
                "arguments": {
                    "turn_id": turn_id,
                    "type": "evaluation",
                    "name": "unit-test gate",
                    "parent": root_span_id,
                    "attributes": {
                        "secret_token": "mcp-span-secret",
                        "suite": "unit"
                    }
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_start["result"]["isError"], false);
    let mcp_span_id = mcp_start["result"]["structuredContent"]["span"]["span_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        mcp_start["result"]["structuredContent"]["span"]["trace_id"],
        trace_id
    );

    let mcp_end = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "crabdb.span_end",
                "arguments": {
                    "span_id": mcp_span_id,
                    "status": "failed",
                    "result": {
                        "token": "mcp-result-secret",
                        "passed": false
                    }
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_end["result"]["isError"], false);
    assert_eq!(
        mcp_end["result"]["structuredContent"]["span"]["status"],
        "failed"
    );

    let cli_start = run_crabdb_json(
        temp.path(),
        &[
            "agent",
            "trace",
            "start",
            &turn_id,
            "--type",
            "guardrail",
            "--name",
            "private path check",
            "--parent",
            &root_span_id,
            "--attributes-json",
            r#"{"password":"cli-span-secret","passed":true}"#,
        ],
    );
    let cli_span_id = cli_start["span"]["span_id"].as_str().unwrap().to_string();
    assert_eq!(cli_start["span"]["trace_id"], trace_id);

    let cli_end = run_crabdb_json(
        temp.path(),
        &[
            "agent",
            "trace",
            "end",
            &cli_span_id,
            "--status",
            "completed",
            "--result-json",
            r#"{"client_secret":"cli-result-secret","passed":true}"#,
        ],
    );
    assert_eq!(cli_end["span"]["status"], "completed");

    let cli_list = run_crabdb_json(
        temp.path(),
        &[
            "agent", "trace", "list", "--turn", &turn_id, "--limit", "10",
        ],
    );
    assert!(cli_list
        .as_array()
        .unwrap()
        .iter()
        .any(|span| span["span_id"] == child_span_id));
    let cli_summary = run_crabdb_json(
        temp.path(),
        &[
            "agent",
            "trace",
            "summary",
            "--turn",
            &turn_id,
            "--trace-id",
            &trace_id,
        ],
    );
    assert_eq!(cli_summary["span_count"], 4);
    assert_eq!(cli_summary["open_span_count"], 1);
    assert_eq!(cli_summary["ended_span_count"], 3);
    assert_eq!(cli_summary["failed_span_count"], 1);
    assert_eq!(
        cli_summary["status_counts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|count| count["name"] == "failed")
            .unwrap()["count"],
        1
    );

    let api_list = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            &format!("/v1/agent/spans?trace={trace_id}&limit=10"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(api_list.status, 200);
    let api_list: serde_json::Value = api_list.body_json().unwrap();
    assert!(api_list.as_array().unwrap().len() >= 4);

    let api_summary = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            &format!("/v1/agent/spans/summary?trace={trace_id}&slowest=3"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(api_summary.status, 200);
    let api_summary: serde_json::Value = api_summary.body_json().unwrap();
    assert_eq!(api_summary["span_count"], 4);
    assert_eq!(api_summary["trace_id"], trace_id);
    assert!(api_summary["slowest_spans"].as_array().unwrap().len() <= 3);

    let mcp_summary = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "crabdb.span_summary",
                "arguments": {
                    "turn_id": turn_id,
                    "trace_id": trace_id,
                    "slowest": 2
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_summary["result"]["isError"], false);
    assert_eq!(
        mcp_summary["result"]["structuredContent"]["failed_span_count"],
        1
    );

    let mcp_show = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "crabdb.span_show",
                "arguments": {
                    "span_id": child_span_id
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_show["result"]["isError"], false);
    assert_eq!(
        mcp_show["result"]["structuredContent"]["span_id"],
        child_span_id
    );

    let spans = db
        .list_agent_trace_spans(None, None, Some(&turn_id), Some(&trace_id), 10)
        .unwrap();
    assert!(spans.iter().any(|span| span.span_id == root_span_id));
    assert!(spans
        .iter()
        .any(|span| span.parent_span_id.as_deref() == Some(root_span_id.as_str())));
    let summary = db
        .summarize_agent_trace_spans(None, None, Some(&turn_id), Some(&trace_id), 5)
        .unwrap();
    assert_eq!(summary.span_count, 4);
    assert_eq!(summary.open_span_count, 1);
    assert_eq!(summary.ended_span_count, 3);
    assert_eq!(summary.failed_span_count, 1);
    assert!(summary
        .span_type_counts
        .iter()
        .any(|count| count.name == "evaluation" && count.count == 1));

    let events = db
        .list_agent_events(None, None, Some(&turn_id), None, 50)
        .unwrap();
    let serialized = serde_json::to_string(&(
        spans,
        events,
        cli_list,
        cli_summary,
        api_list,
        api_summary,
        mcp_summary,
        summary,
    ))
    .unwrap();
    for secret in [
        "root-span-secret",
        "sk-child-span-secret",
        "child-result-secret",
        "mcp-span-secret",
        "mcp-result-secret",
        "cli-span-secret",
        "cli-result-secret",
    ] {
        assert!(!serialized.contains(secret), "{secret} leaked");
    }
    assert!(serialized.contains("inspect trace span surfaces"));
}

#[test]
fn hardcoded_private_key_denylist_is_not_recorded() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    fs::write(temp.path().join("id_rsa"), "PRIVATE\n").unwrap();
    fs::write(temp.path().join("client.p12"), "CERT\n").unwrap();

    let checked = run_crabdb_json(temp.path(), &["ignore", "check", "id_rsa"]);
    assert_eq!(checked["ignored"], true);
    assert_eq!(checked["source"], "hardcoded");

    let db = CrabDb::open(temp.path()).unwrap();
    let status = db.status(Some("main")).unwrap();
    assert!(!status
        .changed_paths
        .iter()
        .any(|path| path.path == "id_rsa" || path.path == "client.p12"));
}

#[test]
fn local_agent_http_api_records_turn_messages_and_patches() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let health = crabdb::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/health", serde_json::Value::Null),
    );
    assert_eq!(health.status, 200);

    let turn_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/agent/turns",
            serde_json::json!({
                "agent": "api-agent",
                "branch": "main",
                "session_title": "API smoke"
            }),
        ),
    );
    assert_eq!(turn_response.status, 201);
    let turn: AgentTurnStartReport = turn_response.body_json().unwrap();
    assert_eq!(turn.session.title.as_deref(), Some("API smoke"));
    assert_eq!(turn.turn.status, "started");

    let message_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/agent/turns/{}/messages", turn.turn.turn_id),
            serde_json::json!({
                "role": "user",
                "content": "Add a small API file."
            }),
        ),
    );
    assert_eq!(message_response.status, 201);
    let message: AgentMessageReport = message_response.body_json().unwrap();
    assert_eq!(
        message.session_id.as_deref(),
        Some(turn.session.session_id.as_str())
    );

    let event_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/agent/turns/{}/events", turn.turn.turn_id),
            serde_json::json!({
                "type": "tool_call",
                "payload": {
                    "tool": "editor.apply_patch",
                    "status": "started",
                    "input": { "path": "src/api.rs" }
                },
                "message_id": message.message_id.0.clone()
            }),
        ),
    );
    assert_eq!(event_response.status, 201);
    let event: AgentTurnEventReport = event_response.body_json().unwrap();
    assert_eq!(event.event.event_type, "tool_call");
    assert_eq!(event.event.message_id, Some(message.message_id.clone()));
    assert_eq!(
        event.event.payload.as_ref().unwrap()["tool"],
        "editor.apply_patch"
    );

    let patch_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/agent/turns/{}/patches", turn.turn.turn_id),
            serde_json::json!({
                "message": "add API file",
                "files": [
                    {
                        "type": "add_text",
                        "path": "src/api.rs",
                        "content": "pub fn api_ready() -> bool { true }\n"
                    }
                ]
            }),
        ),
    );
    assert_eq!(patch_response.status, 200);
    let patch: AgentPatchReport = patch_response.body_json().unwrap();
    assert_eq!(patch.changed_paths.len(), 1);
    assert_eq!(patch.changed_paths[0].path, "src/api.rs");

    let details_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            &format!("/v1/agent/turns/{}", turn.turn.turn_id),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(details_response.status, 200);
    let details: AgentTurnDetails = details_response.body_json().unwrap();
    assert_eq!(details.turn.turn_id, turn.turn.turn_id);
    assert_eq!(details.messages.len(), 2);
    assert_eq!(details.operations.len(), 1);
    assert!(details
        .events
        .iter()
        .any(|item| item.event_type == "tool_call"));
    assert!(details
        .events
        .iter()
        .any(|item| item.change_id.as_ref() == Some(&patch.operation)));

    let end_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/agent/turns/{}/end", turn.turn.turn_id),
            serde_json::json!({ "status": "completed" }),
        ),
    );
    assert_eq!(end_response.status, 200);
    let ended: AgentTurnEndReport = end_response.body_json().unwrap();
    assert_eq!(ended.turn.status, "completed");
    assert_eq!(ended.turn.after_change, Some(patch.operation));

    let diff = db.diff_agent("api-agent", false).unwrap();
    assert!(diff.files.iter().any(|file| file.path == "src/api.rs"));

    let session = db.show_agent_session(&turn.session.session_id).unwrap();
    assert_eq!(session.messages.len(), 2);
    assert_eq!(session.operations.len(), 1);
    assert!(session
        .events
        .iter()
        .any(|event| event.event_type == "turn_ended"));
}

#[test]
fn local_api_and_mcp_patch_payloads_respect_ignore_policy() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.ignore_add("host-secret.txt").unwrap();

    let turn_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/agent/turns",
            serde_json::json!({
                "agent": "privacy-api",
                "branch": "main",
                "session_title": "Privacy policy"
            }),
        ),
    );
    assert_eq!(turn_response.status, 201);
    let turn: AgentTurnStartReport = turn_response.body_json().unwrap();

    let blocked = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/agent/turns/{}/patches", turn.turn.turn_id),
            serde_json::json!({
                "message": "blocked ignored write",
                "files": [
                    {
                        "type": "add_text",
                        "path": "host-secret.txt",
                        "content": "blocked\n"
                    }
                ]
            }),
        ),
    );
    assert_eq!(blocked.status, 400);
    let error: serde_json::Value = blocked.body_json().unwrap();
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("ignored path `host-secret.txt`"));

    let tools = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }),
    )
    .unwrap();
    let apply_patch_schema = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "crabdb.apply_patch")
        .unwrap();
    assert_eq!(
        apply_patch_schema["inputSchema"]["properties"]["allow_ignored"]["type"],
        "boolean"
    );

    let allowed = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "crabdb.apply_patch",
                "arguments": {
                    "turn_id": turn.turn.turn_id,
                    "message": "explicit ignored write",
                    "allow_ignored": true,
                    "files": [
                        {
                            "type": "add_text",
                            "path": "host-secret.txt",
                            "content": "allowed fixture\n"
                        }
                    ]
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(allowed["result"]["isError"], false);
    assert_eq!(
        allowed["result"]["structuredContent"]["changed_paths"][0]["path"],
        "host-secret.txt"
    );

    let details = db.show_agent_turn(&turn.turn.turn_id).unwrap();
    let patch_event = details
        .events
        .iter()
        .find(|event| event.event_type == "patch_applied")
        .unwrap();
    assert_eq!(
        patch_event.payload.as_ref().unwrap()["allow_ignored"],
        serde_json::Value::Bool(true)
    );
}

#[test]
fn local_agent_http_api_manages_agent_branch_lifecycle() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let status_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/status", serde_json::Value::Null),
    );
    assert_eq!(status_response.status, 200);
    let status: serde_json::Value = status_response.body_json().unwrap();
    assert_eq!(status["branch"], "main");

    let spawn_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/agents",
            serde_json::json!({
                "name": "api-branch-agent",
                "from_ref": "main",
                "materialize": true
            }),
        ),
    );
    assert_eq!(spawn_response.status, 201);
    let spawned: serde_json::Value = spawn_response.body_json().unwrap();
    let agent_id = spawned["agent_id"].as_str().unwrap().to_string();
    assert_eq!(spawned["ref_name"], "refs/agents/api-branch-agent");
    let workdir = spawned["workdir"].as_str().unwrap().to_string();

    let agents_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/agents", serde_json::Value::Null),
    );
    assert_eq!(agents_response.status, 200);
    let agents: serde_json::Value = agents_response.body_json().unwrap();
    assert_eq!(agents.as_array().unwrap().len(), 1);
    assert_eq!(agents[0]["record"]["name"], "api-branch-agent");
    assert_eq!(
        agents[0]["branch"]["ref_name"],
        "refs/agents/api-branch-agent"
    );

    let agent_status_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            &format!("/v1/agents/{agent_id}/status"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(agent_status_response.status, 200);
    let agent_status: serde_json::Value = agent_status_response.body_json().unwrap();
    assert_eq!(agent_status["agent"]["record"]["name"], "api-branch-agent");

    let patch_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/agents/{agent_id}/patches"),
            serde_json::json!({
                "message": "add API file",
                "files": [
                    {
                        "type": "add_text",
                        "path": "src/api.rs",
                        "content": "pub fn api() -> bool { true }\n"
                    }
                ]
            }),
        ),
    );
    assert_eq!(patch_response.status, 200);
    let patch: AgentPatchReport = patch_response.body_json().unwrap();
    assert_eq!(patch.agent_id, agent_id);
    assert_eq!(patch.changed_paths[0].path, "src/api.rs");
    assert_eq!(
        fs::read_to_string(std::path::Path::new(&workdir).join("src/api.rs")).unwrap(),
        "pub fn api() -> bool { true }\n"
    );

    fs::write(
        std::path::Path::new(&workdir).join("README.md"),
        "hello\napi dirty\n",
    )
    .unwrap();
    let sync_conflict = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/agents/{agent_id}/sync-workdir"),
            serde_json::json!({}),
        ),
    );
    assert_eq!(sync_conflict.status, 409);

    let sync_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/agents/{agent_id}/sync-workdir"),
            serde_json::json!({ "force": true }),
        ),
    );
    assert_eq!(sync_response.status, 200);
    let synced: serde_json::Value = sync_response.body_json().unwrap();
    assert_eq!(synced["forced"], true);
    assert_eq!(
        fs::read_to_string(std::path::Path::new(&workdir).join("README.md")).unwrap(),
        "hello\n"
    );

    let test_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/agents/{agent_id}/tests"),
            serde_json::json!({
                "command": ["sh", "-c", "printf api-test"],
                "timeout_secs": 5
            }),
        ),
    );
    assert_eq!(test_response.status, 200);
    let test: serde_json::Value = test_response.body_json().unwrap();
    assert_eq!(test["success"], true);
    assert_eq!(test["stdout_preview"], "api-test");

    let diff_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            &format!("/v1/agents/{agent_id}/diff?patch=true"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(diff_response.status, 200);
    let diff: serde_json::Value = diff_response.body_json().unwrap();
    assert_eq!(diff["files"][0]["path"], "src/api.rs");
    assert!(diff["files"][0]["patch"]
        .as_str()
        .unwrap()
        .contains("api()"));

    let contribution_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            &format!("/v1/agents/{agent_id}/contribution?limit=5"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(contribution_response.status, 200);
    let contribution: serde_json::Value = contribution_response.body_json().unwrap();
    assert_eq!(
        contribution["status"]["agent"]["record"]["name"],
        "api-branch-agent"
    );
    assert!(contribution["status"]["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "src/api.rs"));
    assert!(contribution["operations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|operation| operation["message"] == "add API file"));
    assert_eq!(contribution["status"]["latest_test"]["success"], true);
    assert!(contribution["recent_events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["event_type"] == "test_finished"));

    let cli_contribution = run_crabdb_json(
        temp.path(),
        &["agent", "contribution", "api-branch-agent", "--limit", "5"],
    );
    assert_eq!(
        cli_contribution["status"]["agent"]["record"]["agent_id"],
        agent_id
    );

    let readiness_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            &format!("/v1/agents/{agent_id}/readiness"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(readiness_response.status, 200);
    let readiness: serde_json::Value = readiness_response.body_json().unwrap();
    assert_eq!(readiness["agent"]["record"]["name"], "api-branch-agent");
    assert_eq!(readiness["ready"], true);
    assert!(readiness["blockers"].as_array().unwrap().is_empty());
    assert_eq!(readiness["latest_test"]["success"], true);
    assert!(readiness["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue["code"] == "missing_latest_eval"));

    let cli_readiness = run_crabdb_json(temp.path(), &["agent", "readiness", "api-branch-agent"]);
    assert_eq!(cli_readiness["agent"]["record"]["agent_id"], agent_id);
    assert_eq!(cli_readiness["ready"], true);

    let handoff_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            &format!("/v1/agents/{agent_id}/handoff?limit=5"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(handoff_response.status, 200);
    let handoff: serde_json::Value = handoff_response.body_json().unwrap();
    assert_eq!(handoff["agent"]["record"]["name"], "api-branch-agent");
    assert_eq!(handoff["readiness"]["ready"], true);
    assert!(handoff["current_session"].is_null());
    assert!(handoff["recent_operations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|operation| operation["message"] == "add API file"));
    assert!(handoff["recent_events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["event_type"] == "test_finished"));
    assert!(handoff["next_steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|step| step.as_str().unwrap().contains("Start a new session")));

    let cli_handoff = run_crabdb_json(
        temp.path(),
        &["agent", "handoff", "api-branch-agent", "--limit", "5"],
    );
    assert_eq!(cli_handoff["agent"]["record"]["agent_id"], agent_id);
    assert_eq!(cli_handoff["readiness"]["ready"], true);

    let remove_dirty_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "DELETE",
            &format!("/v1/agents/{agent_id}"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(remove_dirty_response.status, 400);

    let merge_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/branches/main/merge-agent",
            serde_json::json!({
                "agent_id": agent_id,
                "strategy": "line_id_aware"
            }),
        ),
    );
    assert_eq!(merge_response.status, 200);
    let merge: serde_json::Value = merge_response.body_json().unwrap();
    assert_eq!(merge["source_ref"], "refs/agents/api-branch-agent");
    assert_eq!(merge["target_ref"], "refs/branches/main");
    assert!(merge["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "src/api.rs"));

    let why = db.why("src/api.rs:1", Some("main")).unwrap();
    assert_eq!(why.current_text, "pub fn api() -> bool { true }");

    let remove_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "DELETE",
            &format!("/v1/agents/{agent_id}"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(remove_response.status, 200);
    let removed: serde_json::Value = remove_response.body_json().unwrap();
    assert_eq!(removed["agent_id"], agent_id);
    assert_eq!(removed["forced"], false);
    assert_eq!(removed["removed_workdir"], workdir);
    assert!(!std::path::Path::new(&workdir).exists());
    assert_eq!(
        db.agent_details(&agent_id).unwrap().branch.status,
        "removed"
    );
}

#[test]
fn local_agent_http_api_can_require_bearer_token() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let auth = crabdb::server::ServerAuth::bearer("secret-token").unwrap();

    let health = crabdb::server::handle_http_request_with_auth(
        &mut db,
        &api_request("GET", "/v1/health", serde_json::Value::Null),
        &auth,
    );
    assert_eq!(health.status, 200);

    let missing = crabdb::server::handle_http_request_with_auth(
        &mut db,
        &api_request(
            "POST",
            "/v1/agent/turns",
            serde_json::json!({ "agent": "secure-agent", "branch": "main" }),
        ),
        &auth,
    );
    assert_eq!(missing.status, 401);

    let invalid = crabdb::server::handle_http_request_with_auth(
        &mut db,
        &api_request_with_headers(
            "POST",
            "/v1/agent/turns",
            &[("Authorization", "Bearer wrong-token")],
            serde_json::json!({ "agent": "secure-agent", "branch": "main" }),
        ),
        &auth,
    );
    assert_eq!(invalid.status, 401);

    let ok = crabdb::server::handle_http_request_with_auth(
        &mut db,
        &api_request_with_headers(
            "POST",
            "/v1/agent/turns",
            &[("Authorization", "Bearer secret-token")],
            serde_json::json!({ "agent": "secure-agent", "branch": "main" }),
        ),
        &auth,
    );
    assert_eq!(ok.status, 201);
    let turn: AgentTurnStartReport = ok.body_json().unwrap();
    assert!(turn.turn.agent_id.starts_with("agent_"));

    let second = crabdb::server::handle_http_request_with_auth(
        &mut db,
        &api_request_with_headers(
            "POST",
            "/v1/agent/turns",
            &[("X-CrabDB-Token", "secret-token")],
            serde_json::json!({ "agent": "other-secure-agent", "branch": "main" }),
        ),
        &auth,
    );
    assert_eq!(second.status, 201);
}

#[test]
fn local_api_and_cli_export_openapi_contract() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let cli = run_crabdb_json(temp.path(), &["api", "openapi"]);
    assert_eq!(cli["openapi"], "3.1.0");
    assert!(cli["paths"].get("/v1/openapi.json").is_some());
    assert!(cli["paths"]["/v1/agents"]["get"].is_object());
    assert!(cli["paths"]["/v1/agents/{agent_or_id}"]["delete"].is_object());
    assert!(cli["paths"].get("/v1/agent/events").is_some());
    assert!(cli["paths"].get("/v1/agent/spans").is_some());
    assert!(cli["paths"]
        .get("/v1/agent/turns/{turn_id}/spans")
        .is_some());
    assert_eq!(
        cli["paths"]["/v1/health"]["get"]["security"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert!(cli["components"]["securitySchemes"]["bearerAuth"].is_object());

    let output_path = temp.path().join("crabdb.openapi.json");
    let output = Command::new(crabdb_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("api")
        .arg("openapi")
        .arg("--output")
        .arg(&output_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "crabdb api openapi --output failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let exported: serde_json::Value =
        serde_json::from_slice(&fs::read(&output_path).unwrap()).unwrap();
    assert_eq!(exported["info"]["title"], "CrabDB Local API");

    let mut db = CrabDb::open(temp.path()).unwrap();
    let response = crabdb::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/openapi.json", serde_json::Value::Null),
    );
    assert_eq!(response.status, 200);
    let api: serde_json::Value = response.body_json().unwrap();
    assert!(api["paths"]["/v1/agent/turns/{turn_id}/patches"]["post"]["requestBody"].is_object());

    let auth = crabdb::server::ServerAuth::bearer("secret-token").unwrap();
    let missing = crabdb::server::handle_http_request_with_auth(
        &mut db,
        &api_request("GET", "/v1/openapi.json", serde_json::Value::Null),
        &auth,
    );
    assert_eq!(missing.status, 401);

    let ok = crabdb::server::handle_http_request_with_auth(
        &mut db,
        &api_request_with_headers(
            "GET",
            "/v1/openapi.json",
            &[("Authorization", "Bearer secret-token")],
            serde_json::Value::Null,
        ),
        &auth,
    );
    assert_eq!(ok.status, 200);
}

#[test]
fn agent_turn_cli_tracks_events_and_closeout() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let started = run_crabdb_json(
        temp.path(),
        &[
            "agent",
            "turn",
            "start",
            "cli-agent",
            "--from",
            "main",
            "--title",
            "CLI turn",
        ],
    );
    let turn_id = started["turn"]["turn_id"].as_str().unwrap().to_string();
    assert_eq!(started["session"]["title"], "CLI turn");

    let message = run_crabdb_json(
        temp.path(),
        &[
            "agent",
            "turn",
            "message",
            &turn_id,
            "--role",
            "user",
            "--text",
            "Add a CLI turn note",
        ],
    );
    assert_eq!(message["role"], "user");
    assert_eq!(message["session_id"], started["session"]["session_id"]);

    let event = run_crabdb_json(
        temp.path(),
        &[
            "agent",
            "turn",
            "event",
            &turn_id,
            "--event-type",
            "tool_call",
            "--payload-json",
            r#"{"tool":"cli.apply_patch","status":"planned"}"#,
        ],
    );
    assert_eq!(event["event"]["event_type"], "tool_call");
    assert_eq!(event["event"]["payload"]["tool"], "cli.apply_patch");

    let patch_path = temp.path().join("turn-patch.json");
    fs::write(
        &patch_path,
        r#"{
          "message": "add CLI turn note",
          "edits": [
            { "op": "write", "path": "cli-turn.md", "content": "tracked by turn\n", "executable": false }
          ]
        }"#,
    )
    .unwrap();
    let patch = run_crabdb_json(
        temp.path(),
        &[
            "agent",
            "turn",
            "apply-patch",
            &turn_id,
            "--patch",
            patch_path.to_str().unwrap(),
        ],
    );
    assert_eq!(patch["changed_paths"][0]["path"], "cli-turn.md");

    let details = run_crabdb_json(temp.path(), &["agent", "turn", "show", &turn_id]);
    assert_eq!(details["turn"]["status"], "patch_applied");
    assert_eq!(details["messages"][0]["body"], "Add a CLI turn note");
    assert!(details["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["event_type"] == "tool_call"));
    assert!(details["operations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|operation| operation["message"] == "add CLI turn note"));

    let ended = run_crabdb_json(
        temp.path(),
        &["agent", "turn", "end", &turn_id, "--status", "completed"],
    );
    assert_eq!(ended["turn"]["status"], "completed");

    let details = run_crabdb_json(temp.path(), &["agent", "turn", "show", &turn_id]);
    assert_eq!(details["turn"]["status"], "completed");
}

#[test]
fn mcp_stdio_tools_drive_agent_turn_workflow() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let init = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        }),
    )
    .unwrap();
    assert_eq!(init["result"]["serverInfo"]["name"], "crabdb");
    assert!(init["result"]["capabilities"]["tools"].is_object());
    assert!(init["result"]["capabilities"]["resources"].is_object());
    assert!(init["result"]["capabilities"]["prompts"].is_object());
    assert!(init["result"]["capabilities"]["completions"].is_object());

    let resources = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "resources/list",
            "params": {}
        }),
    )
    .unwrap();
    let resources_list = resources["result"]["resources"].as_array().unwrap();
    assert!(resources_list
        .iter()
        .any(|resource| resource["uri"] == "crabdb://workspace/status"));
    assert!(resources_list
        .iter()
        .any(|resource| resource["uri"] == "crabdb://docs/agent-workflows"));

    let resource_templates = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 17,
            "method": "resources/templates/list",
            "params": {}
        }),
    )
    .unwrap();
    let template_list = resource_templates["result"]["resourceTemplates"]
        .as_array()
        .unwrap();
    assert!(template_list
        .iter()
        .any(|template| template["uriTemplate"] == "crabdb://workspace/agents/{agent}/status"));
    assert!(template_list.iter().any(
        |template| template["uriTemplate"] == "crabdb://workspace/agents/{agent}/contribution"
    ));
    assert!(template_list
        .iter()
        .any(|template| template["uriTemplate"] == "crabdb://workspace/agents/{agent}/gates"));
    assert!(template_list
        .iter()
        .any(|template| template["uriTemplate"] == "crabdb://workspace/agents/{agent}/readiness"));
    assert!(template_list
        .iter()
        .any(|template| template["uriTemplate"] == "crabdb://workspace/agents/{agent}/handoff"));
    assert!(template_list
        .iter()
        .any(|template| template["uriTemplate"] == "crabdb://workspace/turns/{turn_id}"));

    let status_resource = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "resources/read",
            "params": {
                "uri": "crabdb://workspace/status"
            }
        }),
    )
    .unwrap();
    assert_eq!(
        status_resource["result"]["contents"][0]["mimeType"],
        "application/json"
    );
    let status_text = status_resource["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let status_json: serde_json::Value = serde_json::from_str(status_text).unwrap();
    assert_eq!(status_json["branch"], "main");

    let docs_resource = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 12,
            "method": "resources/read",
            "params": {
                "uri": "crabdb://docs/agent-workflows"
            }
        }),
    )
    .unwrap();
    assert_eq!(
        docs_resource["result"]["contents"][0]["mimeType"],
        "text/markdown"
    );
    assert!(docs_resource["result"]["contents"][0]["text"]
        .as_str()
        .unwrap()
        .contains("Agent Workflows"));

    let missing_resource = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 13,
            "method": "resources/read",
            "params": {
                "uri": "crabdb://workspace/missing"
            }
        }),
    )
    .unwrap();
    assert_eq!(missing_resource["error"]["code"], -32002);

    let prompts = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 14,
            "method": "prompts/list",
            "params": {}
        }),
    )
    .unwrap();
    let prompt_list = prompts["result"]["prompts"].as_array().unwrap();
    assert!(prompt_list
        .iter()
        .any(|prompt| prompt["name"] == "crabdb.agent_task"));
    assert!(prompt_list
        .iter()
        .any(|prompt| prompt["name"] == "crabdb.resolve_conflict"));

    let agent_prompt = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 15,
            "method": "prompts/get",
            "params": {
                "name": "crabdb.agent_task",
                "arguments": {
                    "agent": "mcp-agent",
                    "task": "Improve README setup notes",
                    "branch": "main"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_prompt["result"]["description"],
        "Safe CrabDB agent task workflow"
    );
    let prompt_messages = agent_prompt["result"]["messages"].as_array().unwrap();
    assert!(prompt_messages[0]["content"]["text"]
        .as_str()
        .unwrap()
        .contains("crabdb.begin_turn"));
    assert!(prompt_messages[0]["content"]["text"]
        .as_str()
        .unwrap()
        .contains("mcp-agent"));
    assert!(prompt_messages
        .iter()
        .any(|message| message["content"]["type"] == "resource"));

    let missing_prompt_argument = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 16,
            "method": "prompts/get",
            "params": {
                "name": "crabdb.resolve_conflict",
                "arguments": {}
            }
        }),
    )
    .unwrap();
    assert_eq!(missing_prompt_argument["error"]["code"], -32602);

    let branch_completion = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 18,
            "method": "completion/complete",
            "params": {
                "ref": {
                    "type": "ref/prompt",
                    "name": "crabdb.agent_task"
                },
                "argument": {
                    "name": "branch",
                    "value": "m"
                }
            }
        }),
    )
    .unwrap();
    assert!(branch_completion["result"]["completion"]["values"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value.as_str() == Some("main")));

    let missing_completion_prompt = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 19,
            "method": "completion/complete",
            "params": {
                "ref": {
                    "type": "ref/prompt",
                    "name": "crabdb.missing"
                },
                "argument": {
                    "name": "agent",
                    "value": ""
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(missing_completion_prompt["error"]["code"], -32602);

    let list = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    )
    .unwrap();
    let tools = list["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|tool| tool["name"] == "crabdb.begin_turn"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "crabdb.agent_spawn"));
    assert!(tools.iter().any(|tool| tool["name"] == "crabdb.agent_list"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "crabdb.agent_contribution"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "crabdb.gate_history"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "crabdb.agent_readiness"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "crabdb.agent_handoff"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "crabdb.guardrail_check"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "crabdb.agent_remove"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "crabdb.apply_patch"));
    assert!(tools.iter().any(|tool| tool["name"] == "crabdb.run_test"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "crabdb.sync_workdir"));
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        for key in [
            "readOnlyHint",
            "destructiveHint",
            "idempotentHint",
            "openWorldHint",
        ] {
            assert!(
                tool["annotations"][key].is_boolean(),
                "tool {name} missing {key}"
            );
        }
    }
    let tool_annotation = |name: &str, key: &str| {
        tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("{name} not found"))["annotations"][key]
            .as_bool()
            .unwrap()
    };
    assert!(tool_annotation("crabdb.status", "readOnlyHint"));
    assert!(!tool_annotation("crabdb.status", "destructiveHint"));
    assert!(!tool_annotation("crabdb.apply_patch", "readOnlyHint"));
    assert!(tool_annotation("crabdb.apply_patch", "destructiveHint"));
    assert!(tool_annotation("crabdb.run_test", "openWorldHint"));
    assert!(tool_annotation("crabdb.guardrail_check", "readOnlyHint"));
    assert!(tool_annotation("crabdb.gate_history", "readOnlyHint"));

    let spawned = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 20,
            "method": "tools/call",
            "params": {
                "name": "crabdb.agent_spawn",
                "arguments": {
                    "name": "mcp-lifecycle",
                    "from_ref": "main",
                    "materialize": false
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(spawned["result"]["isError"], false);
    let lifecycle_agent_id = spawned["result"]["structuredContent"]["agent_id"]
        .as_str()
        .unwrap()
        .to_string();

    let agent_list = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 21,
            "method": "tools/call",
            "params": {
                "name": "crabdb.agent_list",
                "arguments": {}
            }
        }),
    )
    .unwrap();
    assert_eq!(agent_list["result"]["isError"], false);
    assert!(agent_list["result"]["structuredContent"]
        .as_array()
        .unwrap()
        .iter()
        .any(|agent| agent["record"]["name"] == "mcp-lifecycle"));

    let agent_show = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 22,
            "method": "tools/call",
            "params": {
                "name": "crabdb.agent_show",
                "arguments": {
                    "agent": lifecycle_agent_id.clone()
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(agent_show["result"]["isError"], false);
    assert_eq!(
        agent_show["result"]["structuredContent"]["record"]["name"],
        "mcp-lifecycle"
    );

    let agent_status = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 23,
            "method": "tools/call",
            "params": {
                "name": "crabdb.agent_status",
                "arguments": {
                    "agent": lifecycle_agent_id.clone()
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(agent_status["result"]["isError"], false);
    assert_eq!(
        agent_status["result"]["structuredContent"]["agent"]["record"]["name"],
        "mcp-lifecycle"
    );

    let templated_agent_status = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 25,
            "method": "resources/read",
            "params": {
                "uri": "crabdb://workspace/agents/mcp-lifecycle/status"
            }
        }),
    )
    .unwrap();
    assert_eq!(
        templated_agent_status["result"]["contents"][0]["mimeType"],
        "application/json"
    );
    let templated_status_text = templated_agent_status["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let templated_status_json: serde_json::Value =
        serde_json::from_str(templated_status_text).unwrap();
    assert_eq!(
        templated_status_json["agent"]["record"]["name"],
        "mcp-lifecycle"
    );

    let contribution = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 26,
            "method": "tools/call",
            "params": {
                "name": "crabdb.agent_contribution",
                "arguments": {
                    "agent": "mcp-lifecycle",
                    "limit": 5
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(contribution["result"]["isError"], false);
    assert_eq!(
        contribution["result"]["structuredContent"]["status"]["agent"]["record"]["name"],
        "mcp-lifecycle"
    );

    let templated_contribution = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 27,
            "method": "resources/read",
            "params": {
                "uri": "crabdb://workspace/agents/mcp-lifecycle/contribution"
            }
        }),
    )
    .unwrap();
    let templated_contribution_text = templated_contribution["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let templated_contribution_json: serde_json::Value =
        serde_json::from_str(templated_contribution_text).unwrap();
    assert_eq!(
        templated_contribution_json["status"]["agent"]["record"]["name"],
        "mcp-lifecycle"
    );

    let readiness = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 29,
            "method": "tools/call",
            "params": {
                "name": "crabdb.agent_readiness",
                "arguments": {
                    "agent": "mcp-lifecycle"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(readiness["result"]["isError"], false);
    assert_eq!(
        readiness["result"]["structuredContent"]["agent"]["record"]["name"],
        "mcp-lifecycle"
    );
    assert_eq!(readiness["result"]["structuredContent"]["ready"], true);
    assert!(readiness["result"]["structuredContent"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue["code"] == "missing_latest_test"));

    let templated_readiness = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 30,
            "method": "resources/read",
            "params": {
                "uri": "crabdb://workspace/agents/mcp-lifecycle/readiness"
            }
        }),
    )
    .unwrap();
    let templated_readiness_text = templated_readiness["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let templated_readiness_json: serde_json::Value =
        serde_json::from_str(templated_readiness_text).unwrap();
    assert_eq!(
        templated_readiness_json["agent"]["record"]["name"],
        "mcp-lifecycle"
    );
    assert_eq!(templated_readiness_json["ready"], true);

    let handoff = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 31,
            "method": "tools/call",
            "params": {
                "name": "crabdb.agent_handoff",
                "arguments": {
                    "agent": "mcp-lifecycle",
                    "limit": 5
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(handoff["result"]["isError"], false);
    assert_eq!(
        handoff["result"]["structuredContent"]["agent"]["record"]["name"],
        "mcp-lifecycle"
    );
    assert_eq!(
        handoff["result"]["structuredContent"]["readiness"]["ready"],
        true
    );
    assert!(handoff["result"]["structuredContent"]["current_session"].is_null());
    assert!(handoff["result"]["structuredContent"]["next_steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|step| step.as_str().unwrap().contains("Start a new session")));

    let templated_handoff = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 32,
            "method": "resources/read",
            "params": {
                "uri": "crabdb://workspace/agents/mcp-lifecycle/handoff"
            }
        }),
    )
    .unwrap();
    let templated_handoff_text = templated_handoff["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let templated_handoff_json: serde_json::Value =
        serde_json::from_str(templated_handoff_text).unwrap();
    assert_eq!(
        templated_handoff_json["agent"]["record"]["name"],
        "mcp-lifecycle"
    );
    assert_eq!(templated_handoff_json["readiness"]["ready"], true);

    let agent_completion = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 28,
            "method": "completion/complete",
            "params": {
                "ref": {
                    "type": "ref/resource",
                    "uri": "crabdb://workspace/agents/{agent}/handoff"
                },
                "argument": {
                    "name": "agent",
                    "value": "mcp"
                }
            }
        }),
    )
    .unwrap();
    assert!(agent_completion["result"]["completion"]["values"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value.as_str() == Some("mcp-lifecycle")));

    let agent_remove = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 24,
            "method": "tools/call",
            "params": {
                "name": "crabdb.agent_remove",
                "arguments": {
                    "agent": lifecycle_agent_id.clone()
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(agent_remove["result"]["isError"], false);
    assert_eq!(
        agent_remove["result"]["structuredContent"]["agent_id"],
        lifecycle_agent_id
    );

    let begin = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "crabdb.begin_turn",
                "arguments": {
                    "agent": "mcp-agent",
                    "branch": "main",
                    "session_title": "MCP smoke"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(begin["result"]["isError"], false);
    let turn_id = begin["result"]["structuredContent"]["turn"]["turn_id"]
        .as_str()
        .unwrap()
        .to_string();

    let templated_turn = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 31,
            "method": "resources/read",
            "params": {
                "uri": format!("crabdb://workspace/turns/{turn_id}")
            }
        }),
    )
    .unwrap();
    let templated_turn_text = templated_turn["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let templated_turn_json: serde_json::Value = serde_json::from_str(templated_turn_text).unwrap();
    assert_eq!(templated_turn_json["turn"]["turn_id"], turn_id);

    let turn_completion = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 32,
            "method": "completion/complete",
            "params": {
                "ref": {
                    "type": "ref/resource",
                    "uri": "crabdb://workspace/turns/{turn_id}"
                },
                "argument": {
                    "name": "turn_id",
                    "value": "turn_"
                }
            }
        }),
    )
    .unwrap();
    assert!(turn_completion["result"]["completion"]["values"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value.as_str() == Some(turn_id.as_str())));

    let event = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "crabdb.add_event",
                "arguments": {
                    "turn_id": turn_id.clone(),
                    "event_type": "tool_call",
                    "payload": {
                        "tool": "crabdb.apply_patch",
                        "status": "planned"
                    }
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(event["result"]["isError"], false);

    let patch = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "crabdb.apply_patch",
                "arguments": {
                    "turn_id": turn_id.clone(),
                    "message": "add MCP file",
                    "files": [
                        {
                            "type": "add_text",
                            "path": "src/mcp_smoke.rs",
                            "content": "pub fn mcp_ready() -> bool { true }\n"
                        }
                    ]
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(patch["result"]["isError"], false);
    assert_eq!(
        patch["result"]["structuredContent"]["changed_paths"][0]["path"],
        "src/mcp_smoke.rs"
    );

    let workdir = db.agent_workdir("mcp-agent").unwrap().workdir.unwrap();
    fs::write(
        std::path::Path::new(&workdir).join("README.md"),
        "hello\nmcp dirty\n",
    )
    .unwrap();
    let sync = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "crabdb.sync_workdir",
                "arguments": {
                    "agent": "mcp-agent",
                    "force": true
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(sync["result"]["isError"], false);
    assert_eq!(sync["result"]["structuredContent"]["forced"], true);

    let test = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "crabdb.run_test",
                "arguments": {
                    "agent": "mcp-agent",
                    "turn_id": turn_id.clone(),
                    "command": ["sh", "-c", "printf mcp-test"],
                    "timeout_secs": 5
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(test["result"]["isError"], false);
    assert_eq!(test["result"]["structuredContent"]["success"], true);
    assert_eq!(
        test["result"]["structuredContent"]["stdout_preview"],
        "mcp-test"
    );

    let show = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 8,
            "method": "tools/call",
            "params": {
                "name": "crabdb.show_turn",
                "arguments": {
                    "turn_id": turn_id.clone()
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(show["result"]["isError"], false);
    assert!(show["result"]["structuredContent"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["event_type"] == "tool_call"));
    assert_eq!(
        show["result"]["structuredContent"]["operations"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(show["result"]["structuredContent"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["event_type"] == "test_finished"));
    assert_eq!(
        db.agent_status("mcp-agent")
            .unwrap()
            .latest_test
            .unwrap()
            .status,
        "test_passed"
    );

    let active_handoff = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 33,
            "method": "tools/call",
            "params": {
                "name": "crabdb.agent_handoff",
                "arguments": {
                    "agent": "mcp-agent",
                    "limit": 5
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(active_handoff["result"]["isError"], false);
    assert!(
        active_handoff["result"]["structuredContent"]["current_session"]["turns"]
            .as_array()
            .unwrap()
            .iter()
            .any(|turn| turn["turn_id"] == turn_id)
    );
    assert!(
        active_handoff["result"]["structuredContent"]["recent_events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["event_type"] == "test_finished")
    );
    assert!(active_handoff["result"]["structuredContent"]["next_steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|step| step.as_str().unwrap().contains("active session")));

    let mut stdio_db = CrabDb::open(temp.path()).unwrap();
    let mut output = Vec::new();
    crabdb::mcp::serve_stdio(
        &mut stdio_db,
        std::io::Cursor::new(
            br#"{"jsonrpc":"2.0","id":7,"method":"tools/list","params":{}}
"#,
        ),
        &mut output,
    )
    .unwrap();
    let response: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(response["id"], 7);
    assert!(response["result"]["tools"].is_array());
}

#[test]
fn config_api_lists_sets_persists_and_validates_keys() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let entries = db.config_entries();
    assert!(entries
        .iter()
        .any(|entry| entry.key == "workspace.id" && entry.read_only));
    assert_eq!(
        db.config_get("recording.ignore_gitignored").unwrap().value,
        "true"
    );

    let http_list = crabdb::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/config", serde_json::Value::Null),
    );
    assert_eq!(http_list.status, 200);
    let http_entries: serde_json::Value = http_list.body_json().unwrap();
    assert!(http_entries
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["key"] == "agent.default_materialize"));
    assert!(http_entries
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["key"] == "agent.require_test_gate"));
    assert!(http_entries
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["key"] == "agent.require_eval_gate"));
    assert!(http_entries
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["key"] == "agent.required_test_suites"));
    assert!(http_entries
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["key"] == "agent.required_eval_suites"));
    assert!(http_entries
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["key"] == "guardrails.policy"));

    let http_get = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/config/text.preserve_similarity",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(http_get.status, 200);
    let http_entry: serde_json::Value = http_get.body_json().unwrap();
    assert_eq!(http_entry["key"], "text.preserve_similarity");

    let http_set = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/config",
            serde_json::json!({
                "key": "text.preserve_similarity",
                "value": "0.55"
            }),
        ),
    );
    assert_eq!(http_set.status, 200);
    let http_set_report: serde_json::Value = http_set.body_json().unwrap();
    assert_eq!(http_set_report["key"], "text.preserve_similarity");
    assert_eq!(
        db.config_get("text.preserve_similarity").unwrap().value,
        "0.55"
    );

    let mcp_tools = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }),
    )
    .unwrap();
    let tools = mcp_tools["result"]["tools"].as_array().unwrap();
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "crabdb.config_list"));
    assert!(tools.iter().any(|tool| tool["name"] == "crabdb.config_get"));
    assert!(tools.iter().any(|tool| tool["name"] == "crabdb.config_set"));

    let mcp_get = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "crabdb.config_get",
                "arguments": { "key": "text.preserve_similarity" }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_get["result"]["isError"], false);
    assert_eq!(mcp_get["result"]["structuredContent"]["value"], "0.55");

    let mcp_set = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "crabdb.config_set",
                "arguments": {
                    "key": "text.preserve_similarity",
                    "value": "0.45"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_set["result"]["isError"], false);
    assert_eq!(
        db.config_get("text.preserve_similarity").unwrap().value,
        "0.45"
    );

    let guardrail_policy = "allow:action:shell.exec; block:keyword:production";
    let policy_set = db
        .config_set("guardrails.policy", guardrail_policy)
        .unwrap();
    assert_eq!(policy_set.new_value, guardrail_policy);
    let test_gate_set = db.config_set("agent.require_test_gate", "yes").unwrap();
    assert_eq!(test_gate_set.old_value, "false");
    assert_eq!(test_gate_set.new_value, "true");
    let eval_gate_set = db.config_set("agent.require_eval_gate", "on").unwrap();
    assert_eq!(eval_gate_set.old_value, "false");
    assert_eq!(eval_gate_set.new_value, "true");
    let test_suites_set = db
        .config_set("agent.required_test_suites", "unit,policy-smoke")
        .unwrap();
    assert_eq!(test_suites_set.old_value, "");
    assert_eq!(test_suites_set.new_value, "unit,policy-smoke");
    let eval_suites_set = db
        .config_set("agent.required_eval_suites", "regression; safety")
        .unwrap();
    assert_eq!(eval_suites_set.old_value, "");
    assert_eq!(eval_suites_set.new_value, "regression,safety");
    let allowed_shell = db
        .guardrail_check(
            None,
            "shell.exec",
            Some("Run local test command"),
            None,
            &Vec::new(),
        )
        .unwrap();
    assert_eq!(allowed_shell.decision, "allowed");
    assert!(allowed_shell
        .reasons
        .iter()
        .any(|reason| reason.code == "policy_allow"));
    let blocked_production = db
        .guardrail_check(
            None,
            "file.write",
            Some("touch production release marker"),
            None,
            &Vec::new(),
        )
        .unwrap();
    assert_eq!(blocked_production.decision, "blocked");
    assert!(blocked_production
        .reasons
        .iter()
        .any(|reason| reason.code == "policy_block"));

    let set = db.config_set("recording.ignore_gitignored", "off").unwrap();
    assert_eq!(set.old_value, "true");
    assert_eq!(set.new_value, "false");
    assert_eq!(
        db.config_get("recording.ignore_gitignored").unwrap().value,
        "false"
    );

    drop(db);
    let mut reopened = CrabDb::open(temp.path()).unwrap();
    assert!(!reopened.config().recording.ignore_gitignored);
    assert_eq!(
        reopened.config_get("guardrails.policy").unwrap().value,
        guardrail_policy
    );
    assert!(reopened.config().agent.require_test_gate);
    assert!(reopened.config().agent.require_eval_gate);
    assert_eq!(
        reopened.config().agent.required_test_suites,
        vec!["unit".to_string(), "policy-smoke".to_string()]
    );
    assert_eq!(
        reopened.config().agent.required_eval_suites,
        vec!["regression".to_string(), "safety".to_string()]
    );

    let err = reopened
        .config_set("recording.ignore_gitignored", "sometimes")
        .unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)));

    let err = reopened
        .config_set("guardrails.policy", "maybe:keyword:prod")
        .unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)));

    let err = reopened
        .config_set("workspace.id", "workspace_other")
        .unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)));

    reopened.create_branch("dev", Some("main")).unwrap();
    let set = reopened
        .config_set("workspace.default_branch", "dev")
        .unwrap();
    assert_eq!(set.new_value, "dev");
    drop(reopened);

    let mut reopened = CrabDb::open(temp.path()).unwrap();
    assert_eq!(reopened.config().workspace.default_branch, "dev");
    let err = reopened
        .config_set("workspace.default_branch", "missing")
        .unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)));
}

#[test]
fn git_import_update_records_current_git_tracked_snapshot() {
    if !git_available() {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    run_git(temp.path(), &["init"]);
    fs::write(temp.path().join("README.md"), "one\ntwo\n").unwrap();
    run_git(temp.path(), &["add", "README.md"]);
    let init = CrabDb::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let mappings = db.git_mappings(10).unwrap();
    assert_eq!(mappings.len(), 1);
    assert_eq!(mappings[0].direction, "import");
    assert_eq!(mappings[0].branch, "main");
    assert_eq!(mappings[0].crab_change, init.operation);
    assert_eq!(mappings[0].crab_root, init.root_id);
    assert!(mappings[0].git_dirty);

    let before = db.why("README.md:2", Some("main")).unwrap();
    fs::write(temp.path().join("README.md"), "one\nTWO\n").unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/lib.rs"), "pub fn added() {}\n").unwrap();
    fs::write(temp.path().join("scratch.txt"), "untracked\n").unwrap();
    run_git(temp.path(), &["add", "src/lib.rs"]);

    let report = db
        .git_import_update(Some("main"), Some("sync git index".to_string()))
        .unwrap();
    assert!(report.operation.is_some());
    let imported_change = report.operation.clone().unwrap();
    let mapping = report.mapping.as_ref().unwrap();
    assert_eq!(mapping.direction, "import");
    assert_eq!(mapping.crab_change, imported_change);
    assert_eq!(mapping.crab_root, report.root_id);
    assert!(mapping.git_dirty);
    assert_eq!(report.imported.files, 2);
    assert!(report
        .changed_paths
        .iter()
        .any(|path| path.path == "README.md" && path.kind == crabdb::FileChangeKind::Modified));
    assert!(report
        .changed_paths
        .iter()
        .any(|path| path.path == "src/lib.rs" && path.kind == crabdb::FileChangeKind::Added));
    assert!(!report
        .changed_paths
        .iter()
        .any(|path| path.path == "scratch.txt"));

    let after = db.why("README.md:2", Some("main")).unwrap();
    assert_eq!(after.current_text, "TWO");
    assert_eq!(after.line_id, before.line_id);

    let shown = db.show(&imported_change.0).unwrap();
    match shown {
        ShowResult::Operation { value } => {
            assert_eq!(value.operation.kind, crabdb::OperationKind::GitImport);
            assert_eq!(value.operation.message.as_deref(), Some("sync git index"));
        }
        other => panic!("expected operation, got {other:?}"),
    }

    let root = db.inspect_root(&report.root_id.0).unwrap();
    assert!(root.files.iter().any(|file| file.path == "README.md"));
    assert!(root.files.iter().any(|file| file.path == "src/lib.rs"));
    assert!(!root.files.iter().any(|file| file.path == "scratch.txt"));

    let no_change = db.git_import_update(Some("main"), None).unwrap();
    assert!(no_change.operation.is_none());
    assert!(no_change.mapping.is_none());
    assert!(no_change.changed_paths.is_empty());

    let mappings = db.git_mappings(10).unwrap();
    assert_eq!(mappings.len(), 2);
    assert_eq!(mappings[0].crab_change, imported_change);
}

#[cfg(unix)]
#[test]
fn git_import_skips_tracked_symlinks_without_false_dirty_status() {
    if !git_available() {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    run_git(temp.path(), &["init"]);
    fs::write(temp.path().join("target.md"), "real target\n").unwrap();
    std::os::unix::fs::symlink("target.md", temp.path().join("link.md")).unwrap();
    run_git(temp.path(), &["add", "target.md", "link.md"]);

    let init = CrabDb::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();
    assert_eq!(init.imported.files, 1);
    assert_eq!(init.imported.text, 1);

    let db = CrabDb::open(temp.path()).unwrap();
    let status = db.status(Some("main")).unwrap();
    assert_eq!(status.worktree_state, WorktreeState::Clean);
    assert!(status.changed_paths.is_empty());

    let root = db.inspect_root(&init.root_id.0).unwrap();
    assert!(root.files.iter().any(|file| file.path == "target.md"));
    assert!(!root.files.iter().any(|file| file.path == "link.md"));
}

#[test]
fn git_export_with_message_creates_commit_object_and_mapping() {
    if !git_available() {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    run_git(temp.path(), &["init"]);
    run_git(
        temp.path(),
        &["config", "user.email", "crabdb@example.test"],
    );
    run_git(temp.path(), &["config", "user.name", "CrabDB Test"]);
    fs::write(temp.path().join("README.md"), "one\ntwo\n").unwrap();
    run_git(temp.path(), &["add", "README.md"]);
    run_git(temp.path(), &["commit", "-m", "initial"]);
    let git_head = git_output(temp.path(), &["rev-parse", "HEAD"]);

    let init = CrabDb::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();
    fs::write(temp.path().join("README.md"), "one\ntwo\nthree\n").unwrap();
    let mut db = CrabDb::open(temp.path()).unwrap();
    let record = db
        .record(
            Some("main"),
            Some("extend readme".to_string()),
            Actor::human(),
            false,
        )
        .unwrap();
    let exported_change = record.operation.unwrap();
    drop(db);

    let range = format!("{}..{}", init.operation.0, exported_change.0);
    let exported = run_crabdb_json(
        temp.path(),
        &["git", "export", &range, "-m", "Export CrabDB change"],
    );
    let commit = exported["commit"].as_str().unwrap();
    assert_ne!(commit, git_head);
    assert_eq!(exported["operation"], exported_change.0);
    assert_eq!(exported["parent"], git_head);
    assert_eq!(exported["mapping"]["direction"], "export");
    assert_eq!(exported["mapping"]["git_head"], commit);

    assert_eq!(git_output(temp.path(), &["rev-parse", "HEAD"]), git_head);
    assert_eq!(
        git_output(temp.path(), &["show", &format!("{commit}:README.md")]),
        "one\ntwo\nthree"
    );
    assert_eq!(
        git_output(temp.path(), &["show", "-s", "--format=%P", commit]),
        git_head
    );

    let db = CrabDb::open(temp.path()).unwrap();
    let mappings = db.git_mappings(10).unwrap();
    assert_eq!(mappings[0].direction, "export");
    assert_eq!(mappings[0].git_head.as_deref(), Some(commit));
    assert_eq!(mappings[0].crab_change, exported_change);
}

#[test]
fn same_position_rewrite_preserves_line_identity() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "one\ntwo\nthree\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let before = db.why("README.md:2", Some("main")).unwrap();
    fs::write(
        temp.path().join("README.md"),
        "one\nagent rewrote this line\nthree\n",
    )
    .unwrap();
    let record = db
        .record(
            Some("main"),
            Some("rewrite line two".to_string()),
            Actor::human(),
            false,
        )
        .unwrap();
    let after = db.why("README.md:2", Some("main")).unwrap();
    assert_eq!(after.current_text, "agent rewrote this line");
    assert_eq!(after.line_id, before.line_id);
    assert!(after
        .history
        .iter()
        .any(|entry| entry.kind == crabdb::LineChangeKind::Modified));

    let line_id = format!(
        "{}:{}",
        before.line_id.origin_change.0, before.line_id.local_seq
    );
    let cli_by_line = run_crabdb_json(temp.path(), &["why", "--line-id", &line_id, "--at", "main"]);
    assert_eq!(cli_by_line["current_text"], "agent rewrote this line");
    let cli_at_root = run_crabdb_json(
        temp.path(),
        &[
            "why",
            "README.md:2",
            "--at",
            &format!("root:{}", record.root_id.0),
        ],
    );
    assert_eq!(
        cli_at_root["line_id"]["local_seq"],
        before.line_id.local_seq
    );

    let http = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            &format!("/v1/why?line_id={line_id}&at=branch:main"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(http.status, 200);
    let http: serde_json::Value = http.body_json().unwrap();
    assert_eq!(
        http["line_id"]["origin_change"],
        before.line_id.origin_change.0
    );

    let mcp = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "crabdb.why",
                "arguments": {
                    "line_id": line_id,
                    "at": "main"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp["result"]["isError"], false);
    assert_eq!(
        mcp["result"]["structuredContent"]["current_text"],
        "agent rewrote this line"
    );
}

#[test]
fn diff_supports_roots_dirty_and_line_id_surfaces() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "one\ntwo\nthree\n").unwrap();
    let init = CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let before = db.why("README.md:2", Some("main")).unwrap();
    fs::write(temp.path().join("README.md"), "one\nTWO\nthree\n").unwrap();
    let record = db
        .record(
            Some("main"),
            Some("rewrite line two".to_string()),
            Actor::human(),
            false,
        )
        .unwrap();
    let change_id = record.operation.clone().unwrap();
    let range = format!("{}..{}", init.operation.0, change_id.0);

    let diff = db.diff_range_with_options(&range, true, true).unwrap();
    assert_eq!(diff.files.len(), 1);
    assert_eq!(diff.files[0].kind, crabdb::FileChangeKind::Modified);
    assert!(diff.files[0].patch.as_ref().unwrap().contains("+TWO"));
    assert_eq!(diff.files[0].line_changes.len(), 1);
    assert_eq!(diff.files[0].line_changes[0].line_id, before.line_id);

    let root_range = format!("{}..{}", init.root_id.0, record.root_id.0);
    let root_diff = db.diff_roots(&root_range, false, true).unwrap();
    assert_eq!(root_diff.from, init.root_id.0);
    assert_eq!(root_diff.files[0].line_changes[0].line_id, before.line_id);

    let cli = run_crabdb_json(temp.path(), &["diff", &range, "--show-line-ids"]);
    assert_eq!(
        cli["files"][0]["line_changes"][0]["line_id"]["origin_change"],
        before.line_id.origin_change.0
    );

    let api_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            &format!("/v1/diff?range={range}&show_line_ids=true"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(api_response.status, 200);
    let api: serde_json::Value = api_response.body_json().unwrap();
    assert_eq!(
        api["files"][0]["line_changes"][0]["line_id"]["local_seq"],
        before.line_id.local_seq
    );

    let mcp = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "crabdb.diff",
                "arguments": {
                    "range": range,
                    "show_line_ids": true
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp["result"]["isError"], false);
    assert_eq!(
        mcp["result"]["structuredContent"]["files"][0]["line_changes"][0]["line_id"]["local_seq"],
        before.line_id.local_seq
    );

    fs::write(temp.path().join("README.md"), "one\nTWO dirty\nthree\n").unwrap();
    let dirty = db.diff_dirty(false, true).unwrap();
    assert_eq!(dirty.from, "main");
    assert_eq!(dirty.to, "dirty");
    assert_eq!(dirty.files[0].line_changes[0].line_id, before.line_id);

    let cli_dirty = run_crabdb_json(temp.path(), &["diff", "--dirty", "--show-line-ids"]);
    assert_eq!(cli_dirty["to"], "dirty");
    assert_eq!(
        cli_dirty["files"][0]["line_changes"][0]["line_id"]["origin_change"],
        before.line_id.origin_change.0
    );
}

#[test]
fn inspection_apis_decode_objects_roots_and_texts() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/lib.rs"), "pub fn answer() {}\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let db = CrabDb::open(temp.path()).unwrap();
    let status = db.status(Some("main")).unwrap();
    let root_object = db.inspect_object(&status.head.root_id.0).unwrap();
    assert_eq!(root_object.info.kind, crabdb::WORKTREE_ROOT_KIND);
    assert_eq!(root_object.summary["file_count"], 2);

    let root = db.inspect_root(&status.head.root_id.0).unwrap();
    assert_eq!(root.files.len(), 2);
    let readme = root
        .files
        .iter()
        .find(|file| file.path == "README.md")
        .unwrap();
    assert_eq!(readme.kind, crabdb::FileKind::Text);
    let text_object = db.inspect_object(&readme.content_object.0).unwrap();
    assert_eq!(text_object.info.kind, crabdb::TEXT_CONTENT_KIND);
    assert_eq!(text_object.summary["line_count"], 2);

    let text = db.inspect_text(&readme.content_object.0, 1).unwrap();
    assert_eq!(text.lines.len(), 1);
    assert!(text.truncated);
    assert_eq!(text.lines[0].line_number, 1);
    assert_eq!(text.lines[0].text, "hello");
    assert!(text.lines[0].line_id.contains(':'));

    let full = db.inspect_text(&readme.content_object.0, 0).unwrap();
    assert_eq!(full.lines.len(), 2);
    assert!(!full.truncated);
}

#[test]
fn map_debug_commands_decode_known_prolly_maps() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/lib.rs"), "pub fn answer() {}\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let initial_status = db.status(Some("main")).unwrap();
    let initial_root = db.inspect_root(&initial_status.head.root_id.0).unwrap();
    let path_map = initial_root.root.path_map_root.as_deref().unwrap();
    let file_index_map = initial_root.root.file_index_map_root.as_deref().unwrap();

    let path_entries = db
        .inspect_map_range(path_map, "path", None, None, 0)
        .unwrap();
    assert_eq!(path_entries.entries.len(), 2);
    assert!(path_entries
        .entries
        .iter()
        .any(|entry| entry.key.text.as_deref() == Some("README.md")
            && entry.value.summary["file_id"].as_str().is_some()));

    let truncated = db
        .inspect_map_range(path_map, "path", None, None, 1)
        .unwrap();
    assert_eq!(truncated.entries.len(), 1);
    assert!(truncated.truncated);

    let cli_range = run_crabdb_json(
        temp.path(),
        &[
            "map",
            "range",
            path_map,
            "--map-type",
            "path",
            "--limit",
            "0",
        ],
    );
    assert_eq!(cli_range["map_type"], "path");
    assert_eq!(cli_range["entries"].as_array().unwrap().len(), 2);

    let file_index_entries = db
        .inspect_map_range(file_index_map, "file-index", None, None, 0)
        .unwrap();
    assert_eq!(file_index_entries.entries.len(), 2);
    assert!(file_index_entries.entries.iter().any(|entry| {
        entry.value.summary["path"]
            .as_str()
            .is_some_and(|path| path == "README.md")
    }));

    let readme = initial_root
        .files
        .iter()
        .find(|file| file.path == "README.md")
        .unwrap();
    let text = db.inspect_text(&readme.content_object.0, 0).unwrap();
    let order_map = text.content.order_map_root.as_deref().unwrap();
    let line_index_map = text.content.line_index_map_root.as_deref().unwrap();

    let order_entries = db
        .inspect_map_range(order_map, "text-order", None, None, 0)
        .unwrap();
    assert_eq!(order_entries.entries.len(), 2);
    assert_eq!(order_entries.entries[0].key.summary["line_number_hint"], 1);
    assert_eq!(order_entries.entries[0].value.summary["text"], "hello");

    let second_line = db
        .inspect_map_range(order_map, "text-order", Some("order:2"), None, 1)
        .unwrap();
    assert_eq!(second_line.entries.len(), 1);
    assert_eq!(second_line.entries[0].value.summary["text"], "world");

    let line_index_entries = db
        .inspect_map_range(line_index_map, "line-index", None, None, 0)
        .unwrap();
    assert_eq!(line_index_entries.entries.len(), 2);
    assert_eq!(
        line_index_entries.entries[0].value.summary["line_number_hint"],
        1
    );

    fs::write(temp.path().join("README.md"), "hello\ncrabdb\n").unwrap();
    let record = db
        .record(
            Some("main"),
            Some("change readme".to_string()),
            Actor::human(),
            false,
        )
        .unwrap();
    assert!(record.operation.is_some());
    let updated_status = db.status(Some("main")).unwrap();
    let updated_root = db.inspect_root(&updated_status.head.root_id.0).unwrap();
    let updated_path_map = updated_root.root.path_map_root.as_deref().unwrap();

    let map_diff = db
        .inspect_map_diff(path_map, updated_path_map, "path", None, None, 0)
        .unwrap();
    assert!(map_diff.changes.iter().any(|change| {
        change.kind == "changed" && change.key.text.as_deref() == Some("README.md")
    }));

    let cli_diff = run_crabdb_json(
        temp.path(),
        &[
            "map",
            "diff",
            path_map,
            updated_path_map,
            "--map-type",
            "path",
            "--limit",
            "0",
        ],
    );
    assert!(cli_diff["changes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|change| { change["kind"] == "changed" && change["key"]["text"] == "README.md" }));
}

#[test]
fn anchors_follow_stable_line_identity() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("note.txt"), "one\ntwo\nthree\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let created = db
        .create_anchor("note.txt:2", "important line", Some("main"))
        .unwrap();
    assert_eq!(db.list_anchors().unwrap().len(), 1);
    let resolved = db
        .resolve_anchor(&created.anchor.id.0, Some("main"))
        .unwrap();
    assert_eq!(resolved.status, "found");
    assert_eq!(resolved.line_number, Some(2));
    assert_eq!(resolved.text.as_deref(), Some("two"));

    fs::write(temp.path().join("note.txt"), "one\ninserted\ntwo\nthree\n").unwrap();
    db.record(
        Some("main"),
        Some("insert line".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();
    let moved = db
        .resolve_anchor(&created.anchor.id.0, Some("main"))
        .unwrap();
    assert_eq!(moved.status, "found");
    assert_eq!(moved.line_number, Some(3));
    assert_eq!(moved.text.as_deref(), Some("two"));

    let deleted = db.delete_anchor(&created.anchor.id.0).unwrap();
    assert_eq!(deleted.anchor_id, created.anchor.id);
    assert!(db.list_anchors().unwrap().is_empty());
}

#[test]
fn local_api_and_mcp_expose_review_provenance_and_anchors() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_str(
        r#"{
          "session_id": "session-review",
          "message": "agent adds review line",
          "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nagent\nworld\n"}
          ]
        }"#,
    )
    .unwrap();
    let applied = db.apply_agent_patch("doc-bot", patch).unwrap();

    let why = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/why?path_line=README.md:2&branch=refs/agents/doc-bot",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(why.status, 200);
    let why: serde_json::Value = why.body_json().unwrap();
    assert_eq!(why["current_text"], "agent");
    let line_id = format!(
        "{}:{}",
        why["line_id"]["origin_change"].as_str().unwrap(),
        why["line_id"]["local_seq"].as_u64().unwrap()
    );

    let history = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            &format!("/v1/history?line_id={line_id}"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(history.status, 200);
    let history: serde_json::Value = history.body_json().unwrap();
    assert!(!history["line_history"].as_array().unwrap().is_empty());

    let tools = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }),
    )
    .unwrap();
    let tool_list = tools["result"]["tools"].as_array().unwrap();
    for name in [
        "crabdb.why",
        "crabdb.history",
        "crabdb.code_from",
        "crabdb.anchor_create",
        "crabdb.anchor_list",
        "crabdb.anchor_resolve",
        "crabdb.anchor_delete",
    ] {
        assert!(tool_list.iter().any(|tool| tool["name"] == name), "{name}");
    }

    let mcp_why = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "crabdb.why",
                "arguments": {
                    "path_line": "README.md:2",
                    "branch": "refs/agents/doc-bot"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_why["result"]["isError"], false);
    assert_eq!(
        mcp_why["result"]["structuredContent"]["current_text"],
        "agent"
    );

    let mcp_code_from = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "crabdb.code_from",
                "arguments": {
                    "selector": "session-review"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_code_from["result"]["isError"], false);
    assert!(mcp_code_from["result"]["structuredContent"]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|operation| operation["change_id"] == applied.operation.0));

    let created = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/anchors",
            serde_json::json!({
                "path_line": "README.md:2",
                "label": "review marker",
                "branch": "refs/agents/doc-bot"
            }),
        ),
    );
    assert_eq!(created.status, 201);
    let created: serde_json::Value = created.body_json().unwrap();
    let anchor_id = created["anchor"]["id"].as_str().unwrap().to_string();

    let mcp_list = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "crabdb.anchor_list",
                "arguments": {}
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_list["result"]["isError"], false);
    assert!(mcp_list["result"]["structuredContent"]
        .as_array()
        .unwrap()
        .iter()
        .any(|anchor| anchor["id"] == anchor_id));

    let move_patch: PatchDocument = serde_json::from_str(
        r#"{
          "session_id": "session-review",
          "message": "move anchored line",
          "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nintro\nagent\nworld\n"}
          ]
        }"#,
    )
    .unwrap();
    db.apply_agent_patch("doc-bot", move_patch).unwrap();

    let resolved = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            &format!("/v1/anchors/{anchor_id}?branch=refs/agents/doc-bot"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(resolved.status, 200);
    let resolved: serde_json::Value = resolved.body_json().unwrap();
    assert_eq!(resolved["status"], "found");
    assert_eq!(resolved["line_number"], 3);
    assert_eq!(resolved["text"], "agent");

    let mcp_resolved = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "crabdb.anchor_resolve",
                "arguments": {
                    "anchor_id": anchor_id.clone(),
                    "branch": "refs/agents/doc-bot"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_resolved["result"]["isError"], false);
    assert_eq!(
        mcp_resolved["result"]["structuredContent"]["line_number"],
        3
    );

    let mcp_delete = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "crabdb.anchor_delete",
                "arguments": {
                    "anchor_id": anchor_id
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_delete["result"]["isError"], false);

    let anchors = crabdb::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/anchors", serde_json::Value::Null),
    );
    assert_eq!(anchors.status, 200);
    let anchors: serde_json::Value = anchors.body_json().unwrap();
    assert!(anchors.as_array().unwrap().is_empty());
}

#[test]
fn checkout_restores_branch_root() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("note.txt"), "one\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let branch = db.create_branch("before-edit", Some("main")).unwrap();

    fs::write(temp.path().join("note.txt"), "two\n").unwrap();
    db.record(Some("main"), None, Actor::human(), false)
        .unwrap();
    db.checkout(&branch.from.0, true).unwrap();

    assert_eq!(
        fs::read_to_string(temp.path().join("note.txt")).unwrap(),
        "one\n"
    );
}

#[test]
fn refish_aliases_accept_branch_agent_and_root_selectors() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("note.txt"), "one\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let branch = db
        .create_branch("before-edit", Some("branch:main"))
        .unwrap();

    fs::write(temp.path().join("note.txt"), "two\n").unwrap();
    let recorded = db
        .record(
            Some("main"),
            Some("record two".to_string()),
            Actor::human(),
            false,
        )
        .unwrap();
    let recorded_change = recorded.operation.clone().unwrap();

    let branch_checkout = db.checkout("branch:before-edit", false).unwrap();
    assert_eq!(branch_checkout.change_id, branch.from);
    assert_eq!(
        fs::read_to_string(temp.path().join("note.txt")).unwrap(),
        "one\n"
    );

    db.spawn_agent_with_workdir("doc-bot", Some("branch:main"), false, None, None, None)
        .unwrap();
    let agent_checkout = db.checkout("agent:doc-bot", true).unwrap();
    assert_eq!(agent_checkout.change_id, recorded_change);
    assert_eq!(
        fs::read_to_string(temp.path().join("note.txt")).unwrap(),
        "two\n"
    );

    let root_selector = format!("root:{}", branch.root_id.0);
    let root_preview = db
        .checkout_with_options(&root_selector, true, true, None, false)
        .unwrap();
    assert!(root_preview.dry_run);
    assert_eq!(root_preview.change_id, branch.from);
    assert_eq!(root_preview.root_id, branch.root_id);

    let raw_root = branch.root_id.0.clone();
    drop(db);

    let cli_root_checkout = run_crabdb_json(temp.path(), &["checkout", &raw_root, "--force"]);
    assert_eq!(cli_root_checkout["root_id"], raw_root);
    assert_eq!(
        fs::read_to_string(temp.path().join("note.txt")).unwrap(),
        "one\n"
    );
}

#[test]
fn checkout_dry_run_and_alternate_workdir_are_safe() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("note.txt"), "one\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let branch = db.create_branch("before-edit", Some("main")).unwrap();
    fs::write(temp.path().join("note.txt"), "two\n").unwrap();
    db.record(Some("main"), None, Actor::human(), false)
        .unwrap();
    fs::write(temp.path().join("note.txt"), "dirty\n").unwrap();

    let dry_run = db
        .checkout_with_options(&branch.from.0, false, true, None, false)
        .unwrap();
    assert!(dry_run.dry_run);
    assert_eq!(dry_run.written_files, 0);
    assert_eq!(dry_run.changed_paths.len(), 1);
    assert_eq!(
        fs::read_to_string(temp.path().join("note.txt")).unwrap(),
        "dirty\n"
    );

    let cli_dry_run = run_crabdb_json(temp.path(), &["checkout", &branch.from.0, "--dry-run"]);
    assert_eq!(cli_dry_run["dry_run"], true);
    assert_eq!(cli_dry_run["written_files"], 0);

    let err = db.checkout(&branch.from.0, false).unwrap_err();
    assert!(matches!(err, Error::DirtyWorktree));

    let preview_parent = tempfile::tempdir().unwrap();
    let preview = preview_parent.path().join("preview");
    let checkout = db
        .checkout_with_options("main", false, false, Some(&preview), false)
        .unwrap();
    assert!(!checkout.dry_run);
    assert_eq!(
        PathBuf::from(checkout.output_root.unwrap())
            .canonicalize()
            .unwrap(),
        preview.canonicalize().unwrap()
    );
    assert_eq!(
        fs::read_to_string(preview.join("note.txt")).unwrap(),
        "two\n"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("note.txt")).unwrap(),
        "dirty\n"
    );

    let nonempty = preview_parent.path().join("nonempty");
    fs::create_dir_all(&nonempty).unwrap();
    fs::write(nonempty.join("keep.txt"), "keep\n").unwrap();
    let err = db
        .checkout_with_options("main", false, false, Some(&nonempty), false)
        .unwrap_err();
    assert!(err.to_string().contains("must be empty or absent"));
    assert_eq!(
        fs::read_to_string(nonempty.join("keep.txt")).unwrap(),
        "keep\n"
    );
}

#[test]
fn checkout_record_dirty_saves_current_work_before_materializing_target() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("note.txt"), "one\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let branch = db.create_branch("before-edit", Some("main")).unwrap();
    fs::write(temp.path().join("note.txt"), "two\n").unwrap();
    db.record(
        Some("main"),
        Some("record two".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();
    fs::write(temp.path().join("note.txt"), "dirty\n").unwrap();
    drop(db);

    let checked_out = run_crabdb_json(temp.path(), &["checkout", &branch.from.0, "--record-dirty"]);
    let recorded_dirty = checked_out["recorded_dirty"].as_str().unwrap().to_string();
    assert_eq!(checked_out["change_id"], branch.from.0);
    assert_eq!(
        fs::read_to_string(temp.path().join("note.txt")).unwrap(),
        "one\n"
    );

    let mut db = CrabDb::open(temp.path()).unwrap();
    assert_eq!(
        db.status(Some("main")).unwrap().head.change_id.0,
        recorded_dirty
    );
    match db.show(&recorded_dirty).unwrap() {
        ShowResult::Operation { value } => {
            let expected_message =
                format!("Record dirty worktree before checkout `{}`", branch.from.0);
            assert_eq!(value.operation.kind, crabdb::OperationKind::Checkout);
            assert_eq!(
                value.operation.message.as_deref(),
                Some(expected_message.as_str())
            );
            assert_eq!(value.operation.changes.len(), 1);
        }
        other => panic!("expected checkout checkpoint operation, got {other:?}"),
    }

    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("note.txt")).unwrap(),
        "dirty\n"
    );
}

#[cfg(unix)]
#[test]
fn checkout_refuses_to_follow_symlink_when_materializing() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let note = temp.path().join("note.txt");
    fs::write(&note, "one\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let branch = db.create_branch("before-edit", Some("main")).unwrap();
    fs::write(&note, "two\n").unwrap();
    db.record(Some("main"), None, Actor::human(), false)
        .unwrap();

    let outside_file = outside.path().join("outside.txt");
    fs::write(&outside_file, "outside\n").unwrap();
    fs::remove_file(&note).unwrap();
    std::os::unix::fs::symlink(&outside_file, &note).unwrap();

    let err = db.checkout(&branch.from.0, true).unwrap_err();
    match err {
        Error::InvalidPath { path, reason } => {
            assert_eq!(path, "note.txt");
            assert!(reason.contains("symlink"));
        }
        other => panic!("expected symlink path safety error, got {other:?}"),
    }
    assert_eq!(fs::read_to_string(&outside_file).unwrap(), "outside\n");
    assert!(fs::symlink_metadata(&note)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn branch_list_rename_and_delete_work() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("note.txt"), "one\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.create_branch("scratch", Some("main")).unwrap();
    let branches = db.list_branches().unwrap();
    assert!(branches
        .iter()
        .any(|branch| branch.name == "main" && branch.is_current));
    assert!(branches.iter().any(|branch| branch.name == "scratch"));

    let renamed = db.rename_branch("main", "trunk").unwrap();
    assert_eq!(renamed.old_name, "main");
    assert_eq!(renamed.new_name, "trunk");
    assert_eq!(db.current_branch().unwrap(), "trunk");
    assert!(!temp.path().join(".crabdb/refs/branches/main").exists());
    assert!(temp.path().join(".crabdb/refs/branches/trunk").exists());

    let deleted = db.delete_branch("scratch").unwrap();
    assert_eq!(deleted.name, "scratch");
    assert!(!temp.path().join(".crabdb/refs/branches/scratch").exists());
    let err = db.delete_branch("trunk").unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)));
}

#[test]
fn timeline_branch_scope_accepts_command_flag_and_ref_aliases() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("note.txt"), "one\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.create_branch("scratch", Some("main")).unwrap();

    fs::write(temp.path().join("note.txt"), "scratch\n").unwrap();
    let scratch_record = db
        .record(
            Some("scratch"),
            Some("scratch edit".to_string()),
            Actor::human(),
            false,
        )
        .unwrap()
        .operation
        .unwrap();

    fs::write(temp.path().join("note.txt"), "main\n").unwrap();
    let main_record = db
        .record(
            Some("main"),
            Some("main edit".to_string()),
            Actor::human(),
            false,
        )
        .unwrap()
        .operation
        .unwrap();
    drop(db);

    let scratch_timeline = run_crabdb_json(
        temp.path(),
        &["timeline", "--branch", "branch:scratch", "--limit", "10"],
    );
    let scratch_entries = scratch_timeline.as_array().unwrap();
    assert_eq!(scratch_entries.len(), 1);
    assert_eq!(scratch_entries[0]["change_id"], scratch_record.0);

    let main_timeline = run_crabdb_json(temp.path(), &["timeline", "--branch", "main"]);
    let main_entries = main_timeline.as_array().unwrap();
    assert!(main_entries
        .iter()
        .any(|entry| entry["change_id"] == main_record.0));
}

#[test]
fn agent_patch_can_merge_into_main() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("doc-bot", Some("main"), true, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_str(
        r#"{
          "message": "agent edits",
          "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nagent\n"},
            {"op": "write", "path": "src/lib.rs", "content": "pub fn answer() -> u32 { 42 }\n"}
          ]
        }"#,
    )
    .unwrap();
    let applied = db.apply_agent_patch("doc-bot", patch).unwrap();
    assert_eq!(applied.changed_paths.len(), 2);

    let merged = db.merge_agent("doc-bot", "main").unwrap();
    assert_eq!(merged.changed_paths.len(), 2);
    db.checkout("main", true).unwrap();

    assert_eq!(
        fs::read_to_string(temp.path().join("src/lib.rs")).unwrap(),
        "pub fn answer() -> u32 { 42 }\n"
    );
    assert!(db.fsck().unwrap().errors.is_empty());
}

#[test]
fn merge_dry_run_reports_without_mutating_refs() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let before_head = db.status(Some("main")).unwrap().head.change_id;
    db.spawn_agent("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "agent edits",
        "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nagent\n"}
        ]
    }))
    .unwrap();
    db.apply_agent_patch("doc-bot", patch).unwrap();

    let cli = run_crabdb_json(
        temp.path(),
        &[
            "merge-agent",
            "doc-bot",
            "--strategy",
            "line-id-aware",
            "--dry-run",
        ],
    );
    assert_eq!(cli["dry_run"], true);
    assert_eq!(cli["changed_paths"][0]["path"], "README.md");

    let api_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/branches/main/merge-agent",
            serde_json::json!({
                "agent": "doc-bot",
                "dry_run": true
            }),
        ),
    );
    assert_eq!(api_response.status, 200);
    let api: serde_json::Value = api_response.body_json().unwrap();
    assert_eq!(api["dry_run"], true);

    let dry_run = db
        .merge_agent_with_options("doc-bot", "main", true)
        .unwrap();
    assert!(dry_run.dry_run);
    assert_eq!(dry_run.changed_paths.len(), 1);
    assert_eq!(db.status(Some("main")).unwrap().head.change_id, before_head);
    assert_eq!(db.agent_details("doc-bot").unwrap().branch.status, "active");

    let merged = db.merge_agent("doc-bot", "main").unwrap();
    assert!(!merged.dry_run);
    assert_eq!(merged.changed_paths.len(), 1);
    assert_ne!(db.status(Some("main")).unwrap().head.change_id, before_head);

    let before_branch_head = db.status(Some("main")).unwrap().head.change_id;
    db.create_branch("feature", Some("main")).unwrap();
    fs::write(temp.path().join("README.md"), "hello\nagent\nbranch\n").unwrap();
    db.record(Some("feature"), None, Actor::human(), false)
        .unwrap();
    let branch_dry_run = db
        .merge_branches_with_options("feature", "main", true)
        .unwrap();
    assert!(branch_dry_run.dry_run);
    assert_eq!(branch_dry_run.changed_paths.len(), 1);
    assert_eq!(
        db.status(Some("main")).unwrap().head.change_id,
        before_branch_head
    );
}

#[test]
fn merge_dry_run_reports_conflicts_without_opening_conflict_state() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "one\ntwo\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write", "path": "README.md", "content": "one\nagent\n"}
        ]
    }))
    .unwrap();
    db.apply_agent_patch("doc-bot", patch).unwrap();

    fs::write(temp.path().join("README.md"), "one\nhuman\n").unwrap();
    db.record(
        Some("main"),
        Some("human edit".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();

    let dry_run = db
        .merge_agent_with_options("doc-bot", "main", true)
        .unwrap();
    assert!(dry_run.dry_run);
    assert!(dry_run.changed_paths.is_empty());
    assert_eq!(
        dry_run.conflicts,
        vec!["both changed `README.md` differently"]
    );
    assert_eq!(db.agent_details("doc-bot").unwrap().branch.status, "active");

    let cli = run_crabdb_json(temp.path(), &["merge-agent", "doc-bot", "--dry-run"]);
    assert_eq!(cli["dry_run"], true);
    assert_eq!(cli["conflicts"][0], "both changed `README.md` differently");
    assert!(db.list_conflicts().unwrap().is_empty());

    let err = db.merge_agent("doc-bot", "main").unwrap_err();
    assert!(matches!(&err, Error::Conflict(_)));
    assert_eq!(
        db.agent_details("doc-bot").unwrap().branch.status,
        "conflicted"
    );
    let conflicts = db.list_conflicts().unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(
        conflicts[0].source_ref.as_deref(),
        Some("refs/agents/doc-bot")
    );
    assert_eq!(
        conflicts[0].target_ref.as_deref(),
        Some("refs/branches/main")
    );
    assert!(conflicts[0]
        .details
        .iter()
        .any(|detail| detail == "both changed `README.md` differently"));
    assert!(err.to_string().contains(&conflicts[0].conflict_set_id));

    let repeated = db.merge_agent("doc-bot", "main").unwrap_err();
    assert!(matches!(&repeated, Error::Conflict(_)));
    assert!(repeated.to_string().contains(&conflicts[0].conflict_set_id));
    assert_eq!(db.list_conflicts().unwrap().len(), 1);

    let resolved = db
        .resolve_conflict(&conflicts[0].conflict_set_id, "source")
        .unwrap();
    assert_eq!(resolved.resolution, "source");
    assert_eq!(db.agent_details("doc-bot").unwrap().branch.status, "merged");
    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "one\nagent\n"
    );
}

#[test]
fn local_api_direct_merge_agent_conflict_records_conflict_set() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "one\ntwo\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("api-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write", "path": "README.md", "content": "one\nagent-api\n"}
        ]
    }))
    .unwrap();
    db.apply_agent_patch("api-bot", patch).unwrap();

    fs::write(temp.path().join("README.md"), "one\nhuman-api\n").unwrap();
    db.record(
        Some("main"),
        Some("human edit".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();

    let response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/branches/main/merge-agent",
            serde_json::json!({ "agent_id": "api-bot" }),
        ),
    );
    assert_eq!(response.status, 409);
    let body: serde_json::Value = response.body_json().unwrap();
    assert_eq!(body["error"]["code"], 6);
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("conflict_"));

    let conflicts = crabdb::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/conflicts", serde_json::Value::Null),
    );
    assert_eq!(conflicts.status, 200);
    let conflicts: serde_json::Value = conflicts.body_json().unwrap();
    assert_eq!(conflicts.as_array().unwrap().len(), 1);
    assert_eq!(conflicts[0]["source_ref"], "refs/agents/api-bot");
    assert_eq!(conflicts[0]["target_ref"], "refs/branches/main");
}

#[test]
fn agent_patch_can_replace_stable_line_with_expected_text() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "one\ntwo\nthree\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let why = db.why("README.md:2", Some("refs/agents/doc-bot")).unwrap();
    let line_id = format!("{}:{}", why.line_id.origin_change.0, why.line_id.local_seq);
    let patch_json = serde_json::json!({
        "message": "line-id patch",
        "edits": [{
            "op": "replace_line",
            "path": "README.md",
            "line_id": line_id,
            "expected_text": "two",
            "new_text": "agent two"
        }]
    });
    let patch: PatchDocument = serde_json::from_value(patch_json).unwrap();
    let applied = db.apply_agent_patch("doc-bot", patch).unwrap();
    assert_eq!(applied.changed_paths.len(), 1);

    let changed = db.why("README.md:2", Some("refs/agents/doc-bot")).unwrap();
    assert_eq!(changed.current_text, "agent two");
    assert_eq!(changed.line_id, why.line_id);
    let shown = db.show(&applied.operation.0).unwrap();
    match shown {
        ShowResult::Operation { value } => {
            assert_eq!(value.operation.changes.len(), 1);
            assert_eq!(value.operation.changes[0].line_changes.len(), 1);
            assert_eq!(
                value.operation.changes[0].line_changes[0].kind,
                crabdb::LineChangeKind::Modified
            );
        }
        other => panic!("expected operation, got {other:?}"),
    }

    let stale_patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [{
            "op": "replace_line",
            "path": "README.md",
            "line_id": format!("{}:{}", why.line_id.origin_change.0, why.line_id.local_seq),
            "expected_text": "two",
            "new_text": "stale"
        }]
    }))
    .unwrap();
    let err = db.apply_agent_patch("doc-bot", stale_patch).unwrap_err();
    assert!(matches!(err, Error::PatchRejected(_)));
    assert!(err.to_string().contains("expected text mismatch"));
}

#[test]
fn agent_merge_combines_non_overlapping_text_line_edits() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "one\ntwo\nthree\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_str(
        r#"{
          "message": "agent edits line two",
          "edits": [
            {"op": "write", "path": "README.md", "content": "one\nagent two\nthree\n"}
          ]
        }"#,
    )
    .unwrap();
    db.apply_agent_patch("doc-bot", patch).unwrap();

    fs::write(temp.path().join("README.md"), "one\ntwo\nhuman three\n").unwrap();
    db.record(
        Some("main"),
        Some("human edits line three".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();

    let merged = db.merge_agent("doc-bot", "main").unwrap();
    assert_eq!(merged.changed_paths.len(), 1);
    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "one\nagent two\nhuman three\n"
    );

    let line_two = db.why("README.md:2", Some("main")).unwrap();
    assert_eq!(line_two.current_text, "agent two");
    let line_three = db.why("README.md:3", Some("main")).unwrap();
    assert_eq!(line_three.current_text, "human three");
}

#[test]
fn agent_management_commands_have_backing_apis() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let spawned = db
        .spawn_agent(
            "doc-bot",
            Some("main"),
            true,
            Some("openai".to_string()),
            Some("gpt-5".to_string()),
        )
        .unwrap();
    assert_eq!(db.list_agents().unwrap().len(), 1);
    let details = db.agent_details("doc-bot").unwrap();
    assert_eq!(details.record.provider.as_deref(), Some("openai"));
    assert_eq!(details.branch.ref_name, spawned.ref_name);

    let message = db
        .add_agent_message(
            "doc-bot",
            "user",
            "Please improve the docs",
            Some("session-agent-management".to_string()),
        )
        .unwrap();
    assert_eq!(
        message.session_id.as_deref(),
        Some("session-agent-management")
    );

    let patch: PatchDocument = serde_json::from_str(
        r#"{
          "session_id": "session-agent-management",
          "message": "agent edits readme",
          "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nagent\n"}
          ]
        }"#,
    )
    .unwrap();
    let applied = db.apply_agent_patch("doc-bot", patch).unwrap();
    let status = db.agent_status("doc-bot").unwrap();
    assert_eq!(status.changed_paths.len(), 1);
    assert_eq!(
        status.agent.branch.session_id.as_deref(),
        Some("session-agent-management")
    );
    let timeline = db.agent_timeline("doc-bot", 10).unwrap();
    assert!(timeline
        .iter()
        .any(|entry| entry.change_id == applied.operation));
    let contribution = db.agent_contribution("doc-bot", 10).unwrap();
    assert_eq!(contribution.status.agent.record.name, "doc-bot");
    assert!(contribution
        .status
        .changed_paths
        .iter()
        .any(|path| path.path == "README.md"));
    assert!(contribution
        .operations
        .iter()
        .any(|entry| entry.change_id == applied.operation));
    assert_eq!(contribution.sessions.len(), 1);

    db.checkout_agent("doc-bot", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "hello\nagent\n"
    );
    let err = db.remove_agent("doc-bot", false).unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)));
    let removed = db.remove_agent("doc-bot", true).unwrap();
    assert_eq!(removed.agent_id, details.record.agent_id);
    assert!(!temp.path().join(".crabdb/refs/agents/doc-bot").exists());
    if let Some(workdir) = removed.removed_workdir {
        assert!(!std::path::Path::new(&workdir).exists());
    }
    assert_eq!(
        db.agent_details("doc-bot").unwrap().branch.status,
        "removed"
    );

    let conn = Connection::open(temp.path().join(".crabdb/index/crabdb.sqlite")).unwrap();
    let messages: i64 = conn
        .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
        .unwrap();
    let events: i64 = conn
        .query_row("SELECT COUNT(*) FROM agent_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(messages, 2);
    assert!(events >= 4);
}

#[test]
fn agent_test_runs_in_workdir_and_records_events_and_output_blobs() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("doc-bot", Some("main"), true, None, None)
        .unwrap();
    drop(db);

    let tested = run_crabdb_json(
        temp.path(),
        &[
            "agent",
            "test",
            "doc-bot",
            "--timeout-secs",
            "5",
            "--",
            "sh",
            "-c",
            "printf ok; printf err >&2",
        ],
    );
    assert_eq!(tested["success"], true);
    assert_eq!(tested["status"], "test_passed");
    assert_eq!(tested["exit_code"], 0);
    assert_eq!(tested["stdout_preview"], "ok");
    assert_eq!(tested["stderr_preview"], "err");

    let turn_id = tested["turn_id"].as_str().unwrap();
    let stdout_object = tested["stdout_object"].as_str().unwrap().to_string();
    let stderr_object = tested["stderr_object"].as_str().unwrap().to_string();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let turn = db.show_agent_turn(turn_id).unwrap();
    assert_eq!(turn.turn.status, "test_passed");
    assert!(turn
        .events
        .iter()
        .any(|event| event.event_type == "test_started"));
    assert!(turn
        .events
        .iter()
        .any(|event| event.event_type == "test_finished"));

    let stdout = db.inspect_object(&stdout_object).unwrap();
    assert_eq!(stdout.info.kind, "Blob");
    assert_eq!(stdout.summary["byte_count"], 2);
    let stderr = db.inspect_object(&stderr_object).unwrap();
    assert_eq!(stderr.info.kind, "Blob");
    assert_eq!(stderr.summary["byte_count"], 3);

    let failed = db
        .run_agent_test(
            "doc-bot",
            vec!["sh".to_string(), "-c".to_string(), "exit 7".to_string()],
            None,
            5,
        )
        .unwrap();
    assert!(!failed.success);
    assert_eq!(failed.status, "test_failed");
    assert_eq!(failed.exit_code, Some(7));
    assert_eq!(
        db.show_agent_turn(&failed.turn_id).unwrap().turn.status,
        "test_failed"
    );
    let latest_test = db.agent_status("doc-bot").unwrap().latest_test.unwrap();
    assert_eq!(latest_test.status, "test_failed");
    assert_eq!(latest_test.exit_code, Some(7));
    assert_eq!(latest_test.command, vec!["sh", "-c", "exit 7"]);

    drop(db);
    let evaled = run_crabdb_json(
        temp.path(),
        &[
            "agent",
            "eval",
            "doc-bot",
            "--timeout-secs",
            "5",
            "--suite",
            "policy-smoke",
            "--score",
            "0.95",
            "--threshold",
            "0.9",
            "--",
            "sh",
            "-c",
            "printf score=1",
        ],
    );
    assert_eq!(evaled["kind"], "eval");
    assert_eq!(evaled["success"], true);
    assert_eq!(evaled["status"], "eval_passed");
    assert_eq!(evaled["suite"], "policy-smoke");
    assert_eq!(evaled["score"], 0.95);
    assert_eq!(evaled["threshold"], 0.9);
    assert_eq!(evaled["stdout_preview"], "score=1");

    let mut db = CrabDb::open(temp.path()).unwrap();
    let eval_turn = db
        .show_agent_turn(evaled["turn_id"].as_str().unwrap())
        .unwrap();
    assert_eq!(eval_turn.turn.status, "eval_passed");
    assert!(eval_turn
        .events
        .iter()
        .any(|event| event.event_type == "eval_started"));
    assert!(eval_turn
        .events
        .iter()
        .any(|event| event.event_type == "eval_finished"));

    let api_eval = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/agents/doc-bot/evals",
            serde_json::json!({
                "command": ["sh", "-c", "printf api-score"],
                "timeout_secs": 5,
                "suite": "regression-set",
                "score": 0.72,
                "threshold": 0.8
            }),
        ),
    );
    assert_eq!(api_eval.status, 200);
    let api_eval: serde_json::Value = api_eval.body_json().unwrap();
    assert_eq!(api_eval["kind"], "eval");
    assert_eq!(api_eval["success"], false);
    assert_eq!(api_eval["status"], "eval_failed");
    assert_eq!(api_eval["exit_code"], 0);
    assert_eq!(api_eval["suite"], "regression-set");
    assert_eq!(api_eval["score"], 0.72);
    assert_eq!(api_eval["threshold"], 0.8);
    let failed_eval_turn = db
        .show_agent_turn(api_eval["turn_id"].as_str().unwrap())
        .unwrap();
    let failed_eval_event = failed_eval_turn
        .events
        .iter()
        .find(|event| event.event_type == "eval_finished")
        .unwrap();
    let failed_eval_payload = failed_eval_event.payload.as_ref().unwrap();
    assert_eq!(failed_eval_payload["process_success"], true);
    assert_eq!(failed_eval_payload["threshold_met"], false);

    let mcp_eval = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "crabdb.run_eval",
                "arguments": {
                    "agent": "doc-bot",
                    "command": ["sh", "-c", "printf eval-ok"],
                    "timeout_secs": 5,
                    "suite": "nightly",
                    "score": 0.91,
                    "threshold": 0.9
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_eval["result"]["isError"], false);
    assert_eq!(
        mcp_eval["result"]["structuredContent"]["status"],
        "eval_passed"
    );

    let latest_eval = db.agent_status("doc-bot").unwrap().latest_eval.unwrap();
    assert_eq!(latest_eval.kind, "eval");
    assert_eq!(latest_eval.status, "eval_passed");
    assert_eq!(latest_eval.suite.as_deref(), Some("nightly"));
    assert_eq!(latest_eval.score, Some(0.91));
    assert_eq!(latest_eval.threshold, Some(0.9));
    assert_eq!(latest_eval.command, vec!["sh", "-c", "printf eval-ok"]);

    let gate_history = db.agent_gate_history("doc-bot", None, 10).unwrap();
    assert_eq!(gate_history.kind, "all");
    assert!(gate_history.gates.len() >= 5);
    assert_eq!(gate_history.gates[0].kind, "eval");
    assert_eq!(gate_history.gates[0].suite.as_deref(), Some("nightly"));
    assert!(gate_history
        .gates
        .iter()
        .any(|gate| gate.kind == "test" && gate.status == "test_failed"));
    let eval_history = db.agent_gate_history("doc-bot", Some("eval"), 2).unwrap();
    assert_eq!(eval_history.kind, "eval");
    assert_eq!(eval_history.gates.len(), 2);
    assert_eq!(eval_history.gates[0].suite.as_deref(), Some("nightly"));
    assert_eq!(eval_history.gates[1].status, "eval_failed");
    assert_eq!(
        eval_history.gates[1].suite.as_deref(),
        Some("regression-set")
    );

    let api_gates = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/agents/doc-bot/gates?kind=eval&limit=2",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(api_gates.status, 200);
    let api_gates: serde_json::Value = api_gates.body_json().unwrap();
    assert_eq!(api_gates["kind"], "eval");
    assert_eq!(api_gates["gates"].as_array().unwrap().len(), 2);
    assert_eq!(api_gates["gates"][0]["suite"], "nightly");

    let mcp_gates = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "crabdb.gate_history",
                "arguments": {
                    "agent": "doc-bot",
                    "kind": "eval",
                    "limit": 2
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_gates["result"]["isError"], false);
    assert_eq!(
        mcp_gates["result"]["structuredContent"]["gates"][0]["suite"],
        "nightly"
    );

    let mcp_gate_resource = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "resources/read",
            "params": {
                "uri": "crabdb://workspace/agents/doc-bot/gates"
            }
        }),
    )
    .unwrap();
    let resource_text = mcp_gate_resource["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let resource_json: serde_json::Value = serde_json::from_str(resource_text).unwrap();
    assert_eq!(resource_json["gates"][0]["suite"], "nightly");

    drop(db);
    let cli_gates = run_crabdb_json(
        temp.path(),
        &[
            "agent", "gates", "doc-bot", "--kind", "eval", "--limit", "2",
        ],
    );
    assert_eq!(cli_gates["kind"], "eval");
    assert_eq!(cli_gates["gates"].as_array().unwrap().len(), 2);
    assert_eq!(cli_gates["gates"][0]["suite"], "nightly");

    let mut db = CrabDb::open(temp.path()).unwrap();
    let gc = db.gc(false).unwrap();
    assert!(gc.errors.is_empty(), "{:?}", gc.errors);
    assert!(db.inspect_object(&stdout_object).is_ok());
    assert!(db.inspect_object(&stderr_object).is_ok());
}

#[test]
fn agent_sessions_track_messages_patches_and_turns() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let started = db
        .start_agent_session(
            "doc-bot",
            Some("Improve docs".to_string()),
            Some("session-docs".to_string()),
        )
        .unwrap();
    assert_eq!(started.session.session_id, "session-docs");
    assert_eq!(started.session.status, "active");
    assert_eq!(
        db.agent_details("doc-bot")
            .unwrap()
            .branch
            .session_id
            .as_deref(),
        Some("session-docs")
    );

    let message = db
        .add_agent_message("doc-bot", "user", "Please improve README", None)
        .unwrap();
    assert_eq!(message.session_id.as_deref(), Some("session-docs"));

    let patch: PatchDocument = serde_json::from_str(
        r#"{
          "message": "agent edits readme",
          "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nsession\n"}
          ]
        }"#,
    )
    .unwrap();
    let applied = db.apply_agent_patch("doc-bot", patch).unwrap();
    let by_session = db.code_from("session-docs").unwrap();
    assert_eq!(by_session.operations.len(), 1);
    assert_eq!(by_session.operations[0].change_id, applied.operation);
    assert_eq!(
        by_session.operations[0].session_id.as_deref(),
        Some("session-docs")
    );
    let session_timeline = db.session_timeline("session-docs", 10).unwrap();
    assert_eq!(session_timeline.len(), 1);
    assert_eq!(session_timeline[0].change_id, applied.operation);
    let agent_timeline = db.timeline_query(None, None, Some("doc-bot"), 10).unwrap();
    assert!(agent_timeline
        .iter()
        .any(|entry| entry.change_id == applied.operation));

    let http_timeline = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/timeline?session=session-docs&limit=5",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(http_timeline.status, 200);
    let http_entries: serde_json::Value = http_timeline.body_json().unwrap();
    assert_eq!(http_entries.as_array().unwrap().len(), 1);
    assert_eq!(http_entries[0]["change_id"], applied.operation.0);

    let mcp_tools = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }),
    )
    .unwrap();
    assert!(mcp_tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == "crabdb.timeline"));

    let mcp_timeline = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "crabdb.timeline",
                "arguments": {
                    "session": "session-docs",
                    "limit": 5
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_timeline["result"]["isError"], false);
    assert_eq!(
        mcp_timeline["result"]["structuredContent"][0]["change_id"],
        applied.operation.0
    );

    let cli_session_timeline = run_crabdb_json(
        temp.path(),
        &["timeline", "--session", "session-docs", "--limit", "5"],
    );
    assert_eq!(cli_session_timeline.as_array().unwrap().len(), 1);
    assert_eq!(cli_session_timeline[0]["change_id"], applied.operation.0);

    let cli_agent_timeline = run_crabdb_json(
        temp.path(),
        &["timeline", "--agent", "doc-bot", "--limit", "5"],
    );
    assert!(cli_agent_timeline
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["change_id"] == applied.operation.0));

    let details = db.show_agent_session("session-docs").unwrap();
    assert_eq!(details.messages.len(), 2);
    assert_eq!(details.operations.len(), 1);
    assert_eq!(details.turns.len(), 2);
    assert!(details
        .turns
        .iter()
        .any(|turn| turn.status == "patch_applied"
            && turn.after_change.as_ref() == Some(&applied.operation)));
    assert!(details
        .events
        .iter()
        .any(|event| event.event_type == "session_started"));
    assert!(details
        .events
        .iter()
        .any(|event| event.event_type == "patch_applied"
            && event.turn_id.is_some()
            && event.session_id.as_deref() == Some("session-docs")));

    let ended = db.end_agent_session("session-docs", "completed").unwrap();
    assert_eq!(ended.session.status, "completed");
    assert!(ended.session.ended_at.is_some());
    assert_eq!(
        db.agent_details("doc-bot")
            .unwrap()
            .branch
            .session_id
            .as_deref(),
        None
    );
}

#[test]
fn agent_workdir_record_advances_agent_branch() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let spawned = db
        .spawn_agent("doc-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = spawned.workdir.unwrap();
    fs::write(
        std::path::Path::new(&workdir).join("README.md"),
        "hello\nworkdir\n",
    )
    .unwrap();
    fs::create_dir_all(std::path::Path::new(&workdir).join("docs")).unwrap();
    fs::write(
        std::path::Path::new(&workdir).join("docs/notes.md"),
        "agent notes\n",
    )
    .unwrap();

    let recorded = db
        .record_agent_workdir("doc-bot", Some("record workdir".to_string()))
        .unwrap();
    assert!(recorded.operation.is_some());
    assert_eq!(recorded.changed_paths.len(), 2);

    let clean = db.record_agent_workdir("doc-bot", None).unwrap();
    assert!(clean.operation.is_none());
    let timeline = db.agent_timeline("doc-bot", 10).unwrap();
    assert!(timeline
        .iter()
        .any(|entry| entry.kind == crabdb::OperationKind::AgentRecord));

    db.merge_agent("doc-bot", "main").unwrap();
    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "hello\nworkdir\n"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("docs/notes.md")).unwrap(),
        "agent notes\n"
    );
}

#[test]
fn agent_spawn_supports_custom_and_configured_workdirs() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let workdir_parent = tempfile::tempdir().unwrap();
    let cli_workdir = workdir_parent.path().join("cli-bot");
    let cli_spawn = run_crabdb_json(
        temp.path(),
        &[
            "agent",
            "spawn",
            "cli-bot",
            "--from",
            "main",
            "--workdir",
            cli_workdir.to_str().unwrap(),
        ],
    );
    assert_eq!(
        PathBuf::from(cli_spawn["workdir"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        cli_workdir.canonicalize().unwrap()
    );
    assert_eq!(
        fs::read_to_string(cli_workdir.join("README.md")).unwrap(),
        "hello\n"
    );

    let headless_spawn = run_crabdb_json(
        temp.path(),
        &[
            "agent",
            "spawn",
            "headless-bot",
            "--from",
            "main",
            "--materialize=false",
        ],
    );
    assert!(headless_spawn["workdir"].is_null());

    let no_materialize_spawn = run_crabdb_json(
        temp.path(),
        &[
            "agent",
            "spawn",
            "no-workdir-bot",
            "--from",
            "main",
            "--no-materialize",
        ],
    );
    assert!(no_materialize_spawn["workdir"].is_null());

    let mut db = CrabDb::open(temp.path()).unwrap();
    assert_eq!(
        db.agent_status("cli-bot").unwrap().workdir_state,
        Some(WorktreeState::Clean)
    );

    let api_workdir = workdir_parent.path().join("api-bot");
    let api_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/agents",
            serde_json::json!({
                "name": "api-bot",
                "from_ref": "main",
                "materialize": true,
                "workdir": api_workdir
            }),
        ),
    );
    assert_eq!(api_response.status, 201);
    let api_spawn: serde_json::Value = api_response.body_json().unwrap();
    assert_eq!(
        PathBuf::from(api_spawn["workdir"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        api_workdir.canonicalize().unwrap()
    );
    assert!(api_workdir.join("README.md").is_file());

    let mcp_workdir = workdir_parent.path().join("mcp-bot");
    let mcp_spawn = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "crabdb.agent_spawn",
                "arguments": {
                    "name": "mcp-bot",
                    "from_ref": "main",
                    "materialize": true,
                    "workdir": mcp_workdir
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_spawn["result"]["isError"], false);
    assert_eq!(
        PathBuf::from(
            mcp_spawn["result"]["structuredContent"]["workdir"]
                .as_str()
                .unwrap()
        )
        .canonicalize()
        .unwrap(),
        mcp_workdir.canonicalize().unwrap()
    );
    assert!(mcp_workdir.join("README.md").is_file());

    db.config_set("agent.worktrees_dir", ".crabdb/custom-worktrees")
        .unwrap();
    let configured = db
        .spawn_agent("configured-bot", Some("main"), true, None, None)
        .unwrap();
    let configured_workdir = PathBuf::from(configured.workdir.unwrap());
    assert!(configured_workdir.ends_with(".crabdb/custom-worktrees/configured-bot"));
    assert!(configured_workdir.join("README.md").is_file());

    let nonempty = workdir_parent.path().join("nonempty");
    fs::create_dir_all(&nonempty).unwrap();
    fs::write(nonempty.join("keep.txt"), "do not delete\n").unwrap();
    let err = db
        .spawn_agent_with_workdir(
            "nonempty-bot",
            Some("main"),
            true,
            None,
            None,
            Some(nonempty.clone()),
        )
        .unwrap_err();
    assert!(err.to_string().contains("must be empty or absent"));
    assert!(db.agent_details("nonempty-bot").is_err());
    assert_eq!(
        fs::read_to_string(nonempty.join("keep.txt")).unwrap(),
        "do not delete\n"
    );

    let unsafe_inside_workspace = temp.path().join("unsafe-agent-workdir");
    let err = db
        .spawn_agent_with_workdir(
            "unsafe-bot",
            Some("main"),
            true,
            None,
            None,
            Some(unsafe_inside_workspace),
        )
        .unwrap_err();
    assert!(err
        .to_string()
        .contains("agent workdirs inside the workspace must live under"));

    let err = db
        .spawn_agent_with_workdir(
            "disabled-bot",
            Some("main"),
            false,
            None,
            None,
            Some(workdir_parent.path().join("disabled-bot")),
        )
        .unwrap_err();
    assert!(err
        .to_string()
        .contains("custom agent workdir requires materialization"));
}

#[test]
fn agent_workdir_sync_refuses_dirty_and_force_refreshes() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let spawned = db
        .spawn_agent("doc-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = spawned.workdir.unwrap();
    let readme = std::path::Path::new(&workdir).join("README.md");
    fs::write(&readme, "hello\ndirty\n").unwrap();

    let err = db.sync_agent_workdir("doc-bot", false).unwrap_err();
    assert!(matches!(err, Error::DirtyWorktreeWithMessage(_)));
    drop(db);

    let synced = run_crabdb_json(
        temp.path(),
        &["agent", "sync-workdir", "doc-bot", "--force"],
    );
    assert_eq!(synced["forced"], true);
    assert!(synced["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "README.md"));
    assert_eq!(fs::read_to_string(&readme).unwrap(), "hello\n");

    let db = CrabDb::open(temp.path()).unwrap();
    let status = db.agent_status("doc-bot").unwrap();
    assert_eq!(status.workdir_state, Some(WorktreeState::Clean));
    drop(db);

    fs::remove_dir_all(&workdir).unwrap();
    let recreated = run_crabdb_json(temp.path(), &["agent", "sync-workdir", "doc-bot"]);
    assert_eq!(recreated["forced"], false);
    assert_eq!(fs::read_to_string(&readme).unwrap(), "hello\n");
}

#[test]
fn agent_workdir_watch_records_only_agent_branch() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let spawned = db
        .spawn_agent("doc-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = spawned.workdir.unwrap();
    let workdir_report = db.agent_workdir("doc-bot").unwrap();
    assert_eq!(workdir_report.workdir.as_deref(), Some(workdir.as_str()));

    fs::write(
        std::path::Path::new(&workdir).join("README.md"),
        "hello\nwatched\n",
    )
    .unwrap();
    let watched = db
        .watch_agent_workdir(
            "doc-bot",
            Some("watch workdir".to_string()),
            std::time::Duration::from_millis(0),
            Some(1),
        )
        .unwrap();
    assert_eq!(watched.iterations, 1);
    assert_eq!(watched.recorded_operations.len(), 1);
    assert_eq!(watched.changed_paths.len(), 1);

    let agent_status = db.agent_status("doc-bot").unwrap();
    assert_eq!(agent_status.workdir_state, Some(WorktreeState::Clean));
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "hello\n"
    );
    let main_status = db.status(Some("main")).unwrap();
    assert_eq!(main_status.worktree_state, WorktreeState::Clean);
}

#[test]
fn dirty_agent_workdir_must_be_recorded_before_merge() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let spawned = db
        .spawn_agent("doc-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = spawned.workdir.unwrap();
    fs::write(
        std::path::Path::new(&workdir).join("README.md"),
        "hello\nunrecorded\n",
    )
    .unwrap();
    let status = db.agent_status("doc-bot").unwrap();
    assert_eq!(status.workdir_state, Some(WorktreeState::DirtyTracked));
    assert_eq!(status.workdir_changed_paths.len(), 1);

    let err = db.merge_agent("doc-bot", "main").unwrap_err();
    assert!(matches!(err, Error::DirtyWorktreeWithMessage(_)));
    assert!(err.to_string().contains("agent record doc-bot"));

    db.enqueue_merge("doc-bot", "main", 0).unwrap();
    let run = db.run_merge_queue(None).unwrap();
    assert_eq!(run.processed.len(), 1);
    assert_eq!(run.processed[0].status, "failed");
    assert!(run.stopped_on_failure);
    assert!(run.processed[0]
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("unrecorded changes"));

    let recorded = db
        .record_agent_workdir("doc-bot", Some("record before merge".to_string()))
        .unwrap();
    assert!(recorded.operation.is_some());
    let clean_status = db.agent_status("doc-bot").unwrap();
    assert_eq!(clean_status.workdir_state, Some(WorktreeState::Clean));
    assert!(clean_status.workdir_changed_paths.is_empty());
    let merged = db.merge_agent("doc-bot", "main").unwrap();
    assert_eq!(merged.changed_paths.len(), 1);
    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "hello\nunrecorded\n"
    );
}

#[test]
fn advisory_leases_coordinate_agent_paths() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("doc-bot", Some("main"), false, None, None)
        .unwrap();
    db.spawn_agent("test-bot", Some("main"), false, None, None)
        .unwrap();

    let claim = db.claim_agent_path("doc-bot", "README.md", 600).unwrap();
    assert!(claim.claimed);
    assert_eq!(claim.path, "README.md");
    assert_eq!(claim.mode, "write");
    let lease = claim.lease.as_ref().unwrap();
    assert_eq!(lease.mode, "write");
    assert_eq!(lease.path.as_deref(), Some("README.md"));
    assert!(lease.file_id.is_some());

    let conflicting_claim = db.claim_agent_path("test-bot", "README.md", 600).unwrap();
    assert!(!conflicting_claim.claimed);
    assert_eq!(conflicting_claim.conflicts.len(), 1);
    assert!(conflicting_claim
        .warning
        .as_deref()
        .unwrap()
        .contains("already claimed"));

    let same = db
        .acquire_lease("doc-bot", Some("README.md"), "write", 3600)
        .unwrap();
    assert_eq!(same.lease.lease_id, lease.lease_id);

    let err = db
        .acquire_lease("test-bot", Some("README.md"), "read", 3600)
        .unwrap_err();
    assert!(matches!(err, Error::Conflict(_)));
    let active = db.list_leases(false).unwrap();
    assert_eq!(active.len(), 1);

    let released = db.release_lease(&lease.lease_id).unwrap();
    assert!(released.released);
    assert!(db.list_leases(false).unwrap().is_empty());

    let read_lease = db
        .acquire_lease("test-bot", Some("README.md"), "read", 3600)
        .unwrap();
    assert_eq!(read_lease.lease.mode, "read");

    let conn = Connection::open(temp.path().join(".crabdb/index/crabdb.sqlite")).unwrap();
    conn.execute("UPDATE leases SET expires_at = 0", [])
        .unwrap();
    assert!(db.list_leases(false).unwrap().is_empty());
    assert_eq!(db.list_leases(true).unwrap().len(), 1);
}

#[test]
fn agent_claims_are_soft_leases_across_cli_api_and_mcp() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("doc-bot", Some("main"), false, None, None)
        .unwrap();
    db.spawn_agent("test-bot", Some("main"), false, None, None)
        .unwrap();
    drop(db);

    let cli_claim = run_crabdb_json(
        temp.path(),
        &[
            "agent",
            "claim",
            "doc-bot",
            "README.md",
            "--ttl-secs",
            "120",
        ],
    );
    assert_eq!(cli_claim["claimed"], true);
    assert_eq!(cli_claim["path"], "README.md");
    assert_eq!(cli_claim["lease"]["mode"], "write");
    let cli_lease_id = cli_claim["lease"]["lease_id"].as_str().unwrap().to_string();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let api_conflict = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/agents/test-bot/claims",
            serde_json::json!({
                "path": "README.md",
                "ttl_secs": 120
            }),
        ),
    );
    assert_eq!(api_conflict.status, 200);
    let api_conflict: serde_json::Value = api_conflict.body_json().unwrap();
    assert_eq!(api_conflict["claimed"], false);
    assert_eq!(api_conflict["conflicts"][0]["lease_id"], cli_lease_id);
    assert!(api_conflict["warning"]
        .as_str()
        .unwrap()
        .contains("already claimed"));

    let tools = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }),
    )
    .unwrap();
    let tool_list = tools["result"]["tools"].as_array().unwrap();
    assert!(tool_list
        .iter()
        .any(|tool| tool["name"] == "crabdb.agent_claim"));

    let mcp_conflict = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "crabdb.agent_claim",
                "arguments": {
                    "agent": "test-bot",
                    "path": "README.md"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_conflict["result"]["isError"], false);
    assert_eq!(
        mcp_conflict["result"]["structuredContent"]["claimed"],
        false
    );

    let same_claim = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "crabdb.agent_claim",
                "arguments": {
                    "agent": "doc-bot",
                    "path": "README.md"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(same_claim["result"]["isError"], false);
    assert_eq!(same_claim["result"]["structuredContent"]["claimed"], true);
    assert_eq!(
        same_claim["result"]["structuredContent"]["lease"]["lease_id"],
        cli_lease_id
    );
}

#[test]
fn local_api_and_mcp_expose_advisory_leases() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("doc-bot", Some("main"), false, None, None)
        .unwrap();
    db.spawn_agent("test-bot", Some("main"), false, None, None)
        .unwrap();

    let acquired = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/leases",
            serde_json::json!({
                "agent": "doc-bot",
                "path": "README.md",
                "mode": "write",
                "ttl_secs": 120
            }),
        ),
    );
    assert_eq!(acquired.status, 201);
    let acquired: serde_json::Value = acquired.body_json().unwrap();
    assert_eq!(acquired["lease"]["ref_name"], "refs/agents/doc-bot");
    assert_eq!(acquired["lease"]["path"], "README.md");
    assert_eq!(acquired["lease"]["mode"], "write");
    let lease_id = acquired["lease"]["lease_id"].as_str().unwrap().to_string();

    let listed = crabdb::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/leases", serde_json::Value::Null),
    );
    assert_eq!(listed.status, 200);
    let listed: serde_json::Value = listed.body_json().unwrap();
    assert!(listed
        .as_array()
        .unwrap()
        .iter()
        .any(|lease| lease["lease_id"] == lease_id));

    let tools = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }),
    )
    .unwrap();
    let tool_list = tools["result"]["tools"].as_array().unwrap();
    for name in [
        "crabdb.lease_acquire",
        "crabdb.lease_list",
        "crabdb.lease_release",
    ] {
        assert!(tool_list.iter().any(|tool| tool["name"] == name), "{name}");
    }

    let conflicting = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "crabdb.lease_acquire",
                "arguments": {
                    "agent": "test-bot",
                    "path": "README.md",
                    "mode": "read",
                    "ttl_secs": 120
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(conflicting["result"]["isError"], true);
    assert!(conflicting["result"]["structuredContent"]["message"]
        .as_str()
        .unwrap()
        .contains("active lease conflict"));

    let released = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "DELETE",
            &format!("/v1/leases/{lease_id}"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(released.status, 200);
    let released: serde_json::Value = released.body_json().unwrap();
    assert_eq!(released["released"], true);

    let read_lease = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "crabdb.lease_acquire",
                "arguments": {
                    "agent": "test-bot",
                    "path": "README.md",
                    "mode": "read"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(read_lease["result"]["isError"], false);
    assert_eq!(
        read_lease["result"]["structuredContent"]["lease"]["mode"],
        "read"
    );

    let all_leases = crabdb::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/leases?all=true", serde_json::Value::Null),
    );
    assert_eq!(all_leases.status, 200);
    let all_leases: serde_json::Value = all_leases.body_json().unwrap();
    assert_eq!(all_leases.as_array().unwrap().len(), 1);

    let mcp_list = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "crabdb.lease_list",
                "arguments": {}
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_list["result"]["isError"], false);
    assert_eq!(
        mcp_list["result"]["structuredContent"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let read_lease_id = read_lease["result"]["structuredContent"]["lease"]["lease_id"]
        .as_str()
        .unwrap();
    let mcp_release = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "crabdb.lease_release",
                "arguments": {
                    "lease_id": read_lease_id
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_release["result"]["isError"], false);
    assert_eq!(mcp_release["result"]["structuredContent"]["released"], true);
}

#[test]
fn merge_agent_and_queue_enforce_readiness_blockers() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("approval-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "approval gated edit",
        "edits": [
            {"op": "write", "path": "docs/approval.md", "content": "needs approval\n"}
        ]
    }))
    .unwrap();
    db.apply_agent_patch("approval-bot", patch).unwrap();
    db.request_agent_approval(
        "approval-bot",
        "deploy.preview",
        "Publish preview before merge",
        None,
        None,
        None,
    )
    .unwrap();

    let dry_run = db
        .merge_agent_with_options("approval-bot", "main", true)
        .unwrap();
    assert_eq!(dry_run.changed_paths.len(), 1);
    let direct_err = db.merge_agent("approval-bot", "main").unwrap_err();
    assert!(matches!(direct_err, Error::InvalidInput(_)));
    assert!(direct_err.to_string().contains("not merge-ready"));
    assert!(direct_err.to_string().contains("pending_approvals"));

    db.enqueue_merge("approval-bot", "main", 0).unwrap();
    let run = db.run_merge_queue(None).unwrap();
    assert_eq!(run.processed.len(), 1);
    assert_eq!(run.processed[0].status, "failed");
    assert!(run.stopped_on_failure);
    assert!(run.processed[0]
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("pending_approvals"));
    assert!(!temp.path().join("docs/approval.md").exists());

    db.spawn_agent("test-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = db.agent_workdir("test-bot").unwrap().workdir.unwrap();
    fs::create_dir_all(std::path::Path::new(&workdir).join("docs")).unwrap();
    fs::write(
        std::path::Path::new(&workdir).join("docs/test.md"),
        "needs tests\n",
    )
    .unwrap();
    db.record_agent_workdir("test-bot", Some("test gated edit".to_string()))
        .unwrap();
    let failed = db
        .run_agent_test(
            "test-bot",
            vec!["sh".to_string(), "-c".to_string(), "exit 7".to_string()],
            None,
            30,
        )
        .unwrap();
    assert!(!failed.success);
    let readiness = db.agent_readiness("test-bot").unwrap();
    assert!(readiness
        .blockers
        .iter()
        .any(|issue| issue.code == "latest_test_failed"));
    let test_err = db.merge_agent("test-bot", "main").unwrap_err();
    assert!(matches!(test_err, Error::InvalidInput(_)));
    assert!(test_err.to_string().contains("latest_test_failed"));
}

#[test]
fn required_gate_config_blocks_merge_until_test_and_eval_pass() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("strict-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = db.agent_workdir("strict-bot").unwrap().workdir.unwrap();
    fs::create_dir_all(Path::new(&workdir).join("docs")).unwrap();
    fs::write(Path::new(&workdir).join("docs/strict.md"), "strict gates\n").unwrap();
    db.record_agent_workdir("strict-bot", Some("strict gated edit".to_string()))
        .unwrap();

    db.config_set("agent.require_test_gate", "true").unwrap();
    db.config_set("agent.required_test_suites", "unit").unwrap();
    let readiness = db.agent_readiness("strict-bot").unwrap();
    assert_eq!(readiness.status, "blocked");
    assert!(readiness
        .blockers
        .iter()
        .any(|issue| issue.code == "missing_latest_test"));
    assert!(readiness
        .blockers
        .iter()
        .any(|issue| issue.code == "missing_required_test_suite"));
    assert!(readiness
        .warnings
        .iter()
        .any(|issue| issue.code == "missing_latest_eval"));

    let dry_run = db
        .merge_agent_with_options("strict-bot", "main", true)
        .unwrap();
    assert_eq!(dry_run.changed_paths.len(), 1);
    let missing_test = db.merge_agent("strict-bot", "main").unwrap_err();
    assert!(matches!(missing_test, Error::InvalidInput(_)));
    assert!(missing_test.to_string().contains("missing_latest_test"));
    assert!(!temp.path().join("docs/strict.md").exists());

    let passed_test = db
        .run_agent_test_with_options(
            "strict-bot",
            vec!["sh".to_string(), "-c".to_string(), "exit 0".to_string()],
            None,
            30,
            AgentGateOptions {
                suite: Some("unit".to_string()),
                score: None,
                threshold: None,
            },
        )
        .unwrap();
    assert!(passed_test.success);
    assert_eq!(passed_test.suite.as_deref(), Some("unit"));
    let readiness = db.agent_readiness("strict-bot").unwrap();
    assert!(readiness.ready);
    assert!(readiness
        .warnings
        .iter()
        .any(|issue| issue.code == "missing_latest_eval"));

    db.config_set("agent.require_eval_gate", "true").unwrap();
    db.config_set("agent.required_eval_suites", "policy-smoke")
        .unwrap();
    let readiness = db.agent_readiness("strict-bot").unwrap();
    assert_eq!(readiness.status, "blocked");
    assert!(readiness
        .blockers
        .iter()
        .any(|issue| issue.code == "missing_latest_eval"));
    assert!(readiness
        .blockers
        .iter()
        .any(|issue| issue.code == "missing_required_eval_suite"));
    let missing_eval = db.merge_agent("strict-bot", "main").unwrap_err();
    assert!(matches!(missing_eval, Error::InvalidInput(_)));
    assert!(missing_eval.to_string().contains("missing_latest_eval"));

    let failed_eval = db
        .run_agent_eval_with_options(
            "strict-bot",
            vec!["sh".to_string(), "-c".to_string(), "exit 0".to_string()],
            None,
            30,
            AgentGateOptions {
                suite: Some("policy-smoke".to_string()),
                score: Some(0.4),
                threshold: Some(0.9),
            },
        )
        .unwrap();
    assert!(!failed_eval.success);
    assert_eq!(failed_eval.suite.as_deref(), Some("policy-smoke"));
    let readiness = db.agent_readiness("strict-bot").unwrap();
    assert!(readiness
        .blockers
        .iter()
        .any(|issue| issue.code == "required_eval_suite_failed"));
    let failed_suite = db.merge_agent("strict-bot", "main").unwrap_err();
    assert!(matches!(failed_suite, Error::InvalidInput(_)));
    assert!(failed_suite
        .to_string()
        .contains("required_eval_suite_failed"));

    let passed_eval = db
        .run_agent_eval_with_options(
            "strict-bot",
            vec!["sh".to_string(), "-c".to_string(), "exit 0".to_string()],
            None,
            30,
            AgentGateOptions {
                suite: Some("policy-smoke".to_string()),
                score: Some(0.95),
                threshold: Some(0.9),
            },
        )
        .unwrap();
    assert!(passed_eval.success);
    assert_eq!(passed_eval.suite.as_deref(), Some("policy-smoke"));
    let readiness = db.agent_readiness("strict-bot").unwrap();
    assert!(readiness.ready);
    assert!(readiness.blockers.is_empty());

    db.merge_agent("strict-bot", "main").unwrap();
    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("docs/strict.md")).unwrap(),
        "strict gates\n"
    );
}

#[test]
fn merge_queue_runs_agent_branch_into_main() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_str(
        r##"{
          "message": "agent adds docs",
          "edits": [
            {"op": "write", "path": "docs/guide.md", "content": "# Guide\n"}
          ]
        }"##,
    )
    .unwrap();
    db.apply_agent_patch("doc-bot", patch).unwrap();

    let queued = db.enqueue_merge("doc-bot", "main", 10).unwrap();
    assert_eq!(queued.entry.status, "queued");
    assert_eq!(db.list_merge_queue().unwrap().len(), 1);

    let run = db.run_merge_queue(None).unwrap();
    assert_eq!(run.processed.len(), 1);
    assert_eq!(run.processed[0].status, "merged");
    assert!(!run.stopped_on_conflict);
    assert!(!run.stopped_on_failure);

    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("docs/guide.md")).unwrap(),
        "# Guide\n"
    );

    let conn = Connection::open(temp.path().join(".crabdb/index/crabdb.sqlite")).unwrap();
    let queue_status: String = conn
        .query_row(
            "SELECT status FROM merge_queue WHERE queue_id = ?1",
            [&queued.entry.queue_id],
            |row| row.get(0),
        )
        .unwrap();
    let merge_results: i64 = conn
        .query_row("SELECT COUNT(*) FROM merge_results", [], |row| row.get(0))
        .unwrap();
    assert_eq!(queue_status, "merged");
    assert_eq!(merge_results, 1);
}

#[test]
fn merge_queue_pauses_on_conflict() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_str(
        r#"{
          "message": "agent edits readme",
          "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nagent\n"}
          ]
        }"#,
    )
    .unwrap();
    db.apply_agent_patch("doc-bot", patch).unwrap();

    fs::write(temp.path().join("README.md"), "hello\nhuman\n").unwrap();
    db.record(
        Some("main"),
        Some("human edit".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();
    let queued = db.enqueue_merge("doc-bot", "main", 0).unwrap();

    let run = db.run_merge_queue(None).unwrap();
    assert_eq!(run.processed.len(), 1);
    assert_eq!(run.processed[0].status, "conflicted");
    assert!(run.stopped_on_conflict);
    assert!(!run.stopped_on_failure);

    let conn = Connection::open(temp.path().join(".crabdb/index/crabdb.sqlite")).unwrap();
    let queue_status: String = conn
        .query_row(
            "SELECT status FROM merge_queue WHERE queue_id = ?1",
            [&queued.entry.queue_id],
            |row| row.get(0),
        )
        .unwrap();
    let conflict_sets: i64 = conn
        .query_row("SELECT COUNT(*) FROM conflict_sets", [], |row| row.get(0))
        .unwrap();
    assert_eq!(queue_status, "conflicted");
    assert_eq!(conflict_sets, 1);

    let conflicts = db.list_conflicts().unwrap();
    assert_eq!(conflicts.len(), 1);
    assert!(conflicts[0]
        .details
        .iter()
        .any(|detail| detail.contains("README.md")));
    let shown = db.show_conflict(&conflicts[0].conflict_set_id).unwrap();
    assert_eq!(shown.conflict_set_id, conflicts[0].conflict_set_id);

    let resolved = db
        .resolve_conflict(&conflicts[0].conflict_set_id, "source")
        .unwrap();
    assert_eq!(resolved.resolution, "source");
    assert_eq!(resolved.changed_paths.len(), 1);
    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "hello\nagent\n"
    );

    let queue_status: String = conn
        .query_row(
            "SELECT status FROM merge_queue WHERE queue_id = ?1",
            [&queued.entry.queue_id],
            |row| row.get(0),
        )
        .unwrap();
    let conflict_status: String = conn
        .query_row(
            "SELECT status FROM conflict_sets WHERE conflict_set_id = ?1",
            [&conflicts[0].conflict_set_id],
            |row| row.get(0),
        )
        .unwrap();
    let result_change: String = conn
        .query_row(
            "SELECT result_change FROM merge_results WHERE conflict_set = ?1",
            [&conflicts[0].conflict_set_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(queue_status, "merged");
    assert_eq!(conflict_status, "resolved");
    assert_eq!(result_change, resolved.operation.0);
}

#[test]
fn manual_conflict_resolution_works_through_db_cli_http_and_mcp() {
    let (temp, mut db, conflict_id) =
        conflicted_readme_workspace("hello\nagent-db\n", "hello\nhuman-db\n");
    let report = db
        .resolve_conflict_manual(
            &conflict_id,
            ConflictManualResolution {
                files: BTreeMap::from([(
                    "README.md".to_string(),
                    ConflictManualFile::Text("hello\nmanual-db\n".to_string()),
                )]),
            },
        )
        .unwrap();
    assert_eq!(report.resolution, "manual");
    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "hello\nmanual-db\n"
    );

    let (temp, db, conflict_id) =
        conflicted_readme_workspace("hello\nagent-cli\n", "hello\nhuman-cli\n");
    drop(db);
    let resolution_path = temp.path().join("resolution.json");
    fs::write(
        &resolution_path,
        serde_json::to_vec(&serde_json::json!({
            "README.md": "hello\nmanual-cli\n"
        }))
        .unwrap(),
    )
    .unwrap();
    let resolved = run_crabdb_json(
        temp.path(),
        &[
            "conflicts",
            "resolve",
            &conflict_id,
            "--manual",
            resolution_path.to_str().unwrap(),
        ],
    );
    assert_eq!(resolved["resolution"], "manual");
    let mut db = CrabDb::open(temp.path()).unwrap();
    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "hello\nmanual-cli\n"
    );

    let (temp, mut db, conflict_id) =
        conflicted_readme_workspace("hello\nagent-api\n", "hello\nhuman-api\n");
    let resolved = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/conflicts/{conflict_id}/resolve"),
            serde_json::json!({
                "manual": {
                    "files": {
                        "README.md": {
                            "content": "hello\nmanual-api\n"
                        }
                    }
                }
            }),
        ),
    );
    assert_eq!(resolved.status, 200);
    let resolved: serde_json::Value = resolved.body_json().unwrap();
    assert_eq!(resolved["resolution"], "manual");
    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "hello\nmanual-api\n"
    );

    let (temp, mut db, conflict_id) =
        conflicted_readme_workspace("hello\nagent-mcp\n", "hello\nhuman-mcp\n");
    let resolved = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "crabdb.conflict_resolve",
                "arguments": {
                    "conflict_set_id": conflict_id,
                    "manual": {
                        "files": {
                            "README.md": "hello\nmanual-mcp\n"
                        }
                    }
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(resolved["result"]["isError"], false);
    assert_eq!(
        resolved["result"]["structuredContent"]["resolution"],
        "manual"
    );
    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "hello\nmanual-mcp\n"
    );
}

#[test]
fn local_api_and_mcp_drive_merge_queue_and_conflicts() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("api-queue-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_str(
        r#"{
          "message": "agent edits readme",
          "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nagent-api\n"}
          ]
        }"#,
    )
    .unwrap();
    db.apply_agent_patch("api-queue-bot", patch).unwrap();

    fs::write(temp.path().join("README.md"), "hello\nhuman-api\n").unwrap();
    db.record(
        Some("main"),
        Some("human edit".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();

    let queued = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/merge-queue",
            serde_json::json!({
                "source": "api-queue-bot",
                "target": "main",
                "priority": 5
            }),
        ),
    );
    assert_eq!(queued.status, 201);
    let queued: serde_json::Value = queued.body_json().unwrap();
    assert_eq!(queued["entry"]["status"], "queued");
    let queue_id = queued["entry"]["queue_id"].as_str().unwrap().to_string();

    let listed = crabdb::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/merge-queue", serde_json::Value::Null),
    );
    assert_eq!(listed.status, 200);
    let listed: serde_json::Value = listed.body_json().unwrap();
    assert!(listed
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["queue_id"] == queue_id));

    let tools = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }),
    )
    .unwrap();
    let tool_list = tools["result"]["tools"].as_array().unwrap();
    for name in [
        "crabdb.merge_queue_add",
        "crabdb.merge_queue_list",
        "crabdb.merge_queue_run",
        "crabdb.merge_queue_remove",
        "crabdb.conflict_list",
        "crabdb.conflict_show",
        "crabdb.conflict_resolve",
    ] {
        assert!(tool_list.iter().any(|tool| tool["name"] == name), "{name}");
    }

    let run = crabdb::server::handle_http_request(
        &mut db,
        &api_request("POST", "/v1/merge-queue/run", serde_json::json!({})),
    );
    assert_eq!(run.status, 200);
    let run: serde_json::Value = run.body_json().unwrap();
    assert_eq!(run["processed"][0]["status"], "conflicted");
    assert_eq!(run["stopped_on_conflict"], true);

    let conflicts = crabdb::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/conflicts", serde_json::Value::Null),
    );
    assert_eq!(conflicts.status, 200);
    let conflicts: serde_json::Value = conflicts.body_json().unwrap();
    let conflict_id = conflicts[0]["conflict_set_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(conflicts[0]["details"]
        .as_array()
        .unwrap()
        .iter()
        .any(|detail| detail.as_str().unwrap().contains("README.md")));

    let shown = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            &format!("/v1/conflicts/{conflict_id}"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(shown.status, 200);
    let shown: serde_json::Value = shown.body_json().unwrap();
    assert_eq!(shown["conflict_set_id"], conflict_id);

    let mcp_show = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "crabdb.conflict_show",
                "arguments": {
                    "conflict_set_id": conflict_id.clone()
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_show["result"]["isError"], false);
    assert_eq!(
        mcp_show["result"]["structuredContent"]["conflict_set_id"],
        conflict_id
    );

    let mcp_resolve = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "crabdb.conflict_resolve",
                "arguments": {
                    "conflict_set_id": conflict_id,
                    "take": "source"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_resolve["result"]["isError"], false);
    assert_eq!(
        mcp_resolve["result"]["structuredContent"]["resolution"],
        "source"
    );

    let queue = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "crabdb.merge_queue_list",
                "arguments": {}
            }
        }),
    )
    .unwrap();
    assert_eq!(queue["result"]["isError"], false);
    assert!(queue["result"]["structuredContent"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["queue_id"] == queue_id && entry["status"] == "merged"));

    db.spawn_agent("cancel-queue-bot", Some("main"), false, None, None)
        .unwrap();
    let mcp_add = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "crabdb.merge_queue_add",
                "arguments": {
                    "source": "cancel-queue-bot",
                    "target": "main",
                    "priority": 1
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_add["result"]["isError"], false);
    let cancel_queue_id = mcp_add["result"]["structuredContent"]["entry"]["queue_id"]
        .as_str()
        .unwrap()
        .to_string();

    let removed = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "DELETE",
            &format!("/v1/merge-queue/{cancel_queue_id}"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(removed.status, 200);
    let removed: serde_json::Value = removed.body_json().unwrap();
    assert_eq!(removed["entry"]["status"], "cancelled");

    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "hello\nagent-api\n"
    );
}

#[test]
fn copying_a_file_allocates_a_new_file_identity() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), "same\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    fs::write(temp.path().join("b.txt"), "same\n").unwrap();
    let mut db = CrabDb::open(temp.path()).unwrap();
    let record = db
        .record(Some("main"), None, Actor::human(), false)
        .unwrap();
    assert!(record.operation.is_some());

    let fsck = db.fsck().unwrap();
    assert!(fsck.errors.is_empty(), "{:?}", fsck.errors);
    assert_eq!(
        db.status(Some("main")).unwrap().worktree_state,
        WorktreeState::Clean
    );
}

#[test]
fn status_does_not_persist_unreferenced_objects() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let conn = Connection::open(temp.path().join(".crabdb/index/crabdb.sqlite")).unwrap();
    let before: i64 = conn
        .query_row("SELECT COUNT(*) FROM objects", [], |row| row.get(0))
        .unwrap();

    fs::write(temp.path().join("README.md"), "hello\nstatus\n").unwrap();
    let db = CrabDb::open(temp.path()).unwrap();
    let status = db.status(Some("main")).unwrap();
    assert_ne!(status.worktree_state, WorktreeState::Clean);

    let after: i64 = conn
        .query_row("SELECT COUNT(*) FROM objects", [], |row| row.get(0))
        .unwrap();
    assert_eq!(before, after);
}

#[test]
fn workspace_lock_blocks_mutating_operations() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    fs::write(temp.path().join("README.md"), "changed\n").unwrap();
    fs::write(temp.path().join(".crabdb/lock"), "test writer").unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let err = db
        .record(Some("main"), None, Actor::human(), false)
        .unwrap_err();
    assert!(matches!(err, Error::WorkspaceLocked(_)));
}

#[test]
fn agent_patch_records_message_and_event_indexes() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_str(
        r#"{
          "session_id": "session-test",
          "message": "agent edits readme",
          "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nagent\n"}
          ]
        }"#,
    )
    .unwrap();
    db.apply_agent_patch("doc-bot", patch).unwrap();

    let conn = Connection::open(temp.path().join(".crabdb/index/crabdb.sqlite")).unwrap();
    let messages: i64 = conn
        .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
        .unwrap();
    let events: i64 = conn
        .query_row("SELECT COUNT(*) FROM agent_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(messages, 1);
    assert_eq!(events, 2);
}

#[test]
fn show_history_and_code_from_use_recorded_indexes() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_str(
        r#"{
          "session_id": "session-show",
          "message": "agent adds line",
          "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nagent\n"}
          ]
        }"#,
    )
    .unwrap();
    let applied = db.apply_agent_patch("doc-bot", patch).unwrap();

    match db.show(&applied.operation.0).unwrap() {
        ShowResult::Operation { value } => {
            assert_eq!(value.operation.change_id, applied.operation);
            assert_eq!(value.changed_paths.len(), 1);
            assert_eq!(value.messages.len(), 1);
        }
        other => panic!("expected operation show result, got {other:?}"),
    }

    let conn = Connection::open(temp.path().join(".crabdb/index/crabdb.sqlite")).unwrap();
    let message_id: String = conn
        .query_row("SELECT message_id FROM messages LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    match db.show(&message_id).unwrap() {
        ShowResult::Message { value } => assert_eq!(value.body, "agent adds line"),
        other => panic!("expected message show result, got {other:?}"),
    }

    let file_history = db.history_for_path("README.md").unwrap();
    assert!(file_history.file_history.len() >= 2);

    let why = db.why("README.md:2", Some("refs/agents/doc-bot")).unwrap();
    let line_id = format!("{}:{}", why.line_id.origin_change.0, why.line_id.local_seq);
    let line_history = db.history_for_line_id(&line_id).unwrap();
    assert!(!line_history.line_history.is_empty());

    let by_agent = db.code_from("agent:doc-bot").unwrap();
    assert!(by_agent
        .operations
        .iter()
        .any(|operation| operation.change_id == applied.operation));
    let by_session = db.code_from("session-show").unwrap();
    assert_eq!(by_session.operations.len(), 1);
    assert_eq!(by_session.operations[0].change_id, applied.operation);
}

#[test]
fn index_rebuild_restores_derived_history_from_objects() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_agent("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_str(
        r#"{
          "session_id": "session-rebuild",
          "message": "agent edits readme",
          "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nagent\n"}
          ]
        }"#,
    )
    .unwrap();
    db.apply_agent_patch("doc-bot", patch).unwrap();

    let conn = Connection::open(temp.path().join(".crabdb/index/crabdb.sqlite")).unwrap();
    conn.execute_batch(
        "\
        DELETE FROM operations;
        DELETE FROM operation_parents;
        DELETE FROM file_history;
        DELETE FROM line_history;
        DELETE FROM messages;
        ",
    )
    .unwrap();
    assert!(db.timeline(None, 10).unwrap().is_empty());

    let report = db.rebuild_indexes().unwrap();
    assert_eq!(report.errors, Vec::<String>::new());
    assert_eq!(report.messages, 1);
    assert!(report.operations >= 2);
    assert!(!db.timeline(None, 10).unwrap().is_empty());
    let messages: i64 = conn
        .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(messages, 1);
}

#[test]
fn gc_prunes_unreachable_known_objects_and_preserves_reachable_roots() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let conn = Connection::open(temp.path().join(".crabdb/index/crabdb.sqlite")).unwrap();
    conn.execute(
        "INSERT INTO objects \
         (object_id, kind, version, codec, hash_alg, size_bytes, bytes, created_at) \
         VALUES ('obj_unreachable_test', 'Blob', 1, 'cbor', 'sha256', 0, x'', 0)",
        [],
    )
    .unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let dry_run = db.gc(true).unwrap();
    assert!(dry_run.prunable_objects >= 1);
    assert_eq!(dry_run.pruned_objects, 0);

    let report = db.gc(false).unwrap();
    assert!(report.pruned_objects >= 1);
    let still_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM objects WHERE object_id = 'obj_unreachable_test'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(still_exists, 0);
    assert!(db.fsck().unwrap().errors.is_empty());
}
