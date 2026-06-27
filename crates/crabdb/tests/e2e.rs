use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

use crabdb::{
    Actor, ConflictManualFile, ConflictManualResolution, CrabDb, Error, InitImportMode,
    LaneGateOptions, LaneMessageReport, LanePatchReport, LaneRewindReport, LaneTurnDetails,
    LaneTurnEndReport, LaneTurnEventReport, LaneTurnStartReport, OperationKind, PatchDocument,
    ShowResult, TextContent, TextRepresentation, WorktreeState,
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

fn patch_with_lane_head(db: &CrabDb, lane: &str, mut patch: PatchDocument) -> PatchDocument {
    if patch.base_change.is_none() {
        patch.base_change = Some(db.lane_details(lane).unwrap().branch.head_change.0);
    }
    patch
}

fn apply_lane_patch_at_head(
    db: &mut CrabDb,
    lane: &str,
    patch: PatchDocument,
) -> Result<LanePatchReport, Error> {
    let patch = patch_with_lane_head(db, lane, patch);
    db.apply_lane_patch(lane, patch)
}

fn only_conflict_path_class(db: &CrabDb) -> (String, String) {
    let conflicts = db.list_conflicts().unwrap();
    assert_eq!(conflicts.len(), 1);
    let shown = db.show_conflict(&conflicts[0].conflict_set_id).unwrap();
    let explanation = shown.explanation.as_ref().unwrap();
    assert_eq!(explanation.paths.len(), 1);
    (
        explanation.paths[0].path.clone(),
        explanation.paths[0].conflict_class.clone(),
    )
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

fn run_crabdb_json_daemon(workspace: &Path, daemon_url: &str, args: &[&str]) -> serde_json::Value {
    let output = Command::new(crabdb_bin())
        .arg("--workspace")
        .arg(workspace)
        .arg("--daemon-url")
        .arg(daemon_url)
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "crabdb --daemon-url {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn wait_for_child_exit(child: &mut Child) {
    for _ in 0..100 {
        if child.try_wait().unwrap().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("daemon did not exit");
}

fn wait_for_daemon_endpoint(path: &Path) -> serde_json::Value {
    for _ in 0..100 {
        if let Ok(bytes) = fs::read(path) {
            if let Ok(value) = serde_json::from_slice(&bytes) {
                return value;
            }
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("daemon endpoint was not published at {}", path.display());
}

struct DaemonGuard {
    child: Child,
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_loopback_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn wait_for_daemon_health(port: u16) {
    for _ in 0..100 {
        if daemon_health_ok(port) {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("daemon did not become healthy on port {port}");
}

fn daemon_health_ok(port: u16) -> bool {
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    if stream
        .write_all(b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut response = String::new();
    stream.read_to_string(&mut response).is_ok() && response.contains(" 200 ")
}

fn raw_http_request(port: u16, request: &[u8]) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let _ = stream.set_read_timeout(Some(Duration::from_millis(1000)));
    stream.write_all(request).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn api_request(method: &str, path: &str, body: serde_json::Value) -> Vec<u8> {
    api_request_with_headers(method, path, &[], body)
}

fn conflicted_readme_workspace(
    lane_content: &str,
    human_content: &str,
) -> (tempfile::TempDir, CrabDb, String) {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("manual-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "lane edits readme",
        "edits": [
            {"op": "write", "path": "README.md", "content": lane_content}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "manual-bot", patch).unwrap();

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
                && check.details.as_ref().unwrap()["sqlite_user_version"] == 2
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

    db.spawn_lane("doctor-bot", Some("main"), false, None, None)
        .unwrap();
    db.request_lane_approval(
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
fn init_creates_lane_observability_indexes() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let conn = Connection::open(temp.path().join(".crabdb/index/crabdb.sqlite")).unwrap();
    let indexes = {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'index'")
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
    };
    for expected in [
        "lane_turns_session_started_idx",
        "lane_turns_lane_started_idx",
        "lane_events_lane_created_idx",
        "lane_events_session_created_idx",
        "lane_events_turn_created_idx",
        "lane_events_type_created_idx",
        "lane_events_lane_type_created_idx",
        "lane_events_session_type_created_idx",
        "lane_events_turn_type_created_idx",
        "lane_trace_span_events_span_created_idx",
        "lane_trace_span_events_trace_created_idx",
        "lane_acp_sessions_lane_idx",
        "lane_acp_sessions_crabdb_session_idx",
        "external_mutation_audit_created_idx",
        "external_mutation_audit_surface_created_idx",
        "external_mutation_audit_lane_created_idx",
        "http_idempotency_keys_updated_idx",
    ] {
        assert!(
            indexes.iter().any(|index| index == expected),
            "missing index {expected}"
        );
    }
}

#[test]
fn acp_setup_commands_report_profiles_install_and_doctor() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let profiles = run_crabdb_json(temp.path(), &["acp", "list"]);
    assert!(profiles
        .as_array()
        .unwrap()
        .iter()
        .any(|profile| profile["agent"] == "claude-code"));

    let install = run_crabdb_json(
        temp.path(),
        &[
            "acp",
            "install",
            "--agent",
            "claude-code",
            "--editor",
            "generic",
            "--dry-run",
        ],
    );
    assert_eq!(install["agent"], "claude-code");
    assert!(install["relay_command"]
        .as_array()
        .unwrap()
        .iter()
        .any(|part| part == "relay"));
    assert!(install["snippet"]
        .as_str()
        .unwrap()
        .contains("crabdb acp relay"));

    let doctor = run_crabdb_json(temp.path(), &["acp", "doctor", "--agent", "fake"]);
    assert_eq!(doctor["status"], "ok");
    let checks = doctor["checks"].as_array().unwrap();
    assert!(checks
        .iter()
        .any(|check| check["name"] == "workspace" && check["status"] == "ok"));
    for expected in ["initialize", "session", "prompt", "mapping", "capture"] {
        assert!(
            checks
                .iter()
                .any(|check| check["name"] == expected && check["status"] == "ok"),
            "missing doctor check {expected}: {checks:?}"
        );
    }
    assert!(doctor["lane"].as_str().is_some());
    assert!(doctor["session_id"].as_str().is_some());

    let second_doctor = run_crabdb_json(temp.path(), &["acp", "doctor", "--agent", "fake"]);
    assert_eq!(second_doctor["status"], "ok");
    assert_ne!(doctor["session_id"], second_doctor["session_id"]);
    let doctor_sessions =
        run_crabdb_json(temp.path(), &["acp", "sessions", "--lane", "acp-doctor"]);
    let doctor_acp_session_ids = doctor_sessions["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|session| session["acp_session_id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(doctor_acp_session_ids.len(), 2);
    assert!(doctor_acp_session_ids
        .iter()
        .all(|session_id| session_id.starts_with("sess_fake_acp_doctor_")));
    assert_ne!(doctor_acp_session_ids[0], doctor_acp_session_ids[1]);

    let demo = run_crabdb_json(temp.path(), &["demo", "acp", "--agent", "fake"]);
    assert_eq!(demo["status"], "ok");
    assert_eq!(demo["lane"], "acp-demo");
    assert!(demo["session_id"].as_str().is_some());
    let demo_workspace = demo["workspace"].as_str().unwrap();
    assert!(Path::new(demo_workspace).join(".crabdb").exists());
}

#[cfg(unix)]
#[test]
fn acp_relay_captures_session_prompt_mcp_and_workdir_edits() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let fake_agent = temp.path().join("fake-acp-agent.sh");
    let session_request_log = temp.path().join("session-new.jsonl");
    let lane_workdir = temp
        .path()
        .canonicalize()
        .unwrap()
        .join(".crabdb/worktrees/acp-test");
    fs::write(
        &fake_agent,
        format!(
            r#"#!/bin/sh
set -eu
IFS= read -r init
printf '%s\n' '{{"jsonrpc":"2.0","id":0,"result":{{"protocolVersion":1,"agentCapabilities":{{}}}}}}'
IFS= read -r session_new
printf '%s\n' "$session_new" > "{}"
printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"sessionId":"sess_fake"}}}}'
IFS= read -r prompt
printf '%s\n' '{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"sess_fake","update":{{"sessionUpdate":"available_commands_update","commands":[{{"name":"write_file","description":"large command description"}}]}}}}}}'
printf '%s\n' '{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"sess_fake","update":{{"sessionUpdate":"tool_call","toolCallId":"tool_1","title":"write README","kind":"edit","status":"pending"}}}}}}'
printf '%s\n' '{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"sess_fake","update":{{"sessionUpdate":"tool_call_update","toolCallId":"tool_1","status":"completed"}}}}}}'
printf '%s\n' '{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"sess_fake","update":{{"sessionUpdate":"agent_message_chunk","messageId":"msg_1","content":{{"type":"text","text":"done"}}}}}}}}'
printf '%s\n' 'changed by fake agent' > "{}/README.md"
printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"stopReason":"end_turn"}}}}'
"#,
            session_request_log.display(),
            lane_workdir.display()
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&fake_agent).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&fake_agent, permissions).unwrap();

    let mut child = Command::new(crabdb_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("acp")
        .arg("relay")
        .arg("--lane")
        .arg("acp-test")
        .arg("--materialize")
        .arg("--provider")
        .arg("fake")
        .arg("--")
        .arg(&fake_agent)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stdout = std::io::BufReader::new(stdout);

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":0,"method":"initialize","params":{{"protocolVersion":1}}}}"#
    )
    .unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let init_response: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(init_response["result"]["_meta"]["crabdb"]["relay"], true);

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"session/new","params":{{"cwd":"{}","mcpServers":[]}}}}"#,
        temp.path().display()
    )
    .unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    let session_response: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(session_response["result"]["sessionId"], "sess_fake");

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{{"sessionId":"sess_fake","prompt":[{{"type":"text","text":"change README"}}]}}}}"#
    )
    .unwrap();
    let mut update_kinds = Vec::new();
    let prompt_response = loop {
        line.clear();
        stdout.read_line(&mut line).unwrap();
        let message: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        if message.get("id").and_then(|id| id.as_i64()) == Some(2) {
            break message;
        }
        update_kinds.push(
            message["params"]["update"]["sessionUpdate"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    };
    assert!(update_kinds
        .iter()
        .any(|kind| kind == "available_commands_update"));
    assert!(update_kinds.iter().any(|kind| kind == "tool_call"));
    assert!(update_kinds.iter().any(|kind| kind == "tool_call_update"));
    assert!(update_kinds
        .iter()
        .any(|kind| kind == "agent_message_chunk"));
    assert_eq!(prompt_response["result"]["stopReason"], "end_turn");
    drop(stdin);

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "relay failed\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let forwarded_session_new: serde_json::Value =
        serde_json::from_slice(&fs::read(&session_request_log).unwrap()).unwrap();
    assert_eq!(
        forwarded_session_new["params"]["cwd"].as_str().unwrap(),
        lane_workdir.to_str().unwrap()
    );
    let mcp_servers = forwarded_session_new["params"]["mcpServers"]
        .as_array()
        .unwrap();
    assert!(mcp_servers.iter().any(|server| server["name"] == "crabdb"));

    let db = CrabDb::open(temp.path()).unwrap();
    let mapping = db.try_lane_acp_session("sess_fake").unwrap().unwrap();
    let lane = db.lane_details("acp-test").unwrap();
    assert_eq!(mapping.lane_id, lane.record.lane_id);
    assert_eq!(
        lane.branch.workdir.as_deref(),
        Some(lane_workdir.to_str().unwrap())
    );

    let session = db.show_lane_session(&mapping.crabdb_session_id).unwrap();
    assert_eq!(session.turns.len(), 1);
    assert!(session
        .messages
        .iter()
        .any(|message| message.role == "user" && message.body.contains("change README")));
    assert!(session
        .messages
        .iter()
        .any(|message| message.role == "assistant" && message.body.contains("done")));
    assert!(session
        .events
        .iter()
        .any(|event| event.event_type == "acp_session_started"));
    for expected in [
        "acp_available_commands_update",
        "tool_call",
        "tool_call_update",
        "span_started",
        "span_ended",
    ] {
        assert!(
            session
                .events
                .iter()
                .any(|event| event.event_type == expected),
            "missing event {expected}"
        );
    }

    let turn = db.show_lane_turn(&session.turns[0].turn_id).unwrap();
    assert!(turn
        .operations
        .iter()
        .any(|operation| operation.kind == OperationKind::LaneRecord));
    let status = db.lane_status("acp-test").unwrap();
    assert!(status
        .changed_paths
        .iter()
        .any(|path| path.path == "README.md"));

    let acp_sessions = run_crabdb_json(temp.path(), &["acp", "sessions", "--lane", "acp-test"]);
    assert_eq!(acp_sessions["sessions"][0]["acp_session_id"], "sess_fake");

    let transcript = run_crabdb_json(temp.path(), &["transcript", "acp-test"]);
    assert_eq!(transcript["resolved_kind"], "lane");
    assert_eq!(transcript["acp_session"]["acp_session_id"], "sess_fake");
    assert!(transcript["turns"][0]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|message| message["role"] == "assistant"
            && message["body"].as_str().unwrap().contains("done")));
    assert!(transcript["turns"][0]["tool_summaries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|summary| summary.as_str().unwrap().contains("write README")));

    let turn_alias = run_crabdb_json(
        temp.path(),
        &["turn", "show", session.turns[0].turn_id.as_str()],
    );
    assert_eq!(turn_alias["turn"]["turn_id"], session.turns[0].turn_id);

    let workspace_status = run_crabdb_json(temp.path(), &["status"]);
    assert!(workspace_status["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|suggestion| suggestion["command"]
            .as_str()
            .unwrap()
            .contains("crabdb transcript acp-test")));
}

#[test]
fn acp_relay_closes_failed_turn_on_upstream_crash() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let output =
        run_builtin_acp_relay_scenario(temp.path(), "acp-crash", &["--crash-after-update"], false);
    assert!(
        !output.status.success(),
        "relay should fail on upstream crash\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let db = CrabDb::open(temp.path()).unwrap();
    let mapping = db
        .try_lane_acp_session("sess_fake_doctor")
        .unwrap()
        .unwrap();
    let session = db.show_lane_session(&mapping.crabdb_session_id).unwrap();
    assert_eq!(session.turns[0].status, "failed");
    assert!(session
        .events
        .iter()
        .any(|event| event.event_type == "acp_relay_turn_closed"));
    assert!(
        session
            .messages
            .iter()
            .any(|message| message.role == "assistant"
                && message.body.contains("diagnostic complete"))
    );
}

#[test]
fn acp_relay_closes_failed_turn_on_malformed_upstream_json() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let output = run_builtin_acp_relay_scenario(
        temp.path(),
        "acp-malformed",
        &["--malformed-after-update"],
        false,
    );
    assert!(
        !output.status.success(),
        "relay should fail on malformed upstream JSON\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let db = CrabDb::open(temp.path()).unwrap();
    let mapping = db
        .try_lane_acp_session("sess_fake_doctor")
        .unwrap()
        .unwrap();
    let session = db.show_lane_session(&mapping.crabdb_session_id).unwrap();
    assert_eq!(session.turns[0].status, "failed");
    assert!(session
        .events
        .iter()
        .any(|event| event.event_type == "acp_relay_turn_closed"
            && event
                .payload
                .as_ref()
                .is_some_and(|payload| payload.to_string().contains("malformed JSON"))));
}

#[test]
fn acp_relay_truncates_oversized_assistant_capture() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let output = run_builtin_acp_relay_scenario(
        temp.path(),
        "acp-huge",
        &["--huge-message-bytes", "300000"],
        true,
    );
    assert!(
        output.status.success(),
        "relay should succeed with truncated capture\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let db = CrabDb::open(temp.path()).unwrap();
    let mapping = db
        .try_lane_acp_session("sess_fake_doctor")
        .unwrap()
        .unwrap();
    let session = db.show_lane_session(&mapping.crabdb_session_id).unwrap();
    let event_types = session
        .events
        .iter()
        .map(|event| event.event_type.as_str())
        .collect::<Vec<_>>();
    let assistant_len = session
        .messages
        .iter()
        .find(|message| message.role == "assistant")
        .map(|message| message.body.len())
        .unwrap_or(0);
    assert!(
        session
            .events
            .iter()
            .any(|event| event.event_type == "acp_capture_truncated"),
        "missing truncation event; assistant_len={assistant_len}; events={event_types:?}"
    );
    let assistant = session
        .messages
        .iter()
        .find(|message| message.role == "assistant")
        .unwrap();
    assert!(assistant.body.len() < 300000);
}

#[test]
fn acp_relay_mirrors_permission_requests_to_approvals() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let output = run_builtin_acp_relay_scenario(
        temp.path(),
        "acp-permission",
        &["--request-permission"],
        true,
    );
    assert!(
        output.status.success(),
        "relay should succeed with permission request\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let db = CrabDb::open(temp.path()).unwrap();
    let approvals = db
        .list_lane_approvals(Some("acp-permission"), Some("pending"))
        .unwrap();
    assert!(approvals
        .iter()
        .any(|approval| approval.action == "acp_permission"
            && approval.summary == "approve diagnostic write"));
}

#[test]
fn acp_relay_records_cancel_requests() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut child = Command::new(crabdb_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("acp")
        .arg("relay")
        .arg("--lane")
        .arg("acp-cancel")
        .arg("--materialize")
        .arg("--provider")
        .arg("fake")
        .arg("--")
        .arg(crabdb_bin())
        .arg("acp")
        .arg("test-agent")
        .arg("--sleep-before-result-ms")
        .arg("200")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stdout = std::io::BufReader::new(stdout);
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":0,"method":"initialize","params":{{"protocolVersion":1}}}}"#
    )
    .unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"session/new","params":{{"cwd":"{}","mcpServers":[]}}}}"#,
        temp.path().display()
    )
    .unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{{"sessionId":"sess_fake_doctor","prompt":[{{"type":"text","text":"run diagnostic"}}]}}}}"#
    )
    .unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    assert!(line.contains("session/update"));
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":3,"method":"session/cancel","params":{{"sessionId":"sess_fake_doctor"}}}}"#
    )
    .unwrap();
    loop {
        line.clear();
        let bytes = stdout.read_line(&mut line).unwrap();
        assert!(bytes > 0, "relay stdout closed before prompt response");
        if line.contains(r#""id":2"#) {
            break;
        }
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "relay should succeed after cancel request\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db = CrabDb::open(temp.path()).unwrap();
    let mapping = db
        .try_lane_acp_session("sess_fake_doctor")
        .unwrap()
        .unwrap();
    let session = db.show_lane_session(&mapping.crabdb_session_id).unwrap();
    assert!(session
        .events
        .iter()
        .any(|event| event.event_type == "acp_prompt_cancel_requested"));
}

fn run_builtin_acp_relay_scenario(
    workspace: &Path,
    lane: &str,
    test_agent_args: &[&str],
    expect_prompt_result: bool,
) -> std::process::Output {
    run_builtin_acp_relay_scenario_with_session_id(
        workspace,
        lane,
        "sess_fake_doctor",
        test_agent_args,
        expect_prompt_result,
    )
}

fn run_builtin_acp_relay_scenario_with_session_id(
    workspace: &Path,
    lane: &str,
    session_id: &str,
    test_agent_args: &[&str],
    expect_prompt_result: bool,
) -> std::process::Output {
    let mut child = Command::new(crabdb_bin())
        .arg("--workspace")
        .arg(workspace)
        .arg("acp")
        .arg("relay")
        .arg("--lane")
        .arg(lane)
        .arg("--materialize")
        .arg("--provider")
        .arg("fake")
        .arg("--")
        .arg(crabdb_bin())
        .arg("acp")
        .arg("test-agent")
        .arg("--session-id")
        .arg(session_id)
        .args(test_agent_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stdout = std::io::BufReader::new(stdout);
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":0,"method":"initialize","params":{{"protocolVersion":1}}}}"#
    )
    .unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert!(line.contains(r#""id":0"#));
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"session/new","params":{{"cwd":"{}","mcpServers":[]}}}}"#,
        workspace.display()
    )
    .unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    assert!(line.contains(session_id));
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{{"sessionId":"{}","prompt":[{{"type":"text","text":"run diagnostic"}}]}}}}"#,
        session_id
    )
    .unwrap();
    if expect_prompt_result {
        loop {
            line.clear();
            let bytes = stdout.read_line(&mut line).unwrap();
            assert!(bytes > 0, "relay stdout closed before prompt response");
            if line.contains(r#""id":2"#) {
                break;
            }
        }
    } else {
        while stdout.read_line(&mut line).unwrap() > 0 {
            line.clear();
        }
    }
    drop(stdin);
    child.wait_with_output().unwrap()
}

#[test]
fn acp_relay_runs_two_concurrent_relays_on_distinct_lanes() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let workspace_a = temp.path().to_path_buf();
    let workspace_b = temp.path().to_path_buf();
    let relay_a = thread::spawn(move || {
        run_builtin_acp_relay_scenario_with_session_id(
            &workspace_a,
            "acp-parallel-a",
            "sess_parallel_a",
            &["--sleep-before-result-ms", "100"],
            true,
        )
    });
    let relay_b = thread::spawn(move || {
        run_builtin_acp_relay_scenario_with_session_id(
            &workspace_b,
            "acp-parallel-b",
            "sess_parallel_b",
            &["--sleep-before-result-ms", "100"],
            true,
        )
    });

    let output_a = relay_a.join().unwrap();
    let output_b = relay_b.join().unwrap();
    assert!(
        output_a.status.success(),
        "relay a failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output_a.stdout),
        String::from_utf8_lossy(&output_a.stderr)
    );
    assert!(
        output_b.status.success(),
        "relay b failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output_b.stdout),
        String::from_utf8_lossy(&output_b.stderr)
    );

    let db = CrabDb::open(temp.path()).unwrap();
    for (lane, session_id) in [
        ("acp-parallel-a", "sess_parallel_a"),
        ("acp-parallel-b", "sess_parallel_b"),
    ] {
        let mapping = db.try_lane_acp_session(session_id).unwrap().unwrap();
        let lane_details = db.lane_details(lane).unwrap();
        assert_eq!(mapping.lane_id, lane_details.record.lane_id);
        let session = db.show_lane_session(&mapping.crabdb_session_id).unwrap();
        assert_eq!(session.turns.len(), 1);
        assert_eq!(session.turns[0].status, "completed");
        assert!(session.turns[0].after_change.is_some());
        assert!(session
            .messages
            .iter()
            .any(|message| message.role == "assistant"
                && message.body.contains("diagnostic complete")));
    }
}

#[cfg(unix)]
#[test]
fn acp_relay_waits_for_transient_workspace_writer_lock() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let fake_agent = temp.path().join("fake-acp-lock-agent.sh");
    fs::write(
        &fake_agent,
        r#"#!/bin/sh
set -eu
IFS= read -r init
printf '%s\n' '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
IFS= read -r session_new
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"sess_wait"}}'
IFS= read -r prompt
printf '%s\n' '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"sess_wait","update":{"sessionUpdate":"agent_message_chunk","messageId":"msg_1","content":{"type":"text","text":"captured"}}}}'
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"stopReason":"end_turn"}}'
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&fake_agent).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&fake_agent, permissions).unwrap();

    let mut child = Command::new(crabdb_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("acp")
        .arg("relay")
        .arg("--lane")
        .arg("acp-lock-wait")
        .arg("--no-materialize")
        .arg("--provider")
        .arg("fake")
        .arg("--")
        .arg(&fake_agent)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stdout = std::io::BufReader::new(stdout);

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":0,"method":"initialize","params":{{"protocolVersion":1}}}}"#
    )
    .unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let init_response: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(init_response["result"]["_meta"]["crabdb"]["relay"], true);

    let lock_path = temp.path().join(".crabdb/lock");
    fs::write(
        &lock_path,
        format!("pid={} created_at=0", std::process::id()),
    )
    .unwrap();
    let lock_remover = {
        let lock_path = lock_path.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            fs::remove_file(lock_path).unwrap();
        })
    };

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"session/new","params":{{"cwd":"{}","mcpServers":[]}}}}"#,
        temp.path().display()
    )
    .unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    let session_response: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(session_response["result"]["sessionId"], "sess_wait");
    lock_remover.join().unwrap();

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{{"sessionId":"sess_wait","prompt":[{{"type":"text","text":"capture after wait"}}]}}}}"#
    )
    .unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    let update: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(
        update["params"]["update"]["sessionUpdate"],
        "agent_message_chunk"
    );
    line.clear();
    stdout.read_line(&mut line).unwrap();
    let prompt_response: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(prompt_response["result"]["stopReason"], "end_turn");
    drop(stdin);

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "relay failed\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("capture warning"),
        "unexpected capture warning\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db = CrabDb::open(temp.path()).unwrap();
    let mapping = db.try_lane_acp_session("sess_wait").unwrap().unwrap();
    let session = db.show_lane_session(&mapping.crabdb_session_id).unwrap();
    assert_eq!(session.turns.len(), 1);
    assert!(session
        .messages
        .iter()
        .any(|message| message.role == "assistant" && message.body.contains("captured")));
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
    db.spawn_lane("backup-bot", Some("main"), true, None, None)
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

    let lane = restored_db.lane_details("backup-bot").unwrap();
    let workdir = lane.branch.workdir.as_ref().unwrap();
    let restored_db_dir = restored.path().canonicalize().unwrap().join(".crabdb");
    assert!(workdir.starts_with(&restored_db_dir.to_string_lossy().to_string()));
    assert!(PathBuf::from(workdir).is_dir());
    let status = restored_db.lane_status("backup-bot").unwrap();
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
fn record_paths_records_selected_directory_deletions() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
    fs::write(temp.path().join("other.txt"), "one\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    fs::remove_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("other.txt"), "two\n").unwrap();

    let recorded = run_crabdb_json(
        temp.path(),
        &["record", "--paths", "src", "-m", "record deleted src"],
    );
    assert!(recorded["operation"].as_str().is_some());
    assert_eq!(recorded["changed_paths"].as_array().unwrap().len(), 1);
    assert_eq!(recorded["changed_paths"][0]["path"], "src/lib.rs");
    assert_eq!(recorded["changed_paths"][0]["kind"], "Deleted");

    let db = CrabDb::open(temp.path()).unwrap();
    let status = db.status(Some("main")).unwrap();
    assert_eq!(status.changed_paths.len(), 1);
    assert_eq!(status.changed_paths[0].path, "other.txt");
}

#[test]
fn record_paths_records_existing_directory_selection() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
    fs::write(temp.path().join("other.txt"), "one\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    fs::write(temp.path().join("src/lib.rs"), "pub fn lib() -> u8 { 1 }\n").unwrap();
    fs::write(temp.path().join("other.txt"), "two\n").unwrap();

    let recorded = run_crabdb_json(
        temp.path(),
        &["record", "--paths", "src", "-m", "record src only"],
    );
    assert!(recorded["operation"].as_str().is_some());
    assert_eq!(recorded["changed_paths"].as_array().unwrap().len(), 1);
    assert_eq!(recorded["changed_paths"][0]["path"], "src/lib.rs");
    assert_eq!(recorded["changed_paths"][0]["kind"], "Modified");

    let db = CrabDb::open(temp.path()).unwrap();
    let status = db.status(Some("main")).unwrap();
    assert_eq!(status.changed_paths.len(), 1);
    assert_eq!(status.changed_paths[0].path, "other.txt");
}

#[test]
fn record_paths_rejects_empty_selected_directory() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("empty")).unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let err = db
        .record_with_options(
            Some("main"),
            Some("record empty dir".to_string()),
            Actor::human(),
            crabdb::RecordOptions {
                paths: vec!["empty".to_string()],
                ..crabdb::RecordOptions::default()
            },
        )
        .unwrap_err();
    assert!(matches!(err, Error::IgnoredPath(path) if path == "empty"));
}

#[test]
fn git_tracked_dirty_paths_record_modified_and_deleted_files() {
    if !git_available() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    run_git(temp.path(), &["init"]);
    run_git(temp.path(), &["config", "user.email", "crabdb@example.com"]);
    run_git(temp.path(), &["config", "user.name", "CrabDB"]);
    fs::write(temp.path().join("a.txt"), "a1\n").unwrap();
    fs::write(temp.path().join("b.txt"), "b1\n").unwrap();
    run_git(temp.path(), &["add", "."]);
    run_git(temp.path(), &["commit", "-m", "initial"]);
    CrabDb::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();

    fs::write(temp.path().join("a.txt"), "a1\na2\n").unwrap();
    fs::remove_file(temp.path().join("b.txt")).unwrap();
    fs::write(temp.path().join("c.txt"), "c1\n").unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let status = db.status(Some("main")).unwrap();
    let status_paths = status
        .changed_paths
        .iter()
        .map(|path| (path.path.as_str(), path.kind.clone()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        status_paths.get("a.txt"),
        Some(&crabdb::FileChangeKind::Modified)
    );
    assert_eq!(
        status_paths.get("b.txt"),
        Some(&crabdb::FileChangeKind::Deleted)
    );
    assert_eq!(
        status_paths.get("c.txt"),
        Some(&crabdb::FileChangeKind::Added)
    );

    let record = db
        .record(
            Some("main"),
            Some("record tracked dirty paths".to_string()),
            Actor::human(),
            false,
        )
        .unwrap();
    assert!(record.operation.is_some());
    assert_eq!(record.changed_paths.len(), 3);
    assert!(record
        .changed_paths
        .iter()
        .any(|path| path.path == "a.txt" && path.kind == crabdb::FileChangeKind::Modified));
    assert!(record
        .changed_paths
        .iter()
        .any(|path| path.path == "b.txt" && path.kind == crabdb::FileChangeKind::Deleted));
    assert!(record
        .changed_paths
        .iter()
        .any(|path| path.path == "c.txt" && path.kind == crabdb::FileChangeKind::Added));

    let clean = db.status(Some("main")).unwrap();
    assert!(clean.changed_paths.is_empty());
    let diff = db.diff_dirty(false, false).unwrap();
    assert!(diff.files.is_empty());
    let noop = db
        .record(
            Some("main"),
            Some("ignore stale git dirty paths".to_string()),
            Actor::human(),
            false,
        )
        .unwrap();
    assert!(noop.operation.is_none());
}

#[test]
fn record_kind_session_and_allow_ignored_path_are_audited() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("auditor", Some("main"), false, None, None)
        .unwrap();
    db.start_lane_session(
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
    db.spawn_lane("watch-bot", Some("main"), false, None, None)
        .unwrap();
    db.start_lane_session(
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
fn lane_patch_respects_ignore_policy_and_explicit_opt_in() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.ignore_add("ignored-lane-output.txt").unwrap();
    db.spawn_lane("privacy-bot", Some("main"), false, None, None)
        .unwrap();

    let blocked: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "try ignored write",
        "edits": [
            {
                "op": "write",
                "path": "ignored-lane-output.txt",
                "content": "secret-ish\n"
            }
        ]
    }))
    .unwrap();
    let err = apply_lane_patch_at_head(&mut db, "privacy-bot", blocked).unwrap_err();
    assert!(matches!(err, Error::IgnoredPath(path) if path == "ignored-lane-output.txt"));

    let allowed: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "explicit ignored fixture",
        "allow_ignored": true,
        "edits": [
            {
                "op": "write",
                "path": "ignored-lane-output.txt",
                "content": "intentional fixture\n"
            }
        ]
    }))
    .unwrap();
    let report = apply_lane_patch_at_head(&mut db, "privacy-bot", allowed).unwrap();
    assert!(report
        .changed_paths
        .iter()
        .any(|path| path.path == "ignored-lane-output.txt"));

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
    let err = apply_lane_patch_at_head(&mut db, "privacy-bot", internal).unwrap_err();
    assert!(matches!(err, Error::IgnoredPath(path) if path == ".crabdb/leak.txt"));
}

#[test]
fn lane_payload_secret_scan_rejects_patch_content_and_redacts_stored_payloads() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("secret-bot", Some("main"), false, None, None)
        .unwrap();

    let secret_content: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "normal patch",
        "edits": [
            {
                "op": "write",
                "path": "README.md",
                "content": "OPENAI_API_KEY=sk-live-secret\n"
            }
        ]
    }))
    .unwrap();
    let err = apply_lane_patch_at_head(&mut db, "secret-bot", secret_content).unwrap_err();
    assert!(
        matches!(err, Error::PatchRejected(message) if message.contains("secret scan rejected patch content"))
    );

    let secret_message: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "password=hunter2",
        "edits": [
            {
                "op": "write",
                "path": "README.md",
                "content": "hello\n"
            }
        ]
    }))
    .unwrap();
    let err = apply_lane_patch_at_head(&mut db, "secret-bot", secret_message).unwrap_err();
    assert!(
        matches!(err, Error::PatchRejected(message) if message.contains("secret scan rejected patch message"))
    );

    let benign_keyword: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "document token expiration behavior",
        "edits": [
            {
                "op": "write",
                "path": "README.md",
                "content": "hello\ntoken expiration logic\n"
            }
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "secret-bot", benign_keyword).unwrap();

    let message = db
        .add_lane_message("secret-bot", "assistant", "password=hunter2", None)
        .unwrap();
    let conn = Connection::open(temp.path().join(".crabdb/index/crabdb.sqlite")).unwrap();
    let stored_message: String = conn
        .query_row(
            "SELECT body FROM messages WHERE message_id = ?1",
            [message.message_id.0.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(stored_message.contains("[REDACTED]"));
    assert!(!stored_message.contains("hunter2"));

    let session = db
        .start_lane_session("secret-bot", Some("Secret scan".to_string()), None)
        .unwrap();
    db.add_lane_session_event(
        "secret-bot",
        &session.session.session_id,
        "tool_output",
        Some(serde_json::json!({
            "api_key": "event-secret",
            "safe": "token expiration logic"
        })),
    )
    .unwrap();
    let event_payload: String = conn
        .query_row(
            "SELECT payload_json FROM lane_events WHERE event_type = 'tool_output' ORDER BY created_at DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(event_payload.contains("[REDACTED]"));
    assert!(!event_payload.contains("event-secret"));
    assert!(event_payload.contains("token expiration logic"));

    let turn = db
        .begin_lane_session_turn("secret-bot", &session.session.session_id, None)
        .unwrap();
    let span = db
        .start_lane_trace_span(
            &turn.turn.turn_id,
            "tool",
            "secret trace",
            None,
            None,
            Some(serde_json::json!({
                "authorization": "Bearer trace-secret",
                "safe": "token expiration logic"
            })),
        )
        .unwrap();
    db.end_lane_trace_span(
        &span.span.span_id,
        "ok",
        Some(serde_json::json!({
            "client_secret": "trace-result-secret"
        })),
    )
    .unwrap();
    let trace_payloads: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT payload_json FROM lane_events WHERE event_type IN ('span_started', 'span_ended')")
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
    };
    let serialized = trace_payloads.join("\n");
    assert!(serialized.contains("[REDACTED]"));
    assert!(!serialized.contains("trace-secret"));
    assert!(!serialized.contains("trace-result-secret"));
}

#[test]
fn lane_patch_requires_base_change_unless_allow_stale() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("fresh-bot", Some("main"), false, None, None)
        .unwrap();

    let missing_base: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write", "path": "README.md", "content": "missing base\n"}
        ]
    }))
    .unwrap();
    let err = db.apply_lane_patch("fresh-bot", missing_base).unwrap_err();
    assert!(matches!(err, Error::PatchRejected(_)));
    assert!(err.to_string().contains("base_change"));
    assert!(err.to_string().contains("allow_stale=true"));

    let stale_base: PatchDocument = serde_json::from_value(serde_json::json!({
        "base_change": "ch_stale",
        "edits": [
            {"op": "write", "path": "README.md", "content": "stale base\n"}
        ]
    }))
    .unwrap();
    let err = db.apply_lane_patch("fresh-bot", stale_base).unwrap_err();
    assert!(matches!(err, Error::PatchRejected(_)));
    assert!(err.to_string().contains("does not match lane head"));

    let allowed_stale: PatchDocument = serde_json::from_value(serde_json::json!({
        "allow_stale": true,
        "edits": [
            {"op": "write", "path": "README.md", "content": "allowed stale\n"}
        ]
    }))
    .unwrap();
    let report = db.apply_lane_patch("fresh-bot", allowed_stale).unwrap();
    assert_eq!(report.changed_paths[0].path, "README.md");
    let events = db
        .list_lane_events(Some("fresh-bot"), None, None, Some("patch_applied"), 10)
        .unwrap();
    assert!(events.iter().any(|event| {
        event.payload.as_ref().unwrap()["allow_stale"] == serde_json::Value::Bool(true)
    }));
}

#[test]
fn lane_patch_rejects_hardened_paths_and_quota_violations() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("policy-bot", Some("main"), false, None, None)
        .unwrap();

    for bad_path in ["CON", "notes:ads.txt", "src\u{2215}lib.rs"] {
        let patch: PatchDocument = serde_json::from_value(serde_json::json!({
            "edits": [
                {"op": "write", "path": bad_path, "content": "blocked\n"}
            ]
        }))
        .unwrap();
        let err = apply_lane_patch_at_head(&mut db, "policy-bot", patch).unwrap_err();
        assert!(
            matches!(err, Error::InvalidPath { .. }),
            "expected invalid path for {bad_path}, got {err:?}"
        );
    }

    db.config_set("lane.max_patch_file_bytes", "4").unwrap();
    let oversized_file: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write", "path": "README.md", "content": "12345"}
        ]
    }))
    .unwrap();
    let err = apply_lane_patch_at_head(&mut db, "policy-bot", oversized_file).unwrap_err();
    assert!(
        matches!(err, Error::PatchRejected(message) if message.contains("max_patch_file_bytes"))
    );

    db.config_set("lane.max_patch_file_bytes", "0").unwrap();
    db.config_set("lane.max_changed_paths", "1").unwrap();
    let too_many_paths: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write", "path": "README.md", "content": "one\n"},
            {"op": "write", "path": "notes.md", "content": "two\n"}
        ]
    }))
    .unwrap();
    let err = apply_lane_patch_at_head(&mut db, "policy-bot", too_many_paths).unwrap_err();
    assert!(matches!(err, Error::PatchRejected(message) if message.contains("max_changed_paths")));

    db.config_set("lane.max_changed_paths", "0").unwrap();
    db.config_set("lane.max_patch_bytes", "64").unwrap();
    let oversized_payload: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "this patch metadata intentionally makes the serialized payload too large",
        "edits": [
            {"op": "write", "path": "README.md", "content": "ok\n"}
        ]
    }))
    .unwrap();
    let err = apply_lane_patch_at_head(&mut db, "policy-bot", oversized_payload).unwrap_err();
    assert!(matches!(err, Error::PatchRejected(message) if message.contains("max_patch_bytes")));
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
            serde_json::json!({ "pattern": "*.lanelocal" }),
        ),
    );
    assert_eq!(add_response.status, 200);
    let added: serde_json::Value = add_response.body_json().unwrap();
    assert_eq!(added["added"], true);

    fs::write(temp.path().join("scratch.lanelocal"), "secret\n").unwrap();
    let check_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/ignore/check",
            serde_json::json!({ "path": "scratch.lanelocal" }),
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
                "paths": ["scratch.lanelocal"]
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
            serde_json::json!({ "pattern": "*.lanelocal" }),
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
fn local_api_and_mcp_manage_lane_sessions() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("api-session-bot", Some("main"), false, None, None)
        .unwrap();

    let started = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/sessions",
            serde_json::json!({
                "lane": "api-session-bot",
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
            "/v1/sessions/current?lane=api-session-bot",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(current.status, 200);
    let current: serde_json::Value = current.body_json().unwrap();
    assert_eq!(current[0]["lane_name"], "api-session-bot");
    assert_eq!(current[0]["session"]["session_id"], "session-api");

    let listed = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/sessions?lane=api-session-bot",
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

    db.add_lane_message(
        "api-session-bot",
        "user",
        "Please improve the docs with a bounded context packet.",
        Some("session-api".to_string()),
    )
    .unwrap();
    db.add_lane_message(
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
            "/v1/sessions/current?lane=api-session-bot",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(current.status, 200);
    let current: serde_json::Value = current.body_json().unwrap();
    assert!(current[0]["session"].is_null());

    db.spawn_lane("mcp-session-bot", Some("main"), false, None, None)
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
                    "lane": "mcp-session-bot",
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
                    "lane": "mcp-session-bot"
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
    db.spawn_lane("approval-bot", Some("main"), false, None, None)
        .unwrap();
    let turn = db
        .begin_lane_turn(
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
                "lane": "approval-bot",
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
    assert_eq!(guardrail["lane"]["record"]["name"], "approval-bot");
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
                "lane": "approval-bot",
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
            &format!("/v1/lane/runs/{run_id}/resume"),
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
            "/v1/lane/runs?lane=approval-bot&status=paused",
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
            "lane",
            "run",
            "list",
            "--lane",
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
            "/v1/approvals?lane=approval-bot&status=pending",
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
            "--lane",
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

    let readiness = db.lane_readiness("approval-bot").unwrap();
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
                    "lane": "approval-bot",
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
                    "lane": "approval-bot",
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
            &format!("/v1/lane/runs/{run_id}/resume"),
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
            &format!("/v1/lane/runs/{run_id}"),
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
                "lane": "approval-bot",
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
            "--lane",
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
        .request_lane_approval(
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
        .decide_lane_approval(
            &rejected.approval.approval_id,
            "rejected",
            Some("human-reviewer".to_string()),
            Some("Preview deploy is not allowed".to_string()),
        )
        .unwrap();
    assert_eq!(rejected_decision.run_states[0].run_id, rejected_run_id);
    assert_eq!(rejected_decision.run_states[0].status, "blocked");
    let rejected_resume = db
        .resume_lane_run(&rejected_run_id, Some("human-reviewer".to_string()), None)
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

    let details = db.show_lane_turn(&turn_id).unwrap();
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
fn lane_trace_metadata_redacts_common_secrets() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let turn_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lane/turns",
            serde_json::json!({
                "lane": "redaction-bot",
                "branch": "main",
                "session_title": "Redaction smoke"
            }),
        ),
    );
    assert_eq!(turn_response.status, 201);
    let turn: LaneTurnStartReport = turn_response.body_json().unwrap();

    let message_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/lane/turns/{}/messages", turn.turn.turn_id),
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
            &format!("/v1/lane/turns/{}/events", turn.turn.turn_id),
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
                "lane": "redaction-bot",
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

    let turn_details = db.show_lane_turn(&turn.turn.turn_id).unwrap();
    let approval = db.show_lane_approval(approval_id).unwrap();
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
fn lane_trace_events_are_queryable_across_cli_api_and_mcp() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let turn = db
        .begin_lane_turn(
            "trace-bot",
            Some("main"),
            Some("Trace inspection".to_string()),
            None,
        )
        .unwrap();
    let turn_id = turn.turn.turn_id.clone();
    let session_id = turn.session.session_id.clone();
    db.add_lane_turn_event(
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
    db.add_lane_turn_event(
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
        .list_lane_events(Some("trace-bot"), None, None, Some("tool_call"), 10)
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
            "/v1/lane/events?lane=trace-bot&type=guardrail&limit=10",
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
            "lane",
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
fn lane_trace_spans_are_parentable_redacted_and_available_across_surfaces() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let turn = db
        .begin_lane_turn(
            "span-bot",
            Some("main"),
            Some("Trace span inspection".to_string()),
            None,
        )
        .unwrap();
    let turn_id = turn.turn.turn_id.clone();

    let root = db
        .start_lane_trace_span(
            &turn_id,
            "lane",
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
            &format!("/v1/lane/turns/{turn_id}/spans"),
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
            &format!("/v1/lane/spans/{child_span_id}/end"),
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

    let conn = Connection::open(temp.path().join(".crabdb/index/crabdb.sqlite")).unwrap();
    let child_span_event_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM lane_trace_span_events WHERE span_id = ?1",
            [&child_span_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(child_span_event_count, 2);
    let root_span_event_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM lane_trace_span_events WHERE span_id = ?1",
            [&root_span_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(root_span_event_count, 1);

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
            "lane",
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
            "lane",
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
        &["lane", "trace", "list", "--turn", &turn_id, "--limit", "10"],
    );
    assert!(cli_list
        .as_array()
        .unwrap()
        .iter()
        .any(|span| span["span_id"] == child_span_id));
    let cli_summary = run_crabdb_json(
        temp.path(),
        &[
            "lane",
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
            &format!("/v1/lane/spans?trace={trace_id}&limit=10"),
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
            &format!("/v1/lane/spans/summary?trace={trace_id}&slowest=3"),
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
        .list_lane_trace_spans(None, None, Some(&turn_id), Some(&trace_id), 10)
        .unwrap();
    assert!(spans.iter().any(|span| span.span_id == root_span_id));
    assert!(spans
        .iter()
        .any(|span| span.parent_span_id.as_deref() == Some(root_span_id.as_str())));
    let summary = db
        .summarize_lane_trace_spans(None, None, Some(&turn_id), Some(&trace_id), 5)
        .unwrap();
    assert_eq!(summary.span_count, 4);
    assert_eq!(summary.open_span_count, 1);
    assert_eq!(summary.ended_span_count, 3);
    assert_eq!(summary.failed_span_count, 1);
    assert!(summary
        .span_type_counts
        .iter()
        .any(|count| count.name == "evaluation" && count.count == 1));

    conn.execute("DELETE FROM lane_trace_span_events", [])
        .unwrap();
    let fallback_spans = db
        .list_lane_trace_spans(None, None, Some(&turn_id), Some(&trace_id), 10)
        .unwrap();
    assert_eq!(fallback_spans.len(), 4);
    assert!(fallback_spans
        .iter()
        .any(|span| span.span_id == child_span_id));
    let fallback_summary = db
        .summarize_lane_trace_spans(None, None, Some(&turn_id), Some(&trace_id), 5)
        .unwrap();
    assert_eq!(fallback_summary.span_count, 4);
    assert_eq!(fallback_summary.failed_span_count, 1);

    let rebuild = db.rebuild_indexes().unwrap();
    assert_eq!(rebuild.errors, Vec::<String>::new());
    let restored_span_event_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM lane_trace_span_events WHERE trace_id = ?1",
            [&trace_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(restored_span_event_count, 7);

    let events = db
        .list_lane_events(None, None, Some(&turn_id), None, 50)
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
fn local_lane_http_api_records_turn_messages_and_patches() {
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
            "/v1/lane/turns",
            serde_json::json!({
                "lane": "api-lane",
                "branch": "main",
                "session_title": "API smoke"
            }),
        ),
    );
    assert_eq!(turn_response.status, 201);
    let turn: LaneTurnStartReport = turn_response.body_json().unwrap();
    assert_eq!(turn.session.title.as_deref(), Some("API smoke"));
    assert_eq!(turn.turn.status, "started");

    let message_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/lane/turns/{}/messages", turn.turn.turn_id),
            serde_json::json!({
                "role": "user",
                "content": "Add a small API file."
            }),
        ),
    );
    assert_eq!(message_response.status, 201);
    let message: LaneMessageReport = message_response.body_json().unwrap();
    assert_eq!(
        message.session_id.as_deref(),
        Some(turn.session.session_id.as_str())
    );

    let event_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/lane/turns/{}/events", turn.turn.turn_id),
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
    let event: LaneTurnEventReport = event_response.body_json().unwrap();
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
            &format!("/v1/lane/turns/{}/patches", turn.turn.turn_id),
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
    let patch: LanePatchReport = patch_response.body_json().unwrap();
    assert_eq!(patch.changed_paths.len(), 1);
    assert_eq!(patch.changed_paths[0].path, "src/api.rs");

    let details_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            &format!("/v1/lane/turns/{}", turn.turn.turn_id),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(details_response.status, 200);
    let details: LaneTurnDetails = details_response.body_json().unwrap();
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
            &format!("/v1/lane/turns/{}/end", turn.turn.turn_id),
            serde_json::json!({ "status": "completed" }),
        ),
    );
    assert_eq!(end_response.status, 200);
    let ended: LaneTurnEndReport = end_response.body_json().unwrap();
    assert_eq!(ended.turn.status, "completed");
    assert_eq!(ended.turn.after_change, Some(patch.operation));

    let diff = db.diff_lane("api-lane", false).unwrap();
    assert!(diff.files.iter().any(|file| file.path == "src/api.rs"));

    let session = db.show_lane_session(&turn.session.session_id).unwrap();
    assert_eq!(session.messages.len(), 2);
    assert_eq!(session.operations.len(), 1);
    assert!(session
        .events
        .iter()
        .any(|event| event.event_type == "turn_ended"));
}

#[test]
fn mutation_json_payloads_reject_unknown_fields() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();

    let bad_turn = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lane/turns",
            serde_json::json!({
                "lane": "strict-api",
                "branch": "main",
                "surprise": true
            }),
        ),
    );
    assert_eq!(bad_turn.status, 400);
    let bad_turn_body: serde_json::Value = bad_turn.body_json().unwrap();
    assert!(bad_turn_body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unknown field"));

    let turn_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lane/turns",
            serde_json::json!({
                "lane": "strict-api",
                "branch": "main"
            }),
        ),
    );
    assert_eq!(turn_response.status, 201);
    let turn: LaneTurnStartReport = turn_response.body_json().unwrap();

    let bad_message = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/lane/turns/{}/messages", turn.turn.turn_id),
            serde_json::json!({
                "role": "user",
                "content": "hello",
                "surprise": true
            }),
        ),
    );
    assert_eq!(bad_message.status, 400);
    let bad_message_body: serde_json::Value = bad_message.body_json().unwrap();
    assert!(bad_message_body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unknown field"));

    let bad_patch = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/lane/turns/{}/patches", turn.turn.turn_id),
            serde_json::json!({
                "message": "bad patch",
                "files": [
                    {
                        "type": "add_text",
                        "path": "src/strict.rs",
                        "content": "pub fn strict() -> bool { true }\n",
                        "surprise": true
                    }
                ]
            }),
        ),
    );
    assert_eq!(bad_patch.status, 400);
    let bad_patch_body: serde_json::Value = bad_patch.body_json().unwrap();
    assert!(bad_patch_body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unknown field"));

    let direct_patch_error = serde_json::from_value::<PatchDocument>(serde_json::json!({
        "message": "bad direct patch",
        "edits": [
            {
                "op": "write",
                "path": "src/direct.rs",
                "content": "pub fn direct() -> bool { true }\n",
                "surprise": true
            }
        ]
    }))
    .unwrap_err();
    assert!(direct_patch_error.to_string().contains("unknown field"));

    let mcp_bad_begin = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "crabdb.begin_turn",
                "arguments": {
                    "lane": "strict-mcp",
                    "branch": "main",
                    "surprise": true
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_bad_begin["result"]["isError"], true);
    assert!(mcp_bad_begin["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("unknown field"));

    let mcp_begin = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "crabdb.begin_turn",
                "arguments": {
                    "lane": "strict-mcp",
                    "branch": "main"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_begin["result"]["isError"], false);
    let turn_id = mcp_begin["result"]["structuredContent"]["turn"]["turn_id"]
        .as_str()
        .unwrap()
        .to_string();

    let mcp_bad_patch = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "crabdb.apply_patch",
                "arguments": {
                    "turn_id": turn_id,
                    "message": "bad mcp patch",
                    "files": [
                        {
                            "type": "add_text",
                            "path": "src/mcp_strict.rs",
                            "content": "pub fn mcp_strict() -> bool { true }\n",
                            "surprise": true
                        }
                    ]
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_bad_patch["result"]["isError"], true);
    assert!(mcp_bad_patch["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("unknown field"));
}

#[test]
fn external_http_and_mcp_mutations_emit_audit_events() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    assert!(db.list_external_mutation_audit(10).unwrap().is_empty());

    let http_turn = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lane/turns",
            serde_json::json!({
                "lane": "audit-http",
                "branch": "main"
            }),
        ),
    );
    assert_eq!(http_turn.status, 201);

    let http_bad_turn = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lane/turns",
            serde_json::json!({
                "lane": "audit-http",
                "branch": "main",
                "unexpected": true
            }),
        ),
    );
    assert_eq!(http_bad_turn.status, 400);

    let mcp_read_only = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "crabdb.status",
                "arguments": {}
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_read_only["result"]["isError"], false);

    let mcp_set = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "crabdb.config_set",
                "arguments": {
                    "key": "lane.default_materialize",
                    "value": "false"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_set["result"]["isError"], false);

    let audits = db.list_external_mutation_audit(20).unwrap();
    assert!(audits.iter().any(|audit| {
        audit.surface == "http"
            && audit.command == "POST /v1/lane/turns"
            && audit.status == "ok"
            && audit.status_code == Some(201)
            && audit.target_ref.as_deref() == Some("refs/lanes/audit-http")
    }));
    assert!(audits.iter().any(|audit| {
        audit.surface == "http"
            && audit.command == "POST /v1/lane/turns"
            && audit.status == "error"
            && audit.status_code == Some(400)
            && audit
                .summary
                .as_ref()
                .and_then(|summary| summary["error"].as_str())
                .is_some_and(|message| message.contains("unknown field"))
    }));
    assert!(audits.iter().any(|audit| {
        audit.surface == "mcp" && audit.command == "crabdb.config_set" && audit.status == "ok"
    }));
    assert!(!audits.iter().any(|audit| audit.command == "crabdb.status"));
}

#[test]
fn local_lane_http_api_replays_idempotent_mutation_requests() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let request_body = serde_json::json!({
        "lane": "idempotent-api",
        "branch": "main",
        "session_title": "First request"
    });
    let request = api_request_with_headers(
        "POST",
        "/v1/lane/turns",
        &[("Idempotency-Key", "turn-key-1")],
        request_body.clone(),
    );

    let mut db = CrabDb::open(temp.path()).unwrap();
    let first = crabdb::server::handle_http_request(&mut db, &request);
    assert_eq!(first.status, 201);
    let first_turn: LaneTurnStartReport = first.body_json().unwrap();
    drop(db);

    let mut reopened = CrabDb::open(temp.path()).unwrap();
    let replayed = crabdb::server::handle_http_request(&mut reopened, &request);
    assert_eq!(replayed.status, 201);
    let replayed_turn: LaneTurnStartReport = replayed.body_json().unwrap();
    assert_eq!(replayed_turn.turn.turn_id, first_turn.turn.turn_id);
    assert_eq!(
        replayed_turn.session.session_id,
        first_turn.session.session_id
    );
    drop(reopened);

    let conn = Connection::open(temp.path().join(".crabdb/index/crabdb.sqlite")).unwrap();
    let turns: i64 = conn
        .query_row("SELECT COUNT(*) FROM lane_turns", [], |row| row.get(0))
        .unwrap();
    assert_eq!(turns, 1);
    drop(conn);

    let mut db = CrabDb::open(temp.path()).unwrap();
    let conflicting = crabdb::server::handle_http_request(
        &mut db,
        &api_request_with_headers(
            "POST",
            "/v1/lane/turns",
            &[("Idempotency-Key", "turn-key-1")],
            serde_json::json!({
                "lane": "idempotent-api",
                "branch": "main",
                "session_title": "Different request"
            }),
        ),
    );
    assert_eq!(conflicting.status, 400);
    let error: serde_json::Value = conflicting.body_json().unwrap();
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("already used for a different request"));

    let auth = crabdb::server::ServerAuth::bearer("secret-token").unwrap();
    let auth_request = api_request_with_headers(
        "POST",
        "/v1/lane/turns",
        &[("Idempotency-Key", "auth-key-1")],
        serde_json::json!({
            "lane": "auth-idempotent-api",
            "branch": "main"
        }),
    );
    let missing_auth = crabdb::server::handle_http_request_with_auth(&mut db, &auth_request, &auth);
    assert_eq!(missing_auth.status, 401);
    let authorized_request = api_request_with_headers(
        "POST",
        "/v1/lane/turns",
        &[
            ("Idempotency-Key", "auth-key-1"),
            ("Authorization", "Bearer secret-token"),
        ],
        serde_json::json!({
            "lane": "auth-idempotent-api",
            "branch": "main"
        }),
    );
    let authorized =
        crabdb::server::handle_http_request_with_auth(&mut db, &authorized_request, &auth);
    assert_eq!(authorized.status, 201);
    let unauthorized_replay =
        crabdb::server::handle_http_request_with_auth(&mut db, &auth_request, &auth);
    assert_eq!(unauthorized_replay.status, 401);
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
            "/v1/lane/turns",
            serde_json::json!({
                "lane": "privacy-api",
                "branch": "main",
                "session_title": "Privacy policy"
            }),
        ),
    );
    assert_eq!(turn_response.status, 201);
    let turn: LaneTurnStartReport = turn_response.body_json().unwrap();

    let blocked = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/lane/turns/{}/patches", turn.turn.turn_id),
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
    assert_eq!(
        apply_patch_schema["inputSchema"]["properties"]["allow_stale"]["type"],
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

    let details = db.show_lane_turn(&turn.turn.turn_id).unwrap();
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
fn local_lane_http_api_manages_lane_branch_lifecycle() {
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
            "/v1/lanes",
            serde_json::json!({
                "name": "api-branch-lane",
                "from_ref": "main",
                "materialize": true
            }),
        ),
    );
    assert_eq!(spawn_response.status, 201);
    let spawned: serde_json::Value = spawn_response.body_json().unwrap();
    let lane_id = spawned["lane_id"].as_str().unwrap().to_string();
    let lane_base_change = spawned["base_change"].as_str().unwrap().to_string();
    assert_eq!(spawned["ref_name"], "refs/lanes/api-branch-lane");
    let workdir = spawned["workdir"].as_str().unwrap().to_string();

    let lanes_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/lanes", serde_json::Value::Null),
    );
    assert_eq!(lanes_response.status, 200);
    let lanes: serde_json::Value = lanes_response.body_json().unwrap();
    assert_eq!(lanes.as_array().unwrap().len(), 1);
    assert_eq!(lanes[0]["record"]["name"], "api-branch-lane");
    assert_eq!(lanes[0]["branch"]["ref_name"], "refs/lanes/api-branch-lane");

    let lane_status_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            &format!("/v1/lanes/{lane_id}/status"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(lane_status_response.status, 200);
    let lane_status: serde_json::Value = lane_status_response.body_json().unwrap();
    assert_eq!(lane_status["lane"]["record"]["name"], "api-branch-lane");

    let patch_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/lanes/{lane_id}/patches"),
            serde_json::json!({
                "base_change": lane_base_change,
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
    let patch: LanePatchReport = patch_response.body_json().unwrap();
    assert_eq!(patch.lane_id, lane_id);
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
            &format!("/v1/lanes/{lane_id}/sync-workdir"),
            serde_json::json!({}),
        ),
    );
    assert_eq!(sync_conflict.status, 409);

    let sync_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/lanes/{lane_id}/sync-workdir"),
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
            &format!("/v1/lanes/{lane_id}/tests"),
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
            &format!("/v1/lanes/{lane_id}/diff?patch=true"),
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
            &format!("/v1/lanes/{lane_id}/contribution?limit=5"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(contribution_response.status, 200);
    let contribution: serde_json::Value = contribution_response.body_json().unwrap();
    assert_eq!(
        contribution["status"]["lane"]["record"]["name"],
        "api-branch-lane"
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
        &["lane", "contribution", "api-branch-lane", "--limit", "5"],
    );
    assert_eq!(
        cli_contribution["status"]["lane"]["record"]["lane_id"],
        lane_id
    );

    let review_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            &format!("/v1/lanes/{lane_id}/review?limit=5"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(review_response.status, 200);
    let review: serde_json::Value = review_response.body_json().unwrap();
    assert_eq!(review["lane"]["record"]["name"], "api-branch-lane");
    assert_eq!(review["readiness"]["ready"], true);
    assert!(review["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "src/api.rs"));
    assert_eq!(review["latest_test"]["success"], true);
    assert!(review["recent_gates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|gate| gate["kind"] == "test"));
    assert!(review["recent_operations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|operation| operation["message"] == "add API file"));
    assert!(review["evidence_summary"]["operations"].as_u64().unwrap() >= 1);
    assert!(review["next_steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|step| step.as_str().unwrap().contains("Start a new session")));

    let cli_review = run_crabdb_json(
        temp.path(),
        &["lane", "review", "api-branch-lane", "--limit", "5"],
    );
    assert_eq!(cli_review["lane"]["record"]["lane_id"], lane_id);
    assert_eq!(cli_review["readiness"]["ready"], true);

    let cli_review_text = Command::new(crabdb_bin())
        .arg("--workspace")
        .arg(temp.path())
        .args(["lane", "review", "api-branch-lane", "--limit", "5"])
        .output()
        .unwrap();
    assert!(cli_review_text.status.success());
    let cli_review_stdout = String::from_utf8_lossy(&cli_review_text.stdout);
    assert!(cli_review_stdout.contains("Lane review: api-branch-lane"));
    assert!(cli_review_stdout.contains("Evidence:"));
    assert!(cli_review_stdout.contains("Next steps:"));

    let readiness_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            &format!("/v1/lanes/{lane_id}/readiness"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(readiness_response.status, 200);
    let readiness: serde_json::Value = readiness_response.body_json().unwrap();
    assert_eq!(readiness["lane"]["record"]["name"], "api-branch-lane");
    assert_eq!(readiness["ready"], true);
    assert!(readiness["blockers"].as_array().unwrap().is_empty());
    assert_eq!(readiness["latest_test"]["success"], true);
    assert!(readiness["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue["code"] == "missing_latest_eval"));

    let cli_readiness = run_crabdb_json(temp.path(), &["lane", "readiness", "api-branch-lane"]);
    assert_eq!(cli_readiness["lane"]["record"]["lane_id"], lane_id);
    assert_eq!(cli_readiness["ready"], true);

    let handoff_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            &format!("/v1/lanes/{lane_id}/handoff?limit=5"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(handoff_response.status, 200);
    let handoff: serde_json::Value = handoff_response.body_json().unwrap();
    assert_eq!(handoff["lane"]["record"]["name"], "api-branch-lane");
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
        &["lane", "handoff", "api-branch-lane", "--limit", "5"],
    );
    assert_eq!(cli_handoff["lane"]["record"]["lane_id"], lane_id);
    assert_eq!(cli_handoff["readiness"]["ready"], true);

    let remove_dirty_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "DELETE",
            &format!("/v1/lanes/{lane_id}"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(remove_dirty_response.status, 400);

    let merge_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/branches/main/merge-lane",
            serde_json::json!({
                "lane_id": lane_id,
                "strategy": "line_id_aware"
            }),
        ),
    );
    assert_eq!(merge_response.status, 200);
    let merge: serde_json::Value = merge_response.body_json().unwrap();
    assert_eq!(merge["source_ref"], "refs/lanes/api-branch-lane");
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
            &format!("/v1/lanes/{lane_id}"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(remove_response.status, 200);
    let removed: serde_json::Value = remove_response.body_json().unwrap();
    assert_eq!(removed["lane_id"], lane_id);
    assert_eq!(removed["forced"], false);
    assert_eq!(removed["removed_workdir"], workdir);
    assert!(!std::path::Path::new(&workdir).exists());
    assert_eq!(db.lane_details(&lane_id).unwrap().branch.status, "removed");
}

#[test]
fn local_lane_http_api_can_require_bearer_token() {
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

    let cross_origin = crabdb::server::handle_http_request_with_auth(
        &mut db,
        &api_request_with_headers(
            "GET",
            "/v1/health",
            &[("Origin", "https://example.com")],
            serde_json::Value::Null,
        ),
        &auth,
    );
    assert_eq!(cross_origin.status, 403);
    let cross_origin_body: serde_json::Value = cross_origin.body_json().unwrap();
    assert!(cross_origin_body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("local loopback origin"));

    let missing = crabdb::server::handle_http_request_with_auth(
        &mut db,
        &api_request(
            "POST",
            "/v1/lane/turns",
            serde_json::json!({ "lane": "secure-lane", "branch": "main" }),
        ),
        &auth,
    );
    assert_eq!(missing.status, 401);

    let invalid = crabdb::server::handle_http_request_with_auth(
        &mut db,
        &api_request_with_headers(
            "POST",
            "/v1/lane/turns",
            &[("Authorization", "Bearer wrong-token")],
            serde_json::json!({ "lane": "secure-lane", "branch": "main" }),
        ),
        &auth,
    );
    assert_eq!(invalid.status, 401);

    let ok = crabdb::server::handle_http_request_with_auth(
        &mut db,
        &api_request_with_headers(
            "POST",
            "/v1/lane/turns",
            &[("Authorization", "Bearer secret-token")],
            serde_json::json!({ "lane": "secure-lane", "branch": "main" }),
        ),
        &auth,
    );
    assert_eq!(ok.status, 201);
    let turn: LaneTurnStartReport = ok.body_json().unwrap();
    assert!(turn.turn.lane_id.starts_with("lane_"));

    let local_origin = crabdb::server::handle_http_request_with_auth(
        &mut db,
        &api_request_with_headers(
            "POST",
            "/v1/lane/turns",
            &[
                ("Authorization", "Bearer secret-token"),
                ("Origin", "http://127.0.0.1:8765"),
            ],
            serde_json::json!({ "lane": "local-origin-lane", "branch": "main" }),
        ),
        &auth,
    );
    assert_eq!(local_origin.status, 201);

    let second = crabdb::server::handle_http_request_with_auth(
        &mut db,
        &api_request_with_headers(
            "POST",
            "/v1/lane/turns",
            &[("X-CrabDB-Token", "secret-token")],
            serde_json::json!({ "lane": "other-secure-lane", "branch": "main" }),
        ),
        &auth,
    );
    assert_eq!(second.status, 201);
}

#[test]
fn local_lane_http_api_rejects_oversized_requests() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let raw = vec![b'x'; 16 * 1024 * 1024 + 1];
    let response = crabdb::server::handle_http_request(&mut db, &raw);
    assert_eq!(response.status, 400);
    let body: serde_json::Value = response.body_json().unwrap();
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("exceeding limit"));
}

#[test]
fn daemon_listener_rate_limits_peer_requests() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let workspace = temp.path().to_path_buf();
    let handle = thread::spawn(move || {
        let mut db = CrabDb::open(workspace).unwrap();
        let rate_limit =
            crabdb::server::ServerRateLimit::per_window(1, Duration::from_secs(60)).unwrap();
        crabdb::server::serve_listener_with_auth_and_rate_limit(
            &mut db,
            listener,
            Some(2),
            crabdb::server::ServerAuth::disabled(),
            rate_limit,
        )
        .unwrap();
    });

    let request = b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    let first = raw_http_request(port, request);
    assert!(first.contains(" 200 "), "{first}");
    let second = raw_http_request(port, request);
    assert!(second.contains(" 429 "), "{second}");
    assert!(second.contains("rate limit exceeded"), "{second}");
    handle.join().unwrap();
}

#[test]
fn cli_daemon_url_routes_hot_lane_commands() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    fs::write(temp.path().join("NOTES.md"), "notes\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let port = free_loopback_port();
    let mut daemon = DaemonGuard {
        child: Command::new(crabdb_bin())
            .arg("--workspace")
            .arg(temp.path())
            .arg("--quiet")
            .arg("daemon")
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(port.to_string())
            .arg("--no-auth")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    };
    wait_for_daemon_health(port);
    assert!(daemon.child.try_wait().unwrap().is_none());

    let daemon_url = format!("http://127.0.0.1:{port}");
    let status = run_crabdb_json_daemon(temp.path(), &daemon_url, &["status"]);
    assert_eq!(status["branch"], "main");

    let spawn = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &[
            "lane",
            "spawn",
            "rpc-bot",
            "--from",
            "main",
            "--no-materialize",
        ],
    );
    assert_eq!(spawn["ref_name"], "refs/lanes/rpc-bot");
    assert!(spawn["workdir"].is_null());

    let list = run_crabdb_json_daemon(temp.path(), &daemon_url, &["lane", "list"]);
    assert!(list
        .as_array()
        .unwrap()
        .iter()
        .any(|lane| lane["record"]["name"] == "rpc-bot"));

    let show = run_crabdb_json_daemon(temp.path(), &daemon_url, &["lane", "show", "rpc-bot"]);
    assert_eq!(show["record"]["name"], "rpc-bot");
    assert_eq!(show["branch"]["ref_name"], "refs/lanes/rpc-bot");

    let no_workdir =
        run_crabdb_json_daemon(temp.path(), &daemon_url, &["lane", "workdir", "rpc-bot"]);
    assert!(no_workdir["workdir"].is_null());

    let claim = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "claim", "rpc-bot", "README.md", "--ttl-secs", "120"],
    );
    assert_eq!(claim["claimed"], true);
    assert_eq!(claim["path"], "README.md");

    let lease = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &[
            "lease",
            "acquire",
            "rpc-bot",
            "--path",
            "NOTES.md",
            "--ttl-secs",
            "120",
        ],
    );
    let lease_id = lease["lease"]["lease_id"].as_str().unwrap().to_string();
    assert_eq!(lease["lease"]["path"], "NOTES.md");
    let leases = run_crabdb_json_daemon(temp.path(), &daemon_url, &["lease", "list"]);
    assert!(leases
        .as_array()
        .unwrap()
        .iter()
        .any(|lease| lease["lease_id"] == lease_id));
    let released =
        run_crabdb_json_daemon(temp.path(), &daemon_url, &["lease", "release", &lease_id]);
    assert_eq!(released["lease_id"], lease_id);

    let session = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &["session", "start", "rpc-bot", "--title", "daemon session"],
    );
    let session_id = session["session"]["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(session["session"]["title"], "daemon session");
    let current_session =
        run_crabdb_json_daemon(temp.path(), &daemon_url, &["session", "current", "rpc-bot"]);
    assert!(current_session
        .as_array()
        .unwrap()
        .iter()
        .any(|report| report["session"]["session_id"] == session_id));
    let session_list = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &["session", "list", "--lane", "rpc-bot"],
    );
    assert!(session_list
        .as_array()
        .unwrap()
        .iter()
        .any(|session| session["session_id"] == session_id));
    let session_show =
        run_crabdb_json_daemon(temp.path(), &daemon_url, &["session", "show", &session_id]);
    assert_eq!(session_show["session"]["session_id"], session_id);
    let session_context = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &["session", "context", &session_id, "--limit", "5"],
    );
    assert_eq!(session_context["session"]["session_id"], session_id);

    let approval = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &[
            "approvals",
            "request",
            "rpc-bot",
            "--action",
            "deploy",
            "--summary",
            "daemon approval",
            "--session",
            &session_id,
            "--payload-json",
            r#"{"risk":"low"}"#,
        ],
    );
    let approval_id = approval["approval"]["approval_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(approval["approval"]["status"], "pending");
    let approvals = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &[
            "approvals",
            "list",
            "--lane",
            "rpc-bot",
            "--status",
            "pending",
        ],
    );
    assert!(approvals
        .as_array()
        .unwrap()
        .iter()
        .any(|approval| approval["approval_id"] == approval_id));
    let approval_show = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &["approvals", "show", &approval_id],
    );
    assert_eq!(approval_show["approval_id"], approval_id);
    let approval_decision = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &[
            "approvals",
            "decide",
            &approval_id,
            "--decision",
            "approved",
            "--reviewer",
            "daemon-reviewer",
            "--note",
            "ok",
        ],
    );
    assert_eq!(approval_decision["approval"]["status"], "approved");

    let patch_path = temp.path().join("rpc-patch.json");
    fs::write(
        &patch_path,
        serde_json::to_vec(&serde_json::json!({
            "base_change": spawn["base_change"].as_str().unwrap(),
            "message": "daemon CLI patch",
            "edits": [
                {"op": "write", "path": "README.md", "content": "hello\nrpc\n"}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    let patch_path = patch_path.to_string_lossy().to_string();
    let patch = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "apply-patch", "rpc-bot", "--patch", &patch_path],
    );
    assert!(patch["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "README.md"));
    fs::remove_file(temp.path().join("rpc-patch.json")).unwrap();

    let read = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "read", "rpc-bot", "README.md"],
    );
    assert_eq!(read["content"], "hello\nrpc\n");

    let diff = run_crabdb_json_daemon(temp.path(), &daemon_url, &["lane", "diff", "rpc-bot"]);
    assert!(diff["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "README.md"));

    let doctor = run_crabdb_json_daemon(temp.path(), &daemon_url, &["doctor"]);
    assert_eq!(doctor["status"], "ok");
    let why = run_crabdb_json_daemon(temp.path(), &daemon_url, &["why", "README.md:1"]);
    assert_eq!(why["path"], "README.md");
    let history = run_crabdb_json_daemon(temp.path(), &daemon_url, &["history", "README.md"]);
    assert!(history["file_history"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["path"] == "README.md"));
    let code_from = run_crabdb_json_daemon(temp.path(), &daemon_url, &["code-from", "rpc-bot"]);
    assert!(code_from["operations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|operation| operation["message"] == "daemon CLI patch"));

    let lane_timeline =
        run_crabdb_json_daemon(temp.path(), &daemon_url, &["lane", "timeline", "rpc-bot"]);
    assert!(lane_timeline
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["message"] == "daemon CLI patch"));
    let timeline = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &["timeline", "--lane", "rpc-bot", "--limit", "20"],
    );
    assert!(timeline
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["message"] == "daemon CLI patch"));

    let readiness =
        run_crabdb_json_daemon(temp.path(), &daemon_url, &["lane", "readiness", "rpc-bot"]);
    assert_eq!(readiness["ready"], true);

    let contribution = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "contribution", "rpc-bot"],
    );
    assert_eq!(contribution["status"]["lane"]["record"]["name"], "rpc-bot");

    let review = run_crabdb_json_daemon(temp.path(), &daemon_url, &["lane", "review", "rpc-bot"]);
    assert_eq!(review["lane"]["record"]["name"], "rpc-bot");
    assert_eq!(review["readiness"]["ready"], true);
    assert!(review["recent_operations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|operation| operation["message"] == "daemon CLI patch"));

    let gates = run_crabdb_json_daemon(temp.path(), &daemon_url, &["lane", "gates", "rpc-bot"]);
    assert_eq!(gates["lane"]["record"]["name"], "rpc-bot");

    let events = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "events", "--lane", "rpc-bot"],
    );
    assert!(events
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["lane_id"] == spawn["lane_id"]));

    let turn = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &[
            "lane",
            "turn",
            "start",
            "rpc-bot",
            "--from",
            "main",
            "--title",
            "daemon trace routing",
        ],
    );
    let turn_id = turn["turn"]["turn_id"].as_str().unwrap().to_string();

    let turn_message = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &[
            "lane",
            "turn",
            "message",
            &turn_id,
            "--role",
            "user",
            "--text",
            "daemon turn message",
        ],
    );
    assert_eq!(turn_message["role"], "user");

    let turn_event = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &[
            "lane",
            "turn",
            "event",
            &turn_id,
            "--event-type",
            "checkpoint",
            "--payload-json",
            r#"{"via":"daemon"}"#,
        ],
    );
    assert_eq!(turn_event["event"]["event_type"], "checkpoint");

    let turn_patch_path = temp.path().join("rpc-turn-patch.json");
    fs::write(
        &turn_patch_path,
        serde_json::to_vec(&serde_json::json!({
            "message": "daemon turn patch",
            "edits": [
                {"op": "write", "path": "TURN.md", "content": "turn rpc\n"}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    let turn_patch_path = turn_patch_path.to_string_lossy().to_string();
    let turn_patch = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &[
            "lane",
            "turn",
            "apply-patch",
            &turn_id,
            "--patch",
            &turn_patch_path,
        ],
    );
    assert!(turn_patch["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "TURN.md"));
    fs::remove_file(temp.path().join("rpc-turn-patch.json")).unwrap();

    let turn_details = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "turn", "show", &turn_id],
    );
    assert_eq!(turn_details["turn"]["turn_id"].as_str().unwrap(), turn_id);
    assert!(turn_details["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|message| message["body"] == "daemon turn message"));
    assert!(turn_details["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["event_type"] == "checkpoint"));

    let trace_start = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &[
            "lane",
            "trace",
            "start",
            &turn_id,
            "--type",
            "tool_call",
            "--name",
            "daemon rpc trace",
        ],
    );
    let span_id = trace_start["span"]["span_id"].as_str().unwrap().to_string();
    let trace_id = trace_start["span"]["trace_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(trace_start["span"]["turn_id"].as_str().unwrap(), turn_id);

    let trace_end = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "trace", "end", &span_id, "--status", "completed"],
    );
    assert_eq!(trace_end["span"]["status"], "completed");

    let trace_list = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "trace", "list", "--turn", &turn_id, "--limit", "10"],
    );
    assert!(trace_list
        .as_array()
        .unwrap()
        .iter()
        .any(|span| span["span_id"] == span_id));

    let trace_summary = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "trace", "summary", "--trace-id", &trace_id],
    );
    assert_eq!(trace_summary["trace_id"].as_str().unwrap(), trace_id);
    assert_eq!(trace_summary["span_count"], 1);
    assert_eq!(trace_summary["ended_span_count"], 1);

    let trace_show = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "trace", "show", &span_id],
    );
    assert_eq!(trace_show["span_id"].as_str().unwrap(), span_id);
    assert_eq!(trace_show["trace_id"].as_str().unwrap(), trace_id);

    let turn_end = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "turn", "end", &turn_id, "--status", "completed"],
    );
    assert_eq!(turn_end["turn"]["status"], "completed");

    let handoff = run_crabdb_json_daemon(temp.path(), &daemon_url, &["lane", "handoff", "rpc-bot"]);
    assert_eq!(handoff["lane"]["record"]["name"], "rpc-bot");

    fs::write(
        temp.path().join("NOTES.md"),
        "notes\nworkspace record through daemon\n",
    )
    .unwrap();
    let workspace_record = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &[
            "record",
            "-m",
            "daemon workspace record",
            "--paths",
            "NOTES.md",
        ],
    );
    assert!(workspace_record["operation"].as_str().is_some());
    assert!(workspace_record["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "NOTES.md"));
    let clean_status = run_crabdb_json_daemon(temp.path(), &daemon_url, &["status"]);
    assert_eq!(clean_status["worktree_state"], "Clean");

    let materialized = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &[
            "lane",
            "spawn",
            "mat-rpc",
            "--from",
            "main",
            "--materialize",
        ],
    );
    let materialized_workdir = materialized["workdir"].as_str().unwrap();
    let materialized_workdir_report =
        run_crabdb_json_daemon(temp.path(), &daemon_url, &["lane", "workdir", "mat-rpc"]);
    assert_eq!(
        materialized_workdir_report["workdir"].as_str().unwrap(),
        materialized_workdir
    );
    fs::write(
        Path::new(materialized_workdir).join("README.md"),
        "hello\nrecorded through daemon\n",
    )
    .unwrap();
    let recorded = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "record", "mat-rpc", "-m", "daemon CLI record"],
    );
    assert!(recorded["operation"].as_str().is_some());
    assert!(recorded["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "README.md"));
    let materialized_status =
        run_crabdb_json_daemon(temp.path(), &daemon_url, &["lane", "status", "mat-rpc"]);
    assert_eq!(materialized_status["workdir_state"], "Clean");

    let merge = run_crabdb_json_daemon(
        temp.path(),
        &daemon_url,
        &["merge-lane", "rpc-bot", "--into", "main", "--dry-run"],
    );
    assert_eq!(merge["dry_run"], true);
    assert!(merge["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "README.md"));
}

#[test]
fn cli_auto_discovers_daemon_for_hot_commands_and_falls_back_on_stale_endpoint() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let endpoint_path = temp.path().join(".crabdb/daemon.json");
    let port = free_loopback_port();
    let mut daemon = DaemonGuard {
        child: Command::new(crabdb_bin())
            .arg("--workspace")
            .arg(temp.path())
            .arg("--quiet")
            .arg("daemon")
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(port.to_string())
            .arg("--max-requests")
            .arg("2")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    };
    wait_for_daemon_health(port);
    let endpoint = wait_for_daemon_endpoint(&endpoint_path);
    assert_eq!(endpoint["url"], format!("http://127.0.0.1:{port}"));
    assert_eq!(endpoint["auth"], true);

    let status = run_crabdb_json(temp.path(), &["status"]);
    assert_eq!(status["branch"], "main");
    wait_for_child_exit(&mut daemon.child);
    assert!(!endpoint_path.exists());

    fs::write(
        &endpoint_path,
        serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "url": "http://127.0.0.1:1",
            "pid": 0,
            "auth": false
        }))
        .unwrap(),
    )
    .unwrap();
    let fallback = run_crabdb_json(temp.path(), &["status"]);
    assert_eq!(fallback["branch"], "main");
}

#[test]
fn local_api_and_cli_export_openapi_contract() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let cli = run_crabdb_json(temp.path(), &["api", "openapi"]);
    assert_eq!(cli["openapi"], "3.1.0");
    assert!(cli["paths"].get("/v1/openapi.json").is_some());
    assert!(cli["paths"]["/v1/lanes"]["get"].is_object());
    assert!(cli["paths"]["/v1/lanes/{lane_or_id}"]["delete"].is_object());
    assert!(cli["paths"]
        .get("/v1/lanes/{lane_or_id}/read-file")
        .is_some());
    assert!(cli["components"]["schemas"]["LaneReadFileRequest"].is_object());
    assert!(cli["paths"].get("/v1/lane/events").is_some());
    assert!(cli["paths"].get("/v1/lane/spans").is_some());
    assert!(cli["paths"].get("/v1/lane/turns/{turn_id}/spans").is_some());
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
    assert!(api["paths"]["/v1/lane/turns/{turn_id}/patches"]["post"]["requestBody"].is_object());
    assert_eq!(
        api["components"]["schemas"]["PatchRequest"]["properties"]["allow_stale"]["type"],
        "boolean"
    );

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
fn lane_turn_cli_tracks_events_and_closeout() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let started = run_crabdb_json(
        temp.path(),
        &[
            "lane", "turn", "start", "cli-lane", "--from", "main", "--title", "CLI turn",
        ],
    );
    let turn_id = started["turn"]["turn_id"].as_str().unwrap().to_string();
    assert_eq!(started["session"]["title"], "CLI turn");

    let message = run_crabdb_json(
        temp.path(),
        &[
            "lane",
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
            "lane",
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
            "lane",
            "turn",
            "apply-patch",
            &turn_id,
            "--patch",
            patch_path.to_str().unwrap(),
        ],
    );
    assert_eq!(patch["changed_paths"][0]["path"], "cli-turn.md");

    let details = run_crabdb_json(temp.path(), &["lane", "turn", "show", &turn_id]);
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
        &["lane", "turn", "end", &turn_id, "--status", "completed"],
    );
    assert_eq!(ended["turn"]["status"], "completed");

    let details = run_crabdb_json(temp.path(), &["lane", "turn", "show", &turn_id]);
    assert_eq!(details["turn"]["status"], "completed");
}

#[test]
fn mcp_stdio_tools_drive_lane_turn_workflow() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.config_set("lane.default_materialize", "true").unwrap();
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
        .any(|resource| resource["uri"] == "crabdb://docs/lane-workflows"));

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
        .any(|template| template["uriTemplate"] == "crabdb://workspace/lanes/{lane}/status"));
    assert!(template_list
        .iter()
        .any(|template| template["uriTemplate"] == "crabdb://workspace/lanes/{lane}/review"));
    assert!(template_list
        .iter()
        .any(|template| template["uriTemplate"] == "crabdb://workspace/lanes/{lane}/contribution"));
    assert!(template_list
        .iter()
        .any(|template| template["uriTemplate"] == "crabdb://workspace/lanes/{lane}/gates"));
    assert!(template_list
        .iter()
        .any(|template| template["uriTemplate"] == "crabdb://workspace/lanes/{lane}/readiness"));
    assert!(template_list
        .iter()
        .any(|template| template["uriTemplate"] == "crabdb://workspace/lanes/{lane}/handoff"));
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
                "uri": "crabdb://docs/lane-workflows"
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
        .contains("Lane Workflows"));

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
        .any(|prompt| prompt["name"] == "crabdb.lane_task"));
    assert!(prompt_list
        .iter()
        .any(|prompt| prompt["name"] == "crabdb.review_lane"));
    assert!(prompt_list
        .iter()
        .any(|prompt| prompt["name"] == "crabdb.resolve_conflict"));

    let lane_prompt = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 15,
            "method": "prompts/get",
            "params": {
                "name": "crabdb.lane_task",
                "arguments": {
                    "lane": "mcp-lane",
                    "task": "Improve README setup notes",
                    "branch": "main"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        lane_prompt["result"]["description"],
        "Safe CrabDB lane task workflow"
    );
    let prompt_messages = lane_prompt["result"]["messages"].as_array().unwrap();
    assert!(prompt_messages[0]["content"]["text"]
        .as_str()
        .unwrap()
        .contains("crabdb.begin_turn"));
    assert!(prompt_messages[0]["content"]["text"]
        .as_str()
        .unwrap()
        .contains("crabdb.lane_rewind"));
    assert!(prompt_messages[0]["content"]["text"]
        .as_str()
        .unwrap()
        .contains("mcp-lane"));
    assert!(prompt_messages
        .iter()
        .any(|message| message["content"]["type"] == "resource"));

    let review_prompt = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 151,
            "method": "prompts/get",
            "params": {
                "name": "crabdb.review_lane",
                "arguments": {
                    "lane": "mcp-lane"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        review_prompt["result"]["description"],
        "CrabDB lane review checklist"
    );
    assert!(review_prompt["result"]["messages"][0]["content"]["text"]
        .as_str()
        .unwrap()
        .contains("crabdb.lane_review"));

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
                    "name": "crabdb.lane_task"
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
                    "name": "lane",
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
    assert!(tools.iter().any(|tool| tool["name"] == "crabdb.lane_spawn"));
    assert!(tools.iter().any(|tool| tool["name"] == "crabdb.lane_list"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "crabdb.lane_review"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "crabdb.lane_contribution"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "crabdb.gate_history"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "crabdb.lane_readiness"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "crabdb.lane_handoff"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "crabdb.guardrail_check"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "crabdb.lane_remove"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "crabdb.lane_rewind"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "crabdb.apply_patch"));
    assert!(tools.iter().any(|tool| tool["name"] == "crabdb.run_test"));
    assert!(tools.iter().any(|tool| tool["name"] == "crabdb.read_file"));
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
    assert!(tool_annotation("crabdb.lane_rewind", "destructiveHint"));
    assert!(tool_annotation("crabdb.run_test", "openWorldHint"));
    assert!(tool_annotation("crabdb.guardrail_check", "readOnlyHint"));
    assert!(tool_annotation("crabdb.lane_review", "readOnlyHint"));
    assert!(tool_annotation("crabdb.gate_history", "readOnlyHint"));

    let spawned = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 20,
            "method": "tools/call",
            "params": {
                "name": "crabdb.lane_spawn",
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
    let lifecycle_lane_id = spawned["result"]["structuredContent"]["lane_id"]
        .as_str()
        .unwrap()
        .to_string();

    let lane_list = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 21,
            "method": "tools/call",
            "params": {
                "name": "crabdb.lane_list",
                "arguments": {}
            }
        }),
    )
    .unwrap();
    assert_eq!(lane_list["result"]["isError"], false);
    assert!(lane_list["result"]["structuredContent"]
        .as_array()
        .unwrap()
        .iter()
        .any(|lane| lane["record"]["name"] == "mcp-lifecycle"));

    let lane_show = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 22,
            "method": "tools/call",
            "params": {
                "name": "crabdb.lane_show",
                "arguments": {
                    "lane": lifecycle_lane_id.clone()
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(lane_show["result"]["isError"], false);
    assert_eq!(
        lane_show["result"]["structuredContent"]["record"]["name"],
        "mcp-lifecycle"
    );

    let lane_status = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 23,
            "method": "tools/call",
            "params": {
                "name": "crabdb.lane_status",
                "arguments": {
                    "lane": lifecycle_lane_id.clone()
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(lane_status["result"]["isError"], false);
    assert_eq!(
        lane_status["result"]["structuredContent"]["lane"]["record"]["name"],
        "mcp-lifecycle"
    );

    let templated_lane_status = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 25,
            "method": "resources/read",
            "params": {
                "uri": "crabdb://workspace/lanes/mcp-lifecycle/status"
            }
        }),
    )
    .unwrap();
    assert_eq!(
        templated_lane_status["result"]["contents"][0]["mimeType"],
        "application/json"
    );
    let templated_status_text = templated_lane_status["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let templated_status_json: serde_json::Value =
        serde_json::from_str(templated_status_text).unwrap();
    assert_eq!(
        templated_status_json["lane"]["record"]["name"],
        "mcp-lifecycle"
    );

    let review = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 251,
            "method": "tools/call",
            "params": {
                "name": "crabdb.lane_review",
                "arguments": {
                    "lane": "mcp-lifecycle",
                    "limit": 5
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(review["result"]["isError"], false);
    assert_eq!(
        review["result"]["structuredContent"]["lane"]["record"]["name"],
        "mcp-lifecycle"
    );
    assert_eq!(
        review["result"]["structuredContent"]["readiness"]["ready"],
        true
    );

    let templated_review = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 252,
            "method": "resources/read",
            "params": {
                "uri": "crabdb://workspace/lanes/mcp-lifecycle/review"
            }
        }),
    )
    .unwrap();
    let templated_review_text = templated_review["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let templated_review_json: serde_json::Value =
        serde_json::from_str(templated_review_text).unwrap();
    assert_eq!(
        templated_review_json["lane"]["record"]["name"],
        "mcp-lifecycle"
    );
    assert_eq!(templated_review_json["readiness"]["ready"], true);

    let contribution = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 26,
            "method": "tools/call",
            "params": {
                "name": "crabdb.lane_contribution",
                "arguments": {
                    "lane": "mcp-lifecycle",
                    "limit": 5
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(contribution["result"]["isError"], false);
    assert_eq!(
        contribution["result"]["structuredContent"]["status"]["lane"]["record"]["name"],
        "mcp-lifecycle"
    );

    let templated_contribution = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 27,
            "method": "resources/read",
            "params": {
                "uri": "crabdb://workspace/lanes/mcp-lifecycle/contribution"
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
        templated_contribution_json["status"]["lane"]["record"]["name"],
        "mcp-lifecycle"
    );

    let readiness = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 29,
            "method": "tools/call",
            "params": {
                "name": "crabdb.lane_readiness",
                "arguments": {
                    "lane": "mcp-lifecycle"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(readiness["result"]["isError"], false);
    assert_eq!(
        readiness["result"]["structuredContent"]["lane"]["record"]["name"],
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
                "uri": "crabdb://workspace/lanes/mcp-lifecycle/readiness"
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
        templated_readiness_json["lane"]["record"]["name"],
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
                "name": "crabdb.lane_handoff",
                "arguments": {
                    "lane": "mcp-lifecycle",
                    "limit": 5
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(handoff["result"]["isError"], false);
    assert_eq!(
        handoff["result"]["structuredContent"]["lane"]["record"]["name"],
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
                "uri": "crabdb://workspace/lanes/mcp-lifecycle/handoff"
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
        templated_handoff_json["lane"]["record"]["name"],
        "mcp-lifecycle"
    );
    assert_eq!(templated_handoff_json["readiness"]["ready"], true);

    let lane_completion = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 28,
            "method": "completion/complete",
            "params": {
                "ref": {
                    "type": "ref/resource",
                    "uri": "crabdb://workspace/lanes/{lane}/handoff"
                },
                "argument": {
                    "name": "lane",
                    "value": "mcp"
                }
            }
        }),
    )
    .unwrap();
    assert!(lane_completion["result"]["completion"]["values"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value.as_str() == Some("mcp-lifecycle")));

    let lane_remove = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 24,
            "method": "tools/call",
            "params": {
                "name": "crabdb.lane_remove",
                "arguments": {
                    "lane": lifecycle_lane_id.clone()
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(lane_remove["result"]["isError"], false);
    assert_eq!(
        lane_remove["result"]["structuredContent"]["lane_id"],
        lifecycle_lane_id
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
                    "lane": "mcp-lane",
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

    let workdir = db.lane_workdir("mcp-lane").unwrap().workdir.unwrap();
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
                    "lane": "mcp-lane",
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
                    "lane": "mcp-lane",
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
        db.lane_status("mcp-lane")
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
                "name": "crabdb.lane_handoff",
                "arguments": {
                    "lane": "mcp-lane",
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
fn mcp_status_read_only_does_not_refresh_worktree_index() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), "a1\n").unwrap();
    fs::write(temp.path().join("b.txt"), "b1\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let conn = Connection::open(temp.path().join(".crabdb/index/crabdb.sqlite")).unwrap();
    let count_rows = |table: &str| -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    };
    let index_before = count_rows("worktree_file_index");
    let objects_before = count_rows("objects");
    let prolly_nodes_before = count_rows("prolly_nodes");
    assert_eq!(index_before, 2);

    fs::write(temp.path().join("a.txt"), "a1\na2\n").unwrap();
    fs::write(temp.path().join("c.txt"), "c1\n").unwrap();

    let status = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "crabdb.status",
                "arguments": {"branch": "main"}
            }
        }),
    )
    .unwrap();
    assert_eq!(status["result"]["isError"], false);
    let changed_paths = status["result"]["structuredContent"]["changed_paths"]
        .as_array()
        .unwrap();
    assert!(changed_paths.iter().any(|path| path["path"] == "a.txt"));
    assert!(changed_paths.iter().any(|path| path["path"] == "c.txt"));

    assert_eq!(count_rows("worktree_file_index"), index_before);
    assert_eq!(count_rows("objects"), objects_before);
    assert_eq!(count_rows("prolly_nodes"), prolly_nodes_before);

    let resource_status = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "resources/read",
            "params": {
                "uri": "crabdb://workspace/status"
            }
        }),
    )
    .unwrap();
    let text = resource_status["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let resource_status: serde_json::Value = serde_json::from_str(text).unwrap();
    let resource_paths = resource_status["changed_paths"].as_array().unwrap();
    assert!(resource_paths.iter().any(|path| path["path"] == "a.txt"));
    assert!(resource_paths.iter().any(|path| path["path"] == "c.txt"));

    assert_eq!(count_rows("worktree_file_index"), index_before);
    assert_eq!(count_rows("objects"), objects_before);
    assert_eq!(count_rows("prolly_nodes"), prolly_nodes_before);
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
        .any(|entry| entry["key"] == "lane.default_materialize"));
    assert!(http_entries
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["key"] == "lane.require_test_gate"));
    assert!(http_entries
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["key"] == "lane.require_eval_gate"));
    assert!(http_entries
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["key"] == "lane.required_test_suites"));
    assert!(http_entries
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["key"] == "lane.required_eval_suites"));
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
    let test_gate_set = db.config_set("lane.require_test_gate", "yes").unwrap();
    assert_eq!(test_gate_set.old_value, "false");
    assert_eq!(test_gate_set.new_value, "true");
    let eval_gate_set = db.config_set("lane.require_eval_gate", "on").unwrap();
    assert_eq!(eval_gate_set.old_value, "false");
    assert_eq!(eval_gate_set.new_value, "true");
    let test_suites_set = db
        .config_set("lane.required_test_suites", "unit,policy-smoke")
        .unwrap();
    assert_eq!(test_suites_set.old_value, "");
    assert_eq!(test_suites_set.new_value, "unit,policy-smoke");
    let eval_suites_set = db
        .config_set("lane.required_eval_suites", "regression; safety")
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
    assert!(reopened.config().lane.require_test_gate);
    assert!(reopened.config().lane.require_eval_gate);
    assert_eq!(
        reopened.config().lane.required_test_suites,
        vec!["unit".to_string(), "policy-smoke".to_string()]
    );
    assert_eq!(
        reopened.config().lane.required_eval_suites,
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

    fs::remove_file(temp.path().join("src/lib.rs")).unwrap();
    let deleted = db
        .git_import_update(Some("main"), Some("remove tracked lib".to_string()))
        .unwrap();
    assert!(deleted.operation.is_some());
    assert!(deleted
        .changed_paths
        .iter()
        .any(|path| { path.path == "src/lib.rs" && path.kind == crabdb::FileChangeKind::Deleted }));
    let deleted_root = db.inspect_root(&deleted.root_id.0).unwrap();
    assert!(!deleted_root
        .files
        .iter()
        .any(|file| file.path == "src/lib.rs"));
    let clean_after_delete = db.status(Some("main")).unwrap();
    assert_eq!(
        clean_after_delete.worktree_state,
        WorktreeState::DirtyUntracked
    );
    assert!(clean_after_delete
        .changed_paths
        .iter()
        .any(|path| path.path == "scratch.txt" && path.kind == crabdb::FileChangeKind::Added));
    assert!(!clean_after_delete
        .changed_paths
        .iter()
        .any(|path| path.path == "src/lib.rs"));

    let mappings = db.git_mappings(10).unwrap();
    assert_eq!(mappings.len(), 3);
    assert_eq!(mappings[0].crab_change, deleted.operation.unwrap());
    assert_eq!(mappings[1].crab_change, imported_change);
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
        "one\nlane rewrote this line\nthree\n",
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
    assert_eq!(after.current_text, "lane rewrote this line");
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
    assert_eq!(cli_by_line["current_text"], "lane rewrote this line");
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
        "lane rewrote this line"
    );
}

#[test]
fn diff_supports_roots_dirty_and_line_id_surfaces() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "one\ntwo\nthree\n").unwrap();
    fs::write(temp.path().join("zz-notes.txt"), "alpha\nbeta\n").unwrap();
    let init = CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let before = db.why("README.md:2", Some("main")).unwrap();
    fs::write(temp.path().join("README.md"), "one\nTWO\nthree\n").unwrap();
    fs::write(temp.path().join("zz-notes.txt"), "alpha\nBETA\n").unwrap();
    let record = db
        .record(
            Some("main"),
            Some("rewrite two files".to_string()),
            Actor::human(),
            false,
        )
        .unwrap();
    let change_id = record.operation.clone().unwrap();
    let range = format!("{}..{}", init.operation.0, change_id.0);

    let diff = db.diff_range_with_options(&range, true, true).unwrap();
    assert_eq!(diff.files.len(), 2);
    assert_eq!(diff.files[0].kind, crabdb::FileChangeKind::Modified);
    assert!(diff.files[0].patch.as_ref().unwrap().contains("+TWO"));
    assert!(diff.files[1].patch.as_ref().unwrap().contains("+BETA"));
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
    CrabDb::init_with_text_policy(
        temp.path(),
        "main",
        InitImportMode::WorkingTree,
        false,
        Some("full"),
    )
    .unwrap();

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
    db.spawn_lane("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_str(
        r#"{
          "session_id": "session-review",
          "message": "lane adds review line",
          "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nlane\nworld\n"}
          ]
        }"#,
    )
    .unwrap();
    let applied = apply_lane_patch_at_head(&mut db, "doc-bot", patch).unwrap();

    let why = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/why?path_line=README.md:2&branch=refs/lanes/doc-bot",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(why.status, 200);
    let why: serde_json::Value = why.body_json().unwrap();
    assert_eq!(why["current_text"], "lane");
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
                    "branch": "refs/lanes/doc-bot"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_why["result"]["isError"], false);
    assert_eq!(
        mcp_why["result"]["structuredContent"]["current_text"],
        "lane"
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
                "branch": "refs/lanes/doc-bot"
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
            {"op": "write", "path": "README.md", "content": "hello\nintro\nlane\nworld\n"}
          ]
        }"#,
    )
    .unwrap();
    apply_lane_patch_at_head(&mut db, "doc-bot", move_patch).unwrap();

    let resolved = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            &format!("/v1/anchors/{anchor_id}?branch=refs/lanes/doc-bot"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(resolved.status, 200);
    let resolved: serde_json::Value = resolved.body_json().unwrap();
    assert_eq!(resolved["status"], "found");
    assert_eq!(resolved["line_number"], 3);
    assert_eq!(resolved["text"], "lane");

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
                    "branch": "refs/lanes/doc-bot"
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
fn refish_aliases_accept_branch_lane_and_root_selectors() {
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

    db.spawn_lane_with_workdir("doc-bot", Some("branch:main"), false, None, None, None)
        .unwrap();
    let lane_checkout = db.checkout("lane:doc-bot", true).unwrap();
    assert_eq!(lane_checkout.change_id, recorded_change);
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
fn lane_patch_can_merge_into_main() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("doc-bot", Some("main"), true, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_str(
        r#"{
          "message": "lane edits",
          "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nlane\n"},
            {"op": "write", "path": "src/lib.rs", "content": "pub fn answer() -> u32 { 42 }\n"}
          ]
        }"#,
    )
    .unwrap();
    let applied = apply_lane_patch_at_head(&mut db, "doc-bot", patch).unwrap();
    assert_eq!(applied.changed_paths.len(), 2);

    let merged = db.merge_lane("doc-bot", "main").unwrap();
    assert_eq!(merged.changed_paths.len(), 2);
    db.checkout("main", true).unwrap();

    assert_eq!(
        fs::read_to_string(temp.path().join("src/lib.rs")).unwrap(),
        "pub fn answer() -> u32 { 42 }\n"
    );
    assert!(db.fsck().unwrap().errors.is_empty());
}

#[test]
fn lane_patch_refreshes_clean_materialized_workdir_incrementally() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    fs::write(temp.path().join("old.txt"), "remove me\n").unwrap();
    fs::write(temp.path().join("untouched.txt"), "stable\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("workdir-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = PathBuf::from(spawned.workdir.unwrap());
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "refresh materialized workdir",
        "edits": [
            {"op": "write", "path": "README.md", "content": "hello\npatched\n"},
            {"op": "delete", "path": "old.txt"},
            {"op": "write", "path": "src/new.rs", "content": "pub fn new_file() {}\n"}
        ]
    }))
    .unwrap();

    let applied = apply_lane_patch_at_head(&mut db, "workdir-bot", patch).unwrap();
    assert_eq!(applied.changed_paths.len(), 3);
    assert_eq!(
        fs::read_to_string(workdir.join("README.md")).unwrap(),
        "hello\npatched\n"
    );
    assert!(!workdir.join("old.txt").exists());
    assert_eq!(
        fs::read_to_string(workdir.join("src/new.rs")).unwrap(),
        "pub fn new_file() {}\n"
    );
    assert_eq!(
        fs::read_to_string(workdir.join("untouched.txt")).unwrap(),
        "stable\n"
    );

    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(workdir.join(".crabdb/workdir-manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["root_id"], applied.root_id.0);
    assert!(manifest["files"].get("README.md").is_some());
    assert!(manifest["files"].get("src/new.rs").is_some());
    assert!(manifest["files"].get("untouched.txt").is_some());
    assert!(manifest["files"].get("old.txt").is_none());
    assert_eq!(
        db.lane_status("workdir-bot").unwrap().workdir_state,
        Some(WorktreeState::Clean)
    );
}

#[test]
fn merge_dry_run_reports_without_mutating_refs() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let before_head = db.status(Some("main")).unwrap().head.change_id;
    db.spawn_lane("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "lane edits",
        "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nlane\n"}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "doc-bot", patch).unwrap();

    let cli = run_crabdb_json(
        temp.path(),
        &[
            "merge-lane",
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
            "/v1/branches/main/merge-lane",
            serde_json::json!({
                "lane": "doc-bot",
                "dry_run": true
            }),
        ),
    );
    assert_eq!(api_response.status, 200);
    let api: serde_json::Value = api_response.body_json().unwrap();
    assert_eq!(api["dry_run"], true);

    let dry_run = db.merge_lane_with_options("doc-bot", "main", true).unwrap();
    assert!(dry_run.dry_run);
    assert_eq!(dry_run.changed_paths.len(), 1);
    assert_eq!(db.status(Some("main")).unwrap().head.change_id, before_head);
    assert_eq!(db.lane_details("doc-bot").unwrap().branch.status, "active");

    let merged = db.merge_lane("doc-bot", "main").unwrap();
    assert!(!merged.dry_run);
    assert_eq!(merged.changed_paths.len(), 1);
    assert_ne!(db.status(Some("main")).unwrap().head.change_id, before_head);

    let before_branch_head = db.status(Some("main")).unwrap().head.change_id;
    db.create_branch("feature", Some("main")).unwrap();
    fs::write(temp.path().join("README.md"), "hello\nlane\nbranch\n").unwrap();
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
    db.spawn_lane("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write", "path": "README.md", "content": "one\nlane\n"}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "doc-bot", patch).unwrap();

    fs::write(temp.path().join("README.md"), "one\nhuman\n").unwrap();
    db.record(
        Some("main"),
        Some("human edit".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();

    let dry_run = db.merge_lane_with_options("doc-bot", "main", true).unwrap();
    assert!(dry_run.dry_run);
    assert!(dry_run.changed_paths.is_empty());
    assert_eq!(
        dry_run.conflicts,
        vec!["both changed `README.md` differently"]
    );
    assert_eq!(db.lane_details("doc-bot").unwrap().branch.status, "active");

    let cli = run_crabdb_json(temp.path(), &["merge-lane", "doc-bot", "--dry-run"]);
    assert_eq!(cli["dry_run"], true);
    assert_eq!(cli["conflicts"][0], "both changed `README.md` differently");
    assert!(db.list_conflicts().unwrap().is_empty());

    let err = db.merge_lane("doc-bot", "main").unwrap_err();
    assert!(matches!(&err, Error::Conflict(_)));
    assert_eq!(
        db.lane_details("doc-bot").unwrap().branch.status,
        "conflicted"
    );
    let conflicts = db.list_conflicts().unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(
        conflicts[0].source_ref.as_deref(),
        Some("refs/lanes/doc-bot")
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

    let repeated = db.merge_lane("doc-bot", "main").unwrap_err();
    assert!(matches!(&repeated, Error::Conflict(_)));
    assert!(repeated.to_string().contains(&conflicts[0].conflict_set_id));
    assert_eq!(db.list_conflicts().unwrap().len(), 1);

    let resolved = db
        .resolve_conflict(&conflicts[0].conflict_set_id, "source")
        .unwrap();
    assert_eq!(resolved.resolution, "source");
    assert_eq!(db.lane_details("doc-bot").unwrap().branch.status, "merged");
    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "one\nlane\n"
    );
}

#[test]
fn local_api_direct_merge_lane_conflict_records_conflict_set() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "one\ntwo\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("api-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write", "path": "README.md", "content": "one\nlane-api\n"}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "api-bot", patch).unwrap();

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
            "/v1/branches/main/merge-lane",
            serde_json::json!({ "lane_id": "api-bot" }),
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
    assert_eq!(conflicts[0]["source_ref"], "refs/lanes/api-bot");
    assert_eq!(conflicts[0]["target_ref"], "refs/branches/main");
}

#[test]
fn lane_patch_can_replace_stable_line_with_expected_text() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "one\ntwo\nthree\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let why = db.why("README.md:2", Some("refs/lanes/doc-bot")).unwrap();
    let line_id = format!("{}:{}", why.line_id.origin_change.0, why.line_id.local_seq);
    let patch_json = serde_json::json!({
        "message": "line-id patch",
        "edits": [{
            "op": "replace_line",
            "path": "README.md",
            "line_id": line_id,
            "expected_text": "two",
            "new_text": "lane two"
        }]
    });
    let patch: PatchDocument = serde_json::from_value(patch_json).unwrap();
    let applied = apply_lane_patch_at_head(&mut db, "doc-bot", patch).unwrap();
    assert_eq!(applied.changed_paths.len(), 1);

    let changed = db.why("README.md:2", Some("refs/lanes/doc-bot")).unwrap();
    assert_eq!(changed.current_text, "lane two");
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
    let err = apply_lane_patch_at_head(&mut db, "doc-bot", stale_patch).unwrap_err();
    assert!(matches!(err, Error::PatchRejected(_)));
    assert!(err.to_string().contains("expected text mismatch"));
}

#[test]
fn lane_patch_replace_line_fuzzes_batch_expected_text_edits() {
    let temp = tempfile::tempdir().unwrap();
    let original = (1..=16)
        .map(|idx| format!("line-{idx:02}\n"))
        .collect::<String>();
    fs::write(temp.path().join("README.md"), original).unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("line-fuzz-bot", Some("main"), false, None, None)
        .unwrap();

    let mut expected_ids = Vec::new();
    let mut edits = Vec::new();
    for idx in 1..=16 {
        let why = db
            .why(
                &format!("README.md:{idx}"),
                Some("refs/lanes/line-fuzz-bot"),
            )
            .unwrap();
        let line_id = format!("{}:{}", why.line_id.origin_change.0, why.line_id.local_seq);
        expected_ids.push(why.line_id);
        edits.push(serde_json::json!({
            "op": "replace_line",
            "path": "README.md",
            "line_id": line_id,
            "expected_text": format!("line-{idx:02}"),
            "new_text": format!("changed-{idx:02}")
        }));
    }

    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "batch line-id fuzz",
        "edits": edits
    }))
    .unwrap();
    let applied = apply_lane_patch_at_head(&mut db, "line-fuzz-bot", patch).unwrap();
    assert_eq!(applied.changed_paths.len(), 1);

    for idx in 1..=16 {
        let changed = db
            .why(
                &format!("README.md:{idx}"),
                Some("refs/lanes/line-fuzz-bot"),
            )
            .unwrap();
        assert_eq!(changed.current_text, format!("changed-{idx:02}"));
        assert_eq!(changed.line_id, expected_ids[idx - 1]);
    }
}

#[test]
fn lane_patch_incrementally_handles_rename_delete_and_write() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    fs::write(temp.path().join("old.txt"), "remove me\n").unwrap();
    fs::create_dir_all(temp.path().join("pkg")).unwrap();
    for idx in 0..50 {
        fs::write(
            temp.path().join("pkg").join(format!("module_{idx:03}.rs")),
            format!("pub fn value_{idx}() -> usize {{ {idx} }}\n"),
        )
        .unwrap();
    }
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("patch-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "rename delete write",
        "edits": [
            {"op": "rename", "from": "README.md", "to": "docs/README.md"},
            {"op": "delete", "path": "old.txt"},
            {"op": "write", "path": "src/new.rs", "content": "pub fn new_file() {}\n"}
        ]
    }))
    .unwrap();
    let applied = apply_lane_patch_at_head(&mut db, "patch-bot", patch).unwrap();
    assert_eq!(applied.changed_paths.len(), 3);
    assert!(applied.changed_paths.iter().any(|path| {
        path.kind == crabdb::FileChangeKind::Renamed
            && path.old_path.as_deref() == Some("README.md")
            && path.path == "docs/README.md"
    }));
    assert!(applied
        .changed_paths
        .iter()
        .any(|path| { path.kind == crabdb::FileChangeKind::Deleted && path.path == "old.txt" }));
    assert!(applied
        .changed_paths
        .iter()
        .any(|path| { path.kind == crabdb::FileChangeKind::Added && path.path == "src/new.rs" }));

    let status = db.lane_status("patch-bot").unwrap();
    assert_eq!(status.changed_paths.len(), 3);
    assert!(status
        .changed_paths
        .iter()
        .any(|path| path.path == "docs/README.md"));
    let untouched = db
        .why("pkg/module_017.rs:1", Some("refs/lanes/patch-bot"))
        .unwrap();
    assert_eq!(untouched.current_text, "pub fn value_17() -> usize { 17 }");

    db.merge_lane("patch-bot", "main").unwrap();
    let renamed = db.why("docs/README.md:1", Some("main")).unwrap();
    assert_eq!(renamed.current_text, "hello");
    let written = db.why("src/new.rs:1", Some("main")).unwrap();
    assert_eq!(written.current_text, "pub fn new_file() {}");
    assert!(matches!(
        db.why("README.md:1", Some("main")).unwrap_err(),
        Error::InvalidInput(_)
    ));
    assert!(matches!(
        db.why("old.txt:1", Some("main")).unwrap_err(),
        Error::InvalidInput(_)
    ));
}

#[test]
fn lane_rewind_preserves_current_head_records_operation_and_syncs_workdir() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "base\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("rewind-bot", Some("main"), true, None, None)
        .unwrap();
    let base_change = spawned.base_change;
    let workdir = PathBuf::from(spawned.workdir.unwrap());
    fs::write(workdir.join("README.md"), "bad workdir\n").unwrap();
    drop(db);

    let cli = run_crabdb_json(
        temp.path(),
        &[
            "lane",
            "rewind",
            "rewind-bot",
            "--to",
            &base_change.0,
            "--record-current",
            "--sync-workdir",
        ],
    );
    assert_eq!(cli["target_change"], base_change.0);
    assert_eq!(cli["workdir_synced"], true);
    assert!(cli["recorded_current"].as_str().is_some());
    let preserved_branch = cli["preserved_branch"].as_str().unwrap().to_string();
    assert!(preserved_branch.starts_with("rewind/rewind-bot/"));

    let db = CrabDb::open(temp.path()).unwrap();
    let details = db.lane_details("rewind-bot").unwrap();
    assert_eq!(details.branch.head_change.0, cli["operation"]);
    assert_eq!(details.branch.head_root.0, cli["target_root"]);
    assert_eq!(
        fs::read_to_string(workdir.join("README.md")).unwrap(),
        "base\n"
    );

    let rewind_op = db.show(cli["operation"].as_str().unwrap()).unwrap();
    match rewind_op {
        ShowResult::Operation { value } => {
            assert!(matches!(
                value.operation.kind,
                crabdb::OperationKind::LaneRewind
            ));
            assert_eq!(value.operation.parents[0].0, cli["previous_change"]);
            assert_eq!(value.operation.after_root.0, cli["target_root"]);
        }
        other => panic!("expected rewind operation, got {other:?}"),
    }

    let preserved_ref = format!("refs/branches/{preserved_branch}");
    let preserved_line = db.why("README.md:1", Some(&preserved_ref)).unwrap();
    assert_eq!(preserved_line.current_text, "bad workdir");
    let rewound_line = db
        .why("README.md:1", Some("refs/lanes/rewind-bot"))
        .unwrap();
    assert_eq!(rewound_line.current_text, "base");

    let events = db
        .list_lane_events(Some("rewind-bot"), None, None, Some("lane_rewound"), 10)
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].change_id.as_ref().unwrap().0, cli["operation"]);
    assert_eq!(
        events[0].payload.as_ref().unwrap()["preserved_branch"],
        preserved_branch
    );
}

#[test]
fn lane_rewind_is_available_through_http_and_mcp() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "base\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("api-rewind", Some("main"), false, None, None)
        .unwrap();
    let base_change = db.lane_details("api-rewind").unwrap().branch.base_change;
    let bad_patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "bad api edit",
        "edits": [
            {"op": "write", "path": "README.md", "content": "bad api\n"}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "api-rewind", bad_patch).unwrap();

    let response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes/api-rewind/rewind",
            serde_json::json!({
                "to": base_change.0.clone(),
                "record_current": true
            }),
        ),
    );
    assert_eq!(response.status, 200);
    let http_report: LaneRewindReport = response.body_json().unwrap();
    assert_eq!(http_report.target_change, base_change);
    assert!(http_report.preserved_branch.is_some());
    assert_eq!(
        db.why("README.md:1", Some("refs/lanes/api-rewind"))
            .unwrap()
            .current_text,
        "base"
    );

    let second_bad_patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "bad mcp edit",
        "edits": [
            {"op": "write", "path": "README.md", "content": "bad mcp\n"}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "api-rewind", second_bad_patch).unwrap();
    let mcp = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "crabdb.lane_rewind",
                "arguments": {
                    "lane": "api-rewind",
                    "to": base_change.0.clone(),
                    "record_current": true
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp["result"]["isError"], false);
    assert_eq!(
        mcp["result"]["structuredContent"]["target_change"],
        base_change.0
    );
    assert!(mcp["result"]["structuredContent"]["preserved_branch"]
        .as_str()
        .unwrap()
        .starts_with("rewind/api-rewind/"));
    assert_eq!(
        db.why("README.md:1", Some("refs/lanes/api-rewind"))
            .unwrap()
            .current_text,
        "base"
    );
}

#[test]
fn lane_merge_combines_non_overlapping_text_line_edits() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "one\ntwo\nthree\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_str(
        r#"{
          "message": "lane edits line two",
          "edits": [
            {"op": "write", "path": "README.md", "content": "one\nlane two\nthree\n"}
          ]
        }"#,
    )
    .unwrap();
    apply_lane_patch_at_head(&mut db, "doc-bot", patch).unwrap();

    fs::write(temp.path().join("README.md"), "one\ntwo\nhuman three\n").unwrap();
    db.record(
        Some("main"),
        Some("human edits line three".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();

    let merged = db.merge_lane("doc-bot", "main").unwrap();
    assert_eq!(merged.changed_paths.len(), 1);
    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "one\nlane two\nhuman three\n"
    );

    let line_two = db.why("README.md:2", Some("main")).unwrap();
    assert_eq!(line_two.current_text, "lane two");
    let line_three = db.why("README.md:3", Some("main")).unwrap();
    assert_eq!(line_three.current_text, "human three");
}

#[test]
fn lane_management_commands_have_backing_apis() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane(
            "doc-bot",
            Some("main"),
            true,
            Some("openai".to_string()),
            Some("gpt-5".to_string()),
        )
        .unwrap();
    assert_eq!(db.list_lanes().unwrap().len(), 1);
    let details = db.lane_details("doc-bot").unwrap();
    assert_eq!(details.record.provider.as_deref(), Some("openai"));
    assert_eq!(details.branch.ref_name, spawned.ref_name);

    let message = db
        .add_lane_message(
            "doc-bot",
            "user",
            "Please improve the docs",
            Some("session-lane-management".to_string()),
        )
        .unwrap();
    assert_eq!(
        message.session_id.as_deref(),
        Some("session-lane-management")
    );

    let patch: PatchDocument = serde_json::from_str(
        r#"{
          "session_id": "session-lane-management",
          "message": "lane edits readme",
          "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nlane\n"}
          ]
        }"#,
    )
    .unwrap();
    let applied = apply_lane_patch_at_head(&mut db, "doc-bot", patch).unwrap();
    let status = db.lane_status("doc-bot").unwrap();
    assert_eq!(status.changed_paths.len(), 1);
    assert_eq!(
        status.lane.branch.session_id.as_deref(),
        Some("session-lane-management")
    );
    let timeline = db.lane_timeline("doc-bot", 10).unwrap();
    assert!(timeline
        .iter()
        .any(|entry| entry.change_id == applied.operation));
    let contribution = db.lane_contribution("doc-bot", 10).unwrap();
    assert_eq!(contribution.status.lane.record.name, "doc-bot");
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

    db.run_lane_test(
        "doc-bot",
        vec!["sh".to_string(), "-c".to_string(), "printf ok".to_string()],
        None,
        5,
    )
    .unwrap();
    db.run_lane_eval(
        "doc-bot",
        vec!["sh".to_string(), "-c".to_string(), "exit 3".to_string()],
        None,
        5,
    )
    .unwrap();
    db.request_lane_approval(
        "doc-bot",
        "shell.exec",
        "Run release smoke tests",
        None,
        Some("session-lane-management"),
        None,
    )
    .unwrap();

    let review = db.lane_review_packet("doc-bot", 1).unwrap();
    assert_eq!(review.lane.record.name, "doc-bot");
    assert!(!review.readiness.ready);
    assert!(review
        .readiness
        .blockers
        .iter()
        .any(|issue| issue.code == "pending_approvals"));
    assert!(review
        .readiness
        .blockers
        .iter()
        .any(|issue| issue.code == "latest_eval_failed"));
    assert!(review
        .changed_paths
        .iter()
        .any(|path| path.path == "README.md"));
    assert_eq!(review.evidence_summary.pending_approvals, 1);
    assert_eq!(
        review.evidence_summary.approvals,
        review.recent_approvals.len()
    );
    assert_eq!(review.evidence_summary.gates, review.recent_gates.len());
    assert!(review.recent_operations.len() <= 1);
    assert!(review.recent_events.len() <= 1);
    assert!(review.recent_spans.len() <= 1);
    assert!(review.recent_gates.len() <= 1);
    assert!(review.latest_test.as_ref().is_some_and(|gate| gate.success));
    assert!(review
        .latest_eval
        .as_ref()
        .is_some_and(|gate| !gate.success));
    assert!(review
        .next_steps
        .iter()
        .any(|step| step.contains("Resolve pending human approvals")));

    db.checkout_lane("doc-bot", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "hello\nlane\n"
    );
    let err = db.remove_lane("doc-bot", false).unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)));
    let removed = db.remove_lane("doc-bot", true).unwrap();
    assert_eq!(removed.lane_id, details.record.lane_id);
    assert!(!temp.path().join(".crabdb/refs/lanes/doc-bot").exists());
    if let Some(workdir) = removed.removed_workdir {
        assert!(!std::path::Path::new(&workdir).exists());
    }
    assert_eq!(db.lane_details("doc-bot").unwrap().branch.status, "removed");

    let conn = Connection::open(temp.path().join(".crabdb/index/crabdb.sqlite")).unwrap();
    let messages: i64 = conn
        .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
        .unwrap();
    let events: i64 = conn
        .query_row("SELECT COUNT(*) FROM lane_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(messages, 2);
    assert!(events >= 4);
}

#[test]
fn lane_test_runs_in_workdir_and_records_events_and_output_blobs() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("doc-bot", Some("main"), true, None, None)
        .unwrap();
    drop(db);

    let tested = run_crabdb_json(
        temp.path(),
        &[
            "lane",
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
    let turn = db.show_lane_turn(turn_id).unwrap();
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
        .run_lane_test(
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
        db.show_lane_turn(&failed.turn_id).unwrap().turn.status,
        "test_failed"
    );
    let latest_test = db.lane_status("doc-bot").unwrap().latest_test.unwrap();
    assert_eq!(latest_test.status, "test_failed");
    assert_eq!(latest_test.exit_code, Some(7));
    assert_eq!(latest_test.command, vec!["sh", "-c", "exit 7"]);

    drop(db);
    let evaled = run_crabdb_json(
        temp.path(),
        &[
            "lane",
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
        .show_lane_turn(evaled["turn_id"].as_str().unwrap())
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
            "/v1/lanes/doc-bot/evals",
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
        .show_lane_turn(api_eval["turn_id"].as_str().unwrap())
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
                    "lane": "doc-bot",
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

    let latest_eval = db.lane_status("doc-bot").unwrap().latest_eval.unwrap();
    assert_eq!(latest_eval.kind, "eval");
    assert_eq!(latest_eval.status, "eval_passed");
    assert_eq!(latest_eval.suite.as_deref(), Some("nightly"));
    assert_eq!(latest_eval.score, Some(0.91));
    assert_eq!(latest_eval.threshold, Some(0.9));
    assert_eq!(latest_eval.command, vec!["sh", "-c", "printf eval-ok"]);

    let gate_history = db.lane_gate_history("doc-bot", None, 10).unwrap();
    assert_eq!(gate_history.kind, "all");
    assert!(gate_history.gates.len() >= 5);
    assert_eq!(gate_history.gates[0].kind, "eval");
    assert_eq!(gate_history.gates[0].suite.as_deref(), Some("nightly"));
    assert!(gate_history
        .gates
        .iter()
        .any(|gate| gate.kind == "test" && gate.status == "test_failed"));
    let eval_history = db.lane_gate_history("doc-bot", Some("eval"), 2).unwrap();
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
            "/v1/lanes/doc-bot/gates?kind=eval&limit=2",
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
                    "lane": "doc-bot",
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
                "uri": "crabdb://workspace/lanes/doc-bot/gates"
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
        &["lane", "gates", "doc-bot", "--kind", "eval", "--limit", "2"],
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
fn lane_sessions_track_messages_patches_and_turns() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let started = db
        .start_lane_session(
            "doc-bot",
            Some("Improve docs".to_string()),
            Some("session-docs".to_string()),
        )
        .unwrap();
    assert_eq!(started.session.session_id, "session-docs");
    assert_eq!(started.session.status, "active");
    assert_eq!(
        db.lane_details("doc-bot")
            .unwrap()
            .branch
            .session_id
            .as_deref(),
        Some("session-docs")
    );

    let message = db
        .add_lane_message("doc-bot", "user", "Please improve README", None)
        .unwrap();
    assert_eq!(message.session_id.as_deref(), Some("session-docs"));

    let patch: PatchDocument = serde_json::from_str(
        r#"{
          "message": "lane edits readme",
          "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nsession\n"}
          ]
        }"#,
    )
    .unwrap();
    let applied = apply_lane_patch_at_head(&mut db, "doc-bot", patch).unwrap();
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
    let lane_timeline = db.timeline_query(None, None, Some("doc-bot"), 10).unwrap();
    assert!(lane_timeline
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

    let cli_lane_timeline = run_crabdb_json(
        temp.path(),
        &["timeline", "--lane", "doc-bot", "--limit", "5"],
    );
    assert!(cli_lane_timeline
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["change_id"] == applied.operation.0));

    let details = db.show_lane_session("session-docs").unwrap();
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

    let ended = db.end_lane_session("session-docs", "completed").unwrap();
    assert_eq!(ended.session.status, "completed");
    assert!(ended.session.ended_at.is_some());
    assert_eq!(
        db.lane_details("doc-bot")
            .unwrap()
            .branch
            .session_id
            .as_deref(),
        None
    );
}

#[test]
fn lane_workdir_record_advances_lane_branch() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("doc-bot", Some("main"), true, None, None)
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
        "lane notes\n",
    )
    .unwrap();

    let recorded = db
        .record_lane_workdir("doc-bot", Some("record workdir".to_string()))
        .unwrap();
    assert!(recorded.operation.is_some());
    assert_eq!(recorded.changed_paths.len(), 2);

    let clean = db.record_lane_workdir("doc-bot", None).unwrap();
    assert!(clean.operation.is_none());
    let timeline = db.lane_timeline("doc-bot", 10).unwrap();
    assert!(timeline
        .iter()
        .any(|entry| entry.kind == crabdb::OperationKind::LaneRecord));

    db.merge_lane("doc-bot", "main").unwrap();
    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "hello\nworkdir\n"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("docs/notes.md")).unwrap(),
        "lane notes\n"
    );
}

#[test]
fn lane_spawn_supports_custom_and_configured_workdirs() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(
        temp.path().join("src/lib.rs"),
        "#[path = \"../shared/helper.rs\"]\nmod helper;\npub fn answer() -> u8 { helper::answer() }\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("src/claimed.rs"),
        "pub fn claimed() -> bool { true }\n",
    )
    .unwrap();
    fs::create_dir_all(temp.path().join("shared")).unwrap();
    fs::write(
        temp.path().join("shared/helper.rs"),
        "pub fn answer() -> u8 { 42 }\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("shared/unrelated.rs"),
        "pub fn unrelated() -> bool { false }\n",
    )
    .unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let workdir_parent = tempfile::tempdir().unwrap();
    let default_spawn = run_crabdb_json(
        temp.path(),
        &["lane", "spawn", "default-bot", "--from", "main"],
    );
    assert!(default_spawn["workdir"].is_null());

    let cli_workdir = workdir_parent.path().join("cli-bot");
    let cli_spawn = run_crabdb_json(
        temp.path(),
        &[
            "lane",
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
            "lane",
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
            "lane",
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
        db.lane_status("cli-bot").unwrap().workdir_state,
        Some(WorktreeState::Clean)
    );

    let sparse_spawn = run_crabdb_json(
        temp.path(),
        &[
            "lane",
            "spawn",
            "sparse-bot",
            "--from",
            "main",
            "--paths",
            "README.md",
        ],
    );
    let sparse_workdir = PathBuf::from(sparse_spawn["workdir"].as_str().unwrap());
    assert!(sparse_workdir.join("README.md").is_file());
    assert!(!sparse_workdir.join("src/lib.rs").exists());
    assert!(!sparse_workdir.join("src/claimed.rs").exists());
    let sparse_clean = db.lane_status("sparse-bot").unwrap();
    assert_eq!(sparse_clean.workdir_state, Some(WorktreeState::Clean));
    assert!(sparse_clean.workdir_changed_paths.is_empty());
    let claimed_hydration = run_crabdb_json(
        temp.path(),
        &[
            "lane",
            "claim",
            "sparse-bot",
            "src/claimed.rs",
            "--ttl-secs",
            "120",
        ],
    );
    assert_eq!(claimed_hydration["claimed"], true);
    assert_eq!(
        claimed_hydration["hydrated_paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|path| path.as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["src/claimed.rs"]
    );
    assert_eq!(
        fs::read_to_string(sparse_workdir.join("src/claimed.rs")).unwrap(),
        "pub fn claimed() -> bool { true }\n"
    );
    assert!(!sparse_workdir.join("src/lib.rs").exists());
    let sparse_read = run_crabdb_json(
        temp.path(),
        &["lane", "read", "sparse-bot", "src/lib.rs", "--no-hydrate"],
    );
    assert_eq!(sparse_read["content_encoding"], "utf-8");
    assert_eq!(
        sparse_read["content"],
        "#[path = \"../shared/helper.rs\"]\nmod helper;\npub fn answer() -> u8 { helper::answer() }\n"
    );
    assert!(sparse_read["hydrated_paths"].is_null());
    assert!(!sparse_workdir.join("src/lib.rs").exists());
    let sparse_read_hydrate = run_crabdb_json(
        temp.path(),
        &[
            "lane",
            "read",
            "sparse-bot",
            "src/lib.rs",
            "--include-neighbors",
        ],
    );
    assert_eq!(
        sparse_read_hydrate["hydrated_paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|path| path.as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["shared/helper.rs", "src/claimed.rs", "src/lib.rs"]
    );
    assert_eq!(
        fs::read_to_string(sparse_workdir.join("src/lib.rs")).unwrap(),
        "#[path = \"../shared/helper.rs\"]\nmod helper;\npub fn answer() -> u8 { helper::answer() }\n"
    );
    assert_eq!(
        fs::read_to_string(sparse_workdir.join("shared/helper.rs")).unwrap(),
        "pub fn answer() -> u8 { 42 }\n"
    );
    assert!(!sparse_workdir.join("shared/unrelated.rs").exists());
    let sparse_dir_spawn = run_crabdb_json(
        temp.path(),
        &[
            "lane",
            "spawn",
            "sparse-dir-bot",
            "--from",
            "main",
            "--paths",
            "src",
        ],
    );
    let sparse_dir_workdir = PathBuf::from(sparse_dir_spawn["workdir"].as_str().unwrap());
    assert!(sparse_dir_workdir.join("src/lib.rs").is_file());
    assert!(!sparse_dir_workdir.join("README.md").exists());
    let sparse_dir_clean = db.lane_status("sparse-dir-bot").unwrap();
    assert_eq!(sparse_dir_clean.workdir_state, Some(WorktreeState::Clean));
    assert!(sparse_dir_clean.workdir_changed_paths.is_empty());
    fs::remove_file(sparse_dir_workdir.join("src/lib.rs")).unwrap();
    let sparse_dir_dirty = db.lane_status("sparse-dir-bot").unwrap();
    assert_eq!(
        sparse_dir_dirty.workdir_state,
        Some(WorktreeState::DirtyTracked)
    );
    assert_eq!(sparse_dir_dirty.workdir_changed_paths.len(), 1);
    assert_eq!(sparse_dir_dirty.workdir_changed_paths[0].path, "src/lib.rs");
    assert_eq!(
        sparse_dir_dirty.workdir_changed_paths[0].kind,
        crabdb::FileChangeKind::Deleted
    );
    let sparse_dir_record = db
        .record_lane_workdir(
            "sparse-dir-bot",
            Some("record sparse directory delete".to_string()),
        )
        .unwrap();
    assert!(sparse_dir_record.operation.is_some());
    assert_eq!(sparse_dir_record.changed_paths.len(), 1);
    assert_eq!(sparse_dir_record.changed_paths[0].path, "src/lib.rs");
    assert_eq!(
        sparse_dir_record.changed_paths[0].kind,
        crabdb::FileChangeKind::Deleted
    );
    assert_eq!(
        db.lane_status("sparse-dir-bot").unwrap().workdir_state,
        Some(WorktreeState::Clean)
    );
    let sparse_neighbor_spawn = run_crabdb_json(
        temp.path(),
        &[
            "lane",
            "spawn",
            "sparse-neighbor-bot",
            "--from",
            "main",
            "--paths",
            "src/lib.rs",
            "--include-neighbors",
        ],
    );
    let sparse_neighbor_workdir = PathBuf::from(sparse_neighbor_spawn["workdir"].as_str().unwrap());
    assert!(sparse_neighbor_workdir.join("src/lib.rs").is_file());
    assert!(sparse_neighbor_workdir.join("src/claimed.rs").is_file());
    assert!(sparse_neighbor_workdir.join("shared/helper.rs").is_file());
    assert!(!sparse_neighbor_workdir.join("shared/unrelated.rs").exists());
    assert!(!sparse_neighbor_workdir.join("README.md").exists());
    fs::write(
        sparse_neighbor_workdir.join("shared/helper.rs"),
        "pub fn answer() -> u8 { 7 }\n",
    )
    .unwrap();
    let dirty_neighbor = db
        .read_lane_file("sparse-neighbor-bot", "src/lib.rs", true, false, true)
        .unwrap_err();
    assert!(matches!(dirty_neighbor, Error::DirtyWorktreeWithMessage(_)));
    let forced_neighbor = db
        .read_lane_file("sparse-neighbor-bot", "src/lib.rs", true, true, true)
        .unwrap();
    assert_eq!(
        forced_neighbor.hydrated_paths,
        vec![
            "shared/helper.rs".to_string(),
            "src/claimed.rs".to_string(),
            "src/lib.rs".to_string()
        ]
    );
    assert_eq!(
        fs::read_to_string(sparse_neighbor_workdir.join("shared/helper.rs")).unwrap(),
        "pub fn answer() -> u8 { 42 }\n"
    );
    let hydrated = run_crabdb_json(
        temp.path(),
        &[
            "lane",
            "sync-workdir",
            "sparse-bot",
            "--paths",
            "src/lib.rs",
        ],
    );
    assert_eq!(hydrated["forced"], false);
    assert!(sparse_workdir.join("src/lib.rs").is_file());
    assert_eq!(
        fs::read_to_string(sparse_workdir.join("src/lib.rs")).unwrap(),
        "#[path = \"../shared/helper.rs\"]\nmod helper;\npub fn answer() -> u8 { helper::answer() }\n"
    );
    let sparse_hydrated = db.lane_status("sparse-bot").unwrap();
    assert_eq!(sparse_hydrated.workdir_state, Some(WorktreeState::Clean));
    assert!(sparse_hydrated.workdir_changed_paths.is_empty());
    #[cfg(unix)]
    let clean_hydrated_inode = fs::metadata(sparse_workdir.join("src/lib.rs"))
        .unwrap()
        .ino();
    let repeat_hydrate = db
        .sync_lane_workdir_with_paths("sparse-bot", false, &["src/lib.rs".to_string()])
        .unwrap();
    assert!(repeat_hydrate.changed_paths.is_empty());
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(sparse_workdir.join("src/lib.rs"))
            .unwrap()
            .ino(),
        clean_hydrated_inode
    );
    fs::write(sparse_workdir.join("src/lib.rs"), "pub fn dirty() {}\n").unwrap();
    let err = db
        .sync_lane_workdir_with_paths("sparse-bot", false, &["src/lib.rs".to_string()])
        .unwrap_err();
    assert!(matches!(err, Error::DirtyWorktreeWithMessage(_)));
    let forced_hydrate = db
        .sync_lane_workdir_with_paths("sparse-bot", true, &["src/lib.rs".to_string()])
        .unwrap();
    assert!(forced_hydrate.forced);
    assert_eq!(
        fs::read_to_string(sparse_workdir.join("src/lib.rs")).unwrap(),
        "#[path = \"../shared/helper.rs\"]\nmod helper;\npub fn answer() -> u8 { helper::answer() }\n"
    );
    fs::write(sparse_workdir.join("README.md"), "hello\nsparse\n").unwrap();
    let unrelated_sync = db
        .sync_lane_workdir_with_paths("sparse-bot", false, &["src/lib.rs".to_string()])
        .unwrap();
    assert!(!unrelated_sync.forced);
    assert!(unrelated_sync.changed_paths.is_empty());
    let sparse_dirty = db.lane_status("sparse-bot").unwrap();
    assert_eq!(
        sparse_dirty.workdir_state,
        Some(WorktreeState::DirtyTracked)
    );
    assert_eq!(sparse_dirty.workdir_changed_paths.len(), 1);
    assert_eq!(sparse_dirty.workdir_changed_paths[0].path, "README.md");
    let sparse_record = db
        .record_lane_workdir("sparse-bot", Some("record sparse workdir".to_string()))
        .unwrap();
    assert_eq!(sparse_record.changed_paths.len(), 1);
    assert_eq!(
        db.lane_status("sparse-bot").unwrap().workdir_state,
        Some(WorktreeState::Clean)
    );
    fs::remove_file(sparse_workdir.join(".crabdb/workdir-manifest.json")).unwrap();
    fs::write(sparse_workdir.join("README.md"), "hello\nstale-manifest\n").unwrap();
    let missing_manifest_sync = db
        .sync_lane_workdir_with_paths("sparse-bot", false, &["src/lib.rs".to_string()])
        .unwrap();
    assert!(missing_manifest_sync.changed_paths.is_empty());
    let still_dirty = db.lane_status("sparse-bot").unwrap();
    assert_eq!(still_dirty.workdir_state, Some(WorktreeState::DirtyTracked));
    assert!(still_dirty
        .workdir_changed_paths
        .iter()
        .any(|path| path.path == "README.md"));
    let cleanup = db
        .record_lane_workdir(
            "sparse-bot",
            Some("record missing manifest dirty".to_string()),
        )
        .unwrap();
    assert_eq!(cleanup.changed_paths.len(), 1);
    assert_eq!(cleanup.changed_paths[0].path, "README.md");

    let api_workdir = workdir_parent.path().join("api-bot");
    let api_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes",
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

    let api_sparse_workdir = workdir_parent.path().join("api-sparse-bot");
    let api_sparse_response = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes",
            serde_json::json!({
                "name": "api-sparse-bot",
                "from_ref": "main",
                "materialize": true,
                "workdir": api_sparse_workdir,
                "paths": ["README.md"]
            }),
        ),
    );
    assert_eq!(api_sparse_response.status, 201);
    let api_sparse_spawn: serde_json::Value = api_sparse_response.body_json().unwrap();
    assert!(api_sparse_workdir.join("README.md").is_file());
    assert!(!api_sparse_workdir.join("src/lib.rs").exists());
    let api_sparse_lane = api_sparse_spawn["lane_id"].as_str().unwrap();
    let api_sparse_read = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/lanes/{api_sparse_lane}/read-file"),
            serde_json::json!({ "path": "src/lib.rs", "hydrate": false }),
        ),
    );
    assert_eq!(api_sparse_read.status, 200);
    let api_sparse_read_body: serde_json::Value = api_sparse_read.body_json().unwrap();
    assert_eq!(
        api_sparse_read_body["content"],
        "#[path = \"../shared/helper.rs\"]\nmod helper;\npub fn answer() -> u8 { helper::answer() }\n"
    );
    assert!(!api_sparse_workdir.join("src/lib.rs").exists());
    let api_sparse_read_hydrate = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/lanes/{api_sparse_lane}/read-file"),
            serde_json::json!({
                "path": "src/lib.rs",
                "include_neighbors": true
            }),
        ),
    );
    assert_eq!(api_sparse_read_hydrate.status, 200);
    let api_sparse_read_hydrate_body: serde_json::Value =
        api_sparse_read_hydrate.body_json().unwrap();
    assert_eq!(
        api_sparse_read_hydrate_body["hydrated_paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|path| path.as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["shared/helper.rs", "src/claimed.rs", "src/lib.rs"]
    );
    assert_eq!(
        fs::read_to_string(api_sparse_workdir.join("src/lib.rs")).unwrap(),
        "#[path = \"../shared/helper.rs\"]\nmod helper;\npub fn answer() -> u8 { helper::answer() }\n"
    );
    assert_eq!(
        fs::read_to_string(api_sparse_workdir.join("shared/helper.rs")).unwrap(),
        "pub fn answer() -> u8 { 42 }\n"
    );
    assert!(!api_sparse_workdir.join("shared/unrelated.rs").exists());
    let api_sparse_sync = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/lanes/{api_sparse_lane}/sync-workdir"),
            serde_json::json!({ "paths": ["src/lib.rs"], "include_neighbors": true }),
        ),
    );
    assert_eq!(api_sparse_sync.status, 200);
    assert_eq!(
        fs::read_to_string(api_sparse_workdir.join("src/lib.rs")).unwrap(),
        "#[path = \"../shared/helper.rs\"]\nmod helper;\npub fn answer() -> u8 { helper::answer() }\n"
    );
    assert_eq!(
        fs::read_to_string(api_sparse_workdir.join("shared/helper.rs")).unwrap(),
        "pub fn answer() -> u8 { 42 }\n"
    );
    assert!(!api_sparse_workdir.join("shared/unrelated.rs").exists());
    assert_eq!(
        fs::read_to_string(api_sparse_workdir.join("src/claimed.rs")).unwrap(),
        "pub fn claimed() -> bool { true }\n"
    );

    let mcp_workdir = workdir_parent.path().join("mcp-bot");
    let mcp_spawn = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "crabdb.lane_spawn",
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
    let mcp_read = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "crabdb.read_file",
                "arguments": {
                    "lane": "mcp-bot",
                    "path": "README.md"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_read["result"]["isError"], false);
    assert_eq!(
        mcp_read["result"]["structuredContent"]["content"],
        "hello\n"
    );

    db.config_set("lane.worktrees_dir", ".crabdb/custom-worktrees")
        .unwrap();
    let configured = db
        .spawn_lane("configured-bot", Some("main"), true, None, None)
        .unwrap();
    let configured_workdir = PathBuf::from(configured.workdir.unwrap());
    assert!(configured_workdir.ends_with(".crabdb/custom-worktrees/configured-bot"));
    assert!(configured_workdir.join("README.md").is_file());

    let nonempty = workdir_parent.path().join("nonempty");
    fs::create_dir_all(&nonempty).unwrap();
    fs::write(nonempty.join("keep.txt"), "do not delete\n").unwrap();
    let err = db
        .spawn_lane_with_workdir(
            "nonempty-bot",
            Some("main"),
            true,
            None,
            None,
            Some(nonempty.clone()),
        )
        .unwrap_err();
    assert!(err.to_string().contains("must be empty or absent"));
    assert!(db.lane_details("nonempty-bot").is_err());
    assert_eq!(
        fs::read_to_string(nonempty.join("keep.txt")).unwrap(),
        "do not delete\n"
    );

    let unsafe_inside_workspace = temp.path().join("unsafe-lane-workdir");
    let err = db
        .spawn_lane_with_workdir(
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
        .contains("lane workdirs inside the workspace must live under"));

    let err = db
        .spawn_lane_with_workdir(
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
        .contains("custom lane workdir requires materialization"));
}

#[test]
fn large_roots_default_lanes_to_no_materialize() {
    let temp = tempfile::tempdir().unwrap();
    let files_dir = temp.path().join("files");
    fs::create_dir_all(&files_dir).unwrap();
    for idx in 0..=10_000 {
        fs::write(files_dir.join(format!("file-{idx:05}.txt")), "tiny\n").unwrap();
    }
    let init = CrabDb::init_with_text_policy(
        temp.path(),
        "main",
        InitImportMode::WorkingTree,
        false,
        Some("minimal"),
    )
    .unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    match db.show(&init.operation.0).unwrap() {
        ShowResult::Operation { value } => {
            assert!(value.operation.changes.is_empty());
        }
        other => panic!("expected operation show result, got {other:?}"),
    }
    let conn = Connection::open(temp.path().join(".crabdb/index/crabdb.sqlite")).unwrap();
    let file_history_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM file_history", [], |row| row.get(0))
        .unwrap();
    assert_eq!(file_history_rows, 0);
    let file_history = db
        .history_for_path("files/file-00000.txt")
        .unwrap()
        .file_history;
    assert_eq!(file_history.len(), 1);
    assert_eq!(file_history[0].kind, crabdb::FileChangeKind::Added);
    assert_eq!(file_history[0].change_id, init.operation);

    db.config_set("lane.default_materialize", "true").unwrap();
    assert!(db.default_lane_materialize());
    assert!(!db.default_lane_materialize_for_ref(Some("main")).unwrap());

    let materialize = db.default_lane_materialize_for_ref(Some("main")).unwrap();
    let report = db
        .spawn_lane("large-default-bot", Some("main"), materialize, None, None)
        .unwrap();
    assert!(report.workdir.is_none());

    db.begin_lane_turn(
        "large-turn-bot",
        Some("main"),
        Some("large turn".to_string()),
        None,
    )
    .unwrap();
    assert!(db
        .lane_details("large-turn-bot")
        .unwrap()
        .branch
        .workdir
        .is_none());
}

#[test]
fn lane_spawn_materialization_ignores_dirty_workspace_for_recorded_root() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    fs::write(temp.path().join("README.md"), "hello\ndirty\n").unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("doc-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = PathBuf::from(spawned.workdir.unwrap());

    assert_eq!(
        fs::read_to_string(workdir.join("README.md")).unwrap(),
        "hello\n"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "hello\ndirty\n"
    );
}

#[test]
fn lane_workdir_sync_refuses_dirty_and_force_refreshes() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("doc-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = spawned.workdir.unwrap();
    let readme = std::path::Path::new(&workdir).join("README.md");
    fs::write(&readme, "hello\ndirty\n").unwrap();

    let err = db.sync_lane_workdir("doc-bot", false).unwrap_err();
    assert!(matches!(err, Error::DirtyWorktreeWithMessage(_)));
    drop(db);

    let synced = run_crabdb_json(temp.path(), &["lane", "sync-workdir", "doc-bot", "--force"]);
    assert_eq!(synced["forced"], true);
    assert!(synced["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "README.md"));
    let rescue = PathBuf::from(synced["rescue_workdir"].as_str().unwrap());
    assert!(rescue.is_dir());
    assert_eq!(
        fs::read_to_string(rescue.join("files").join("README.md")).unwrap(),
        "hello\ndirty\n"
    );
    let rescue_manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(rescue.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(rescue_manifest["lane"], "doc-bot");
    assert!(rescue_manifest["copied_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path == "README.md"));
    assert_eq!(fs::read_to_string(&readme).unwrap(), "hello\n");

    let db = CrabDb::open(temp.path()).unwrap();
    let status = db.lane_status("doc-bot").unwrap();
    assert_eq!(status.workdir_state, Some(WorktreeState::Clean));
    drop(db);

    #[cfg(unix)]
    {
        let inode_before = fs::metadata(&readme).unwrap().ino();
        let clean_sync = run_crabdb_json(temp.path(), &["lane", "sync-workdir", "doc-bot"]);
        assert_eq!(clean_sync["forced"], false);
        assert!(clean_sync.get("rescue_workdir").is_none());
        assert!(clean_sync["changed_paths"].as_array().unwrap().is_empty());
        assert_eq!(fs::metadata(&readme).unwrap().ino(), inode_before);
    }

    fs::remove_dir_all(&workdir).unwrap();
    let recreated = run_crabdb_json(temp.path(), &["lane", "sync-workdir", "doc-bot"]);
    assert_eq!(recreated["forced"], false);
    assert_eq!(fs::read_to_string(&readme).unwrap(), "hello\n");
}

#[test]
fn lane_workdir_watch_records_only_lane_branch() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("doc-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = spawned.workdir.unwrap();
    let workdir_report = db.lane_workdir("doc-bot").unwrap();
    assert_eq!(workdir_report.workdir.as_deref(), Some(workdir.as_str()));

    fs::write(
        std::path::Path::new(&workdir).join("README.md"),
        "hello\nwatched\n",
    )
    .unwrap();
    let watched = db
        .watch_lane_workdir(
            "doc-bot",
            Some("watch workdir".to_string()),
            std::time::Duration::from_millis(0),
            Some(1),
        )
        .unwrap();
    assert_eq!(watched.iterations, 1);
    assert_eq!(watched.recorded_operations.len(), 1);
    assert_eq!(watched.changed_paths.len(), 1);

    let lane_status = db.lane_status("doc-bot").unwrap();
    assert_eq!(lane_status.workdir_state, Some(WorktreeState::Clean));
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "hello\n"
    );
    let main_status = db.status(Some("main")).unwrap();
    assert_eq!(main_status.worktree_state, WorktreeState::Clean);
}

#[test]
fn dirty_lane_workdir_must_be_recorded_before_merge() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("doc-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = spawned.workdir.unwrap();
    fs::write(
        std::path::Path::new(&workdir).join("README.md"),
        "hello\nunrecorded\n",
    )
    .unwrap();
    let status = db.lane_status("doc-bot").unwrap();
    assert_eq!(status.workdir_state, Some(WorktreeState::DirtyTracked));
    assert_eq!(status.workdir_changed_paths.len(), 1);

    let err = db.merge_lane("doc-bot", "main").unwrap_err();
    assert!(matches!(err, Error::DirtyWorktreeWithMessage(_)));
    assert!(err.to_string().contains("lane record doc-bot"));

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
        .record_lane_workdir("doc-bot", Some("record before merge".to_string()))
        .unwrap();
    assert!(recorded.operation.is_some());
    let clean_status = db.lane_status("doc-bot").unwrap();
    assert_eq!(clean_status.workdir_state, Some(WorktreeState::Clean));
    assert!(clean_status.workdir_changed_paths.is_empty());
    let merged = db.merge_lane("doc-bot", "main").unwrap();
    assert_eq!(merged.changed_paths.len(), 1);
    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "hello\nunrecorded\n"
    );
}

#[test]
fn materialized_lane_status_detects_manifest_candidate_paths() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    fs::write(temp.path().join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("doc-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = PathBuf::from(spawned.workdir.unwrap());
    let readme_metadata = fs::symlink_metadata(workdir.join("README.md")).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(workdir.join(".crabdb/workdir-manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        manifest["files"]["README.md"]["stamp"]["device_id"].as_i64(),
        Some(readme_metadata.dev().min(i64::MAX as u64) as i64)
    );
    assert_eq!(
        manifest["files"]["README.md"]["stamp"]["inode"].as_i64(),
        Some(readme_metadata.ino().min(i64::MAX as u64) as i64)
    );
    fs::write(workdir.join("README.md"), "hello\nchanged\n").unwrap();
    fs::remove_file(workdir.join("src/lib.rs")).unwrap();

    let status = db.lane_status("doc-bot").unwrap();
    assert_eq!(status.workdir_state, Some(WorktreeState::DirtyTracked));
    let changed = status
        .workdir_changed_paths
        .iter()
        .map(|path| (path.path.as_str(), path.kind.clone()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        changed.get("README.md"),
        Some(&crabdb::FileChangeKind::Modified)
    );
    assert_eq!(
        changed.get("src/lib.rs"),
        Some(&crabdb::FileChangeKind::Deleted)
    );

    let recorded = db
        .record_lane_workdir("doc-bot", Some("record candidate paths".to_string()))
        .unwrap();
    assert!(recorded.operation.is_some());
    let recorded_paths = recorded
        .changed_paths
        .iter()
        .map(|path| (path.path.as_str(), path.kind.clone()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        recorded_paths.get("README.md"),
        Some(&crabdb::FileChangeKind::Modified)
    );
    assert_eq!(
        recorded_paths.get("src/lib.rs"),
        Some(&crabdb::FileChangeKind::Deleted)
    );
    assert_eq!(
        db.lane_status("doc-bot").unwrap().workdir_state,
        Some(WorktreeState::Clean)
    );
}

#[test]
fn materialized_lane_status_and_record_without_clean_manifest() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    fs::write(temp.path().join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("doc-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = PathBuf::from(spawned.workdir.unwrap());
    let manifest = workdir.join(".crabdb/workdir-manifest.json");
    fs::remove_file(&manifest).unwrap();
    fs::write(workdir.join("README.md"), "hello\nchanged\n").unwrap();
    fs::remove_file(workdir.join("src/lib.rs")).unwrap();
    fs::write(workdir.join("src/new.rs"), "pub fn new() {}\n").unwrap();

    let status = db.lane_status("doc-bot").unwrap();
    assert_eq!(status.workdir_state, Some(WorktreeState::DirtyUntracked));
    let changed = status
        .workdir_changed_paths
        .iter()
        .map(|path| (path.path.as_str(), path.kind.clone()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        changed.get("README.md"),
        Some(&crabdb::FileChangeKind::Modified)
    );
    assert_eq!(
        changed.get("src/lib.rs"),
        Some(&crabdb::FileChangeKind::Deleted)
    );
    assert_eq!(
        changed.get("src/new.rs"),
        Some(&crabdb::FileChangeKind::Added)
    );

    let recorded = db
        .record_lane_workdir("doc-bot", Some("record missing manifest".to_string()))
        .unwrap();
    assert!(recorded.operation.is_some());
    let recorded_paths = recorded
        .changed_paths
        .iter()
        .map(|path| (path.path.as_str(), path.kind.clone()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        recorded_paths.get("README.md"),
        Some(&crabdb::FileChangeKind::Modified)
    );
    assert_eq!(
        recorded_paths.get("src/lib.rs"),
        Some(&crabdb::FileChangeKind::Deleted)
    );
    assert_eq!(
        recorded_paths.get("src/new.rs"),
        Some(&crabdb::FileChangeKind::Added)
    );
    assert!(manifest.exists());
    assert_eq!(
        db.lane_status("doc-bot").unwrap().workdir_state,
        Some(WorktreeState::Clean)
    );
}

#[test]
fn sparse_lane_path_enforcement_blocks_patch_and_record_outside_selected_paths() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    fs::write(temp.path().join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.config_set("lane.enforce_sparse_paths", "true").unwrap();
    let spawned = db
        .spawn_lane_with_workdir_paths(
            "sparse-bot",
            Some("main"),
            true,
            None,
            None,
            None,
            &["README.md".to_string()],
        )
        .unwrap();
    let workdir = PathBuf::from(spawned.workdir.unwrap());

    let allowed_patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nallowed\n"}
        ]
    }))
    .unwrap();
    let report = apply_lane_patch_at_head(&mut db, "sparse-bot", allowed_patch).unwrap();
    assert_eq!(report.changed_paths[0].path, "README.md");

    let blocked_patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write", "path": "src/lib.rs", "content": "pub fn changed() {}\n"}
        ]
    }))
    .unwrap();
    let err = apply_lane_patch_at_head(&mut db, "sparse-bot", blocked_patch).unwrap_err();
    assert!(
        matches!(err, Error::PatchRejected(message) if message.contains("sparse path boundary"))
    );

    fs::write(workdir.join("EXTRA.md"), "outside sparse selection\n").unwrap();
    let err = db
        .record_lane_workdir("sparse-bot", Some("record outside sparse".to_string()))
        .unwrap_err();
    assert!(
        matches!(err, Error::PatchRejected(message) if message.contains("sparse path boundary"))
    );
}

#[test]
fn advisory_leases_coordinate_lane_paths() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("doc-bot", Some("main"), false, None, None)
        .unwrap();
    db.spawn_lane("test-bot", Some("main"), false, None, None)
        .unwrap();

    let claim = db.claim_lane_path("doc-bot", "README.md", 600).unwrap();
    assert!(claim.claimed);
    assert_eq!(claim.path, "README.md");
    assert_eq!(claim.mode, "write");
    let lease = claim.lease.as_ref().unwrap();
    assert_eq!(lease.mode, "write");
    assert_eq!(lease.path.as_deref(), Some("README.md"));
    assert!(lease.file_id.is_some());

    let conflicting_claim = db.claim_lane_path("test-bot", "README.md", 600).unwrap();
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
fn claim_enforcement_can_reject_or_warn_on_unclaimed_lane_paths() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    fs::write(temp.path().join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("claim-bot", Some("main"), false, None, None)
        .unwrap();
    db.claim_lane_path("claim-bot", "README.md", 600).unwrap();
    db.config_set("lane.claim_enforcement", "reject").unwrap();

    let claimed_patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nclaimed\n"}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "claim-bot", claimed_patch).unwrap();

    let unclaimed_patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write", "path": "src/lib.rs", "content": "pub fn outside_claim() {}\n"}
        ]
    }))
    .unwrap();
    let err = apply_lane_patch_at_head(&mut db, "claim-bot", unclaimed_patch.clone()).unwrap_err();
    assert!(
        matches!(err, Error::PatchRejected(message) if message.contains("outside active write claims"))
    );

    db.config_set("lane.claim_enforcement", "warn").unwrap();
    apply_lane_patch_at_head(&mut db, "claim-bot", unclaimed_patch).unwrap();
    let warnings = db
        .list_lane_events(
            Some("claim-bot"),
            None,
            None,
            Some("lane_policy_warning"),
            10,
        )
        .unwrap();
    assert_eq!(warnings.len(), 1);
    assert_eq!(
        warnings[0].payload.as_ref().unwrap()["code"],
        "unclaimed_paths"
    );
}

#[test]
fn lane_event_and_trace_payload_limits_are_enforced() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("payload-bot", Some("main"), false, None, None)
        .unwrap();
    let session = db
        .start_lane_session(
            "payload-bot",
            Some("Payload limits".to_string()),
            Some("session-payload".to_string()),
        )
        .unwrap();

    db.config_set("lane.max_event_payload_bytes", "32").unwrap();
    let err = db
        .add_lane_session_event(
            "payload-bot",
            &session.session.session_id,
            "large_payload",
            Some(serde_json::json!({
                "long": "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz"
            })),
        )
        .unwrap_err();
    assert!(
        matches!(err, Error::InvalidInput(message) if message.contains("max_event_payload_bytes"))
    );

    db.config_set("lane.max_event_payload_bytes", "0").unwrap();
    let turn = db
        .begin_lane_session_turn("payload-bot", &session.session.session_id, None)
        .unwrap();
    db.config_set("lane.max_trace_payload_bytes", "16").unwrap();
    let err = db
        .start_lane_trace_span(
            &turn.turn.turn_id,
            "tool",
            "large trace payload",
            None,
            None,
            Some(serde_json::json!({
                "long": "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz"
            })),
        )
        .unwrap_err();
    assert!(
        matches!(err, Error::InvalidInput(message) if message.contains("max_trace_payload_bytes"))
    );
}

#[test]
fn lane_claims_are_soft_leases_across_cli_api_and_mcp() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("doc-bot", Some("main"), false, None, None)
        .unwrap();
    db.spawn_lane("test-bot", Some("main"), false, None, None)
        .unwrap();
    drop(db);

    let cli_claim = run_crabdb_json(
        temp.path(),
        &["lane", "claim", "doc-bot", "README.md", "--ttl-secs", "120"],
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
            "/v1/lanes/test-bot/claims",
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
        .any(|tool| tool["name"] == "crabdb.lane_claim"));

    let mcp_conflict = crabdb::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "crabdb.lane_claim",
                "arguments": {
                    "lane": "test-bot",
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
                "name": "crabdb.lane_claim",
                "arguments": {
                    "lane": "doc-bot",
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
    db.spawn_lane("doc-bot", Some("main"), false, None, None)
        .unwrap();
    db.spawn_lane("test-bot", Some("main"), false, None, None)
        .unwrap();

    let acquired = crabdb::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/leases",
            serde_json::json!({
                "lane": "doc-bot",
                "path": "README.md",
                "mode": "write",
                "ttl_secs": 120
            }),
        ),
    );
    assert_eq!(acquired.status, 201);
    let acquired: serde_json::Value = acquired.body_json().unwrap();
    assert_eq!(acquired["lease"]["ref_name"], "refs/lanes/doc-bot");
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
                    "lane": "test-bot",
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
                    "lane": "test-bot",
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
fn merge_lane_and_queue_enforce_readiness_blockers() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("approval-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "approval gated edit",
        "edits": [
            {"op": "write", "path": "docs/approval.md", "content": "needs approval\n"}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "approval-bot", patch).unwrap();
    db.request_lane_approval(
        "approval-bot",
        "deploy.preview",
        "Publish preview before merge",
        None,
        None,
        None,
    )
    .unwrap();

    let dry_run = db
        .merge_lane_with_options("approval-bot", "main", true)
        .unwrap();
    assert_eq!(dry_run.changed_paths.len(), 1);
    let direct_err = db.merge_lane("approval-bot", "main").unwrap_err();
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

    db.spawn_lane("test-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = db.lane_workdir("test-bot").unwrap().workdir.unwrap();
    fs::create_dir_all(std::path::Path::new(&workdir).join("docs")).unwrap();
    fs::write(
        std::path::Path::new(&workdir).join("docs/test.md"),
        "needs tests\n",
    )
    .unwrap();
    db.record_lane_workdir("test-bot", Some("test gated edit".to_string()))
        .unwrap();
    let failed = db
        .run_lane_test(
            "test-bot",
            vec!["sh".to_string(), "-c".to_string(), "exit 7".to_string()],
            None,
            30,
        )
        .unwrap();
    assert!(!failed.success);
    let readiness = db.lane_readiness("test-bot").unwrap();
    assert!(readiness
        .blockers
        .iter()
        .any(|issue| issue.code == "latest_test_failed"));
    let test_err = db.merge_lane("test-bot", "main").unwrap_err();
    assert!(matches!(test_err, Error::InvalidInput(_)));
    assert!(test_err.to_string().contains("latest_test_failed"));
}

#[test]
fn lane_readiness_warns_when_lane_base_lags_default_branch() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("stale-bot", Some("main"), false, None, None)
        .unwrap();
    fs::write(temp.path().join("README.md"), "hello\nmain advanced\n").unwrap();
    db.record(
        Some("main"),
        Some("advance main".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();

    let readiness = db.lane_readiness("stale-bot").unwrap();
    let stale = readiness
        .warnings
        .iter()
        .find(|issue| issue.code == "stale_lane_base")
        .expect("expected stale lane base warning");
    assert!(stale.message.contains("1 operation behind `main`"));
    assert_eq!(stale.details.as_ref().unwrap()["operations_behind"], 1);
}

#[test]
fn required_gate_config_blocks_merge_until_test_and_eval_pass() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("strict-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = db.lane_workdir("strict-bot").unwrap().workdir.unwrap();
    fs::create_dir_all(Path::new(&workdir).join("docs")).unwrap();
    fs::write(Path::new(&workdir).join("docs/strict.md"), "strict gates\n").unwrap();
    db.record_lane_workdir("strict-bot", Some("strict gated edit".to_string()))
        .unwrap();

    db.config_set("lane.require_test_gate", "true").unwrap();
    db.config_set("lane.required_test_suites", "unit").unwrap();
    let readiness = db.lane_readiness("strict-bot").unwrap();
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
        .merge_lane_with_options("strict-bot", "main", true)
        .unwrap();
    assert_eq!(dry_run.changed_paths.len(), 1);
    let missing_test = db.merge_lane("strict-bot", "main").unwrap_err();
    assert!(matches!(missing_test, Error::InvalidInput(_)));
    assert!(missing_test.to_string().contains("missing_latest_test"));
    assert!(!temp.path().join("docs/strict.md").exists());

    let passed_test = db
        .run_lane_test_with_options(
            "strict-bot",
            vec!["sh".to_string(), "-c".to_string(), "exit 0".to_string()],
            None,
            30,
            LaneGateOptions {
                suite: Some("unit".to_string()),
                score: None,
                threshold: None,
            },
        )
        .unwrap();
    assert!(passed_test.success);
    assert_eq!(passed_test.suite.as_deref(), Some("unit"));
    let readiness = db.lane_readiness("strict-bot").unwrap();
    assert!(readiness.ready);
    assert!(readiness
        .warnings
        .iter()
        .any(|issue| issue.code == "missing_latest_eval"));

    db.config_set("lane.require_eval_gate", "true").unwrap();
    db.config_set("lane.required_eval_suites", "policy-smoke")
        .unwrap();
    let readiness = db.lane_readiness("strict-bot").unwrap();
    assert_eq!(readiness.status, "blocked");
    assert!(readiness
        .blockers
        .iter()
        .any(|issue| issue.code == "missing_latest_eval"));
    assert!(readiness
        .blockers
        .iter()
        .any(|issue| issue.code == "missing_required_eval_suite"));
    let missing_eval = db.merge_lane("strict-bot", "main").unwrap_err();
    assert!(matches!(missing_eval, Error::InvalidInput(_)));
    assert!(missing_eval.to_string().contains("missing_latest_eval"));

    let failed_eval = db
        .run_lane_eval_with_options(
            "strict-bot",
            vec!["sh".to_string(), "-c".to_string(), "exit 0".to_string()],
            None,
            30,
            LaneGateOptions {
                suite: Some("policy-smoke".to_string()),
                score: Some(0.4),
                threshold: Some(0.9),
            },
        )
        .unwrap();
    assert!(!failed_eval.success);
    assert_eq!(failed_eval.suite.as_deref(), Some("policy-smoke"));
    let readiness = db.lane_readiness("strict-bot").unwrap();
    assert!(readiness
        .blockers
        .iter()
        .any(|issue| issue.code == "required_eval_suite_failed"));
    let failed_suite = db.merge_lane("strict-bot", "main").unwrap_err();
    assert!(matches!(failed_suite, Error::InvalidInput(_)));
    assert!(failed_suite
        .to_string()
        .contains("required_eval_suite_failed"));

    let passed_eval = db
        .run_lane_eval_with_options(
            "strict-bot",
            vec!["sh".to_string(), "-c".to_string(), "exit 0".to_string()],
            None,
            30,
            LaneGateOptions {
                suite: Some("policy-smoke".to_string()),
                score: Some(0.95),
                threshold: Some(0.9),
            },
        )
        .unwrap();
    assert!(passed_eval.success);
    assert_eq!(passed_eval.suite.as_deref(), Some("policy-smoke"));
    let readiness = db.lane_readiness("strict-bot").unwrap();
    assert!(readiness.ready);
    assert!(readiness.blockers.is_empty());

    db.merge_lane("strict-bot", "main").unwrap();
    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("docs/strict.md")).unwrap(),
        "strict gates\n"
    );
}

#[test]
fn merge_queue_runs_lane_branch_into_main() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_str(
        r##"{
          "message": "lane adds docs",
          "edits": [
            {"op": "write", "path": "docs/guide.md", "content": "# Guide\n"}
          ]
        }"##,
    )
    .unwrap();
    apply_lane_patch_at_head(&mut db, "doc-bot", patch).unwrap();

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
fn merge_queue_explain_reports_dry_run_conflicts_without_recording_conflict_state() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("explain-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nlane\n"}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "explain-bot", patch).unwrap();
    fs::write(temp.path().join("README.md"), "hello\nmain\n").unwrap();
    db.record(
        Some("main"),
        Some("main edit".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();
    db.enqueue_merge("explain-bot", "main", 0).unwrap();

    let explain = db.explain_merge_queue("explain-bot").unwrap();
    assert!(explain
        .blockers
        .iter()
        .any(|issue| issue.code == "merge_conflicts"));
    assert_eq!(
        explain.dry_run.as_ref().unwrap().conflicts,
        vec!["both changed `README.md` differently"]
    );
    assert!(db.list_conflicts().unwrap().is_empty());
    drop(db);

    let cli = run_crabdb_json(temp.path(), &["merge-queue", "explain", "explain-bot"]);
    assert_eq!(cli["entry"]["source_ref"], "refs/lanes/explain-bot");
    assert!(cli["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue["code"] == "merge_conflicts"));
    assert_eq!(
        cli["dry_run"]["conflicts"][0],
        "both changed `README.md` differently"
    );
}

#[test]
fn merge_queue_pauses_on_conflict() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_str(
        r#"{
          "message": "lane edits readme",
          "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nlane\n"}
          ]
        }"#,
    )
    .unwrap();
    apply_lane_patch_at_head(&mut db, "doc-bot", patch).unwrap();

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
    let explanation = shown.explanation.as_ref().unwrap();
    assert_eq!(explanation.merge.source_ref, "refs/lanes/doc-bot");
    assert_eq!(explanation.merge.target_ref, "refs/branches/main");
    assert!(explanation.merge.base_root.is_some());
    assert!(explanation.merge.target_root.is_some());
    assert!(explanation.merge.source_root.is_some());
    assert_eq!(explanation.paths.len(), 1);
    assert_eq!(explanation.paths[0].path, "README.md");
    assert_eq!(explanation.paths[0].conflict_class, "modify/modify");
    assert_eq!(explanation.paths[0].recommendation.resolution, "manual");
    assert_eq!(explanation.paths[0].recommendation.confidence, "high");
    assert_eq!(explanation.paths[0].lines.len(), 1);
    assert_eq!(
        explanation.paths[0].lines[0].target.as_deref(),
        Some("human\n")
    );
    assert_eq!(
        explanation.paths[0].lines[0].source.as_deref(),
        Some("lane\n")
    );
    assert_eq!(
        explanation.paths[0].lines[0]
            .target_change
            .as_ref()
            .unwrap()
            .message
            .as_deref(),
        Some("human edit")
    );

    let resolved = db
        .resolve_conflict(&conflicts[0].conflict_set_id, "source")
        .unwrap();
    assert_eq!(resolved.resolution, "source");
    assert_eq!(resolved.changed_paths.len(), 1);
    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "hello\nlane\n"
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
fn conflict_explanations_classify_common_non_line_conflicts() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "base\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("delete-bot", Some("main"), false, None, None)
        .unwrap();
    let delete_patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "delete", "path": "README.md"}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "delete-bot", delete_patch).unwrap();
    fs::write(temp.path().join("README.md"), "target changed\n").unwrap();
    db.record(
        Some("main"),
        Some("target modifies deleted file".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();
    assert!(matches!(
        db.merge_lane("delete-bot", "main").unwrap_err(),
        Error::Conflict(_)
    ));
    assert_eq!(
        only_conflict_path_class(&db),
        ("README.md".to_string(), "delete/modify".to_string())
    );

    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("asset.bin"), [0_u8, 1, 2]).unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("binary-bot", Some("main"), false, None, None)
        .unwrap();
    let binary_patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write_bytes", "path": "asset.bin", "bytes_hex": "0003ff"}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "binary-bot", binary_patch).unwrap();
    fs::write(temp.path().join("asset.bin"), [0_u8, 4, 5]).unwrap();
    db.record(
        Some("main"),
        Some("target binary edit".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();
    assert!(matches!(
        db.merge_lane("binary-bot", "main").unwrap_err(),
        Error::Conflict(_)
    ));
    assert_eq!(
        only_conflict_path_class(&db),
        ("asset.bin".to_string(), "binary".to_string())
    );

    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("script.sh"), "echo base\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("mode-bot", Some("main"), false, None, None)
        .unwrap();
    let mode_patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write", "path": "script.sh", "content": "echo base\n", "executable": true}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "mode-bot", mode_patch).unwrap();
    fs::write(temp.path().join("script.sh"), "echo target\n").unwrap();
    db.record(
        Some("main"),
        Some("target content edit".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();
    assert!(matches!(
        db.merge_lane("mode-bot", "main").unwrap_err(),
        Error::Conflict(_)
    ));
    assert_eq!(
        only_conflict_path_class(&db),
        ("script.sh".to_string(), "mode".to_string())
    );

    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "a\nb\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("gap-bot", Some("main"), false, None, None)
        .unwrap();
    let gap_patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write", "path": "README.md", "content": "a\nlane\nb\n"}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "gap-bot", gap_patch).unwrap();
    fs::write(temp.path().join("README.md"), "a\ntarget\nb\n").unwrap();
    db.record(
        Some("main"),
        Some("target insertion".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();
    assert!(matches!(
        db.merge_lane("gap-bot", "main").unwrap_err(),
        Error::Conflict(_)
    ));
    assert_eq!(
        only_conflict_path_class(&db),
        ("README.md".to_string(), "same_insertion_gap".to_string())
    );
}

#[test]
fn manual_conflict_resolution_works_through_db_cli_http_and_mcp() {
    let (temp, mut db, conflict_id) =
        conflicted_readme_workspace("hello\nlane-db\n", "hello\nhuman-db\n");
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
        conflicted_readme_workspace("hello\nlane-cli\n", "hello\nhuman-cli\n");
    drop(db);
    let shown = run_crabdb_json(
        temp.path(),
        &["conflicts", "show", &conflict_id, "--limit", "1"],
    );
    assert_eq!(shown["conflict_set_id"], conflict_id);
    assert_eq!(shown["explanation"]["paths"][0]["path"], "README.md");
    assert_eq!(
        shown["explanation"]["paths"][0]["recommendation"]["resolution"],
        "manual"
    );
    assert_eq!(
        shown["explanation"]["paths"][0]["lines"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
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
        conflicted_readme_workspace("hello\nlane-api\n", "hello\nhuman-api\n");
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
        conflicted_readme_workspace("hello\nlane-mcp\n", "hello\nhuman-mcp\n");
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
    db.spawn_lane("api-queue-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_str(
        r#"{
          "message": "lane edits readme",
          "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nlane-api\n"}
          ]
        }"#,
    )
    .unwrap();
    apply_lane_patch_at_head(&mut db, "api-queue-bot", patch).unwrap();

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
    assert_eq!(
        shown["explanation"]["paths"][0]["recommendation"]["resolution"],
        "manual"
    );
    assert_eq!(
        shown["explanation"]["paths"][0]["lines"][0]["target"],
        "human-api\n"
    );
    assert_eq!(
        shown["explanation"]["paths"][0]["lines"][0]["source"],
        "lane-api\n"
    );

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
    assert_eq!(
        mcp_show["result"]["structuredContent"]["explanation"]["paths"][0]["recommendation"]
            ["resolution"],
        "manual"
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

    db.spawn_lane("cancel-queue-bot", Some("main"), false, None, None)
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
        "hello\nlane-api\n"
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
fn status_maintains_persisted_worktree_file_index() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), "a1\n").unwrap();
    fs::write(temp.path().join("b.txt"), "b1\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let conn = Connection::open(temp.path().join(".crabdb/index/crabdb.sqlite")).unwrap();
    let baseline_root = |conn: &Connection| -> Option<String> {
        let mut stmt = conn
            .prepare("SELECT value FROM schema_meta WHERE key = 'worktree.index.baseline_root'")
            .unwrap();
        let mut rows = stmt.query([]).unwrap();
        rows.next()
            .unwrap()
            .map(|row| row.get::<_, String>(0).unwrap())
    };
    let initial_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM worktree_file_index", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(initial_count, 2);
    let a_metadata = fs::symlink_metadata(temp.path().join("a.txt")).unwrap();
    let (a_device_id, a_inode): (i64, i64) = conn
        .query_row(
            "SELECT device_id, inode FROM worktree_file_index WHERE path = 'a.txt'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(a_device_id, a_metadata.dev().min(i64::MAX as u64) as i64);
    assert_eq!(a_inode, a_metadata.ino().min(i64::MAX as u64) as i64);
    let initial_scan_ids = {
        let mut stmt = conn
            .prepare("SELECT path, last_seen_scan FROM worktree_file_index ORDER BY path")
            .unwrap();
        stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap()
    };

    let clean_db = CrabDb::open(temp.path()).unwrap();
    let clean_status = clean_db.status(Some("main")).unwrap();
    assert_eq!(clean_status.worktree_state, WorktreeState::Clean);
    assert_eq!(baseline_root(&conn), Some(clean_status.head.root_id.0));
    drop(clean_db);
    let clean_scan_ids = {
        let mut stmt = conn
            .prepare("SELECT path, last_seen_scan FROM worktree_file_index ORDER BY path")
            .unwrap();
        stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap()
    };
    assert_eq!(clean_scan_ids, initial_scan_ids);

    fs::write(temp.path().join("a.txt"), "a1\na2\n").unwrap();
    fs::remove_file(temp.path().join("b.txt")).unwrap();
    let mut db = CrabDb::open(temp.path()).unwrap();
    let status = db.status(Some("main")).unwrap();
    assert_eq!(baseline_root(&conn), None);
    let changes = status
        .changed_paths
        .iter()
        .map(|path| (path.path.as_str(), path.kind.clone()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        changes.get("a.txt"),
        Some(&crabdb::FileChangeKind::Modified)
    );
    assert_eq!(changes.get("b.txt"), Some(&crabdb::FileChangeKind::Deleted));

    let indexed_paths = {
        let mut stmt = conn
            .prepare("SELECT path FROM worktree_file_index ORDER BY path")
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
    };
    assert_eq!(indexed_paths, vec!["a.txt".to_string()]);

    let recorded = db
        .record(Some("main"), None, Actor::human(), false)
        .unwrap();
    assert!(recorded.operation.is_some());
    assert_eq!(baseline_root(&conn), Some(recorded.root_id.0));
}

#[test]
fn small_text_policy_avoids_prolly_text_maps_for_tiny_files() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nsmall\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let db = CrabDb::open(temp.path()).unwrap();
    let status = db.status(Some("main")).unwrap();
    let root = db.inspect_root(&status.head.root_id.0).unwrap();
    let readme = root
        .files
        .iter()
        .find(|file| file.path == "README.md")
        .unwrap();
    let text = db.inspect_text(&readme.content_object.0, 0).unwrap();
    assert!(matches!(
        text.content.representation,
        TextRepresentation::SmallTextTable { .. }
    ));
    assert!(text.content.full_bytes_blob_id.is_none());
    assert!(text.content.order_map_root.is_none());
    assert_eq!(text.lines.len(), 2);

    let full = tempfile::tempdir().unwrap();
    fs::write(full.path().join("README.md"), "hello\nfull\n").unwrap();
    CrabDb::init_with_text_policy(
        full.path(),
        "main",
        InitImportMode::WorkingTree,
        false,
        Some("full"),
    )
    .unwrap();
    let db = CrabDb::open(full.path()).unwrap();
    let status = db.status(Some("main")).unwrap();
    let root = db.inspect_root(&status.head.root_id.0).unwrap();
    let readme = root
        .files
        .iter()
        .find(|file| file.path == "README.md")
        .unwrap();
    let text = db.inspect_text(&readme.content_object.0, 0).unwrap();
    assert!(matches!(
        text.content.representation,
        TextRepresentation::TreeText
    ));
    assert!(text.content.full_bytes_blob_id.is_some());
    assert!(text.content.order_map_root.is_some());
}

#[test]
fn minimal_text_policy_uses_lazy_line_trackable_text() {
    let temp = tempfile::tempdir().unwrap();
    let body = (0..512)
        .map(|idx| format!("line {idx}\n"))
        .collect::<String>();
    assert!(body.len() > 4 * 1024);
    fs::write(temp.path().join("README.md"), body).unwrap();

    CrabDb::init_with_text_policy(
        temp.path(),
        "main",
        InitImportMode::WorkingTree,
        false,
        Some("minimal"),
    )
    .unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    let status = db.status(Some("main")).unwrap();
    let root = db.inspect_root(&status.head.root_id.0).unwrap();
    let readme = root
        .files
        .iter()
        .find(|file| file.path == "README.md")
        .unwrap();
    let text = db.inspect_text(&readme.content_object.0, 0).unwrap();
    assert!(matches!(
        text.content.representation,
        TextRepresentation::LazyText { .. }
    ));
    assert!(text.content.full_bytes_blob_id.is_some());
    assert!(text.content.order_map_root.is_none());
    assert!(text.content.line_index_map_root.is_none());
    assert_eq!(text.lines.len(), 512);

    let why = db.why("README.md:128", Some("main")).unwrap();
    assert_eq!(why.current_text, "line 127");
    let line_id = why.line_id.clone();

    let report = db.rebuild_indexes_with_rich_text().unwrap();
    assert_eq!(report.rich_text_hydrated, 1);

    let status = db.status(Some("main")).unwrap();
    let root = db.inspect_root(&status.head.root_id.0).unwrap();
    let readme = root
        .files
        .iter()
        .find(|file| file.path == "README.md")
        .unwrap();
    let text = db.inspect_text(&readme.content_object.0, 0).unwrap();
    assert!(matches!(
        text.content.representation,
        TextRepresentation::TreeText
    ));
    assert!(text.content.order_map_root.is_some());

    let hydrated_why = db.why("README.md:128", Some("main")).unwrap();
    assert_eq!(hydrated_why.current_text, "line 127");
    assert_eq!(hydrated_why.line_id, line_id);
}

#[test]
fn text_content_full_bytes_blob_is_backward_compatible() {
    #[derive(serde::Serialize)]
    struct LegacyTextContent {
        version: u16,
        content_hash: String,
        line_count: u64,
        byte_count: u64,
        order_map_root: Option<String>,
        line_index_map_root: Option<String>,
        representation: TextRepresentation,
    }

    let legacy = LegacyTextContent {
        version: 1,
        content_hash: "hash".to_string(),
        line_count: 0,
        byte_count: 0,
        order_map_root: None,
        line_index_map_root: None,
        representation: TextRepresentation::TreeText,
    };
    let bytes = serde_cbor::to_vec(&legacy).unwrap();
    let decoded: TextContent = serde_cbor::from_slice(&bytes).unwrap();

    assert_eq!(decoded.content_hash, "hash");
    assert!(decoded.full_bytes_blob_id.is_none());
}

#[test]
fn index_watch_once_refreshes_worktree_file_index() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), "a1\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    fs::write(temp.path().join("b.txt"), "b1\n").unwrap();
    let report = run_crabdb_json(
        temp.path(),
        &["index", "watch", "--once", "--interval-ms", "1"],
    );
    assert_eq!(report["files"].as_u64(), Some(2));
    assert_eq!(report["indexed_entries"].as_u64(), Some(2));
    assert!(report["duration_ms"].as_u64().is_some());

    let conn = Connection::open(temp.path().join(".crabdb/index/crabdb.sqlite")).unwrap();
    let indexed_paths = {
        let mut stmt = conn
            .prepare("SELECT path FROM worktree_file_index ORDER BY path")
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
    };
    assert_eq!(
        indexed_paths,
        vec!["a.txt".to_string(), "b.txt".to_string()]
    );
}

#[test]
fn daemon_worktree_cache_status_tracks_file_events() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.enable_daemon_worktree_cache().unwrap();
    let clean = db.status(None).unwrap();
    assert_eq!(clean.worktree_state, WorktreeState::Clean);

    fs::write(temp.path().join("README.md"), "hello\nwatched\n").unwrap();
    let dirty = wait_for_status(&db, |status| {
        status
            .changed_paths
            .iter()
            .any(|path| path.path == "README.md")
    });
    assert_eq!(dirty.worktree_state, WorktreeState::DirtyTracked);
    assert_eq!(dirty.changed_paths[0].path, "README.md");
    let diff = db.diff_dirty(false, false).unwrap();
    assert_eq!(diff.files.len(), 1);
    assert_eq!(diff.files[0].path, "README.md");

    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    let clean_again = wait_for_status(&db, |status| status.worktree_state == WorktreeState::Clean);
    assert!(clean_again.changed_paths.is_empty());
}

#[test]
fn daemon_worktree_cache_record_clears_watched_dirty_paths() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.enable_daemon_worktree_cache().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nrecorded\n").unwrap();
    wait_for_status(&db, |status| {
        status
            .changed_paths
            .iter()
            .any(|path| path.path == "README.md")
    });

    let recorded = db
        .record(
            Some("main"),
            Some("record watched path".to_string()),
            Actor::human(),
            false,
        )
        .unwrap();
    assert!(recorded.operation.is_some());
    assert_eq!(recorded.changed_paths.len(), 1);
    assert_eq!(recorded.changed_paths[0].path, "README.md");

    let clean = db.status(None).unwrap();
    assert_eq!(clean.worktree_state, WorktreeState::Clean);
}

fn wait_for_status<F>(db: &CrabDb, mut ready: F) -> crabdb::StatusReport
where
    F: FnMut(&crabdb::StatusReport) -> bool,
{
    let mut last = None;
    for _ in 0..100 {
        let status = db.status(None).unwrap();
        if ready(&status) {
            return status;
        }
        last = Some(status);
        thread::sleep(Duration::from_millis(25));
    }
    panic!("status did not reach expected state: {last:?}");
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
fn lane_patch_records_message_and_event_indexes() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    CrabDb::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = CrabDb::open(temp.path()).unwrap();
    db.spawn_lane("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_str(
        r#"{
          "session_id": "session-test",
          "message": "lane edits readme",
          "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nlane\n"}
          ]
        }"#,
    )
    .unwrap();
    apply_lane_patch_at_head(&mut db, "doc-bot", patch).unwrap();

    let conn = Connection::open(temp.path().join(".crabdb/index/crabdb.sqlite")).unwrap();
    let messages: i64 = conn
        .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
        .unwrap();
    let events: i64 = conn
        .query_row("SELECT COUNT(*) FROM lane_events", [], |row| row.get(0))
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
    db.spawn_lane("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_str(
        r#"{
          "session_id": "session-show",
          "message": "lane adds line",
          "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nlane\n"}
          ]
        }"#,
    )
    .unwrap();
    let applied = apply_lane_patch_at_head(&mut db, "doc-bot", patch).unwrap();

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
        ShowResult::Message { value } => assert_eq!(value.body, "lane adds line"),
        other => panic!("expected message show result, got {other:?}"),
    }

    let file_history = db.history_for_path("README.md").unwrap();
    assert!(file_history.file_history.len() >= 2);

    let why = db.why("README.md:2", Some("refs/lanes/doc-bot")).unwrap();
    let line_id = format!("{}:{}", why.line_id.origin_change.0, why.line_id.local_seq);
    let line_history = db.history_for_line_id(&line_id).unwrap();
    assert!(!line_history.line_history.is_empty());

    let by_lane = db.code_from("lane:doc-bot").unwrap();
    assert!(by_lane
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
    db.spawn_lane("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_str(
        r#"{
          "session_id": "session-rebuild",
          "message": "lane edits readme",
          "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nlane\n"}
          ]
        }"#,
    )
    .unwrap();
    apply_lane_patch_at_head(&mut db, "doc-bot", patch).unwrap();
    let turn = db
        .begin_lane_turn(
            "doc-bot",
            Some("main"),
            Some("rebuild trace span index".to_string()),
            None,
        )
        .unwrap();
    let span = db
        .start_lane_trace_span(
            &turn.turn.turn_id,
            "tool_call",
            "cargo test",
            None,
            None,
            None,
        )
        .unwrap();
    let span_id = span.span.span_id.clone();
    db.end_lane_trace_span(&span_id, "completed", None).unwrap();

    let conn = Connection::open(temp.path().join(".crabdb/index/crabdb.sqlite")).unwrap();
    conn.execute_batch(
        "\
        DELETE FROM operations;
        DELETE FROM operation_parents;
        DELETE FROM file_history;
        DELETE FROM line_history;
        DELETE FROM messages;
        DELETE FROM lane_trace_span_events;
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
    let span_events: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM lane_trace_span_events WHERE span_id = ?1",
            [&span_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(span_events, 2);
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
