use std::collections::{BTreeMap, BTreeSet};
#[cfg(target_os = "linux")]
use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStringExt;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

use rusqlite::Connection;
use trail::{
    Actor, ConflictManualFile, ConflictManualResolution, Error, InitImportMode, LaneGateOptions,
    LaneMessageReport, LanePatchReport, LaneRewindReport, LaneTurnDetails, LaneTurnEndReport,
    LaneTurnEventReport, LaneTurnStartReport, LaneWorkdirMode, MaterializationFallbackReason,
    ObjectId, OperationKind, PatchDocument, ShowResult, TextContent, TextRepresentation, Trail,
    WorkdirBackend, WorktreeRoot, WorktreeState, WORKTREE_ROOT_KIND,
};

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

fn git_output_raw(cwd: &Path, args: &[&str]) -> String {
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
    String::from_utf8(output.stdout).unwrap()
}

fn git_object_count(cwd: &Path) -> u64 {
    let output = git_output(cwd, &["count-objects", "-v"]);
    let counts = output
        .lines()
        .filter_map(|line| line.split_once(' '))
        .collect::<BTreeMap<_, _>>();
    let loose = counts.get("count:").unwrap().parse::<u64>().unwrap();
    let packed = counts.get("in-pack:").unwrap().parse::<u64>().unwrap();
    loose + packed
}

fn trail_bin() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_trail")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/debug/trail"))
}

#[test]
fn cli_reports_package_version() {
    let output = Command::new(trail_bin()).arg("--version").output().unwrap();
    assert!(
        output.status.success(),
        "trail --version failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("trail {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn agent_default_provider_is_a_typed_workspace_config_value() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let set = run_trail_json(
        temp.path(),
        &["config", "set", "agent.default_provider", "codex"],
    );
    assert_eq!(set["key"], "agent.default_provider");
    assert_eq!(set["new_value"], "codex");

    let get = run_trail_json(temp.path(), &["config", "get", "agent.default_provider"]);
    assert_eq!(get["value"], "codex");
}

#[cfg(unix)]
#[test]
fn terminal_agent_uses_configured_provider_when_argument_is_absent() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    run_trail_json(
        temp.path(),
        &["config", "set", "agent.default_provider", "custom"],
    );

    let report = run_trail_json(temp.path(), &["agent", "start", "--", "/usr/bin/true"]);
    assert_eq!(report["provider"], "custom");
}

#[test]
fn agent_acp_setup_plan_uses_the_hidden_run_command() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let report = run_trail_json(
        temp.path(),
        &[
            "agent", "acp", "setup", "codex", "--editor", "generic", "--print",
        ],
    );
    assert_eq!(report["transport"], "acp");
    assert_eq!(report["provider"], "codex");
    assert_eq!(report["editor"], "generic");
    assert_eq!(report["applied"], false);
    assert_eq!(
        report["command"],
        serde_json::json!([
            trail_bin().canonicalize().unwrap(),
            "--workspace",
            temp.path().canonicalize().unwrap(),
            "agent",
            "acp",
            "run",
            "codex"
        ])
    );
}

#[test]
fn agent_acp_setup_falls_back_to_a_generic_entry_for_unknown_editors() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let report = run_trail_json(
        temp.path(),
        &[
            "agent", "acp", "setup", "codex", "--editor", "neovim", "--print",
        ],
    );
    assert_eq!(report["editor"], "neovim");
    assert_eq!(report["action"], "print");
    assert!(report["snippet"].as_str().unwrap().contains("ACP command:"));
    assert!(report["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning.as_str().unwrap().contains("generic entry")));
}

#[test]
fn agent_acp_setup_merges_the_owned_zed_entry() {
    let workspace = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    fs::write(workspace.path().join("README.md"), "hello\n").unwrap();
    Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let settings = test_zed_settings_path(home.path());
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(&settings, "{\"theme\":\"One Dark\"}\n").unwrap();

    let output = Command::new(trail_bin())
        .arg("--workspace")
        .arg(workspace.path())
        .arg("--json")
        .args(["agent", "acp", "setup", "codex", "--editor", "zed", "--yes"])
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join(".config"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "ACP setup failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["applied"], true);
    assert_eq!(report["action"], "update");
    let configured: serde_json::Value =
        serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
    assert_eq!(configured["theme"], "One Dark");
    assert_eq!(
        configured["agent_servers"]["trail-codex"]["args"]
            .as_array()
            .unwrap()
            .last()
            .unwrap(),
        "codex"
    );

    let repeated = Command::new(trail_bin())
        .arg("--workspace")
        .arg(workspace.path())
        .arg("--json")
        .args(["agent", "acp", "setup", "codex", "--editor", "zed", "--yes"])
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join(".config"))
        .output()
        .unwrap();
    assert!(repeated.status.success());
    let repeated: serde_json::Value = serde_json::from_slice(&repeated.stdout).unwrap();
    assert_eq!(repeated["action"], "noop");
    assert_eq!(repeated["applied"], true);
}

fn test_zed_settings_path(home: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library/Application Support/Zed/settings.json")
    }
    #[cfg(not(target_os = "macos"))]
    {
        home.join(".config/zed/settings.json")
    }
}

#[cfg(unix)]
#[test]
fn terminal_agent_start_aligns_process_context_with_the_lane_workdir() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let output = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("--json")
        .args([
            "agent",
            "start",
            "--provider",
            "claude-code",
            "--name",
            "context",
            "--workdir-mode",
            "native-cow",
            "--",
            "/usr/bin/env",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "agent start failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let workdir = report["workdir"].as_str().unwrap();
    let workspace = temp.path().canonicalize().unwrap();
    let environment = String::from_utf8_lossy(&output.stderr);
    assert!(environment
        .lines()
        .any(|line| line == format!("PWD={workdir}")));
    assert!(environment
        .lines()
        .any(|line| line == format!("TRAIL_WORKSPACE={}", workspace.display())));
    assert!(environment
        .lines()
        .any(|line| line.starts_with("TRAIL_LANE=lane_")));
    assert!(environment
        .lines()
        .any(|line| line.starts_with("TRAIL_SOURCE_ROOT=object_")));
    assert!(environment.lines().any(|line| {
        line.strip_prefix("GIT_CEILING_DIRECTORIES=")
            .is_some_and(|value| {
                value
                    .split(':')
                    .any(|path| path == workspace.to_string_lossy())
            })
    }));
}

#[cfg(unix)]
#[test]
fn terminal_agent_start_loads_project_hook_settings_in_the_isolated_provider() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let install = run_trail_json(
        temp.path(),
        &[
            "agent",
            "hooks",
            "setup",
            "claude-code",
            "--scope",
            "project",
            "--yes",
        ],
    );
    let settings = install["config_path"].as_str().unwrap().to_string();

    let bin = tempfile::tempdir().unwrap();
    let fake_claude = bin.path().join("claude");
    fs::write(
        &fake_claude,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > CLAUDE_ARGS.txt\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&fake_claude).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&fake_claude, permissions).unwrap();
    let path = std::env::join_paths(std::iter::once(bin.path().to_path_buf()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .unwrap();

    let output = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("--json")
        .env("PATH", path)
        .args([
            "agent",
            "start",
            "--provider",
            "claude-code",
            "--name",
            "hook-settings",
            "--workdir-mode",
            "auto",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "agent start failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let workdir = PathBuf::from(report["workdir"].as_str().unwrap());
    assert_eq!(
        fs::read_to_string(workdir.join("CLAUDE_ARGS.txt")).unwrap(),
        format!("--settings\n{settings}\n")
    );
}

#[cfg(unix)]
#[test]
fn terminal_agent_native_cow_does_not_discover_or_write_the_parent_git_checkout() {
    if !git_available() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "root baseline\n").unwrap();
    run_git(temp.path(), &["init", "-q"]);
    run_git(temp.path(), &["config", "user.email", "trail@example.com"]);
    run_git(temp.path(), &["config", "user.name", "Trail Test"]);
    run_git(temp.path(), &["add", "README.md"]);
    run_git(temp.path(), &["commit", "-qm", "baseline"]);
    Trail::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();

    let provider = tempfile::NamedTempFile::new().unwrap();
    fs::write(
        provider.path(),
        "#!/bin/sh\nset -eu\nprintf 'lane change\\n' > LANE_ONLY.md\nif root=$(git rev-parse --show-toplevel 2>/dev/null); then\n  printf 'escaped\\n' > \"$root/ESCAPED.md\"\nfi\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(provider.path()).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(provider.path(), permissions).unwrap();

    let output = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("--json")
        .args([
            "agent",
            "start",
            "--provider",
            "claude-code",
            "--name",
            "containment",
            "--workdir-mode",
            "native-cow",
            "--",
        ])
        .arg(provider.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "agent start failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let workdir = PathBuf::from(report["workdir"].as_str().unwrap());
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "root baseline\n"
    );
    assert!(!temp.path().join("ESCAPED.md").exists());
    assert_eq!(
        fs::read_to_string(workdir.join("LANE_ONLY.md")).unwrap(),
        "lane change\n"
    );
    assert!(report["recorded"]["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "LANE_ONLY.md"));
}

#[cfg(target_os = "macos")]
#[test]
fn terminal_agent_native_cow_denies_explicit_writes_to_the_original_workspace() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "root baseline\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let provider = tempfile::NamedTempFile::new().unwrap();
    fs::write(
        provider.path(),
        "#!/bin/sh\nprintf 'lane change\\n' > LANE_ONLY.md\nif printf 'escaped\\n' > \"$1/ESCAPED.md\"; then\n  printf 'workspace write was allowed\\n' > CONTAINMENT.txt\nelse\n  printf 'workspace write was blocked\\n' > CONTAINMENT.txt\nfi\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(provider.path()).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(provider.path(), permissions).unwrap();

    let output = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("--json")
        .args([
            "agent",
            "start",
            "--provider",
            "claude-code",
            "--name",
            "explicit-containment",
            "--workdir-mode",
            "native-cow",
            "--",
        ])
        .arg(provider.path())
        .arg(temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "agent start failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let workdir = PathBuf::from(report["workdir"].as_str().unwrap());
    assert!(!temp.path().join("ESCAPED.md").exists());
    assert_eq!(
        fs::read_to_string(workdir.join("LANE_ONLY.md")).unwrap(),
        "lane change\n"
    );
    assert_eq!(
        fs::read_to_string(workdir.join("CONTAINMENT.txt")).unwrap(),
        "workspace write was blocked\n"
    );
}

#[cfg(unix)]
#[test]
fn terminal_agent_native_hooks_enrich_the_existing_task_without_duplication() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let provider = tempfile::NamedTempFile::new().unwrap();
    fs::write(
        provider.path(),
        "#!/bin/sh\nset -eu\nsend() {\n  event=$1\n  payload=$2\n  printf '%s' \"$payload\" | \"$TRAIL_TEST_BIN\" --workspace \"$TRAIL_WORKSPACE\" agent hook receive claude-code \"$event\" >/dev/null\n}\ncwd=$(pwd)\nsend SessionStart \"{\\\"session_id\\\":\\\"native-terminal-1\\\",\\\"cwd\\\":\\\"$cwd\\\"}\"\nsend UserPromptSubmit \"{\\\"session_id\\\":\\\"native-terminal-1\\\",\\\"cwd\\\":\\\"$cwd\\\",\\\"prompt\\\":\\\"edit lane\\\"}\"\nprintf 'hooked change\\n' > HOOKED.md\nsend Stop \"{\\\"session_id\\\":\\\"native-terminal-1\\\",\\\"cwd\\\":\\\"$cwd\\\",\\\"last_assistant_message\\\":\\\"done\\\"}\"\nsend SessionEnd \"{\\\"session_id\\\":\\\"native-terminal-1\\\",\\\"cwd\\\":\\\"$cwd\\\"}\"\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(provider.path()).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(provider.path(), permissions).unwrap();

    let output = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("--json")
        .env("TRAIL_TEST_BIN", trail_bin())
        .args([
            "agent",
            "start",
            "--provider",
            "claude-code",
            "--name",
            "hooked-terminal",
            "--workdir-mode",
            "native-cow",
            "--",
        ])
        .arg(provider.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "agent start failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let task_id = report["task"]["task_id"].as_str().unwrap();
    let list = run_trail_json(temp.path(), &["agent", "list", "--all"]);
    assert_eq!(list["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(list["tasks"][0]["task_id"], task_id);
    assert_eq!(list["tasks"][0]["turns"], 1);
    assert!(list["tasks"][0]["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "HOOKED.md"));
    let view = run_trail_json(temp.path(), &["agent", "view", task_id]);
    assert_eq!(
        view["review"]["recent_sessions"].as_array().unwrap().len(),
        1
    );
    assert_eq!(view["transcript"]["turns"].as_array().unwrap().len(), 1);
}

#[test]
fn native_hook_cli_journals_codex_stop_and_returns_required_success_json() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let payload = serde_json::json!({
        "session_id": "codex-session-1",
        "turn_id": "codex-turn-1",
        "hook_event_name": "Stop",
        "last_assistant_message": "done"
    })
    .to_string();
    let mut child = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .args(["agent", "hook", "receive", "codex", "Stop"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "hook ingress failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "{\"continue\":true}"
    );
    let db = Trail::open(temp.path()).unwrap();
    let receipts = db
        .list_agent_hook_receipts(Some("codex"), Some("processed"), 10)
        .unwrap();
    assert_eq!(receipts.len(), 1);
    assert_eq!(
        receipts[0].native_session_id.as_deref(),
        Some("codex-session-1")
    );
    assert_eq!(receipts[0].native_turn_id.as_deref(), Some("codex-turn-1"));
}

#[test]
fn agent_hook_http_openapi_and_mcp_surfaces_share_durable_receipts() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    let payload = serde_json::json!({
        "session_id": "codex-http-session",
        "turn_id": "codex-http-turn",
        "hook_event_name": "Stop"
    });

    let unauthenticated = trail::server::handle_http_request(
        &mut db,
        &api_request("POST", "/v1/agent-hooks/codex/Stop", payload.clone()),
    );
    assert_eq!(unauthenticated.status, 401);

    let auth = trail::server::ServerAuth::bearer("agent-hook-test-token").unwrap();
    let ingested = trail::server::handle_http_request_with_auth(
        &mut db,
        &api_request_with_headers(
            "POST",
            "/v1/agent-hooks/codex/Stop",
            &[("Authorization", "Bearer agent-hook-test-token")],
            payload,
        ),
        &auth,
    );
    assert_eq!(ingested.status, 200);
    let ingested: serde_json::Value = ingested.body_json().unwrap();
    assert_eq!(ingested["continue"], true);

    let receipts = trail::server::handle_http_request_with_auth(
        &mut db,
        &api_request_with_headers(
            "GET",
            "/v1/agent-hooks/receipts?provider=codex",
            &[("Authorization", "Bearer agent-hook-test-token")],
            serde_json::Value::Null,
        ),
        &auth,
    );
    assert_eq!(receipts.status, 200);
    let receipts: serde_json::Value = receipts.body_json().unwrap();
    assert_eq!(receipts.as_array().unwrap().len(), 1);
    let receipt_id = receipts[0]["receipt_id"].as_str().unwrap().to_string();
    let cli_receipts = run_trail_json(
        temp.path(),
        &["agent", "hooks", "events", "codex", "--last", "10"],
    );
    assert_eq!(cli_receipts[0]["receipt_id"], receipt_id);
    let cli_receipts_after_first = run_trail_json(
        temp.path(),
        &[
            "agent", "hooks", "events", "codex", "--offset", "1", "--last", "1",
        ],
    );
    assert_eq!(cli_receipts_after_first, serde_json::json!([]));

    let receipts_after_first = trail::server::handle_http_request_with_auth(
        &mut db,
        &api_request_with_headers(
            "GET",
            "/v1/agent-hooks/receipts?provider=codex&offset=1&limit=1",
            &[("Authorization", "Bearer agent-hook-test-token")],
            serde_json::Value::Null,
        ),
        &auth,
    );
    assert_eq!(receipts_after_first.status, 200);
    let receipts_after_first: serde_json::Value = receipts_after_first.body_json().unwrap();
    assert_eq!(receipts_after_first, serde_json::json!([]));

    let capabilities = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_integrations",
                "arguments": {"provider": "codex"}
            }
        }),
    )
    .unwrap();
    assert_eq!(capabilities["result"]["isError"], false);
    assert_eq!(
        capabilities["result"]["structuredContent"]["provider"],
        "codex"
    );

    let mcp_receipts = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_hook_receipts",
                "arguments": {"provider": "codex"}
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_receipts["result"]["isError"], false);
    assert_eq!(
        mcp_receipts["result"]["structuredContent"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        mcp_receipts["result"]["structuredContent"][0]["receipt_id"],
        receipt_id
    );
    let mcp_receipts_after_first = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_hook_receipts",
                "arguments": {"provider": "codex", "offset": 1, "limit": 1}
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_receipts_after_first["result"]["isError"], false);
    assert_eq!(
        mcp_receipts_after_first["result"]["structuredContent"],
        serde_json::json!([])
    );

    let resource = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "resources/read",
            "params": {"uri": "trail://workspace/agent-hooks/receipts"}
        }),
    )
    .unwrap();
    assert!(resource["result"]["contents"][0]["text"]
        .as_str()
        .unwrap()
        .contains("codex-http-session"));

    let spec = trail::server::openapi_spec();
    assert!(spec["paths"]["/v1/agent-hooks/{provider}/{event}"]["post"].is_object());
    assert!(spec["paths"]["/v1/agent-sessions/{id}/provenance"]["get"].is_object());
    assert_eq!(
        spec["components"]["schemas"]["AgentCaptureRunRequest"]["additionalProperties"],
        false
    );
}

#[test]
fn native_hook_management_cli_installs_reports_and_removes_owned_config() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let setup = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("--json")
        .args(["agent", "hooks", "setup", "codex", "--yes"])
        .output()
        .unwrap();
    assert!(
        setup.status.success(),
        "hook install failed: {}",
        String::from_utf8_lossy(&setup.stderr)
    );
    let setup: serde_json::Value = serde_json::from_slice(&setup.stdout).unwrap();
    assert_eq!(setup["provider"], "codex");
    let hooks_path = temp.path().join(".codex/hooks.json");
    let hooks = fs::read_to_string(&hooks_path).unwrap();
    assert!(hooks.contains(" agent hook receive codex Stop --installation hook_"));

    let db = Trail::open(temp.path()).unwrap();
    let records = db.list_agent_hook_installations(Some("codex")).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, "installed");
    drop(db);

    let status = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("--json")
        .args(["agent", "hooks", "status", "codex"])
        .output()
        .unwrap();
    assert!(status.status.success());
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["installations"][0]["filesystem_status"], "installed");

    let remove = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .args(["agent", "hooks", "remove", "codex"])
        .output()
        .unwrap();
    assert!(
        remove.status.success(),
        "hook removal failed: {}",
        String::from_utf8_lossy(&remove.stderr)
    );
    let remaining: serde_json::Value =
        serde_json::from_slice(&fs::read(&hooks_path).unwrap()).unwrap();
    assert!(remaining.get("hooks").is_none());
    let db = Trail::open(temp.path()).unwrap();
    assert_eq!(
        db.list_agent_hook_installations(Some("codex")).unwrap()[0].status,
        "removed"
    );
}

#[test]
fn native_hook_cli_spools_during_database_failure_and_replays_after_recovery() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let index = temp.path().join(".trail/index/trail.sqlite");
    let backup = temp.path().join(".trail/index/trail.sqlite.saved");
    fs::rename(&index, &backup).unwrap();
    fs::create_dir(&index).unwrap();

    let payload = serde_json::json!({
        "session_id": "spooled-session-1",
        "turn_id": "spooled-turn-1",
        "hook_event_name": "Stop"
    })
    .to_string();
    let mut child = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .args(["agent", "hook", "receive", "codex", "Stop"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "{\"continue\":true}"
    );
    let spool = temp.path().join(".trail/runtime/agent-hooks-spool");
    assert_eq!(fs::read_dir(&spool).unwrap().count(), 1);

    fs::remove_dir(&index).unwrap();
    fs::rename(&backup, &index).unwrap();
    let replay = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("--json")
        .args(["agent", "hooks", "replay", "--pending"])
        .output()
        .unwrap();
    assert!(
        replay.status.success(),
        "spool replay failed: {}",
        String::from_utf8_lossy(&replay.stderr)
    );
    let replay: serde_json::Value = serde_json::from_slice(&replay.stdout).unwrap();
    assert_eq!(replay["spool"]["imported"], 1);
    assert_eq!(replay["replayed"].as_array().unwrap().len(), 1);
    assert_eq!(fs::read_dir(&spool).unwrap().count(), 0);
}

#[test]
fn agent_capture_and_portable_evidence_cli_surface_round_trips() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let begin = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("--json")
        .args([
            "agent",
            "capture",
            "begin",
            "--owner",
            "codex",
            "--session",
            "owner-1",
            "--workdir",
        ])
        .arg(temp.path())
        .output()
        .unwrap();
    assert!(
        begin.status.success(),
        "{}",
        String::from_utf8_lossy(&begin.stderr)
    );
    let begin: serde_json::Value = serde_json::from_slice(&begin.stdout).unwrap();
    let run_id = begin["capture_run_id"].as_str().unwrap();
    let status = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("--json")
        .args(["agent", "capture", "status"])
        .output()
        .unwrap();
    let status: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status[0]["capture_run_id"], run_id);
    let end = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("--json")
        .args([
            "agent",
            "capture",
            "end",
            run_id,
            "--owner",
            "codex",
            "--session",
            "owner-1",
        ])
        .output()
        .unwrap();
    assert!(
        end.status.success(),
        "{}",
        String::from_utf8_lossy(&end.stderr)
    );

    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("portable", None, false, Some("codex".to_string()), None)
        .unwrap();
    let session = db
        .start_lane_session("portable", Some("portable".to_string()), None)
        .unwrap()
        .session;
    let turn = db
        .begin_lane_session_turn("portable", &session.session_id, None)
        .unwrap()
        .turn;
    db.add_lane_turn_message(&turn.turn_id, "user", "portable trace")
        .unwrap();
    db.end_lane_turn(&turn.turn_id, "completed").unwrap();
    db.create_turn_evidence_manifest(&turn.turn_id).unwrap();
    let attestation = db
        .create_session_attestation(&session.session_id, "test", None)
        .unwrap();
    drop(db);

    let verify = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("--json")
        .args(["agent", "attest", "verify", &attestation.attestation_id])
        .output()
        .unwrap();
    assert!(verify.status.success());
    let verify: serde_json::Value = serde_json::from_slice(&verify.stdout).unwrap();
    assert_eq!(verify["valid"], true);

    let export = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .args(["agent", "export", &session.session_id])
        .output()
        .unwrap();
    assert!(
        export.status.success(),
        "{}",
        String::from_utf8_lossy(&export.stderr)
    );
    let trace = trail::PortableAgentTrace::from_json(&export.stdout).unwrap();
    assert_eq!(trace.session_id, session.session_id);
    assert!(trace.verify().valid);
}

#[cfg(unix)]
struct StubAcpAgentOptions<'a> {
    session_id: &'a str,
    lane_workdir: Option<&'a Path>,
    assistant_text_before_tool: Option<String>,
    assistant_text: String,
    write_text: Option<&'a str>,
    crash_after_update: bool,
    malformed_after_update: bool,
    request_permission: bool,
    sleep_before_result_ms: Option<u64>,
}

#[cfg(unix)]
impl<'a> StubAcpAgentOptions<'a> {
    fn new(session_id: &'a str) -> Self {
        Self {
            session_id,
            lane_workdir: None,
            assistant_text_before_tool: None,
            assistant_text: "diagnostic complete".to_string(),
            write_text: Some("diagnostic complete"),
            crash_after_update: false,
            malformed_after_update: false,
            request_permission: false,
            sleep_before_result_ms: None,
        }
    }
}

#[cfg(unix)]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(unix)]
fn write_stub_acp_agent(
    workspace: &Path,
    filename: &str,
    options: StubAcpAgentOptions<'_>,
) -> PathBuf {
    let agent = workspace.join(filename);
    let write_dir = options
        .lane_workdir
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_default();
    let assistant_before_tool = options
        .assistant_text_before_tool
        .as_ref()
        .map(|text| {
            let update = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": options.session_id,
                    "update": {
                        "sessionUpdate": "agent_message_chunk",
                        "messageId": "msg_before_tool",
                        "content": {
                            "type": "text",
                            "text": text
                        }
                    }
                }
            });
            format!("printf '%s\\n' {}\n", shell_quote(&update.to_string()))
        })
        .unwrap_or_default();
    let permission_request = if options.request_permission {
        format!(
            "printf '%s\\n' '{{\"jsonrpc\":\"2.0\",\"id\":50,\"method\":\"session/request_permission\",\"params\":{{\"sessionId\":\"{}\",\"toolCall\":{{\"title\":\"approve diagnostic write\"}},\"options\":[{{\"optionId\":\"allow\",\"kind\":\"allow_once\",\"name\":\"Allow\"}}]}}}}'\n",
            options.session_id
        )
    } else {
        String::new()
    };
    let malformed = if options.malformed_after_update {
        "printf '%s\\n' '{not-json'\nexit 0\n"
    } else {
        ""
    };
    let crash = if options.crash_after_update {
        "exit 42\n"
    } else {
        ""
    };
    let sleep = options
        .sleep_before_result_ms
        .map(|ms| format!("sleep {}\n", (ms as f64) / 1000.0))
        .unwrap_or_default();
    let write_file = options
        .write_text
        .map(|text| {
            format!(
                "if [ -n \"$WRITE_DIR\" ]; then\n  mkdir -p \"$WRITE_DIR\"\n  printf '%s\\n' {} > \"$WRITE_DIR/README.md\"\nfi\n",
                shell_quote(text)
            )
        })
        .unwrap_or_default();

    fs::write(
        &agent,
        format!(
            r#"#!/bin/sh
set -eu
SESSION_ID={}
WRITE_DIR={}
IFS= read -r init
printf '%s\n' '{{"jsonrpc":"2.0","id":0,"result":{{"protocolVersion":1,"agentCapabilities":{{}}}}}}'
IFS= read -r session_new
if [ -z "$WRITE_DIR" ]; then
  WRITE_DIR=$(printf '%s\n' "$session_new" | sed -n 's/.*"cwd":"\([^"]*\)".*/\1/p')
fi
printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"sessionId":"{}"}}}}'
IFS= read -r prompt
printf '%s\n' '{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"{}","update":{{"sessionUpdate":"available_commands_update","commands":[{{"name":"write_file","description":"diagnostic command"}}]}}}}}}'
{}
printf '%s\n' '{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"{}","update":{{"sessionUpdate":"tool_call","toolCallId":"tool_stub","title":"write README","kind":"edit","status":"pending"}}}}}}'
{}
printf '%s\n' '{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"{}","update":{{"sessionUpdate":"tool_call_update","toolCallId":"tool_stub","status":"completed"}}}}}}'
printf '%s\n' '{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"{}","update":{{"sessionUpdate":"agent_message_chunk","messageId":"msg_stub","content":{{"type":"text","text":{}}}}}}}}}'
{}{}{}{}
printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"stopReason":"end_turn"}}}}'
"#,
            shell_quote(options.session_id),
            shell_quote(&write_dir),
            options.session_id,
            options.session_id,
            assistant_before_tool,
            options.session_id,
            permission_request,
            options.session_id,
            options.session_id,
            serde_json::to_string(&options.assistant_text).unwrap(),
            malformed,
            crash,
            sleep,
            write_file
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&agent).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&agent, permissions).unwrap();
    agent
}

fn patch_with_lane_head(db: &Trail, lane: &str, mut patch: PatchDocument) -> PatchDocument {
    if patch.base_change.is_none() {
        patch.base_change = Some(db.lane_details(lane).unwrap().branch.head_change.0);
    }
    patch
}

fn apply_lane_patch_at_head(
    db: &mut Trail,
    lane: &str,
    patch: PatchDocument,
) -> Result<LanePatchReport, Error> {
    let patch = patch_with_lane_head(db, lane, patch);
    db.apply_lane_patch(lane, patch)
}

fn ready_agent_lane_with_mode(mode: InitImportMode) -> (tempfile::TempDir, Trail) {
    let temp = tempfile::tempdir().unwrap();
    run_git(temp.path(), &["init"]);
    run_git(temp.path(), &["config", "user.email", "trail@example.test"]);
    run_git(temp.path(), &["config", "user.name", "Trail Test"]);
    fs::write(temp.path().join("README.md"), "base\n").unwrap();
    run_git(temp.path(), &["add", "README.md"]);
    run_git(temp.path(), &["commit", "-m", "initial"]);
    Trail::init(temp.path(), "main", mode, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("apply-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "one changed path",
        "edits": [{"op": "write", "path": "AGENT.md", "content": "agent change\n"}]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "apply-bot", patch).unwrap();
    db.agent_mark_reviewed("apply-bot", None).unwrap();
    (temp, db)
}

fn ready_agent_lane_with_changed_paths(changed_path_count: usize) -> (tempfile::TempDir, Trail) {
    let temp = tempfile::tempdir().unwrap();
    run_git(temp.path(), &["init"]);
    run_git(temp.path(), &["config", "user.email", "trail@example.test"]);
    run_git(temp.path(), &["config", "user.name", "Trail Test"]);
    fs::write(temp.path().join("README.md"), "base\n").unwrap();
    run_git(temp.path(), &["add", "README.md"]);
    run_git(temp.path(), &["commit", "-m", "initial"]);
    Trail::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("apply-bot", Some("main"), false, None, None)
        .unwrap();
    let edits = (0..changed_path_count)
        .map(|index| {
            serde_json::json!({
                "op": "write",
                "path": format!("agent-{index:03}.md"),
                "content": format!("agent change {index}\n"),
            })
        })
        .collect::<Vec<_>>();
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "many changed paths",
        "edits": edits,
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "apply-bot", patch).unwrap();
    db.agent_mark_reviewed("apply-bot", None).unwrap();
    (temp, db)
}

#[test]
fn agent_apply_reports_one_tracked_status_query() {
    if !git_available() {
        return;
    }

    let (_temp, mut db) = ready_agent_lane_with_mode(InitImportMode::GitTracked);
    let dry_run = db.agent_apply("apply-bot", true, None).unwrap();
    assert_eq!(dry_run.performance.tracked_status_count, 1);
    assert_eq!(dry_run.performance.full_root_file_count, 0);
    assert_eq!(dry_run.performance.export_mode, "mapped_delta");
}

#[test]
fn agent_apply_actual_reports_mapped_delta_metrics() {
    if !git_available() {
        return;
    }

    let (_temp, mut db) = ready_agent_lane_with_mode(InitImportMode::GitTracked);
    let applied = db.agent_apply("apply-bot", false, None).unwrap();
    assert_eq!(applied.performance.tracked_status_count, 1);
    assert_eq!(applied.performance.full_root_file_count, 0);
    assert_eq!(applied.performance.export_mode, "mapped_delta");
    assert_eq!(applied.performance.changed_path_count, 1);
    assert_eq!(applied.performance.blob_write_count, 1);
}

#[test]
fn agent_apply_batches_git_plumbing_for_many_paths() {
    if !git_available() {
        return;
    }

    let (temp, mut db) = ready_agent_lane_with_changed_paths(100);
    let applied = db.agent_apply("apply-bot", false, None).unwrap();
    assert_eq!(applied.performance.changed_path_count, 100);
    assert_eq!(applied.performance.blob_write_count, 100);
    assert_eq!(applied.performance.git_plumbing_command_count, 5);
    assert_eq!(
        applied
            .git_export
            .as_ref()
            .unwrap()
            .performance
            .git_plumbing_command_count,
        5
    );
    let tmp = temp.path().join(".trail/tmp");
    if tmp.is_dir() {
        assert!(!fs::read_dir(tmp).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("git-delta-")
        }));
    }
}

#[test]
fn agent_apply_batch_preserves_modes_deletions_and_safe_special_paths() {
    if !git_available() {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    run_git(temp.path(), &["init"]);
    run_git(temp.path(), &["config", "user.email", "trail@example.test"]);
    run_git(temp.path(), &["config", "user.name", "Trail Test"]);
    fs::write(temp.path().join("delete-me.txt"), "remove me\n").unwrap();
    fs::write(temp.path().join("mode-change.sh"), "echo old\n").unwrap();
    run_git(temp.path(), &["add", "delete-me.txt", "mode-change.sh"]);
    run_git(temp.path(), &["commit", "-m", "initial"]);
    Trail::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("apply-bot", Some("main"), false, None, None)
        .unwrap();
    let special_path = "dir with space/-leading-ünicode.sh";
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "mixed Git index batch",
        "edits": [
            {"op": "delete", "path": "delete-me.txt"},
            {
                "op": "write",
                "path": "mode-change.sh",
                "content": "echo changed\n",
                "executable": true
            },
            {
                "op": "write",
                "path": special_path,
                "content": "echo special\n",
                "executable": true
            }
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "apply-bot", patch).unwrap();
    db.agent_mark_reviewed("apply-bot", None).unwrap();

    let applied = db.agent_apply("apply-bot", false, None).unwrap();

    assert_eq!(applied.performance.git_plumbing_command_count, 5);
    assert_eq!(
        git_output(temp.path(), &["ls-tree", "HEAD", "--", "delete-me.txt"]),
        ""
    );
    for path in ["mode-change.sh", special_path] {
        let entry = git_output(temp.path(), &["ls-tree", "HEAD", "--", path]);
        assert!(
            entry.starts_with("100755 blob "),
            "expected executable Git entry for {path:?}, got {entry:?}"
        );
    }
    assert_eq!(
        git_output_raw(temp.path(), &["show", "HEAD:mode-change.sh"]),
        "echo changed\n"
    );
    assert_eq!(
        git_output_raw(temp.path(), &["show", &format!("HEAD:{special_path}")]),
        "echo special\n"
    );
}

#[test]
fn agent_apply_batch_hashes_exact_trail_bytes_without_git_filters() {
    if !git_available() {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    run_git(temp.path(), &["init"]);
    run_git(temp.path(), &["config", "user.email", "trail@example.test"]);
    run_git(temp.path(), &["config", "user.name", "Trail Test"]);
    run_git(
        temp.path(),
        &["config", "filter.trail-uppercase.clean", "tr a-z A-Z"],
    );
    run_git(
        temp.path(),
        &["config", "filter.trail-uppercase.smudge", "cat"],
    );
    run_git(
        temp.path(),
        &["config", "filter.trail-uppercase.required", "true"],
    );
    fs::write(
        temp.path().join(".gitattributes"),
        "* filter=trail-uppercase\n.gitattributes -filter\n",
    )
    .unwrap();
    run_git(temp.path(), &["add", ".gitattributes"]);
    run_git(temp.path(), &["commit", "-m", "initial"]);
    Trail::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("apply-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "preserve exact bytes",
        "edits": [{
            "op": "write",
            "path": "payload.txt",
            "content": "Exact Trail bytes\n"
        }]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "apply-bot", patch).unwrap();
    db.agent_mark_reviewed("apply-bot", None).unwrap();

    db.agent_apply("apply-bot", false, None).unwrap();

    assert_eq!(
        git_output_raw(temp.path(), &["show", "HEAD:payload.txt"]),
        "Exact Trail bytes\n"
    );
}

#[test]
fn agent_apply_batches_git_plumbing_with_external_db_dir() {
    if !git_available() {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    #[cfg(target_os = "linux")]
    let external_db = temp
        .path()
        .join(OsString::from_vec(b"external-trail-db-\xff".to_vec()));
    #[cfg(not(target_os = "linux"))]
    let external_db = temp.path().join("external-trail-db");
    fs::create_dir(&workspace).unwrap();
    run_git(&workspace, &["init"]);
    run_git(&workspace, &["config", "user.email", "trail@example.test"]);
    run_git(&workspace, &["config", "user.name", "Trail Test"]);
    fs::write(workspace.join("README.md"), "base\n").unwrap();
    run_git(&workspace, &["add", "README.md"]);
    run_git(&workspace, &["commit", "-m", "initial"]);
    Trail::init(&workspace, "main", InitImportMode::GitTracked, false).unwrap();
    fs::rename(workspace.join(".trail"), &external_db).unwrap();

    let mut db = Trail::open_with_db_dir(&workspace, &external_db).unwrap();
    db.spawn_lane("apply-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "apply with external database",
        "edits": [{
            "op": "write",
            "path": "external-db.txt",
            "content": "external database path\n"
        }]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "apply-bot", patch).unwrap();
    db.agent_mark_reviewed("apply-bot", None).unwrap();

    let applied = db.agent_apply("apply-bot", false, None).unwrap();

    assert_eq!(applied.performance.git_plumbing_command_count, 5);
    assert_eq!(
        git_output_raw(&workspace, &["show", "HEAD:external-db.txt"]),
        "external database path\n"
    );
    let tmp = external_db.join("tmp");
    if tmp.is_dir() {
        assert!(!fs::read_dir(tmp).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("git-delta-")
        }));
    }
}

#[test]
fn agent_apply_requires_mapping_before_git_or_trail_mutation() {
    if !git_available() {
        return;
    }

    let (temp, mut db) = ready_agent_lane_with_mode(InitImportMode::WorkingTree);
    let git_head = git_output(temp.path(), &["rev-parse", "HEAD"]);
    assert!(db.git_mappings(10).unwrap().is_empty());

    let err = db.agent_apply("apply-bot", true, None).unwrap_err();
    assert!(matches!(err, Error::GitMappingRequired(_)));
    assert!(db.git_mappings(10).unwrap().is_empty());
    assert_eq!(git_output(temp.path(), &["rev-parse", "HEAD"]), git_head);
}

#[cfg(unix)]
#[test]
fn agent_apply_preserves_git_only_symlinks() {
    if !git_available() {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    run_git(temp.path(), &["init"]);
    run_git(temp.path(), &["config", "user.email", "trail@example.test"]);
    run_git(temp.path(), &["config", "user.name", "Trail Test"]);
    fs::write(temp.path().join("target.md"), "target\n").unwrap();
    std::os::unix::fs::symlink("target.md", temp.path().join("link.md")).unwrap();
    run_git(temp.path(), &["add", "target.md", "link.md"]);
    run_git(temp.path(), &["commit", "-m", "initial"]);
    let link_tree_entry_before = git_output(temp.path(), &["ls-tree", "HEAD", "--", "link.md"]);
    assert!(
        link_tree_entry_before.starts_with("120000 blob "),
        "expected a Git symlink entry, got {link_tree_entry_before:?}"
    );

    let init = Trail::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    let root = db.inspect_root(&init.root_id.0).unwrap();
    assert!(!root.files.iter().any(|file| file.path == "link.md"));
    db.spawn_lane("apply-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "add readme",
        "edits": [{"op": "write", "path": "README.md", "content": "agent change"}]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "apply-bot", patch).unwrap();
    db.agent_mark_reviewed("apply-bot", None).unwrap();

    db.agent_apply("apply-bot", false, None).unwrap();

    assert_eq!(
        git_output(temp.path(), &["ls-tree", "HEAD", "--", "link.md"]),
        link_tree_entry_before
    );
    assert_eq!(
        git_output_raw(temp.path(), &["show", "HEAD:link.md"]),
        "target.md"
    );
    assert_eq!(
        git_output_raw(temp.path(), &["show", "HEAD:target.md"]),
        "target\n"
    );
    assert_eq!(
        git_output_raw(temp.path(), &["show", "HEAD:README.md"]),
        "agent change"
    );
}

#[test]
fn agent_apply_dry_run_writes_no_git_or_mapping_state() {
    if !git_available() {
        return;
    }

    let (temp, mut db) = ready_agent_lane_with_mode(InitImportMode::GitTracked);
    let head_before = git_output(temp.path(), &["rev-parse", "HEAD"]);
    let index_before = fs::read(temp.path().join(".git/index")).unwrap();
    let mappings_before = db.git_mappings(100).unwrap().len();
    let objects_before = git_object_count(temp.path());

    let report = db.agent_apply("apply-bot", true, None).unwrap();
    assert!(report.dry_run);

    assert_eq!(git_output(temp.path(), &["rev-parse", "HEAD"]), head_before);
    assert_eq!(
        fs::read(temp.path().join(".git/index")).unwrap(),
        index_before
    );
    assert_eq!(db.git_mappings(100).unwrap().len(), mappings_before);
    assert_eq!(git_object_count(temp.path()), objects_before);
}

fn only_conflict_path_class(db: &Trail) -> (String, String) {
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

fn run_trail_json(workspace: &Path, args: &[&str]) -> serde_json::Value {
    let output = Command::new(trail_bin())
        .arg("--workspace")
        .arg(workspace)
        .arg("--json")
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "trail {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn make_current_branch_root_legacy(workspace: &Path) -> ObjectId {
    let sqlite_path = workspace.join(".trail/index/trail.sqlite");
    let conn = Connection::open(sqlite_path).unwrap();
    let root_id: String = conn
        .query_row(
            "SELECT root_id FROM refs WHERE name = 'refs/branches/main'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let (version, bytes): (i64, Vec<u8>) = conn
        .query_row(
            "SELECT version, bytes FROM objects WHERE object_id = ?1 AND kind = ?2",
            rusqlite::params![root_id, WORKTREE_ROOT_KIND],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let mut root: WorktreeRoot = serde_cbor::from_slice(&bytes).unwrap();
    assert!(root.file_count > 0);
    assert!(root.case_fold_map_root.take().is_some());
    let legacy_bytes = serde_cbor::to_vec(&root).unwrap();
    let legacy_root = ObjectId::for_bytes(
        WORKTREE_ROOT_KIND,
        u16::try_from(version).unwrap(),
        &legacy_bytes,
    );
    conn.execute(
        "INSERT INTO objects (object_id, kind, version, codec, hash_alg, size_bytes, bytes, created_at) \
         VALUES (?1, ?2, ?3, 'cbor', 'sha256', ?4, ?5, 0)",
        rusqlite::params![
            legacy_root.0.as_str(),
            WORKTREE_ROOT_KIND,
            version,
            legacy_bytes.len() as i64,
            legacy_bytes
        ],
    )
    .unwrap();
    conn.execute(
        "UPDATE refs SET root_id = ?1 WHERE name = 'refs/branches/main'",
        rusqlite::params![legacy_root.0.as_str()],
    )
    .unwrap();
    legacy_root
}

fn run_trail_json_daemon(workspace: &Path, daemon_url: &str, args: &[&str]) -> serde_json::Value {
    let output = Command::new(trail_bin())
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
        "trail --daemon-url {:?} failed\nstdout:\n{}\nstderr:\n{}",
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
    for _ in 0..400 {
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
) -> (tempfile::TempDir, Trail, String) {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
    db.enqueue_lane_merge("manual-bot", "main", 0).unwrap();
    let run = db.run_lane_merge_queue(None).unwrap();
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

    let parse_output = Command::new(trail_bin())
        .arg("--json")
        .arg("definitely-not-a-command")
        .output()
        .unwrap();
    assert!(!parse_output.status.success());
    assert_eq!(parse_output.status.code(), Some(2));
    assert!(parse_output.stdout.is_empty());
    let parse_stderr: serde_json::Value = serde_json::from_slice(&parse_output.stderr).unwrap();
    assert_eq!(parse_stderr["error"]["code"], "INVALID_INPUT");
    assert_eq!(parse_stderr["error"]["exit"], 2);
    assert!(parse_stderr["error"]["message"]
        .as_str()
        .unwrap()
        .contains("definitely-not-a-command"));

    let output = Command::new(trail_bin())
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
    assert_eq!(stderr["error"]["exit"], 3);
    assert!(stderr["error"]["message"]
        .as_str()
        .unwrap()
        .contains("workspace not found"));

    let format_output = Command::new(trail_bin())
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

    let env_parse_output = Command::new(trail_bin())
        .env("TRAIL_FORMAT", "json")
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
fn cli_path_index_required_human_json_rebuild_and_retry_lifecycle() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "base\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let legacy_root = make_current_branch_root_legacy(temp.path());

    let status = run_trail_json(temp.path(), &["status"]);
    assert_eq!(status["head"]["root_id"], legacy_root.0);
    assert_eq!(status["worktree_state"], "Clean");
    fs::write(temp.path().join("README.md"), "changed\n").unwrap();

    let human = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .args(["record", "-m", "blocked before upgrade"])
        .output()
        .unwrap();
    assert_eq!(human.status.code(), Some(9));
    let human_stderr = String::from_utf8(human.stderr).unwrap();
    assert!(
        human_stderr.contains("Trail workspace upgrade is required"),
        "{human_stderr}"
    );
    assert!(
        human_stderr.contains("trail index rebuild"),
        "{human_stderr}"
    );
    assert!(!human_stderr.contains("trail --help"), "{human_stderr}");

    let json = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("--json")
        .args(["record", "-m", "blocked before upgrade"])
        .output()
        .unwrap();
    assert_eq!(json.status.code(), Some(9));
    assert!(json.stdout.is_empty());
    let json_error: serde_json::Value = serde_json::from_slice(&json.stderr).unwrap();
    assert_eq!(json_error["error"]["code"], "PATH_INDEX_REQUIRED");
    assert_eq!(json_error["error"]["exit"], 9);
    assert!(json_error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("trail index rebuild"));

    let rebuilt = run_trail_json(temp.path(), &["index", "rebuild"]);
    assert_eq!(
        rebuilt["path_index_repaired_roots"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        rebuilt["path_index_repaired_refs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        rebuilt["path_index_repaired_refs"][0]["old_root"],
        legacy_root.0
    );
    let repaired_root = rebuilt["path_index_repaired_refs"][0]["new_root"]
        .as_str()
        .unwrap()
        .to_string();
    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
    let load_root = |root_id: &str| -> WorktreeRoot {
        let bytes: Vec<u8> = conn
            .query_row(
                "SELECT bytes FROM objects WHERE object_id = ?1",
                rusqlite::params![root_id],
                |row| row.get(0),
            )
            .unwrap();
        serde_cbor::from_slice(&bytes).unwrap()
    };
    let legacy = load_root(&legacy_root.0);
    let repaired = load_root(&repaired_root);
    assert_eq!(repaired.path_map_root, legacy.path_map_root);
    assert_eq!(repaired.file_index_map_root, legacy.file_index_map_root);
    assert_eq!(repaired.file_count, legacy.file_count);
    assert_eq!(repaired.total_text_bytes, legacy.total_text_bytes);
    assert_eq!(repaired.created_by, legacy.created_by);
    assert!(repaired.case_fold_map_root.is_some());
    drop(conn);

    let recorded = run_trail_json(temp.path(), &["record", "-m", "after upgrade"]);
    assert_eq!(recorded["changed_paths"].as_array().unwrap().len(), 1);
    let second = run_trail_json(temp.path(), &["index", "rebuild"]);
    assert!(second["path_index_repaired_roots"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(second["path_index_repaired_refs"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[cfg(unix)]
#[test]
fn acp_doctor_and_relay_preflight_report_legacy_path_index() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "base\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    make_current_branch_root_legacy(temp.path());

    let doctor = run_trail_json(temp.path(), &["agent", "acp", "doctor", "claude-code"]);
    assert_eq!(doctor["status"], "failed");
    assert!(doctor["checks"].as_array().unwrap().iter().any(|check| {
        check["name"] == "path_invariant_index"
            && check["status"] == "failed"
            && check["message"]
                .as_str()
                .is_some_and(|message| message.contains("trail index rebuild"))
    }));

    let relay = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("--json")
        .args(["acp", "relay", "--", "/bin/true"])
        .output()
        .unwrap();
    assert_eq!(relay.status.code(), Some(9));
    assert!(relay.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&relay.stderr).unwrap();
    assert_eq!(error["error"]["code"], "PATH_INDEX_REQUIRED");
    assert!(error["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("trail index rebuild")));
}

#[test]
fn cli_env_defaults_select_workspace_db_branch_and_format() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    db.create_branch("scratch", Some("main")).unwrap();
    drop(db);

    let workspace_output = Command::new(trail_bin())
        .env("TRAIL_WORKSPACE", temp.path())
        .env_remove("TRAIL_DIR")
        .env("TRAIL_FORMAT", "json")
        .env("TRAIL_BRANCH", "scratch")
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

    let local_branch_status = run_trail_json(temp.path(), &["status", "--branch", "scratch"]);
    assert_eq!(local_branch_status["branch"], "scratch");

    let db_dir_output = Command::new(trail_bin())
        .env_remove("TRAIL_WORKSPACE")
        .env_remove("TRAIL_BRANCH")
        .env("TRAIL_DIR", temp.path().join(".trail"))
        .env("TRAIL_FORMAT", "json")
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

    let invalid_format = Command::new(trail_bin())
        .env("TRAIL_WORKSPACE", temp.path())
        .env_remove("TRAIL_DIR")
        .env_remove("TRAIL_BRANCH")
        .env("TRAIL_FORMAT", "xml")
        .arg("status")
        .output()
        .unwrap();
    assert!(!invalid_format.status.success());
    assert!(String::from_utf8_lossy(&invalid_format.stderr)
        .contains("TRAIL_FORMAT must be human, plain, json, or ndjson"));
}

#[test]
fn ndjson_rejects_single_report_commands_with_a_structured_diagnostic() {
    let output = Command::new(trail_bin())
        .args(["--format", "ndjson", "status"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "INVALID_INPUT");
    assert!(error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("streaming watch commands"));
}

#[test]
fn plain_redirected_and_quiet_output_follow_terminal_policy() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let plain = Command::new(trail_bin())
        .args([
            "--workspace",
            temp.path().to_str().unwrap(),
            "--format",
            "plain",
            "--color",
            "always",
            "status",
        ])
        .output()
        .unwrap();
    assert!(plain.status.success());
    let plain_stdout = String::from_utf8(plain.stdout).unwrap();
    assert!(plain_stdout.contains("Worktree clean"));
    assert!(!plain_stdout.contains("\u{1b}["));

    let quiet = Command::new(trail_bin())
        .args([
            "--workspace",
            temp.path().to_str().unwrap(),
            "--quiet",
            "status",
        ])
        .output()
        .unwrap();
    assert!(quiet.status.success());
    assert!(quiet.stdout.is_empty());
    assert!(quiet.stderr.is_empty());
}

#[test]
fn clap_diagnostics_keep_usage_on_separate_lines() {
    let output = Command::new(trail_bin())
        .args(["agent", "hook", "receive"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("the following required arguments were not provided:\n  <PROVIDER>"));
    assert!(stderr.contains("\nUsage: trail agent hook receive <PROVIDER> <NATIVE_EVENT>\n"));
    assert!(!stderr.contains(r"\nUsage:"));
}

#[test]
fn agent_help_is_curated_and_hidden_commands_still_work() {
    let help = Command::new(trail_bin())
        .args(["agent", "--help"])
        .output()
        .unwrap();
    assert!(
        help.status.success(),
        "agent --help failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&help.stdout),
        String::from_utf8_lossy(&help.stderr)
    );
    let stdout = String::from_utf8_lossy(&help.stdout);
    assert!(stdout.contains("Daily path:"));
    assert!(stdout.contains("trail agent ask what should I do"));
    assert!(stdout.contains("  action"));
    assert!(stdout.contains("  changes"));
    assert!(stdout.contains("  acp"));
    assert!(stdout.contains("  hooks"));
    assert!(!stdout.contains("  setup "));
    assert!(!stdout.contains("review-data"));
    assert!(!stdout.contains("turn-diff"));

    let acp_help = Command::new(trail_bin())
        .args(["agent", "acp", "--help"])
        .output()
        .unwrap();
    assert!(acp_help.status.success());
    let acp_stdout = String::from_utf8_lossy(&acp_help.stdout);
    assert!(acp_stdout.contains("  setup"));
    assert!(acp_stdout.contains("  status"));
    assert!(!acp_stdout.contains("  run"));

    let hidden_help = Command::new(trail_bin())
        .args(["agent", "review-data", "--help"])
        .output()
        .unwrap();
    assert!(
        hidden_help.status.success(),
        "agent review-data --help failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&hidden_help.stdout),
        String::from_utf8_lossy(&hidden_help.stderr)
    );
    assert!(String::from_utf8_lossy(&hidden_help.stdout).contains("review-data"));
}

#[test]
fn init_record_why_and_fsck_work() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();

    let init = Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    assert_eq!(init.imported.files, 1);
    assert!(init.workspace_id.0.starts_with("workspace_"));
    assert!(init.operation.0.starts_with("change_"));
    assert!(init.root_id.0.starts_with("object_"));

    fs::write(temp.path().join("README.md"), "hello\nTrail\n").unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    let record = db
        .record(
            Some("main"),
            Some("edit readme".to_string()),
            Actor::human(),
            false,
        )
        .unwrap();
    assert!(record.operation.is_some());
    assert!(record.operation.as_ref().unwrap().0.starts_with("change_"));
    assert!(record.root_id.0.starts_with("object_"));
    assert_eq!(record.changed_paths.len(), 1);

    let why = db.why("README.md:2", Some("main")).unwrap();
    assert_eq!(why.current_text, "Trail");
    assert_eq!(why.history.len(), 1);

    let fsck = db.fsck().unwrap();
    assert!(fsck.errors.is_empty(), "{:?}", fsck.errors);
}

#[test]
fn doctor_reports_operational_health_across_cli_api_and_mcp() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    {
        let db = Trail::open(temp.path()).unwrap();
        let clean = db.doctor().unwrap();
        assert_eq!(clean.status, "ok");
        assert!(clean
            .checks
            .iter()
            .any(|check| check.name == "fsck" && check.status == "ok"));
        assert!(clean.checks.iter().any(|check| {
            check.name == "schema_version"
                && check.status == "ok"
                && check.details.as_ref().unwrap()["sqlite_user_version"] == 18
        }));
    }

    let cli = run_trail_json(temp.path(), &["doctor"]);
    assert_eq!(cli["status"], "ok");
    assert!(cli["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|check| check["name"] == "current_branch" && check["status"] == "ok"));

    let mut db = Trail::open(temp.path()).unwrap();
    let api = trail::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/doctor", serde_json::Value::Null),
    );
    assert_eq!(api.status, 200);
    let api: serde_json::Value = api.body_json().unwrap();
    assert_eq!(api["status"], "ok");

    let tools = trail::mcp::handle_json_rpc(
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
    assert!(tool_list.iter().any(|tool| tool["name"] == "trail.doctor"));

    let mcp = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "trail.doctor",
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
fn environment_component_report_keeps_component_and_adapter_identities_separate_json() {
    let report = trail::EnvironmentComponentStateReport {
        view_id: "view-1".to_string(),
        component: trail::EnvironmentComponentIdentityReport {
            component_id: "web.dependencies".to_string(),
            kind: "dependency".to_string(),
        },
        adapter: trail::EnvironmentAdapterIdentityReport {
            namespace: "trail".to_string(),
            name: "node".to_string(),
            contract_major: 1,
            implementation_version: "0.5.0".to_string(),
            distribution_digest: Some("builtin".to_string()),
        },
        expected_key: "expected".to_string(),
        attached_key: Some("attached".to_string()),
        status: "stale".to_string(),
        reason: Some("lock changed".to_string()),
        updated_at: 7,
    };

    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["component"]["component_id"], "web.dependencies");
    assert_eq!(json["adapter"]["namespace"], "trail");
    assert_eq!(json["adapter"]["contract_major"], 1);
    assert!(json["component"].get("adapter").is_none());
}

#[test]
fn environment_component_report_keeps_component_and_adapter_identities_separate() {
    let report = trail::EnvironmentComponentStateReport {
        view_id: "view-1".to_string(),
        component: trail::EnvironmentComponentIdentityReport {
            component_id: "web.dependencies".to_string(),
            kind: "dependency".to_string(),
        },
        adapter: trail::EnvironmentAdapterIdentityReport {
            namespace: "trail".to_string(),
            name: "node".to_string(),
            contract_major: 1,
            implementation_version: "0.5.0".to_string(),
            distribution_digest: Some("builtin".to_string()),
        },
        expected_key: "expected".to_string(),
        attached_key: Some("attached".to_string()),
        status: "stale".to_string(),
        reason: Some("lock changed".to_string()),
        updated_at: 7,
    };

    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["component"]["component_id"], "web.dependencies");
    assert_eq!(json["adapter"]["namespace"], "trail");
    assert_eq!(json["adapter"]["contract_major"], 1);
    assert!(json["component"].get("adapter").is_none());
}

#[test]
fn environment_plugin_publisher_trust_cli_is_append_only_and_catalogued() {
    let workspace = tempfile::tempdir().unwrap();
    fs::write(workspace.path().join("README.md"), "root\n").unwrap();
    Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let key = workspace.path().join("publisher-key.toml");
    fs::write(
        &key,
        r#"schema = "trail.environment-adapter-publisher-key/v1"
publisher = "rfc8032-test"
public_key = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
"#,
    )
    .unwrap();
    let key_path = key.to_string_lossy().into_owned();

    let added = run_trail_json(
        workspace.path(),
        &["env", "plugin", "trust", "add", &key_path],
    );
    assert_eq!(added["action"], "trust");
    assert_eq!(added["publisher"], "rfc8032-test");
    let key_id = added["key_id"].as_str().unwrap().to_string();
    assert!(key_id.starts_with("sha256:"));

    let listed = run_trail_json(workspace.path(), &["env", "plugin", "trust", "list"]);
    assert_eq!(listed["keys"].as_array().unwrap().len(), 1);
    assert_eq!(listed["keys"][0]["key_id"], key_id);

    let catalog = run_trail_json(workspace.path(), &["env", "adapters"]);
    assert!(catalog["adapters"].as_array().unwrap().iter().all(|entry| {
        entry["trust"] == "builtin"
            && entry["publisher"] == "trail"
            && entry["certification_tier"].as_str().is_some()
    }));

    let removed = run_trail_json(
        workspace.path(),
        &["env", "plugin", "trust", "remove", &key_id],
    );
    assert_eq!(removed["action"], "remove");
    assert_eq!(removed["key_id"], key_id);
    let listed = run_trail_json(workspace.path(), &["env", "plugin", "trust", "list"]);
    assert!(listed["keys"].as_array().unwrap().is_empty());
}

#[test]
fn trail_refuses_workspaces_with_newer_schema_versions() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
    conn.execute_batch("PRAGMA user_version = 999;").unwrap();
    drop(conn);

    let err = match Trail::open(temp.path()) {
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
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
        "lane_acp_sessions_trail_session_idx",
        "external_mutation_audit_created_idx",
        "external_mutation_audit_surface_created_idx",
        "external_mutation_audit_lane_created_idx",
        "http_idempotency_keys_updated_idx",
        "conflict_resolution_suggestions_signature_idx",
    ] {
        assert!(
            indexes.iter().any(|index| index == expected),
            "missing index {expected}"
        );
    }
}

#[test]
fn agent_acp_doctor_reports_status_setup_and_verifiable_evidence() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let profiles = run_trail_json(temp.path(), &["agent", "acp", "status"]);
    assert!(profiles
        .as_array()
        .unwrap()
        .iter()
        .any(|profile| profile["agent"] == "claude-code"));
    assert!(profiles
        .as_array()
        .unwrap()
        .iter()
        .any(|profile| profile["agent"] == "codex"));
    assert!(profiles
        .as_array()
        .unwrap()
        .iter()
        .any(|profile| profile["agent"] == "cursor"
            && profile["supports_acp"] == true
            && profile["supports_terminal"] == true));
    assert!(!profiles
        .as_array()
        .unwrap()
        .iter()
        .any(|profile| profile["agent"] == "fake"));

    let setup = run_trail_json(
        temp.path(),
        &[
            "agent",
            "acp",
            "setup",
            "claude-code",
            "--editor",
            "generic",
            "--print",
        ],
    );
    assert_eq!(setup["transport"], "acp");
    assert_eq!(setup["provider"], "claude-code");
    assert_eq!(setup["editor"], "generic");
    assert!(setup["snippet"]
        .as_str()
        .unwrap()
        .contains("agent acp run claude-code"));
    assert!(setup["command"]
        .as_array()
        .unwrap()
        .iter()
        .any(|part| part == "run"));

    let doctor = run_trail_json(temp.path(), &["agent", "acp", "doctor", "claude-code"]);
    let checks = doctor["checks"].as_array().unwrap();
    assert!(checks
        .iter()
        .any(|check| check["name"] == "workspace" && check["status"] == "ok"));
    assert!(checks
        .iter()
        .any(|check| { check["name"] == "capture_journal" && check["status"] == "ok" }));
    assert!(checks
        .iter()
        .any(|check| check["name"] == "path_mapping" && check["status"] == "ok"));
    assert_eq!(doctor["conformance"]["wire_version"], 1);
    assert_eq!(
        doctor["conformance"]["schema_commit"],
        "64cbd71ae520b89aac54164d8c1d364333c8ee5f"
    );
    assert_eq!(
        doctor["conformance"]["schema_sha256"],
        "92c1dfcda10dd47e99127500a3763da2b471f9ac61e12b9bf0430c32cf953796"
    );
    assert_eq!(
        doctor["conformance"]["meta_sha256"],
        "e0bf36f8123b2544b499174197fdc371ec49a1b4572a35114513d56492741599"
    );
    assert_eq!(doctor["conformance"]["transport"], "stdio");
    assert_eq!(doctor["conformance"]["method_count"], 23);
    let source_revision = option_env!("TRAIL_SOURCE_REVISION")
        .filter(|revision| !revision.is_empty())
        .unwrap_or("unverified");
    let expected_evidence = if source_revision != "unverified"
        && option_env!("TRAIL_ACP_V1_CONFORMANCE_VERIFIED") == Some(source_revision)
    {
        "verified"
    } else {
        "unverified"
    };
    assert_eq!(doctor["conformance"]["evidence_status"], expected_evidence);
    assert!(doctor["conformance"]["build_identifier"]
        .as_str()
        .unwrap()
        .starts_with(concat!(env!("CARGO_PKG_VERSION"), "+")));
    assert_eq!(
        doctor["conformance"]["exclusions"],
        serde_json::json!(["ACP v2", "draft remote HTTP transport"])
    );
    assert!(doctor["lane"].is_null());
    assert!(doctor["session_id"].is_null());

    let human = Command::new(trail_bin())
        .current_dir(temp.path())
        .args(["agent", "acp", "doctor", "claude-code"])
        .output()
        .unwrap();
    assert!(human.status.success());
    let human = String::from_utf8_lossy(&human.stdout);
    for expected in [
        "Wire version",
        "Schema commit",
        "Schema SHA-256",
        "Metadata SHA-256",
        "Transport",
        "Evidence",
        "ACP v2",
        "draft remote HTTP transport",
        "capture_journal",
        "path_mapping",
    ] {
        assert!(
            human.contains(expected),
            "missing `{expected}` in:\n{human}"
        );
    }
    if expected_evidence == "unverified" {
        assert!(!human.contains("ACP v1 conformant"));
    } else {
        assert!(human.contains("ACP v1 conformant"));
    }

    let custom_doctor = run_trail_json(
        temp.path(),
        &[
            "agent",
            "acp",
            "doctor",
            "custom-acp",
            "--relay-command",
            "trail",
            "acp",
            "relay",
            "--",
            "<custom-acp>",
        ],
    );
    assert_eq!(custom_doctor["provider"], "custom-acp");
    assert!(custom_doctor["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|check| check["name"] == "relay" && check["status"] == "ok"));

    let codex = run_trail_json(temp.path(), &["agent", "acp", "status", "codex"]);
    assert_eq!(codex.as_array().unwrap().len(), 1);
    assert_eq!(codex[0]["agent"], "codex");
}

#[test]
fn agent_hooks_setup_is_read_only_until_yes_is_passed() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let config_path = temp.path().join(".codex/hooks.json");
    let preview = run_trail_json(
        temp.path(),
        &["agent", "hooks", "setup", "codex", "--print"],
    );
    assert_eq!(preview["provider"], "codex");
    assert_eq!(preview["dry_run"], true);
    assert!(!config_path.exists());

    let applied = run_trail_json(temp.path(), &["agent", "hooks", "setup", "codex", "--yes"]);
    assert_eq!(applied["provider"], "codex");
    assert_eq!(applied["dry_run"], false);
    assert!(config_path.is_file());
}

#[cfg(unix)]
#[test]
fn acp_relay_accepts_a_built_in_agent_shortcut() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let output = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .args(["acp", "relay", "codex", "--", "/usr/bin/true"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "trail acp relay codex failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn agent_acp_setup_and_hidden_runner_use_fresh_lanes() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let setup = run_trail_json(
        temp.path(),
        &[
            "agent",
            "acp",
            "setup",
            "claude-code",
            "--editor",
            "vscode",
            "--print",
        ],
    );
    assert_eq!(setup["transport"], "acp");
    assert_eq!(setup["provider"], "claude-code");
    assert_eq!(setup["editor"], "vscode");
    assert_eq!(setup["applied"], false);
    assert!(setup["command"]
        .as_array()
        .unwrap()
        .windows(3)
        .any(|parts| parts == ["acp", "run", "claude-code"]));

    let gemini_doctor = run_trail_json(temp.path(), &["agent", "doctor", "gemini"]);
    assert_eq!(gemini_doctor["provider"], "gemini");
    assert_eq!(gemini_doctor["capabilities"]["terminal"], true);
    assert_eq!(gemini_doctor["capabilities"]["mcp"], true);

    {
        let mut db = Trail::open(temp.path()).unwrap();
        db.spawn_lane("manual-low-level", Some("main"), false, None, None)
            .unwrap();
    }
    let empty_list = run_trail_json(temp.path(), &["agent", "list"]);
    assert_eq!(empty_list["tasks"].as_array().unwrap().len(), 0);
    let empty_status = run_trail_json(temp.path(), &["agent", "status"]);
    assert_eq!(empty_status["status"], "empty");
    let empty_next = run_trail_json(temp.path(), &["agent", "next"]);
    assert_eq!(empty_next["focus"], "setup");
    assert_eq!(
        empty_next["primary"]["command"],
        "trail agent acp setup claude-code --editor vscode"
    );
    let empty_actions = run_trail_json(temp.path(), &["agent", "action"]);
    assert_eq!(empty_actions["status"], "empty", "{empty_actions}");
    assert!(empty_actions["task"].is_null());
    assert_eq!(
        empty_actions["next"]["command"],
        "trail agent acp setup claude-code --editor vscode"
    );
    assert!(empty_actions["actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action["id"] == "acp_setup_vscode"
            && action["command"] == "trail agent acp setup claude-code --editor vscode"));
    assert!(empty_actions["actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action["id"] == "acp_setup_codex_vscode"
            && action["command"] == "trail agent acp setup codex --editor vscode"));
    assert!(empty_actions["actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action["id"] == "doctor_codex"
            && action["command"] == "trail agent doctor codex"));
    assert!(empty_actions["actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action["id"] == "acp_setup_cursor_vscode"
            && action["command"] == "trail agent acp setup cursor --editor vscode"));
    assert!(empty_actions["actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action["id"] == "start_terminal_task"
            && action["requires_confirmation"] == true));
    assert!(empty_actions["actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action["id"] == "start_gemini_task"
            && action["command"] == "trail agent start gemini"));
    let empty_ask_actions = run_trail_json(temp.path(), &["agent", "ask", "show", "actions"]);
    assert_eq!(empty_ask_actions["status"], "empty");
    assert!(empty_ask_actions["task"].is_null());
    assert!(empty_ask_actions["actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action["id"] == "acp_setup_vscode"));
    let empty_setup_action = run_trail_json(temp.path(), &["agent", "action", "acp_setup_vscode"]);
    assert_eq!(empty_setup_action["provider"], "claude-code");
    assert_eq!(empty_setup_action["editor"], "vscode");
    assert!(empty_setup_action["command"]
        .as_array()
        .unwrap()
        .iter()
        .any(|part| part == "agent" || part == "acp"));
    let empty_doctor_action =
        run_trail_json(temp.path(), &["agent", "action", "doctor_claude_code"]);
    assert_eq!(empty_doctor_action["provider"], "claude-code");
    assert!(empty_doctor_action["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|check| check["name"] == "workspace" && check["status"] == "ok"));
    let empty_start_print = run_trail_json(
        temp.path(),
        &["agent", "action", "start_terminal_task", "--print"],
    );
    assert_eq!(empty_start_print["status"], "empty");
    assert!(empty_start_print["task"].is_null());
    assert_eq!(empty_start_print["action"]["id"], "start_terminal_task");
    assert!(empty_start_print["action"]["command"]
        .as_str()
        .unwrap()
        .contains("agent start"));
    let empty_start_guard = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("--json")
        .args(["agent", "action", "start_terminal_task"])
        .output()
        .unwrap();
    assert!(!empty_start_guard.status.success());
    let empty_start_stderr: serde_json::Value =
        serde_json::from_slice(&empty_start_guard.stderr).unwrap();
    assert!(empty_start_stderr["error"]["message"]
        .as_str()
        .unwrap()
        .contains("requires --confirm"));
    for (command, requested) in [
        (vec!["agent", "view", "latest"], "view"),
        (vec!["agent", "changes", "latest"], "changes"),
        (vec!["agent", "apply", "latest", "--dry-run"], "apply"),
    ] {
        let empty_hint = run_trail_json(temp.path(), &command);
        assert_eq!(empty_hint["status"], "empty");
        assert!(empty_hint["task"].is_null());
        assert_eq!(empty_hint["requested"], requested);
        assert!(empty_hint["summary"]
            .as_str()
            .unwrap()
            .contains("No agent task is recorded yet"));
        assert!(!empty_hint["summary"]
            .as_str()
            .unwrap()
            .contains("to changes"));
        assert_eq!(
            empty_hint["next"]["command"],
            "trail agent acp setup claude-code --editor vscode"
        );
        assert!(empty_hint["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["id"] == "acp_setup_vscode"));
    }
    let empty_dashboard = run_trail_json(temp.path(), &["agent", "dashboard"]);
    assert_eq!(empty_dashboard["status"], "empty");
    assert!(empty_dashboard["task"].is_null());
    assert_eq!(
        empty_dashboard["next"]["command"],
        "trail agent acp setup claude-code --editor vscode"
    );
    let empty_inbox = run_trail_json(temp.path(), &["agent", "inbox"]);
    assert_eq!(empty_inbox["total"], 0);
    assert_eq!(empty_inbox["attention_count"], 0);
    assert_eq!(empty_inbox["items"].as_array().unwrap().len(), 0);
    assert_eq!(
        empty_inbox["next"]["command"],
        "trail agent acp setup claude-code --editor vscode"
    );
    let empty_bare_agent = run_trail_json(temp.path(), &["agent"]);
    assert_eq!(empty_bare_agent["total"], 0);
    assert_eq!(
        empty_bare_agent["next"]["command"],
        "trail agent acp setup claude-code --editor vscode"
    );

    let doctor = run_trail_json(
        temp.path(),
        &["agent", "doctor", "--provider", "claude-code"],
    );
    assert_eq!(doctor["provider"], "claude-code");
    assert!(doctor["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|check| { check["name"] == "workspace" && check["status"] == "ok" }));
    assert_eq!(
        doctor["suggestions"][0]["command"],
        "trail agent acp setup claude-code --editor vscode"
    );

    let first_agent = write_stub_acp_agent(
        temp.path(),
        "agent-acp-stub-a.sh",
        StubAcpAgentOptions::new("sess_agent_stub_a"),
    );
    let second_agent = write_stub_acp_agent(
        temp.path(),
        "agent-acp-stub-b.sh",
        StubAcpAgentOptions::new("sess_agent_stub_b"),
    );
    run_agent_acp_stub_session(temp.path(), &first_agent);
    let one_task_home = run_trail_json(temp.path(), &["agent"]);
    assert!(one_task_home["task"]["name"]
        .as_str()
        .unwrap()
        .starts_with("agent-claude-code-"));
    assert_eq!(one_task_home["focus"]["path"], "README.md");
    run_agent_acp_stub_session(temp.path(), &second_agent);

    let list = run_trail_json(temp.path(), &["agent", "list"]);
    let tasks = list["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 2);
    let first = tasks[0]["name"].as_str().unwrap();
    let second = tasks[1]["name"].as_str().unwrap();
    assert_ne!(first, second);
    assert!(first.starts_with("agent-claude-code-"));
    assert!(second.starts_with("agent-claude-code-"));
    assert!(tasks[0]["title"].as_str().unwrap().contains("edit README"));

    let inbox = run_trail_json(temp.path(), &["agent", "inbox"]);
    assert_eq!(inbox["total"], 2);
    assert_eq!(inbox["attention_count"], 2);
    assert_eq!(inbox["groups"].as_array().unwrap().len(), 1);
    assert_eq!(inbox["groups"][0]["key"], "ready");
    assert_eq!(inbox["groups"][0]["label"], "Ready to apply");
    assert_eq!(inbox["groups"][0]["tasks"].as_array().unwrap().len(), 2);
    assert_eq!(inbox["groups"][0]["tasks"][0]["name"], first);
    assert_eq!(inbox["items"].as_array().unwrap().len(), 2);
    assert_eq!(inbox["items"][0]["task"]["name"], first);
    assert_eq!(inbox["items"][0]["attention"], "unreviewed");
    assert!(inbox["items"][0]["detail"]
        .as_str()
        .unwrap()
        .contains("have not been reviewed"));
    assert_eq!(inbox["items"][0]["new_changed_paths"], 1);
    assert!(inbox["items"][0]["new_changed_lines"].as_u64().unwrap() > 0);
    assert_eq!(inbox["items"][0]["review_first"]["path"], "README.md");
    assert!(inbox["items"][0]["review_first"]["command"]
        .as_str()
        .unwrap()
        .contains("agent focus"));
    assert_eq!(inbox["groups"][0]["items"].as_array().unwrap().len(), 2);
    assert_eq!(inbox["groups"][0]["items"][0]["attention"], "unreviewed");
    assert_eq!(
        inbox["groups"][0]["next"]["command"],
        format!("trail agent new {first}")
    );
    assert_eq!(inbox["next"]["command"], format!("trail agent new {first}"));
    let board = run_trail_json(temp.path(), &["agent", "board"]);
    assert_eq!(board["total"], 2);
    assert_eq!(board["attention_count"], 2);
    assert_eq!(board["ready_count"], 2);
    assert_eq!(board["columns"].as_array().unwrap().len(), 1);
    assert_eq!(board["columns"][0]["key"], "needs_review");
    assert_eq!(board["columns"][0]["items"].as_array().unwrap().len(), 2);
    assert_eq!(board["columns"][0]["items"][0]["task"]["name"], first);
    assert_eq!(
        board["columns"][0]["next"]["command"],
        format!("trail agent new {first}")
    );
    assert_eq!(board["next"]["command"], format!("trail agent new {first}"));
    let stack = run_trail_json(temp.path(), &["agent", "stack"]);
    assert_eq!(stack["total"], 2);
    assert_eq!(stack["ready_count"], 2);
    assert_eq!(stack["overlap_count"], 1);
    assert_eq!(stack["shared_paths"][0]["path"], "README.md");
    assert!(stack["shared_paths"][0]["lanes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|lane| lane == first));
    assert!(stack["shared_paths"][0]["lanes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|lane| lane == second));
    assert_eq!(stack["apply_order"].as_array().unwrap().len(), 0);
    assert_eq!(stack["items"][0]["status"], "overlap_review");
    assert!(stack["next"]["command"]
        .as_str()
        .unwrap()
        .contains("agent compare"));
    let order_alias = run_trail_json(temp.path(), &["agent", "order"]);
    assert_eq!(order_alias["next"], stack["next"]);
    let multi_task_home = run_trail_json(temp.path(), &["agent"]);
    assert_eq!(multi_task_home["total"], 2);
    assert_eq!(multi_task_home["next"]["command"], inbox["next"]["command"]);
    let ask_board = run_trail_json(temp.path(), &["agent", "ask", "show", "agent", "board"]);
    assert_eq!(ask_board["total"], board["total"]);
    assert_eq!(ask_board["columns"][0]["key"], "needs_review");
    let ask_stack = run_trail_json(temp.path(), &["agent", "ask", "which", "task", "first"]);
    assert_eq!(ask_stack["total"], stack["total"]);
    assert_eq!(ask_stack["next"], stack["next"]);
    let ask_inbox = run_trail_json(temp.path(), &["agent", "ask", "what", "needs", "attention"]);
    assert_eq!(ask_inbox["total"], inbox["total"]);
    assert_eq!(ask_inbox["attention_count"], inbox["attention_count"]);
    assert_eq!(ask_inbox["items"][0]["attention"], "unreviewed");
    assert_eq!(ask_inbox["items"][0]["review_first"]["path"], "README.md");
    let home = run_trail_json(temp.path(), &["agent", "home"]);
    assert_eq!(home["total"], inbox["total"]);
    assert_eq!(home["next"]["command"], inbox["next"]["command"]);

    let status = run_trail_json(temp.path(), &["agent", "status"]);
    assert!(status["latest"]["name"]
        .as_str()
        .unwrap()
        .starts_with("agent-claude-code-"));
    assert!(status["latest"]["title"]
        .as_str()
        .unwrap()
        .contains("edit README"));
    assert_eq!(status["risk"]["level"], "medium");
    assert!(status["risk"]["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|reason| reason["code"] == "missing_latest_test"));
    let bare_agent = run_trail_json(temp.path(), &["agent"]);
    assert_eq!(bare_agent["total"], inbox["total"]);
    assert_eq!(bare_agent["attention_count"], inbox["attention_count"]);
    assert_eq!(bare_agent["groups"][0]["key"], "ready");
    assert_eq!(
        bare_agent["next"]["command"],
        format!("trail agent new {first}")
    );
    let view = run_trail_json(temp.path(), &["agent", "view", "latest"]);
    assert_eq!(view["transcript"]["turns"].as_array().unwrap().len(), 1);
    assert!(view["task"]["tool_events"].as_u64().unwrap() > 0);
    assert!(view["task"]["title"]
        .as_str()
        .unwrap()
        .contains("edit README"));
    let agent_tools = run_trail_json(temp.path(), &["agent", "tools", "latest"]);
    assert_eq!(agent_tools["task"]["lane"], view["task"]["lane"]);
    assert!(agent_tools["total_tool_events"].as_u64().unwrap() > 0);
    assert!(agent_tools["unique_tools"].as_u64().unwrap() > 0);
    assert!(agent_tools["available_commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|command| command == "write_file"));
    assert!(agent_tools["tools"].as_array().unwrap().iter().any(|tool| {
        tool["name"] == "write README"
            && tool["turns"][0]["changed_paths"]
                .as_array()
                .unwrap()
                .iter()
                .any(|path| path["path"] == "README.md")
    }));
    let impact = run_trail_json(temp.path(), &["agent", "impact", "latest"]);
    assert_eq!(impact["task"]["lane"], view["task"]["lane"]);
    assert_eq!(impact["highest_impact"], "low");
    assert_eq!(impact["areas"][0]["key"], "docs");
    assert_eq!(impact["areas"][0]["changed_paths"][0]["path"], "README.md");
    assert_eq!(impact["validation"]["status"], "missing_test");
    assert!(impact["recommendations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|suggestion| suggestion["command"]
            .as_str()
            .unwrap()
            .contains("agent validate")
            || suggestion["command"]
                .as_str()
                .unwrap()
                .contains("agent test")));
    let ask_impact = run_trail_json(temp.path(), &["agent", "ask", "blast", "radius"]);
    assert_eq!(ask_impact["task"]["lane"], view["task"]["lane"]);
    assert_eq!(ask_impact["areas"], impact["areas"]);
    let ask_areas = run_trail_json(
        temp.path(),
        &["agent", "ask", "what", "areas", "did", "it", "touch"],
    );
    assert_eq!(ask_areas["task"]["lane"], view["task"]["lane"]);
    assert_eq!(ask_areas["highest_impact"], impact["highest_impact"]);
    let review_map = run_trail_json(temp.path(), &["agent", "review-map", "latest"]);
    assert_eq!(review_map["task"]["lane"], view["task"]["lane"]);
    assert_eq!(review_map["review_status"], "unreviewed");
    assert_eq!(review_map["areas"][0]["key"], "docs");
    assert_eq!(review_map["areas"][0]["state"], "needs_review");
    assert_eq!(review_map["areas"][0]["files"][0]["path"], "README.md");
    assert!(review_map["areas"][0]["files"][0]["review_command"]
        .as_str()
        .unwrap()
        .contains("agent focus"));
    assert!(review_map["areas"][0]["files"][0]["why_command"]
        .as_str()
        .unwrap()
        .contains("agent why"));
    assert!(review_map["areas"][0]["files"][0]["reviewed"].is_null());
    let action_palette = run_trail_json(temp.path(), &["agent", "action"]);
    assert_eq!(action_palette["task"]["lane"], view["task"]["lane"]);
    assert!(action_palette["actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action["id"] == "inspect_focus_file"));
    let selected_action_palette = run_trail_json(temp.path(), &["agent", "action", "latest"]);
    assert_eq!(
        selected_action_palette["task"]["lane"],
        view["task"]["lane"]
    );
    assert_eq!(
        selected_action_palette["actions"],
        action_palette["actions"]
    );
    let action_print = run_trail_json(
        temp.path(),
        &["agent", "action", "latest", "show_focus_patch", "--print"],
    );
    assert_eq!(action_print["task"]["lane"], view["task"]["lane"]);
    assert_eq!(action_print["action"]["id"], "show_focus_patch");
    assert!(action_print["action"]["command"]
        .as_str()
        .unwrap()
        .contains("agent focus"));
    let action_focus = run_trail_json(temp.path(), &["agent", "action", "inspect_focus_file"]);
    assert_eq!(action_focus["task"]["lane"], view["task"]["lane"]);
    assert_eq!(action_focus["path"], "README.md");
    let confirm_required = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("--json")
        .args(["agent", "action", "validation_next"])
        .output()
        .unwrap();
    assert!(!confirm_required.status.success());
    let confirm_stderr: serde_json::Value =
        serde_json::from_slice(&confirm_required.stderr).unwrap();
    assert!(confirm_stderr["error"]["message"]
        .as_str()
        .unwrap()
        .contains("requires --confirm"));
    let action_mark_file = run_trail_json(
        temp.path(),
        &[
            "agent",
            "action",
            "mark_focus_file_reviewed",
            "--note",
            "reviewed via action",
        ],
    );
    assert_eq!(action_mark_file["task"]["lane"], view["task"]["lane"]);
    assert_eq!(action_mark_file["path"], "README.md");
    assert_eq!(action_mark_file["marker"]["note"], "reviewed via action");
    let mark_file_reviewed = run_trail_json(
        temp.path(),
        &[
            "agent",
            "mark-file-reviewed",
            "latest",
            "README.md",
            "--note",
            "file looks good",
        ],
    );
    assert_eq!(mark_file_reviewed["task"]["lane"], view["task"]["lane"]);
    assert_eq!(mark_file_reviewed["path"], "README.md");
    assert_eq!(mark_file_reviewed["marker"]["path"], "README.md");
    assert_eq!(mark_file_reviewed["marker"]["note"], "file looks good");
    let review_map_after_file = run_trail_json(temp.path(), &["agent", "review-map", "latest"]);
    assert_eq!(
        review_map_after_file["areas"][0]["files"][0]["state"],
        "reviewed"
    );
    assert_eq!(review_map_after_file["areas"][0]["state"], "reviewed");
    assert_eq!(
        review_map_after_file["areas"][0]["files"][0]["reviewed"]["note"],
        "file looks good"
    );
    let done_file_alias = run_trail_json(temp.path(), &["agent", "done-file", "README.md"]);
    assert_eq!(done_file_alias["path"], "README.md");
    assert_eq!(
        done_file_alias["previous"]["note"],
        serde_json::Value::String("file looks good".to_string())
    );
    let review_map_after_alias = run_trail_json(temp.path(), &["agent", "review-map", "latest"]);
    assert_eq!(
        review_map_after_alias["areas"][0]["files"][0]["state"],
        "reviewed"
    );
    let review_data = run_trail_json(temp.path(), &["agent", "review-data", "latest"]);
    assert_eq!(review_data["task"]["lane"], view["task"]["lane"]);
    assert_eq!(review_data["total_files"], 1);
    assert_eq!(review_data["reviewed_files"], 1);
    assert_eq!(review_data["needs_review_files"], 0);
    assert_eq!(review_data["focus"]["path"], "README.md");
    assert_eq!(
        review_data["review_map"]["areas"],
        review_map_after_alias["areas"]
    );
    assert_eq!(review_data["changes_by_file"]["grouping"], "file");
    assert_eq!(
        review_data["files"]["files"][0]["change"]["path"],
        "README.md"
    );
    let review_actions = review_data["actions"].as_array().unwrap();
    assert!(review_actions
        .iter()
        .any(|action| action["id"] == "open_focus_file"
            && action["kind"] == "open_file"
            && action["enabled"] == true
            && action["path"] == "README.md"));
    assert!(review_actions
        .iter()
        .any(|action| action["id"] == "mark_focus_file_reviewed"
            && action["safety"] == "workspace_write"
            && action["enabled"] == false
            && action["disabled_reason"] == "all changed files are already reviewed"
            && action["mcp_arguments"]["path"] == "README.md"));
    assert!(review_actions
        .iter()
        .any(|action| action["id"] == "validation_next"
            && action["safety"] == "open_world"
            && action["requires_confirmation"] == true
            && (action["mcp_tool"] == "trail.agent_test"
                || action["mcp_tool"] == "trail.agent_eval")));
    assert!(review_actions
        .iter()
        .any(|action| action["id"] == "apply_dry_run"
            && action["safety"] == "read_only"
            && action["enabled"] == true
            && action["mcp_tool"] == "trail.agent_apply"
            && action["mcp_arguments"]["dry-run"] == true));
    assert!(review_actions
        .iter()
        .any(|action| action["id"] == "apply_task"
            && action["safety"] == "destructive"
            && action["requires_confirmation"] == true
            && action["disabled_reason"].is_string()));
    let cockpit_alias = run_trail_json(temp.path(), &["agent", "cockpit", "latest"]);
    assert_eq!(cockpit_alias["summary"], review_data["summary"]);
    let ask_review_data = run_trail_json(
        temp.path(),
        &["agent", "ask", "show", "editor", "panel", "data"],
    );
    assert_eq!(ask_review_data["task"]["lane"], view["task"]["lane"]);
    assert_eq!(ask_review_data["needs_review_files"], 0);
    assert_eq!(ask_review_data["changes_by_file"]["grouping"], "file");
    assert!(ask_review_data["actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action["id"] == "apply_dry_run" && action["safety"] == "read_only"));
    let ask_actions = run_trail_json(
        temp.path(),
        &["agent", "ask", "what", "actions", "can", "I", "run"],
    );
    assert_eq!(ask_actions["task"]["lane"], view["task"]["lane"]);
    assert!(ask_actions["actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action["id"] == "apply_dry_run"));
    let ask_review_map = run_trail_json(temp.path(), &["agent", "ask", "review", "map"]);
    assert_eq!(ask_review_map["task"]["lane"], view["task"]["lane"]);
    assert_eq!(ask_review_map["areas"], review_map_after_alias["areas"]);
    let review_files_alias = run_trail_json(temp.path(), &["agent", "review-files", "latest"]);
    assert_eq!(review_files_alias["areas"], review_map_after_alias["areas"]);
    let ask_tools = run_trail_json(
        temp.path(),
        &["agent", "ask", "what", "tools", "were", "used"],
    );
    assert_eq!(ask_tools["task"]["lane"], view["task"]["lane"]);
    assert_eq!(
        ask_tools["available_commands"],
        agent_tools["available_commands"]
    );
    assert_eq!(ask_tools["tools"], agent_tools["tools"]);
    let ask_transcript = run_trail_json(temp.path(), &["agent", "ask", "show", "transcript"]);
    assert_eq!(ask_transcript["task"]["lane"], view["task"]["lane"]);
    assert_eq!(ask_transcript["transcript"], view["transcript"]);
    let ask_prompt_history = run_trail_json(temp.path(), &["agent", "ask", "prompt", "history"]);
    assert_eq!(ask_prompt_history["task"]["lane"], view["task"]["lane"]);
    assert_eq!(ask_prompt_history["transcript"], view["transcript"]);
    let task_workdir = view["task"]["workdir"].as_str().unwrap();
    assert!(std::path::Path::new(task_workdir).is_dir());
    let lane = view["task"]["lane"].as_str().unwrap().to_string();
    let workdir = run_trail_json(temp.path(), &["agent", "workdir", "latest"]);
    assert_eq!(workdir["task"]["lane"], lane);
    assert_eq!(workdir["workdir"], task_workdir);
    let cd_command = workdir["cd_command"].as_str().unwrap();
    assert!(cd_command.starts_with("cd "));
    assert!(cd_command.contains(".trail/worktrees/"));
    let ask_workdir = run_trail_json(
        temp.path(),
        &["agent", "ask", "where", "is", "the", "workdir"],
    );
    assert_eq!(ask_workdir["task"]["lane"], lane);
    assert_eq!(ask_workdir["workdir"], task_workdir);
    assert_eq!(ask_workdir["cd_command"], cd_command);
    let ask_edit_location = run_trail_json(
        temp.path(),
        &["agent", "ask", "where", "did", "the", "agent", "edit"],
    );
    assert_eq!(ask_edit_location["task"]["lane"], lane);
    assert_eq!(ask_edit_location["files"][0]["change"]["path"], "README.md");
    let next = run_trail_json(temp.path(), &["agent", "next", "latest"]);
    assert_eq!(next["focus"], "review_new");
    assert_eq!(next["task"]["lane"], lane);
    assert_eq!(
        next["primary"]["command"],
        format!("trail agent new {lane}")
    );
    assert!(next["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|suggestion| suggestion["command"] == format!("trail agent action {lane}")));
    assert!(next["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|suggestion| suggestion["command"]
            .as_str()
            .unwrap()
            .contains("agent land")));
    let todo_alias = run_trail_json(temp.path(), &["agent", "todo", "latest"]);
    assert_eq!(todo_alias["task"]["lane"], lane);
    assert_eq!(todo_alias["focus"], next["focus"]);
    assert_eq!(todo_alias["primary"], next["primary"]);
    let ask_next = run_trail_json(
        temp.path(),
        &["agent", "ask", "what", "should", "I", "do", "next"],
    );
    assert_eq!(ask_next["task"]["lane"], lane);
    assert_eq!(ask_next["focus"], next["focus"]);
    assert_eq!(ask_next["primary"], next["primary"]);
    let review_flow = run_trail_json(temp.path(), &["agent", "review-flow", "latest"]);
    assert_eq!(review_flow["task"]["lane"], lane);
    assert_eq!(review_flow["review_status"], "unreviewed");
    assert_eq!(review_flow["new_changed_paths"], 1);
    assert_eq!(review_flow["focus"]["path"], "README.md");
    assert!(review_flow["summary"]
        .as_str()
        .unwrap()
        .contains("review `unreviewed`"));
    let review_flow_steps = review_flow["steps"].as_array().unwrap();
    assert_eq!(review_flow_steps[0]["label"], "Inspect changes");
    assert_eq!(review_flow_steps[0]["state"], "current");
    assert!(review_flow_steps[0]["command"]
        .as_str()
        .unwrap()
        .contains("agent new"));
    assert_eq!(review_flow_steps[1]["label"], "Mark reviewed");
    assert_eq!(review_flow_steps[1]["state"], "pending");
    assert!(review_flow["next"]["command"]
        .as_str()
        .unwrap()
        .contains("agent new"));
    let walkthrough_alias = run_trail_json(temp.path(), &["agent", "walkthrough", "latest"]);
    assert_eq!(walkthrough_alias["task"]["lane"], lane);
    assert_eq!(walkthrough_alias["steps"], review_flow["steps"]);
    let ask_review_flow = run_trail_json(
        temp.path(),
        &["agent", "ask", "walk", "me", "through", "review"],
    );
    assert_eq!(ask_review_flow["task"]["lane"], lane);
    assert_eq!(ask_review_flow["review_status"], "unreviewed");
    assert_eq!(ask_review_flow["steps"], review_flow["steps"]);
    let confidence = run_trail_json(temp.path(), &["agent", "confidence", "latest"]);
    assert_eq!(confidence["task"]["lane"], lane);
    assert_eq!(confidence["verdict"], "review");
    assert!(confidence["score"].as_u64().unwrap() < 100);
    assert_eq!(confidence["review_status"], "unreviewed");
    assert_eq!(confidence["validation"]["status"], "missing_test");
    assert!(confidence["factors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|factor| factor["name"] == "review" && factor["state"] == "warn"));
    assert!(confidence["next"]["command"]
        .as_str()
        .unwrap()
        .contains("agent review-flow"));
    let go_alias = run_trail_json(temp.path(), &["agent", "go-no-go", "latest"]);
    assert_eq!(go_alias["task"]["lane"], lane);
    assert_eq!(go_alias["verdict"], confidence["verdict"]);
    let ask_confidence = run_trail_json(temp.path(), &["agent", "ask", "am", "I", "good"]);
    assert_eq!(ask_confidence["task"]["lane"], lane);
    assert_eq!(ask_confidence["verdict"], confidence["verdict"]);
    let brief = run_trail_json(temp.path(), &["agent", "brief", "latest"]);
    assert_eq!(brief["task"]["lane"], lane);
    assert_eq!(brief["task"]["workdir"], task_workdir);
    assert_eq!(brief["risk"]["level"], "medium");
    assert!(brief["risk"]["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|reason| reason["code"] == "missing_latest_test"));
    assert!(brief["task"]["title"]
        .as_str()
        .unwrap()
        .contains("edit README"));
    assert_eq!(brief["next"]["focus"], "review_new");
    assert!(brief["groups"][0]["prompt_preview"]
        .as_str()
        .unwrap()
        .contains("edit README"));
    assert!(brief["latest_change_diff"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "README.md"));
    assert!(brief["tool_summaries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool.as_str().unwrap().contains("write README")));
    let story = run_trail_json(temp.path(), &["agent", "story", "latest"]);
    assert_eq!(story["task"]["lane"], lane);
    assert!(story["summary"].as_str().unwrap().contains("edit README"));
    assert!(story["summary"].as_str().unwrap().contains("`README.md`"));
    assert_eq!(story["turn_summaries"].as_array().unwrap().len(), 1);
    assert!(story["turn_summaries"][0]["prompt_preview"]
        .as_str()
        .unwrap()
        .contains("edit README"));
    assert!(story["changed_files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "README.md"));
    assert!(story["tool_summaries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool.as_str().unwrap().contains("write README")));
    assert!(story["risk_notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note.as_str().unwrap().contains("missing_latest_test")));
    let ask_story = run_trail_json(
        temp.path(),
        &["agent", "ask", "what", "did", "the", "agent", "do"],
    );
    assert_eq!(ask_story["task"]["lane"], lane);
    assert_eq!(ask_story["summary"], story["summary"]);
    assert_eq!(ask_story["changed_files"], story["changed_files"]);
    let risk = run_trail_json(temp.path(), &["agent", "risk", "latest"]);
    assert_eq!(risk["task"]["lane"], lane);
    assert_eq!(risk["level"], "medium");
    assert!(risk["score"].as_u64().unwrap() >= 35);
    assert!(risk["summary"].as_str().unwrap().contains("Risk is medium"));
    assert!(risk["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|reason| reason["code"] == "missing_latest_test"));
    assert!(risk["recommendations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|suggestion| suggestion["command"]
            .as_str()
            .unwrap()
            .contains("agent validate")));
    let validate = run_trail_json(temp.path(), &["agent", "validate", "latest"]);
    assert_eq!(validate["task"]["lane"], lane);
    assert_eq!(validate["status"], "missing_test");
    assert_eq!(validate["needs_test"], true);
    assert!(validate["next"]["command"]
        .as_str()
        .unwrap()
        .contains("agent test"));
    assert!(validate["next"]["command"]
        .as_str()
        .unwrap()
        .contains("<test-command>"));
    let tests_alias = run_trail_json(temp.path(), &["agent", "tests", "latest"]);
    assert_eq!(tests_alias["task"]["lane"], lane);
    assert_eq!(tests_alias["next"], validate["next"]);
    let test_plan = run_trail_json(temp.path(), &["agent", "test-plan", "latest"]);
    assert_eq!(test_plan["task"]["lane"], lane);
    assert_eq!(test_plan["status"], "needs_test");
    assert_eq!(test_plan["validation"]["status"], validate["status"]);
    assert!(test_plan["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|step| step["kind"] == "test"
            && step["required"] == true
            && step["command"].as_str().unwrap().contains("agent test")));
    let ask_tests = run_trail_json(
        temp.path(),
        &["agent", "ask", "what", "tests", "should", "I", "run"],
    );
    assert_eq!(ask_tests["task"]["lane"], lane);
    assert_eq!(ask_tests["status"], test_plan["status"]);
    assert_eq!(ask_tests["steps"], test_plan["steps"]);
    let validation_plan_alias =
        run_trail_json(temp.path(), &["agent", "validation-plan", "latest"]);
    assert_eq!(validation_plan_alias["steps"], test_plan["steps"]);
    let diagnose = run_trail_json(temp.path(), &["agent", "diagnose", "latest"]);
    assert_eq!(diagnose["task"]["lane"], lane);
    assert_eq!(diagnose["status"], "git_blocked");
    assert_eq!(diagnose["severity"], "high");
    assert_eq!(diagnose["ready"], false);
    assert!(diagnose["likely_issue"]
        .as_str()
        .unwrap()
        .contains("requires a Git working tree"));
    assert!(diagnose["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap().contains("Git preflight failed")));
    assert!(diagnose["evidence"].as_array().unwrap().iter().any(|item| {
        item.as_str()
            .unwrap()
            .contains("friendly checkpoint target")
    }));
    assert!(diagnose["recovery_options"]
        .as_array()
        .unwrap()
        .iter()
        .any(|suggestion| suggestion["command"]
            .as_str()
            .unwrap()
            .contains("turn-diff")));
    assert!(diagnose["recovery_options"]
        .as_array()
        .unwrap()
        .iter()
        .any(|suggestion| suggestion["command"]
            .as_str()
            .unwrap()
            .contains("agent summary")));
    assert!(diagnose["next"]["command"]
        .as_str()
        .unwrap()
        .contains("turn-diff"));
    let recover_alias = run_trail_json(temp.path(), &["agent", "recover", "latest"]);
    assert_eq!(recover_alias["task"]["lane"], lane);
    assert_eq!(recover_alias["status"], diagnose["status"]);
    assert_eq!(recover_alias["likely_issue"], diagnose["likely_issue"]);
    let ask_recover = run_trail_json(temp.path(), &["agent", "ask", "recover"]);
    assert_eq!(ask_recover["task"]["lane"], lane);
    assert_eq!(ask_recover["status"], diagnose["status"]);
    assert_eq!(ask_recover["likely_issue"], diagnose["likely_issue"]);
    let ask_failed = run_trail_json(temp.path(), &["agent", "ask", "why", "did", "it", "fail"]);
    assert_eq!(ask_failed["task"]["lane"], lane);
    assert_eq!(ask_failed["status"], diagnose["status"]);
    assert_eq!(ask_failed["likely_issue"], diagnose["likely_issue"]);
    let ask_wrong = run_trail_json(temp.path(), &["agent", "ask", "what", "went", "wrong"]);
    assert_eq!(ask_wrong["task"]["lane"], lane);
    assert_eq!(ask_wrong["status"], diagnose["status"]);
    assert_eq!(ask_wrong["likely_issue"], diagnose["likely_issue"]);
    let diagnose_text = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("agent")
        .arg("diagnose")
        .arg("latest")
        .output()
        .unwrap();
    assert!(
        diagnose_text.status.success(),
        "agent diagnose failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&diagnose_text.stdout),
        String::from_utf8_lossy(&diagnose_text.stderr)
    );
    let diagnose_stdout = String::from_utf8_lossy(&diagnose_text.stdout);
    assert!(
        diagnose_stdout.contains("Agent diagnosis: git blocked"),
        "{diagnose_stdout}"
    );
    assert!(diagnose_stdout.contains("Likely Issue"));
    assert!(diagnose_stdout.contains("Recovery Options"));
    assert!(diagnose_stdout.contains("requires a Git working tree"));
    let report = run_trail_json(temp.path(), &["agent", "report", "latest"]);
    assert_eq!(report["task"]["lane"], lane);
    assert_eq!(report["risk"]["level"], "medium");
    assert_eq!(report["changes"]["grouping"], "turn");
    assert!(report["summary"].as_str().unwrap().contains("changed file"));
    assert!(report["markdown"]
        .as_str()
        .unwrap()
        .contains("# Agent Task Report"));
    assert!(report["markdown"]
        .as_str()
        .unwrap()
        .contains("## Next Action"));
    let receipt = run_trail_json(temp.path(), &["agent", "receipt", "latest"]);
    assert_eq!(receipt["task"]["lane"], lane);
    assert_eq!(receipt["status"], "ready");
    assert_eq!(receipt["risk"]["level"], "medium");
    assert!(receipt["validation"].as_array().unwrap().is_empty());
    assert!(receipt["markdown"]
        .as_str()
        .unwrap()
        .contains("# Agent Task Receipt"));
    assert!(receipt["markdown"]
        .as_str()
        .unwrap()
        .contains("## Validation"));
    assert!(receipt["markdown"]
        .as_str()
        .unwrap()
        .contains("No test or eval gate has been recorded"));
    let ask_receipt = run_trail_json(
        temp.path(),
        &["agent", "ask", "give", "me", "a", "summary", "to", "share"],
    );
    assert_eq!(ask_receipt["task"]["lane"], lane);
    assert!(ask_receipt["markdown"]
        .as_str()
        .unwrap()
        .contains("# Agent Task Receipt"));
    let pr = run_trail_json(temp.path(), &["agent", "pr", "latest"]);
    assert_eq!(pr["task"]["lane"], lane);
    assert!(pr["title"].as_str().unwrap().starts_with("Apply "));
    assert!(pr["body"].as_str().unwrap().contains("## Summary"));
    assert!(pr["body"].as_str().unwrap().contains("## Validation"));
    assert!(pr["body"].as_str().unwrap().contains("README.md"));
    let ask_pr = run_trail_json(
        temp.path(),
        &[
            "agent", "ask", "what", "should", "I", "put", "in", "the", "PR",
        ],
    );
    assert_eq!(ask_pr["task"]["lane"], lane);
    assert!(ask_pr["title"].as_str().unwrap().starts_with("Apply "));
    assert!(ask_pr["body"].as_str().unwrap().contains("## Summary"));
    let ask_merge_pr = run_trail_json(
        temp.path(),
        &["agent", "ask", "can", "I", "merge", "the", "PR"],
    );
    assert_eq!(ask_merge_pr["task"]["lane"], lane);
    assert_eq!(ask_merge_pr["status"], "git_blocked");
    assert!(ask_merge_pr["title"].is_null());
    let summary = run_trail_json(temp.path(), &["agent", "summary", "latest"]);
    assert_eq!(summary["task"]["lane"], lane);
    assert_eq!(summary["risk"]["level"], "medium");
    assert!(summary["summary"]
        .as_str()
        .unwrap()
        .contains("changed file"));
    assert!(summary["receipt_markdown"]
        .as_str()
        .unwrap()
        .contains("# Agent Task Receipt"));
    assert!(summary["pr_title"].as_str().unwrap().starts_with("Apply "));
    assert!(summary["pr_body"]
        .as_str()
        .unwrap()
        .contains("## Trail Review"));
    let risk = run_trail_json(temp.path(), &["agent", "risk", "latest"]);
    assert_eq!(risk["task"]["lane"], lane);
    assert_eq!(risk["level"], "medium");
    let ask_red_flags = run_trail_json(
        temp.path(),
        &["agent", "ask", "what", "should", "I", "worry", "about"],
    );
    assert_eq!(ask_red_flags["task"]["lane"], lane);
    assert_eq!(ask_red_flags["level"], risk["level"]);
    assert_eq!(ask_red_flags["reasons"], risk["reasons"]);
    let review = run_trail_json(temp.path(), &["agent", "review", "latest"]);
    assert_eq!(review["task"]["lane"], lane);
    assert_eq!(review["risk"]["level"], "medium");
    assert_eq!(review["transcript_turns"], 1);
    assert_eq!(review["tool_events"], view["task"]["tool_events"]);
    assert!(review["summary"]
        .as_str()
        .unwrap()
        .contains("Review 1 prioritized file"));
    assert_eq!(review["priorities"].as_array().unwrap().len(), 1);
    assert_eq!(review["priorities"][0]["rank"], 1);
    assert_eq!(review["priorities"][0]["change"]["path"], "README.md");
    assert!(review["priorities"][0]["why_command"]
        .as_str()
        .unwrap()
        .contains("agent why"));
    assert!(review["priorities"][0]["diff_command"]
        .as_str()
        .unwrap()
        .contains("turn-diff"));
    assert!(review["next"]["command"]
        .as_str()
        .unwrap()
        .contains("agent validate"));
    let review_plan = run_trail_json(temp.path(), &["agent", "review-plan", "latest"]);
    assert_eq!(review_plan["task"]["lane"], lane);
    assert_eq!(review_plan["priorities"], review["priorities"]);
    assert_eq!(review_plan["next"], review["next"]);
    let ask_review_plan = run_trail_json(
        temp.path(),
        &["agent", "ask", "what", "should", "I", "review"],
    );
    assert_eq!(ask_review_plan["task"]["lane"], lane);
    assert_eq!(ask_review_plan["priorities"], review["priorities"]);
    let ask_show_review = run_trail_json(temp.path(), &["agent", "ask", "show", "review", "plan"]);
    assert_eq!(ask_show_review["task"]["lane"], lane);
    assert_eq!(ask_show_review["priorities"], review["priorities"]);
    let ask_open_review = run_trail_json(temp.path(), &["agent", "ask", "open", "review"]);
    assert_eq!(ask_open_review["task"]["lane"], lane);
    assert_eq!(ask_open_review["priorities"], review["priorities"]);
    let ask_start_review = run_trail_json(temp.path(), &["agent", "ask", "start", "review"]);
    assert_eq!(ask_start_review["task"]["lane"], lane);
    assert_eq!(ask_start_review["priorities"], review["priorities"]);
    let ask_review_task = run_trail_json(temp.path(), &["agent", "ask", "review", "this", "task"]);
    assert_eq!(ask_review_task["task"]["lane"], lane);
    assert_eq!(ask_review_task["priorities"], review["priorities"]);
    let focus = run_trail_json(temp.path(), &["agent", "focus", "latest"]);
    assert_eq!(focus["task"]["lane"], lane);
    assert_eq!(focus["path"], "README.md");
    assert_eq!(focus["source"], "review_priority");
    assert_eq!(focus["priority"]["change"]["path"], "README.md");
    assert_eq!(focus["why"]["path"], "README.md");
    assert_eq!(focus["diff"]["file_filter"], "README.md");
    assert_eq!(focus["diff"]["diff"]["files"].as_array().unwrap().len(), 1);
    let focus_open_path = focus["open_path"].as_str().unwrap();
    assert!(focus_open_path.ends_with("README.md"));
    assert!(Path::new(focus_open_path).is_file());
    assert!(focus["open_command"]
        .as_str()
        .unwrap()
        .contains("${EDITOR:-vi}"));
    assert!(focus["next"]["command"]
        .as_str()
        .unwrap()
        .contains("--patch"));
    let dashboard = run_trail_json(temp.path(), &["agent", "dashboard", "latest"]);
    assert_eq!(dashboard["task"]["lane"], lane);
    assert_eq!(dashboard["status"], "ready");
    assert_eq!(dashboard["focus"]["path"], "README.md");
    assert_eq!(dashboard["focus"]["open_path"], focus["open_path"]);
    assert_eq!(dashboard["validation"]["status"], "missing_test");
    assert_eq!(dashboard["ready"]["task"]["lane"], lane);
    assert_eq!(
        dashboard["changes"]["total_changed_paths"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(dashboard["summary"].as_str().unwrap().contains("README.md"));
    let open_print = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("agent")
        .arg("open")
        .arg("latest")
        .arg("--print")
        .output()
        .unwrap();
    assert!(
        open_print.status.success(),
        "agent open --print failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&open_print.stdout),
        String::from_utf8_lossy(&open_print.stderr)
    );
    let open_print_stdout = String::from_utf8_lossy(&open_print.stdout);
    assert!(open_print_stdout.contains("${EDITOR:-vi}"));
    assert!(open_print_stdout.contains("README.md"));
    let open_json_output = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("--json")
        .arg("agent")
        .arg("open")
        .arg("latest")
        .env("EDITOR", "false")
        .output()
        .unwrap();
    assert!(
        open_json_output.status.success(),
        "agent open --json failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&open_json_output.stdout),
        String::from_utf8_lossy(&open_json_output.stderr)
    );
    let open_json: serde_json::Value = serde_json::from_slice(&open_json_output.stdout).unwrap();
    assert_eq!(open_json["open_path"], focus["open_path"]);
    assert_eq!(open_json["open_command"], focus["open_command"]);
    let open_launch = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("agent")
        .arg("open")
        .arg("latest")
        .env("EDITOR", "true")
        .output()
        .unwrap();
    assert!(
        open_launch.status.success(),
        "agent open failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&open_launch.stdout),
        String::from_utf8_lossy(&open_launch.stderr)
    );
    assert!(
        String::from_utf8_lossy(&open_launch.stdout).contains("Opened agent focus"),
        "{}",
        String::from_utf8_lossy(&open_launch.stdout)
    );
    let ask_dashboard = run_trail_json(
        temp.path(),
        &["agent", "ask", "show", "me", "the", "dashboard"],
    );
    assert_eq!(ask_dashboard["task"]["lane"], lane);
    assert_eq!(ask_dashboard["focus"]["path"], dashboard["focus"]["path"]);
    let ask_focus = run_trail_json(
        temp.path(),
        &["agent", "ask", "what", "should", "I", "review", "first"],
    );
    assert_eq!(ask_focus["task"]["lane"], lane);
    assert_eq!(ask_focus["path"], focus["path"]);
    assert_eq!(ask_focus["source"], focus["source"]);
    let ask_focus_file = run_trail_json(
        temp.path(),
        &[
            "agent", "ask", "what", "file", "should", "I", "review", "first",
        ],
    );
    assert_eq!(ask_focus_file["task"]["lane"], lane);
    assert_eq!(ask_focus_file["path"], focus["path"]);
    assert_eq!(ask_focus_file["source"], focus["source"]);
    let ask_open_file = run_trail_json(
        temp.path(),
        &["agent", "ask", "what", "file", "should", "I", "open"],
    );
    assert_eq!(ask_open_file["task"]["lane"], lane);
    assert_eq!(ask_open_file["path"], focus["path"]);
    assert_eq!(ask_open_file["source"], focus["source"]);
    assert_eq!(ask_open_file["open_path"], focus["open_path"]);
    assert_eq!(ask_open_file["open_command"], focus["open_command"]);
    let ask_look_first = run_trail_json(
        temp.path(),
        &["agent", "ask", "where", "should", "I", "look", "first"],
    );
    assert_eq!(ask_look_first["task"]["lane"], lane);
    assert_eq!(ask_look_first["path"], focus["path"]);
    assert_eq!(ask_look_first["source"], focus["source"]);
    let focus_patch = run_trail_json(
        temp.path(),
        &["agent", "focus", "latest", "--file", "README.md", "--patch"],
    );
    assert_eq!(focus_patch["source"], "file");
    assert!(focus_patch["diff"]["diff"]["files"][0]["patch"]
        .as_str()
        .unwrap()
        .contains("diagnostic complete"));
    let markdown_report = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("agent")
        .arg("report")
        .arg("latest")
        .arg("--markdown")
        .output()
        .unwrap();
    assert!(
        markdown_report.status.success(),
        "agent report markdown failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&markdown_report.stdout),
        String::from_utf8_lossy(&markdown_report.stderr)
    );
    let markdown_stdout = String::from_utf8_lossy(&markdown_report.stdout);
    assert!(markdown_stdout.contains("# Agent Task Report"));
    assert!(markdown_stdout.contains("## Changes"));
    let receipt_markdown = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("agent")
        .arg("receipt")
        .arg("latest")
        .output()
        .unwrap();
    assert!(
        receipt_markdown.status.success(),
        "agent receipt failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&receipt_markdown.stdout),
        String::from_utf8_lossy(&receipt_markdown.stderr)
    );
    let receipt_stdout = String::from_utf8_lossy(&receipt_markdown.stdout);
    assert!(receipt_stdout.contains("# Agent Task Receipt"));
    assert!(receipt_stdout.contains("## Useful Commands"));
    let handoff_markdown = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("agent")
        .arg("handoff")
        .arg("latest")
        .output()
        .unwrap();
    assert!(
        handoff_markdown.status.success(),
        "agent handoff failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&handoff_markdown.stdout),
        String::from_utf8_lossy(&handoff_markdown.stderr)
    );
    let handoff_stdout = String::from_utf8_lossy(&handoff_markdown.stdout);
    assert!(handoff_stdout.contains("# Agent Task Handoff"));
    assert!(handoff_stdout.contains("## Receiver Next Step"));
    assert!(handoff_stdout.contains("trail agent focus"));
    let ask_handoff = run_trail_json(
        temp.path(),
        &["agent", "ask", "handoff", "this", "to", "another", "agent"],
    );
    assert!(ask_handoff["markdown"]
        .as_str()
        .unwrap()
        .contains("# Agent Task Handoff"));
    let pr_title = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("agent")
        .arg("pr")
        .arg("latest")
        .arg("--title-only")
        .output()
        .unwrap();
    assert!(
        pr_title.status.success(),
        "agent pr title failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&pr_title.stdout),
        String::from_utf8_lossy(&pr_title.stderr)
    );
    assert!(String::from_utf8_lossy(&pr_title.stdout).starts_with("Apply "));
    let pr_body = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("agent")
        .arg("pr")
        .arg("latest")
        .arg("--body-only")
        .output()
        .unwrap();
    assert!(
        pr_body.status.success(),
        "agent pr body failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&pr_body.stdout),
        String::from_utf8_lossy(&pr_body.stderr)
    );
    assert!(String::from_utf8_lossy(&pr_body.stdout).contains("## Trail Review"));
    let summary_text = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("agent")
        .arg("summary")
        .arg("latest")
        .output()
        .unwrap();
    assert!(
        summary_text.status.success(),
        "agent summary failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&summary_text.stdout),
        String::from_utf8_lossy(&summary_text.stderr)
    );
    let summary_stdout = String::from_utf8_lossy(&summary_text.stdout);
    assert!(summary_stdout.contains("Agent summary:"));
    assert!(summary_stdout.contains("Pr Title"), "{summary_stdout}");
    let compare = run_trail_json(temp.path(), &["agent", "compare", first, second]);
    assert_eq!(compare["left"]["lane"], first);
    assert_eq!(compare["right"]["lane"], second);
    assert_eq!(compare["left_risk"]["level"], "medium");
    assert_eq!(compare["right_risk"]["level"], "medium");
    assert!(compare["summary"]
        .as_str()
        .unwrap()
        .contains("both changed"));
    assert_eq!(compare["shared_paths"].as_array().unwrap().len(), 1);
    assert_eq!(compare["shared_paths"][0]["path"], "README.md");
    assert_eq!(compare["left_only_paths"].as_array().unwrap().len(), 0);
    assert_eq!(compare["right_only_paths"].as_array().unwrap().len(), 0);
    assert!(compare["recommendation"]["command"]
        .as_str()
        .unwrap()
        .contains("agent why"));
    let acp_session_id = view["task"]["acp_session_id"].as_str().unwrap().to_string();
    let trail_session_id = view["task"]["session_id"].as_str().unwrap().to_string();
    let by_acp_session = run_trail_json(temp.path(), &["agent", "view", &acp_session_id]);
    assert_eq!(by_acp_session["task"]["lane"], lane);
    let by_trail_session = run_trail_json(temp.path(), &["agent", "view", &trail_session_id]);
    assert_eq!(by_trail_session["task"]["lane"], lane);

    let changes = run_trail_json(temp.path(), &["agent", "changes", "latest"]);
    assert_eq!(changes["grouping"], "turn");
    assert!(changes["summary"].as_str().unwrap().contains("review card"));
    assert!(changes["next"]["command"]
        .as_str()
        .unwrap()
        .contains("agent change"));
    assert!(changes["next"]["reason"]
        .as_str()
        .unwrap()
        .contains("highest-priority"));
    let cards = changes["cards"].as_array().unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0]["rank"], 1);
    assert_eq!(cards[0]["key"], "docs");
    assert_eq!(cards[0]["title"], "Docs and getting-started");
    assert_eq!(cards[0]["changed_paths"][0]["path"], "README.md");
    assert_eq!(cards[0]["touched_by"][0]["kind"], "turn");
    assert!(cards[0]["review_command"]
        .as_str()
        .unwrap()
        .contains("agent change"));
    assert!(cards[0]["focus_command"]
        .as_str()
        .unwrap()
        .contains("agent focus"));
    assert!(cards[0]["why_command"]
        .as_str()
        .unwrap()
        .contains("agent why"));
    assert!(cards[0]["diff_command"]
        .as_str()
        .unwrap()
        .contains("--file README.md"));
    let groups = changes["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["kind"], "turn");
    assert_eq!(groups[0]["index"], 1);
    assert!(groups[0]["prompt_preview"]
        .as_str()
        .unwrap()
        .contains("edit README"));
    assert!(groups[0]["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "README.md"));

    let change_set = run_trail_json(temp.path(), &["agent", "change", "latest", "docs"]);
    assert_eq!(change_set["task"]["lane"], lane);
    assert_eq!(change_set["card"]["key"], "docs");
    assert_eq!(change_set["selector"], "docs");
    assert!(change_set["summary"]
        .as_str()
        .unwrap()
        .contains("Docs and getting-started"));
    assert_eq!(change_set["files"][0]["change"]["path"], "README.md");
    assert_eq!(change_set["groups"][0]["kind"], "turn");
    assert!(change_set["diffs"].as_array().unwrap().is_empty());
    assert!(change_set["next"]["command"]
        .as_str()
        .unwrap()
        .contains("--patch"));
    let change_set_patch =
        run_trail_json(temp.path(), &["agent", "change", "latest", "1", "--patch"]);
    assert_eq!(change_set_patch["card"]["key"], "docs");
    assert_eq!(change_set_patch["diffs"].as_array().unwrap().len(), 1);
    assert_eq!(
        change_set_patch["diffs"][0]["file_filter"],
        serde_json::json!("README.md")
    );
    assert!(change_set_patch["diffs"][0]["diff"]["files"][0]["patch"]
        .as_str()
        .unwrap()
        .contains("diagnostic complete"));
    let change_text = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("agent")
        .arg("change")
        .arg("latest")
        .arg("docs")
        .output()
        .unwrap();
    assert!(
        change_text.status.success(),
        "agent change failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&change_text.stdout),
        String::from_utf8_lossy(&change_text.stderr)
    );
    let change_stdout = String::from_utf8_lossy(&change_text.stdout);
    assert!(
        change_stdout.contains("Agent change set"),
        "{change_stdout}"
    );
    assert!(change_stdout.contains("Docs and getting-started"));
    assert!(change_stdout.contains("Files"));

    let timeline = run_trail_json(temp.path(), &["agent", "timeline", "latest"]);
    assert_eq!(timeline["task"]["lane"], lane);
    assert_eq!(timeline["mode"], "turn");
    assert!(timeline["summary"]
        .as_str()
        .unwrap()
        .contains("timeline item"));
    let timeline_items = timeline["items"].as_array().unwrap();
    assert_eq!(timeline_items.len(), 1);
    assert_eq!(timeline_items[0]["kind"], "turn");
    assert_eq!(timeline_items[0]["index"], 1);
    assert!(timeline_items[0]["title"]
        .as_str()
        .unwrap()
        .contains("edit README"));
    assert!(timeline_items[0]["message_count"].as_u64().unwrap() >= 2);
    assert!(timeline_items[0]["event_count"].as_u64().unwrap() > 0);
    assert!(timeline_items[0]["view_command"]
        .as_str()
        .unwrap()
        .contains("agent turn"));
    assert!(timeline_items[0]["diff_command"]
        .as_str()
        .unwrap()
        .contains("--turn 1"));
    assert!(timeline_items[0]["rewind_before_command"]
        .as_str()
        .unwrap()
        .contains("before-turn:1"));
    assert!(timeline["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|suggestion| suggestion["command"]
            .as_str()
            .unwrap()
            .contains("agent changes")));
    let timeline_text = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("agent")
        .arg("timeline")
        .arg("latest")
        .output()
        .unwrap();
    assert!(
        timeline_text.status.success(),
        "agent timeline failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&timeline_text.stdout),
        String::from_utf8_lossy(&timeline_text.stderr)
    );
    let timeline_stdout = String::from_utf8_lossy(&timeline_text.stdout);
    assert!(timeline_stdout.contains("Agent timeline"));
    assert!(timeline_stdout.contains("Items"), "{timeline_stdout}");
    assert!(timeline_stdout.contains("Before Change"));

    let delta = run_trail_json(temp.path(), &["agent", "delta", "latest"]);
    assert_eq!(delta["task"]["lane"], lane);
    assert_eq!(delta["mode"], "turn");
    assert_eq!(delta["group"]["kind"], "turn");
    assert_eq!(delta["group"]["index"], 1);
    assert_eq!(delta["changed_paths"][0]["path"], "README.md");
    assert_eq!(delta["diff"]["target_kind"], "turn");
    assert_eq!(delta["diff"]["diff"]["files"][0]["path"], "README.md");
    assert!(delta["next"]["command"]
        .as_str()
        .unwrap()
        .contains("--patch"));
    let delta_patch = run_trail_json(
        temp.path(),
        &["agent", "delta", "latest", "--file", "README.md", "--patch"],
    );
    assert_eq!(delta_patch["file_filter"], "README.md");
    assert_eq!(delta_patch["matched"], true);
    assert!(delta_patch["diff"]["diff"]["files"][0]["patch"]
        .as_str()
        .unwrap()
        .contains("diagnostic complete"));
    let ask_last_patch = run_trail_json(temp.path(), &["agent", "ask", "show", "last", "patch"]);
    assert_eq!(ask_last_patch["task"]["lane"], lane);
    assert_eq!(ask_last_patch["mode"], "turn");
    assert!(ask_last_patch["diff"]["diff"]["files"][0]["patch"]
        .as_str()
        .unwrap()
        .contains("diagnostic complete"));
    let ask_turn_diff = run_trail_json(temp.path(), &["agent", "ask", "show", "turn", "diff"]);
    assert_eq!(ask_turn_diff["task"]["lane"], lane);
    assert_eq!(ask_turn_diff["target_kind"], "turn");
    assert!(ask_turn_diff["diff"]["files"][0]["patch"]
        .as_str()
        .unwrap()
        .contains("diagnostic complete"));
    let ask_turn_file_diff = run_trail_json(
        temp.path(),
        &["agent", "ask", "show", "turn", "diff", "for", "README.md"],
    );
    assert_eq!(ask_turn_file_diff["target_kind"], "turn");
    assert_eq!(ask_turn_file_diff["file_filter"], "README.md");
    assert!(ask_turn_file_diff["diff"]["files"][0]["patch"]
        .as_str()
        .unwrap()
        .contains("diagnostic complete"));
    let delta_text = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("agent")
        .arg("delta")
        .arg("latest")
        .output()
        .unwrap();
    assert!(
        delta_text.status.success(),
        "agent delta failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&delta_text.stdout),
        String::from_utf8_lossy(&delta_text.stderr)
    );
    let delta_stdout = String::from_utf8_lossy(&delta_text.stdout);
    assert!(delta_stdout.contains("Agent delta"));
    assert!(
        delta_stdout.contains("Latest turn 1 changed 1 file(s)"),
        "{delta_stdout}"
    );
    let last_alias = run_trail_json(temp.path(), &["agent", "last", "latest"]);
    assert_eq!(last_alias["task"]["lane"], lane);
    assert_eq!(last_alias["mode"], delta["mode"]);
    assert_eq!(last_alias["group"]["id"], delta["group"]["id"]);
    let ask_last = run_trail_json(temp.path(), &["agent", "ask", "what", "just", "changed"]);
    assert_eq!(ask_last["task"]["lane"], lane);
    assert_eq!(ask_last["mode"], delta["mode"]);
    assert_eq!(ask_last["group"]["id"], delta["group"]["id"]);
    let ask_prompt_delta = run_trail_json(
        temp.path(),
        &[
            "agent", "ask", "what", "changed", "in", "the", "last", "prompt",
        ],
    );
    assert_eq!(ask_prompt_delta["task"]["lane"], lane);
    assert_eq!(ask_prompt_delta["mode"], delta["mode"]);
    assert_eq!(ask_prompt_delta["group"]["id"], delta["group"]["id"]);
    assert_eq!(ask_prompt_delta["changed_paths"][0]["path"], "README.md");
    let ask_prompt_file_delta = run_trail_json(
        temp.path(),
        &[
            "agent",
            "ask",
            "what",
            "changed",
            "in",
            "README.md",
            "in",
            "the",
            "last",
            "prompt",
        ],
    );
    assert_eq!(ask_prompt_file_delta["task"]["lane"], lane);
    assert_eq!(ask_prompt_file_delta["mode"], delta["mode"]);
    assert_eq!(ask_prompt_file_delta["file_filter"], "README.md");
    assert_eq!(ask_prompt_file_delta["matched"], true);
    assert_eq!(
        ask_prompt_file_delta["changed_paths"][0]["path"],
        "README.md"
    );

    let new_changes = run_trail_json(temp.path(), &["agent", "new", "latest"]);
    assert_eq!(new_changes["task"]["lane"], lane);
    assert_eq!(new_changes["status"], "unreviewed");
    assert!(new_changes["reviewed"].is_null());
    assert_eq!(new_changes["changed_paths"][0]["path"], "README.md");
    assert_eq!(new_changes["new_groups"][0]["kind"], "turn");
    assert!(new_changes["next"]["command"]
        .as_str()
        .unwrap()
        .contains("--patch"));
    let what_changed_alias = run_trail_json(temp.path(), &["agent", "what-changed", "latest"]);
    assert_eq!(what_changed_alias["task"]["lane"], lane);
    assert_eq!(what_changed_alias["status"], new_changes["status"]);
    assert_eq!(
        what_changed_alias["changed_paths"],
        new_changes["changed_paths"]
    );
    let ask_new = run_trail_json(
        temp.path(),
        &["agent", "ask", "what", "changed", "since", "I", "looked"],
    );
    assert_eq!(ask_new["task"]["lane"], lane);
    assert_eq!(ask_new["status"], new_changes["status"]);
    assert_eq!(ask_new["changed_paths"], new_changes["changed_paths"]);
    let mark_reviewed = run_trail_json(
        temp.path(),
        &[
            "agent",
            "mark-reviewed",
            "latest",
            "--note",
            "reviewed in test",
        ],
    );
    assert_eq!(mark_reviewed["task"]["lane"], lane);
    assert_eq!(mark_reviewed["marker"]["changed_paths"], 1);
    assert_eq!(mark_reviewed["marker"]["note"], "reviewed in test");
    assert!(mark_reviewed["previous"].is_null());
    let done_alias = run_trail_json(
        temp.path(),
        &["agent", "done", "latest", "--note", "reviewed via alias"],
    );
    assert_eq!(done_alias["task"]["lane"], lane);
    assert_eq!(done_alias["marker"]["changed_paths"], 1);
    assert_eq!(done_alias["marker"]["note"], "reviewed via alias");
    assert_eq!(
        done_alias["previous"]["checkpoint"],
        mark_reviewed["marker"]["checkpoint"]
    );
    let no_new_changes = run_trail_json(temp.path(), &["agent", "new", "latest"]);
    assert_eq!(no_new_changes["status"], "up_to_date");
    assert_eq!(
        no_new_changes["reviewed"]["checkpoint"],
        done_alias["marker"]["checkpoint"]
    );
    assert!(no_new_changes["changed_paths"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(no_new_changes["new_groups"].as_array().unwrap().is_empty());
    let new_text = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("agent")
        .arg("new")
        .arg("latest")
        .output()
        .unwrap();
    assert!(
        new_text.status.success(),
        "agent new failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&new_text.stdout),
        String::from_utf8_lossy(&new_text.stderr)
    );
    let new_stdout = String::from_utf8_lossy(&new_text.stdout);
    assert!(
        new_stdout.contains("Created agent task: up to date"),
        "{new_stdout}"
    );
    assert!(new_stdout.contains("Status"));
    assert!(new_stdout.contains("up to date"));
    let next_after_reviewed = run_trail_json(temp.path(), &["agent", "next", "latest"]);
    assert_eq!(next_after_reviewed["focus"], "preview_apply");
    assert_eq!(
        next_after_reviewed["primary"]["command"],
        format!("trail agent land {lane} --dry-run")
    );
    let review_flow_after_done = run_trail_json(temp.path(), &["agent", "review-loop", "latest"]);
    assert_eq!(review_flow_after_done["task"]["lane"], lane);
    assert_eq!(review_flow_after_done["review_status"], "up_to_date");
    assert_eq!(review_flow_after_done["new_changed_paths"], 0);
    assert_eq!(review_flow_after_done["steps"][0]["state"], "done");
    assert_eq!(review_flow_after_done["steps"][1]["state"], "done");
    assert!(review_flow_after_done["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|step| step["state"] == "current" || step["state"] == "blocked"));
    let confidence_after_done = run_trail_json(temp.path(), &["agent", "go", "latest"]);
    assert_eq!(confidence_after_done["task"]["lane"], lane);
    assert_eq!(confidence_after_done["review_status"], "up_to_date");
    assert_eq!(confidence_after_done["verdict"], "validate");
    assert!(confidence_after_done["next"]["command"]
        .as_str()
        .unwrap()
        .contains("agent test"));

    let files = run_trail_json(temp.path(), &["agent", "files", "latest"]);
    assert_eq!(files["task"]["lane"], lane);
    assert_eq!(files["grouping"], "turn");
    assert_eq!(files["files"].as_array().unwrap().len(), 1);
    assert_eq!(files["files"][0]["change"]["path"], "README.md");
    assert_eq!(files["files"][0]["touched_by"][0]["kind"], "turn");
    assert!(files["files"][0]["touched_by"][0]["diff_command"]
        .as_str()
        .unwrap()
        .contains("--file README.md"));
    assert!(files["files"][0]["why_command"]
        .as_str()
        .unwrap()
        .contains("agent why"));
    assert!(files["files"][0]["diff_command"]
        .as_str()
        .unwrap()
        .contains("turn-diff"));
    assert!(files["files"][0]["diff_command"]
        .as_str()
        .unwrap()
        .contains("--file README.md"));
    let changed_files_alias = run_trail_json(temp.path(), &["agent", "changed-files", "latest"]);
    assert_eq!(changed_files_alias["task"]["lane"], lane);
    assert_eq!(changed_files_alias["grouping"], files["grouping"]);
    assert_eq!(changed_files_alias["files"], files["files"]);
    let ask_files = run_trail_json(temp.path(), &["agent", "ask", "changed", "files"]);
    assert_eq!(ask_files["task"]["lane"], lane);
    assert_eq!(ask_files["grouping"], files["grouping"]);
    assert_eq!(ask_files["files"], files["files"]);
    let ask_agent_change = run_trail_json(
        temp.path(),
        &["agent", "ask", "what", "did", "the", "agent", "change"],
    );
    assert_eq!(ask_agent_change["task"]["lane"], lane);
    assert_eq!(ask_agent_change["grouping"], files["grouping"]);
    assert_eq!(ask_agent_change["files"], files["files"]);
    let ask_files_touched = run_trail_json(
        temp.path(),
        &["agent", "ask", "what", "files", "did", "it", "touch"],
    );
    assert_eq!(ask_files_touched["task"]["lane"], lane);
    assert_eq!(ask_files_touched["grouping"], files["grouping"]);
    assert_eq!(ask_files_touched["files"], files["files"]);

    let file = run_trail_json(temp.path(), &["agent", "file", "latest", "README.md"]);
    assert_eq!(file["task"]["lane"], lane);
    assert_eq!(file["path"], "README.md");
    assert_eq!(file["matched"], true);
    assert_eq!(file["change"]["path"], "README.md");
    assert_eq!(file["file"]["change"]["path"], "README.md");
    assert_eq!(file["change_cards"][0]["key"], "docs");
    assert_eq!(file["groups"][0]["kind"], "turn");
    assert!(file["diff"].is_null());
    assert!(file["next"]["command"]
        .as_str()
        .unwrap()
        .contains("--patch"));
    let inspect_alias = run_trail_json(temp.path(), &["agent", "inspect", "README.md"]);
    assert_eq!(inspect_alias["task"]["lane"], lane);
    assert_eq!(inspect_alias["path"], "README.md");
    assert_eq!(inspect_alias["matched"], true);
    let file_patch = run_trail_json(temp.path(), &["agent", "file", "README.md", "--patch"]);
    assert_eq!(file_patch["path"], "README.md");
    assert_eq!(file_patch["diff"]["file_filter"], "README.md");
    assert!(file_patch["diff"]["diff"]["files"][0]["patch"]
        .as_str()
        .unwrap()
        .contains("diagnostic complete"));
    let ask_file_patch = run_trail_json(
        temp.path(),
        &["agent", "ask", "show", "patch", "for", "README.md"],
    );
    assert_eq!(ask_file_patch["task"]["lane"], lane);
    assert_eq!(ask_file_patch["path"], "README.md");
    assert_eq!(ask_file_patch["diff"]["file_filter"], "README.md");
    assert!(ask_file_patch["diff"]["diff"]["files"][0]["patch"]
        .as_str()
        .unwrap()
        .contains("diagnostic complete"));
    let file_text = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("agent")
        .arg("file")
        .arg("README.md")
        .output()
        .unwrap();
    assert!(
        file_text.status.success(),
        "agent file failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&file_text.stdout),
        String::from_utf8_lossy(&file_text.stderr)
    );
    let file_stdout = String::from_utf8_lossy(&file_text.stdout);
    assert!(file_stdout.contains("Agent file"), "{file_stdout}");
    assert!(file_stdout.contains("Path   : README.md"));
    assert!(file_stdout.contains("Change Cards"));
    assert!(file_stdout.contains("Touched By"));

    let checkpoints = run_trail_json(temp.path(), &["agent", "checkpoints", "latest"]);
    assert_eq!(checkpoints["task"]["lane"], lane);
    assert_eq!(checkpoints["entries"].as_array().unwrap().len(), 1);
    assert_eq!(checkpoints["entries"][0]["kind"], "turn");
    assert_eq!(checkpoints["entries"][0]["before_target"], "before-turn:1");
    assert_eq!(checkpoints["entries"][0]["checkpoint_target"], "turn:1");
    assert!(checkpoints["entries"][0]["rewind_before_command"]
        .as_str()
        .unwrap()
        .contains("agent rewind"));
    assert!(checkpoints["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|suggestion| suggestion["command"]
            .as_str()
            .unwrap()
            .contains("before-last-turn")));
    let rewind_points_alias = run_trail_json(temp.path(), &["agent", "rewind-points", "latest"]);
    assert_eq!(rewind_points_alias["task"]["lane"], lane);
    assert_eq!(rewind_points_alias["entries"], checkpoints["entries"]);

    let why_latest = run_trail_json(temp.path(), &["agent", "why", "README.md"]);
    assert_eq!(why_latest["task"]["lane"], lane);
    assert_eq!(why_latest["path"], "README.md");
    assert_eq!(why_latest["matched"], true);
    assert!(why_latest["summary"]
        .as_str()
        .unwrap()
        .contains("README.md"));
    assert_eq!(why_latest["groups"].as_array().unwrap().len(), 1);
    assert_eq!(why_latest["groups"][0]["kind"], "turn");
    assert_eq!(why_latest["groups"][0]["index"], 1);
    assert!(why_latest["groups"][0]["prompt_preview"]
        .as_str()
        .unwrap()
        .contains("edit README"));
    assert!(why_latest["groups"][0]["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .all(|path| path["path"] == "README.md"));
    let explain_alias = run_trail_json(temp.path(), &["agent", "explain", "README.md"]);
    assert_eq!(explain_alias["task"]["lane"], lane);
    assert_eq!(explain_alias["path"], "README.md");
    assert_eq!(explain_alias["groups"], why_latest["groups"]);
    let ask_explain = run_trail_json(temp.path(), &["agent", "ask", "explain", "README.md"]);
    assert_eq!(ask_explain["task"]["lane"], lane);
    assert_eq!(ask_explain["path"], "README.md");
    assert_eq!(ask_explain["groups"], why_latest["groups"]);
    let ask_prompt_provenance = run_trail_json(
        temp.path(),
        &["agent", "ask", "which", "prompt", "changed", "README.md"],
    );
    assert_eq!(ask_prompt_provenance["task"]["lane"], lane);
    assert_eq!(ask_prompt_provenance["path"], "README.md");
    assert_eq!(ask_prompt_provenance["groups"], why_latest["groups"]);
    let ask_turn_provenance = run_trail_json(
        temp.path(),
        &["agent", "ask", "which", "turn", "touched", "README.md"],
    );
    assert_eq!(ask_turn_provenance["task"]["lane"], lane);
    assert_eq!(ask_turn_provenance["path"], "README.md");
    assert_eq!(ask_turn_provenance["groups"], why_latest["groups"]);
    assert!(why_latest["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|suggestion| suggestion["command"]
            .as_str()
            .unwrap()
            .contains("--file README.md")));

    let why_explicit_task = run_trail_json(temp.path(), &["agent", "why", &lane, "README.md:1"]);
    assert_eq!(why_explicit_task["task"]["lane"], lane);
    assert_eq!(why_explicit_task["path"], "README.md");

    let turn_latest = run_trail_json(temp.path(), &["agent", "turn"]);
    assert_eq!(turn_latest["task"]["lane"], lane);
    assert_eq!(turn_latest["index"], 1);
    assert_eq!(
        turn_latest["turn_envelope"]["schema"],
        "trail.turn_envelope"
    );
    assert_eq!(turn_latest["turn_envelope"]["version"], 2);
    assert_eq!(turn_latest["turn_envelope"]["provider"], "claude-code");
    assert_eq!(turn_latest["turn_envelope"]["kind"], "acp_prompt");
    assert_eq!(turn_latest["turn_envelope"]["protocol"], "acp");
    assert_eq!(
        turn_latest["turn_envelope"]["session"]["upstream_session_id"],
        "sess_agent_stub_b"
    );
    assert!(
        turn_latest["turn_envelope"]["prompt"]["hash"]
            .as_str()
            .unwrap()
            .len()
            > 8
    );
    assert_eq!(
        turn_latest["turn_envelope"]["outcome"]["checkpoint"],
        turn_latest["checkpoint"]
    );
    assert_eq!(turn_latest["turn_envelope"]["outcome"]["no_changes"], false);
    assert!(
        turn_latest["turn_envelope"]["capture"]["event_count"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(
        turn_latest["turn_envelope"]["capture"]["tool_event_count"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(turn_latest["prompt_preview"]
        .as_str()
        .unwrap()
        .contains("edit README"));
    assert_eq!(turn_latest["changed_paths"][0]["path"], "README.md");
    assert!(turn_latest["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|suggestion| suggestion["command"]
            .as_str()
            .unwrap()
            .contains("agent turn")));
    let turn_id = turn_latest["turn_id"].as_str().unwrap();
    let lane_turn = run_trail_json(temp.path(), &["lane", "turn", "show", turn_id]);
    assert_eq!(lane_turn["turn_envelope"], turn_latest["turn_envelope"]);
    let ask_last_prompt = run_trail_json(temp.path(), &["agent", "ask", "last", "prompt"]);
    assert_eq!(ask_last_prompt["task"]["lane"], lane);
    assert_eq!(ask_last_prompt["index"], 1);
    assert_eq!(ask_last_prompt["changed_paths"][0]["path"], "README.md");
    let turn_file_patch = run_trail_json(
        temp.path(),
        &["agent", "turn", "1", "--file", "README.md", "--patch"],
    );
    assert_eq!(turn_file_patch["index"], 1);
    assert_eq!(turn_file_patch["diff"]["file_filter"], "README.md");
    assert!(turn_file_patch["diff"]["diff"]["files"][0]["patch"]
        .as_str()
        .unwrap()
        .contains("diagnostic complete"));
    let turn_diff = run_trail_json(temp.path(), &["agent", "turn-diff", "latest", "--patch"]);
    assert_eq!(turn_diff["target_kind"], "turn");
    assert_eq!(turn_diff["turn_id"], turn_latest["turn_id"]);
    assert!(turn_diff["diff"]["files"][0]["patch"]
        .as_str()
        .unwrap()
        .contains("diagnostic complete"));
    let turn_diff_specific = run_trail_json(
        temp.path(),
        &[
            "agent",
            "turn-diff",
            "latest",
            "--turn",
            "1",
            "--file",
            "README.md",
            "--patch",
        ],
    );
    assert_eq!(turn_diff_specific["target_kind"], "turn");
    assert_eq!(turn_diff_specific["file_filter"], "README.md");
    assert!(turn_diff_specific["diff"]["files"][0]["patch"]
        .as_str()
        .unwrap()
        .contains("diagnostic complete"));

    let task_readme = Path::new(task_workdir).join("README.md");
    let task_readme = task_readme.to_string_lossy().to_string();
    let why_workdir_path = run_trail_json(temp.path(), &["agent", "why", &task_readme]);
    assert_eq!(why_workdir_path["task"]["lane"], lane);
    assert_eq!(why_workdir_path["path"], "README.md");

    let timeline = run_trail_json(temp.path(), &["agent", "timeline", "latest"]);
    assert_eq!(timeline["items"][0]["id"], changes["groups"][0]["turn_id"]);

    let last_turn_diff = run_trail_json(temp.path(), &["agent", "diff", "latest", "--last-turn"]);
    assert_eq!(last_turn_diff["target_kind"], "turn");
    assert!(last_turn_diff["diff"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "README.md"));

    let turn_patch = run_trail_json(
        temp.path(),
        &["agent", "diff", "latest", "--turn", "1", "--patch"],
    );
    assert!(turn_patch["diff"]["files"][0]["patch"]
        .as_str()
        .unwrap()
        .contains("diagnostic complete"));
    let file_turn_patch = run_trail_json(
        temp.path(),
        &[
            "agent",
            "diff",
            "latest",
            "--turn",
            "1",
            "--file",
            "README.md",
            "--patch",
        ],
    );
    assert_eq!(file_turn_patch["file_filter"], "README.md");
    assert_eq!(
        file_turn_patch["diff"]["files"].as_array().unwrap().len(),
        1
    );
    assert_eq!(file_turn_patch["diff"]["files"][0]["path"], "README.md");
    assert!(file_turn_patch["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|suggestion| suggestion["command"]
            .as_str()
            .unwrap()
            .contains("agent why")));

    let checkpoint = changes["groups"][0]["checkpoint"].as_str().unwrap();
    let checkpoint_alias = checkpoint.replacen("change_", "checkpoint_", 1);
    let before_turn = changes["groups"][0]["before_change"].as_str().unwrap();
    let checkpoint_diff = run_trail_json(
        temp.path(),
        &["agent", "diff", "latest", "--checkpoint", &checkpoint_alias],
    );
    assert_eq!(checkpoint_diff["after_change"], checkpoint);

    let review = run_trail_json(temp.path(), &["agent", "review", "latest"]);
    assert_eq!(review["task"]["lane"], lane);
    assert!(review["priorities"][0]["diff_command"]
        .as_str()
        .unwrap()
        .contains("--file README.md"));

    let undone = run_trail_json(
        temp.path(),
        &["agent", "undo-last", "latest", "--prompt", "tail-marker"],
    );
    assert_eq!(undone["target_change"], before_turn);
    let after_rewind_diff = run_trail_json(temp.path(), &["agent", "diff", "latest"]);
    assert!(after_rewind_diff["diff"]["files"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[cfg(unix)]
fn run_agent_acp_stub_session(workspace: &Path, agent: &Path) {
    let mut child = Command::new(trail_bin())
        .arg("--workspace")
        .arg(workspace)
        .arg("agent")
        .arg("acp")
        .arg("run")
        .arg("claude-code")
        .arg("--no-mcp")
        .arg("--")
        .arg(agent)
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
    assert_eq!(init_response["id"], 0);

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"session/new","params":{{"cwd":"{}"}}}}"#,
        workspace.display()
    )
    .unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    let session_response: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(session_response["id"], 1);

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{{"sessionId":"{}","prompt":[{{"type":"text","text":"edit README with enough planning context to exceed the transcript preview window before this unique tail-marker appears at the end of the prompt"}}]}}}}"#,
        session_response["result"]["sessionId"].as_str().unwrap()
    )
    .unwrap();
    loop {
        line.clear();
        stdout.read_line(&mut line).unwrap();
        let message: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        if message.get("id").and_then(|id| id.as_i64()) == Some(2) {
            assert_eq!(message["result"]["stopReason"], "end_turn");
            break;
        }
    }
    drop(stdin);

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "agent acp failed\nstdout tail:\n{}\nstderr:\n{}",
        line,
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn agent_start_custom_command_applies_task_to_git_with_guided_flow() {
    if !git_available() {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    run_git(temp.path(), &["init"]);
    run_git(temp.path(), &["config", "user.email", "trail@example.test"]);
    run_git(temp.path(), &["config", "user.name", "Trail Test"]);
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    run_git(temp.path(), &["add", "README.md"]);
    run_git(temp.path(), &["commit", "-m", "initial"]);
    let initial_git_head = git_output(temp.path(), &["rev-parse", "HEAD"]);
    let initial_git_index = fs::read(temp.path().join(".git/index")).unwrap();
    Trail::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();
    let empty_guide = run_trail_json(temp.path(), &["agent", "guide"]);
    assert_eq!(empty_guide["status"], "empty");
    assert!(empty_guide["headline"]
        .as_str()
        .unwrap()
        .contains("Set up one editor"));
    assert!(empty_guide["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|step| step["command"] == "trail agent acp setup claude-code --editor vscode"));
    let edit_script = temp.path().join("edit-readme.sh");
    fs::write(
        &edit_script,
        "#!/bin/sh\nset -eu\nprintf '%s\\n' 'hello' 'edited by custom agent' > README.md\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&edit_script).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&edit_script, permissions).unwrap();

    let started = run_trail_json(
        temp.path(),
        &[
            "agent",
            "start",
            "--provider",
            "claude-code",
            "--name",
            "doc-edit",
            "--",
            edit_script.to_str().unwrap(),
        ],
    );
    assert_eq!(started["status"], "completed");
    let task_name = started["task"]["name"].as_str().unwrap();
    assert!(task_name.starts_with("agent-doc-edit-"));
    assert_eq!(started["task"]["title"], "doc edit");
    let task_workdir = started["task"]["workdir"].as_str().unwrap();
    assert!(std::path::Path::new(task_workdir).is_dir());
    assert_eq!(started["workdir"].as_str().unwrap(), task_workdir);
    assert!(started["recorded"]["operation"].as_str().is_some());
    assert_eq!(
        git_output(temp.path(), &["rev-parse", "HEAD"]),
        initial_git_head
    );
    assert_eq!(
        fs::read(temp.path().join(".git/index")).unwrap(),
        initial_git_index
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "hello\n"
    );

    let task_guide = run_trail_json(temp.path(), &["agent", "guide", "latest"]);
    assert_eq!(task_guide["task"]["lane"], task_name);
    assert_eq!(task_guide["status"], "ready");
    assert!(task_guide["current_state"]
        .as_str()
        .unwrap()
        .contains("changed file"));
    assert!(task_guide["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|step| step["command"] == format!("trail agent action {task_name}")));
    assert!(task_guide["steps"].as_array().unwrap().iter().any(|step| {
        step["command"]
            .as_str()
            .unwrap()
            .contains("trail agent ask --selector")
    }));
    assert!(task_guide["concepts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|concept| concept["name"] == "Apply"));
    let ask_help = run_trail_json(temp.path(), &["agent", "ask", "help", "me"]);
    assert_eq!(ask_help["task"]["lane"], task_name);
    assert_eq!(ask_help["status"], "ready");
    assert!(ask_help["headline"]
        .as_str()
        .unwrap()
        .contains("Use `doc edit` as one agent task"));

    for (key, value) in [
        ("lane.require_test_gate", "true"),
        ("lane.required_test_suites", "smoke"),
        ("lane.require_eval_gate", "true"),
        ("lane.required_eval_suites", "quality"),
    ] {
        run_trail_json(temp.path(), &["config", "set", key, value]);
    }

    let unreviewed_land = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .args(["agent", "land", "latest", "--dry-run"])
        .output()
        .unwrap();
    assert!(!unreviewed_land.status.success());
    assert!(String::from_utf8_lossy(&unreviewed_land.stderr).contains("has not been reviewed"));
    assert_eq!(
        git_output(temp.path(), &["rev-parse", "HEAD"]),
        initial_git_head
    );
    assert_eq!(
        fs::read(temp.path().join(".git/index")).unwrap(),
        initial_git_index
    );
    let reviewed = run_trail_json(
        temp.path(),
        &[
            "agent",
            "mark-reviewed",
            "latest",
            "--note",
            "reviewed before explicit land",
        ],
    );
    assert_eq!(reviewed["task"]["lane"], task_name);
    let ungated_land = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .args(["agent", "land", "latest"])
        .output()
        .unwrap();
    assert!(!ungated_land.status.success());
    assert!(String::from_utf8_lossy(&ungated_land.stderr).contains("missing_latest_test"));
    assert_eq!(
        git_output(temp.path(), &["rev-parse", "HEAD"]),
        initial_git_head
    );
    assert_eq!(
        fs::read(temp.path().join(".git/index")).unwrap(),
        initial_git_index
    );

    let agent_test = run_trail_json(
        temp.path(),
        &[
            "agent",
            "test",
            "latest",
            "--suite",
            "smoke",
            "--",
            "sh",
            "-c",
            "test -f README.md",
        ],
    );
    assert_eq!(agent_test["workdir"], task_workdir);
    assert_eq!(agent_test["kind"], "test");
    assert_eq!(agent_test["suite"], "smoke");
    assert_eq!(agent_test["success"], true);

    let agent_eval = run_trail_json(
        temp.path(),
        &[
            "agent",
            "eval",
            "latest",
            "--suite",
            "quality",
            "--score",
            "1",
            "--threshold",
            "0.5",
            "--",
            "sh",
            "-c",
            "exit 0",
        ],
    );
    assert_eq!(agent_eval["workdir"], task_workdir);
    assert_eq!(agent_eval["kind"], "eval");
    assert_eq!(agent_eval["suite"], "quality");
    assert_eq!(agent_eval["success"], true);

    let operation_changes = run_trail_json(
        temp.path(),
        &["agent", "changes", "latest", "--by-operation"],
    );
    assert_eq!(operation_changes["grouping"], "operation");
    assert!(operation_changes["groups"][0]["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "README.md"));
    let file_changes = run_trail_json(temp.path(), &["agent", "changes", "latest", "--by-file"]);
    assert_eq!(file_changes["grouping"], "file");
    assert_eq!(file_changes["cards"][0]["key"], "README.md");
    assert!(file_changes["cards"][0]["review_command"]
        .as_str()
        .unwrap()
        .contains("agent file"));
    assert!(file_changes["cards"][0]["why_command"]
        .as_str()
        .unwrap()
        .contains("agent why"));
    let ask_changes_by_file = run_trail_json(
        temp.path(),
        &["agent", "ask", "show", "changes", "by", "file"],
    );
    assert_eq!(ask_changes_by_file["grouping"], "file");
    assert_eq!(ask_changes_by_file["cards"][0]["key"], "README.md");
    let ask_risky_files = run_trail_json(
        temp.path(),
        &["agent", "ask", "which", "files", "are", "risky"],
    );
    assert_eq!(ask_risky_files["grouping"], "file");
    assert_eq!(ask_risky_files["cards"][0]["key"], "README.md");
    let operation_timeline = run_trail_json(
        temp.path(),
        &["agent", "timeline", "latest", "--by-operation"],
    );
    assert_eq!(operation_timeline["mode"], "operation");
    assert_eq!(operation_timeline["items"][0]["kind"], "operation");
    assert!(operation_timeline["items"][0]["diff_command"]
        .as_str()
        .unwrap()
        .contains("--operation"));
    let whole_task_diff = run_trail_json(temp.path(), &["agent", "diff", "latest"]);
    assert_eq!(whole_task_diff["target_kind"], "task");
    assert!(whole_task_diff["diff"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "README.md"));
    let ask_task_diff = run_trail_json(temp.path(), &["agent", "ask", "show", "the", "diff"]);
    assert_eq!(ask_task_diff["task"]["lane"], task_name);
    assert_eq!(ask_task_diff["target_kind"], "task");
    assert!(ask_task_diff["diff"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "README.md"));
    assert!(ask_task_diff["diff"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "README.md"
            && path["patch"]
                .as_str()
                .is_some_and(|patch| !patch.is_empty())));

    let dry_run = run_trail_json(temp.path(), &["agent", "land", "latest", "--dry-run"]);
    assert_eq!(dry_run["status"], "ready");
    assert_eq!(dry_run["dry_run"], true);
    assert_eq!(dry_run["git_apply_plan"]["would_create_git_commit"], true);
    assert_eq!(dry_run["git_apply_plan"]["would_fast_forward"], true);
    assert!(dry_run["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|suggestion| suggestion["command"] == format!("trail agent land {task_name}")));
    assert!(dry_run["suggestions"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("Apply agent task: doc edit"));
    assert!(dry_run["git_export"].is_null());
    assert_eq!(
        git_output(temp.path(), &["rev-parse", "HEAD"]),
        initial_git_head
    );
    assert_eq!(
        fs::read(temp.path().join(".git/index")).unwrap(),
        initial_git_index
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "hello\n"
    );

    let ready = run_trail_json(temp.path(), &["agent", "ready", "latest"]);
    assert_eq!(ready["ready"], true);
    assert_eq!(ready["status"], "ready");
    assert_eq!(ready["risk"]["level"], "low");
    assert_eq!(ready["apply_preview"]["status"], "ready");
    assert_eq!(ready["default_apply_message"], "Apply agent task: doc edit");
    assert_eq!(
        ready["next"]["command"],
        format!("trail agent land {task_name}")
    );
    assert!(ready["summary"]
        .as_str()
        .unwrap()
        .contains("ready to apply"));
    let can_land_alias = run_trail_json(temp.path(), &["agent", "can-land", "latest"]);
    assert_eq!(can_land_alias["task"]["lane"], task_name);
    assert_eq!(can_land_alias["ready"], ready["ready"]);
    assert_eq!(can_land_alias["next"], ready["next"]);
    let ask_ready = run_trail_json(
        temp.path(),
        &["agent", "ask", "is", "it", "safe", "to", "land"],
    );
    assert_eq!(ask_ready["task"]["lane"], task_name);
    let ask_commit_message = run_trail_json(
        temp.path(),
        &[
            "agent", "ask", "what", "commit", "message", "should", "I", "use",
        ],
    );
    assert_eq!(ask_commit_message["task"]["lane"], task_name);
    assert_eq!(
        ask_commit_message["default_apply_message"],
        "Apply agent task: doc edit"
    );
    assert_eq!(ask_ready["ready"], ready["ready"]);
    assert_eq!(ask_ready["next"], ready["next"]);
    let ask_merge = run_trail_json(temp.path(), &["agent", "ask", "can", "I", "merge"]);
    assert_eq!(ask_merge["task"]["lane"], task_name);
    assert_eq!(ask_merge["ready"], ready["ready"]);
    assert_eq!(ask_merge["next"], ready["next"]);
    let validate = run_trail_json(temp.path(), &["agent", "validate", "latest"]);
    let ask_tested = run_trail_json(temp.path(), &["agent", "ask", "is", "it", "tested"]);
    assert_eq!(ask_tested["task"]["lane"], task_name);
    assert_eq!(ask_tested["status"], validate["status"]);
    assert_eq!(ask_tested["next"], validate["next"]);
    let ask_why_apply =
        run_trail_json(temp.path(), &["agent", "ask", "why", "can't", "I", "apply"]);
    assert_eq!(ask_why_apply["task"]["lane"], task_name);
    assert_eq!(ask_why_apply["ready"], ready["ready"]);
    assert_eq!(ask_why_apply["next"], ready["next"]);

    let diagnosis = run_trail_json(temp.path(), &["agent", "diagnose", "latest"]);
    assert_eq!(diagnosis["task"]["name"], task_name);
    assert_eq!(diagnosis["status"], "ok");
    assert_eq!(diagnosis["severity"], "low");
    assert_eq!(diagnosis["ready"], true);
    assert_eq!(diagnosis["likely_issue"], "no blocking issue detected");
    let ask_blocking = run_trail_json(
        temp.path(),
        &["agent", "ask", "what", "is", "blocking", "this", "task"],
    );
    assert_eq!(ask_blocking["task"]["name"], task_name);
    assert_eq!(ask_blocking["status"], diagnosis["status"]);
    assert_eq!(ask_blocking["likely_issue"], diagnosis["likely_issue"]);
    assert!(diagnosis["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap().contains("latest checkpoint")));
    assert!(diagnosis["recovery_options"]
        .as_array()
        .unwrap()
        .iter()
        .any(|suggestion| suggestion["command"]
            .as_str()
            .unwrap()
            .contains("turn-diff")));

    let applied = run_trail_json(temp.path(), &["agent", "apply", "latest"]);
    assert_eq!(applied["status"], "applied");
    assert_eq!(applied["fast_forwarded"], true);
    let commit = applied["git_export"]["commit"].as_str().unwrap();
    assert_eq!(git_output(temp.path(), &["rev-parse", "HEAD"]), commit);
    assert_eq!(
        git_output(temp.path(), &["log", "-1", "--pretty=%s"]),
        "Apply agent task: doc edit"
    );
    assert_eq!(
        git_output(temp.path(), &["show", "HEAD:README.md"]),
        "hello\nedited by custom agent"
    );
    run_trail_json(
        temp.path(),
        &["config", "set", "lane.require_test_gate", "false"],
    );
    run_trail_json(
        temp.path(),
        &["config", "set", "lane.require_eval_gate", "false"],
    );
    run_trail_json(
        temp.path(),
        &["config", "set", "lane.required_test_suites", ""],
    );
    run_trail_json(
        temp.path(),
        &["config", "set", "lane.required_eval_suites", ""],
    );

    let view = run_trail_json(temp.path(), &["agent", "view", "latest"]);
    assert_eq!(view["task"]["status"], "applied");
    let repeated_dry_run = run_trail_json(temp.path(), &["agent", "apply", "latest", "--dry-run"]);
    assert_eq!(repeated_dry_run["status"], "already_applied");
    assert_eq!(repeated_dry_run["dry_run"], true);
    assert_eq!(
        repeated_dry_run["git_apply_plan"]["would_create_git_commit"],
        false
    );
    assert_eq!(repeated_dry_run["git_export"], serde_json::Value::Null);
    assert!(repeated_dry_run["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning.as_str().unwrap().contains("agent continue")));
    assert!(repeated_dry_run["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|suggestion| suggestion["command"]
            .as_str()
            .unwrap()
            .contains("agent continue")));
    let repeated_apply = run_trail_json(temp.path(), &["agent", "apply", "latest"]);
    assert_eq!(repeated_apply["status"], "already_applied");
    assert_eq!(repeated_apply["fast_forwarded"], false);
    assert_eq!(git_output(temp.path(), &["rev-parse", "HEAD"]), commit);
    let ready_after_apply = run_trail_json(temp.path(), &["agent", "ready", "latest"]);
    assert_eq!(ready_after_apply["status"], "applied");
    assert!(ready_after_apply["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|suggestion| suggestion["command"]
            .as_str()
            .unwrap()
            .contains("agent continue")));

    let archived = run_trail_json(
        temp.path(),
        &["agent", "archive", "latest", "--note", "landed"],
    );
    assert_eq!(archived["archived"], true);
    assert_eq!(archived["previous_archived"], false);
    assert_eq!(archived["task"]["archived"], true);
    assert!(archived["summary"]
        .as_str()
        .unwrap()
        .contains("hidden from the default agent inbox"));
    let list_after_archive = run_trail_json(temp.path(), &["agent", "list"]);
    assert!(list_after_archive["tasks"].as_array().unwrap().is_empty());
    let list_all_after_archive = run_trail_json(temp.path(), &["agent", "list", "--all"]);
    assert_eq!(list_all_after_archive["tasks"][0]["name"], task_name);
    assert_eq!(list_all_after_archive["tasks"][0]["archived"], true);
    let inbox_after_archive = run_trail_json(temp.path(), &["agent", "inbox"]);
    assert_eq!(inbox_after_archive["total"], 0);
    let inbox_all_after_archive = run_trail_json(temp.path(), &["agent", "inbox", "--all"]);
    assert_eq!(inbox_all_after_archive["total"], 1);
    assert_eq!(inbox_all_after_archive["archived_count"], 1);
    assert_eq!(inbox_all_after_archive["items"][0]["attention"], "archived");
    let board_after_archive = run_trail_json(temp.path(), &["agent", "board"]);
    assert_eq!(board_after_archive["total"], 0);
    let board_all_after_archive = run_trail_json(temp.path(), &["agent", "board", "--all"]);
    assert_eq!(board_all_after_archive["total"], 1);
    assert_eq!(board_all_after_archive["archived_count"], 1);
    assert_eq!(board_all_after_archive["columns"][0]["key"], "archived");
    let status_after_archive = run_trail_json(temp.path(), &["agent", "status"]);
    assert_eq!(status_after_archive["status"], "empty");

    let unarchived = run_trail_json(temp.path(), &["agent", "unarchive", &task_name]);
    assert_eq!(unarchived["archived"], false);
    assert_eq!(unarchived["previous_archived"], true);
    assert_eq!(unarchived["task"]["archived"], false);
    let list_after_unarchive = run_trail_json(temp.path(), &["agent", "list"]);
    assert_eq!(list_after_unarchive["tasks"][0]["name"], task_name);
    assert_eq!(list_after_unarchive["tasks"][0]["archived"], false);

    let followup_script = temp.path().join("followup.sh");
    fs::write(
        &followup_script,
        "#!/bin/sh\nset -eu\nprintf '%s\\n' 'follow-up task' > FOLLOWUP.md\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&followup_script).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&followup_script, permissions).unwrap();

    let followup = run_trail_json(
        temp.path(),
        &[
            "agent",
            "continue",
            &task_name,
            "--provider",
            "claude-code",
            "--name",
            "doc-followup",
            "--",
            followup_script.to_str().unwrap(),
        ],
    );
    assert_eq!(followup["source_task"]["name"], task_name);
    assert_eq!(followup["run"]["status"], "completed");
    let followup_task = followup["run"]["task"]["name"].as_str().unwrap();
    assert_ne!(followup_task, task_name);
    assert!(followup_task.starts_with("agent-doc-followup-"));
    assert!(followup["run"]["task"]["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "FOLLOWUP.md"));
    assert!(followup["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|suggestion| suggestion["command"]
            .as_str()
            .unwrap()
            .contains("agent land")));

    let followup_reviewed = run_trail_json(
        temp.path(),
        &[
            "agent",
            "mark-reviewed",
            followup_task,
            "--note",
            "follow-up reviewed before land preview",
        ],
    );
    assert_eq!(followup_reviewed["task"]["lane"], followup_task);

    let followup_dry_run =
        run_trail_json(temp.path(), &["agent", "land", followup_task, "--dry-run"]);
    assert_eq!(followup_dry_run["status"], "ready");
    assert_eq!(
        followup_dry_run["git_apply_plan"]["would_fast_forward"],
        true
    );

    let finish_dry_run =
        run_trail_json(temp.path(), &["agent", "ship", followup_task, "--dry-run"]);
    assert_eq!(finish_dry_run["status"], "ready");
    assert_eq!(finish_dry_run["dry_run"], true);
    assert_eq!(finish_dry_run["apply"]["status"], "ready");
    assert_eq!(finish_dry_run["would_archive"], true);
    assert_eq!(finish_dry_run["archive"], serde_json::Value::Null);
    assert!(finish_dry_run["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|suggestion| suggestion["command"]
            .as_str()
            .unwrap()
            .contains("agent finish")));
    assert_eq!(git_output(temp.path(), &["rev-parse", "HEAD"]), commit);

    let finished = run_trail_json(
        temp.path(),
        &[
            "agent",
            "finish",
            followup_task,
            "-m",
            "Apply follow-up task",
            "--note",
            "done",
        ],
    );
    assert_eq!(finished["status"], "finished");
    assert_eq!(finished["dry_run"], false);
    assert_eq!(finished["apply"]["status"], "applied");
    assert_eq!(finished["archive"]["archived"], true);
    assert_eq!(finished["archive"]["previous_archived"], false);
    assert_eq!(finished["archive"]["note"], "done");
    assert_eq!(finished["task"]["archived"], true);
    let followup_commit = finished["apply"]["git_export"]["commit"].as_str().unwrap();
    assert_eq!(
        git_output(temp.path(), &["rev-parse", "HEAD"]),
        followup_commit
    );
    assert_eq!(
        git_output(temp.path(), &["log", "-1", "--pretty=%s"]),
        "Apply follow-up task"
    );
    assert_eq!(
        git_output(temp.path(), &["show", "HEAD:FOLLOWUP.md"]),
        "follow-up task"
    );
    let list_after_finish = run_trail_json(temp.path(), &["agent", "list"]);
    assert!(list_after_finish["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .all(|task| task["name"] != followup_task));
    let list_all_after_finish = run_trail_json(temp.path(), &["agent", "list", "--all"]);
    assert!(list_all_after_finish["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|task| task["name"] == followup_task && task["archived"] == true));
}

#[cfg(unix)]
#[test]
fn acp_relay_captures_session_prompt_mcp_and_workdir_edits() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let stub_agent = temp.path().join("stub-acp-agent.sh");
    let session_request_log = temp.path().join(".trail/session-new.jsonl");
    let lane_workdir = temp
        .path()
        .canonicalize()
        .unwrap()
        .join(".trail/worktrees/acp-test");
    fs::write(
        &stub_agent,
        format!(
            r#"#!/bin/sh
set -eu
IFS= read -r init
printf '%s\n' '{{"jsonrpc":"2.0","id":0,"result":{{"protocolVersion":1,"agentCapabilities":{{}}}}}}'
IFS= read -r session_new
printf '%s\n' "$session_new" > "{}"
printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"sessionId":"sess_stub"}}}}'
IFS= read -r prompt
printf '%s\n' '{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"sess_stub","update":{{"sessionUpdate":"available_commands_update","commands":[{{"name":"write_file","description":"large command description"}}]}}}}}}'
printf '%s\n' '{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"sess_stub","update":{{"sessionUpdate":"tool_call","toolCallId":"tool_1","title":"write README","kind":"edit","status":"pending"}}}}}}'
printf '%s\n' '{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"sess_stub","update":{{"sessionUpdate":"tool_call_update","toolCallId":"tool_1","status":"completed"}}}}}}'
printf '%s\n' '{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"sess_stub","update":{{"sessionUpdate":"agent_message_chunk","messageId":"msg_1","content":{{"type":"text","text":"done"}}}}}}}}'
printf '%s\n' 'changed by stub agent' > "{}/README.md"
printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"stopReason":"end_turn"}}}}'
"#,
            session_request_log.display(),
            lane_workdir.display()
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&stub_agent).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&stub_agent, permissions).unwrap();

    let mut child = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("acp")
        .arg("relay")
        .arg("--lane")
        .arg("acp-test")
        .arg("--materialize")
        .arg("--provider")
        .arg("test-stub")
        .arg("--")
        .arg(&stub_agent)
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
    assert_eq!(init_response["result"]["_meta"]["trail"]["relay"], true);

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"session/new","params":{{"cwd":"{}","mcpServers":[]}}}}"#,
        temp.path().display()
    )
    .unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    let session_response: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(session_response["result"]["sessionId"], "sess_stub");

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{{"sessionId":"sess_stub","prompt":[{{"type":"text","text":"change README"}}]}}}}"#
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
    assert!(mcp_servers.iter().any(|server| server["name"] == "trail"));

    let db = Trail::open(temp.path()).unwrap();
    let mapping = db.try_lane_acp_session("sess_stub").unwrap().unwrap();
    let lane = db.lane_details("acp-test").unwrap();
    assert_eq!(mapping.lane_id, lane.record.lane_id);
    assert_eq!(
        lane.branch.workdir.as_deref(),
        Some(lane_workdir.to_str().unwrap())
    );

    let session = db.show_lane_session(&mapping.trail_session_id).unwrap();
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
    let envelope = turn
        .turn_envelope
        .as_ref()
        .expect("ACP prompt must have a typed turn envelope");
    assert_eq!(envelope.outcome.status.as_deref(), Some("completed"));
    assert!(!envelope.outcome.no_changes);
    assert!(envelope.outcome.checkpoint.is_some());
    assert_eq!(envelope.outcome.checkpoint, turn.turn.after_change);
    assert!(turn
        .events
        .iter()
        .any(|event| event.event_type == "workdir_recorded"));
    let status = db.lane_status("acp-test").unwrap();
    assert_eq!(status.workdir_state, Some(WorktreeState::Clean));
    assert!(status
        .changed_paths
        .iter()
        .any(|path| path.path == "README.md"));

    let acp_sessions = run_trail_json(
        temp.path(),
        &["agent", "acp", "sessions", "--lane", "acp-test"],
    );
    assert_eq!(acp_sessions["sessions"][0]["acp_session_id"], "sess_stub");

    let transcript = run_trail_json(temp.path(), &["transcript", "acp-test"]);
    assert_eq!(transcript["resolved_kind"], "lane");
    assert_eq!(transcript["acp_session"]["acp_session_id"], "sess_stub");
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
    assert_eq!(
        transcript["turns"][0]["checkpoint"],
        transcript["turns"][0]["turn_envelope"]["outcome"]["checkpoint"]
    );

    let turn_alias = run_trail_json(
        temp.path(),
        &["turn", "show", session.turns[0].turn_id.as_str()],
    );
    assert_eq!(turn_alias["turn"]["turn_id"], session.turns[0].turn_id);

    let workspace_status = run_trail_json(temp.path(), &["status"]);
    assert!(workspace_status["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|suggestion| suggestion["command"]
            .as_str()
            .unwrap()
            .contains("trail transcript acp-test")));
}

#[cfg(unix)]
#[test]
fn acp_relay_remaps_workspace_file_resources_into_lane() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let stub_agent = temp.path().join("resource-acp-agent.sh");
    let forwarded_prompt_log = temp.path().join(".trail/forwarded-resource-prompt.jsonl");
    let lane_workdir = temp
        .path()
        .canonicalize()
        .unwrap()
        .join(".trail/worktrees/acp-resource-test");
    fs::write(
        &stub_agent,
        format!(
            r#"#!/bin/sh
set -eu
IFS= read -r init
printf '%s\n' '{{"jsonrpc":"2.0","id":0,"result":{{"protocolVersion":1,"agentCapabilities":{{}}}}}}'
IFS= read -r session_new
printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"sessionId":"sess_resource_stub"}}}}'
IFS= read -r prompt
printf '%s\n' "$prompt" > "{}"
resource_uri=${{prompt#*\"uri\":\"}}
resource_uri=${{resource_uri%%\"*}}
case "$resource_uri" in
  file://*) resource_path=${{resource_uri#file://}} ;;
  *) exit 41 ;;
esac
printf '%s\n' 'changed through forwarded resource' > "$resource_path"
printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"stopReason":"end_turn"}}}}'
"#,
            forwarded_prompt_log.display(),
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&stub_agent).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&stub_agent, permissions).unwrap();

    let mut child = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("acp")
        .arg("relay")
        .arg("--lane")
        .arg("acp-resource-test")
        .arg("--materialize")
        .arg("--provider")
        .arg("test-stub")
        .arg("--")
        .arg(&stub_agent)
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
        r#"{{"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{{"sessionId":"sess_resource_stub","prompt":[{{"type":"resource","resource":{{"uri":"file://{}/README.md","text":"hello"}}}},{{"type":"resource_link","name":"source","uri":"file://{}/src/lib.rs"}},{{"type":"resource_link","name":"external","uri":"file:///outside/shared.md"}},{{"type":"resource_link","name":"remote","uri":"https://example.com/context"}}]}}}}"#,
        temp.path().display(),
        temp.path().display(),
    )
    .unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    assert!(line.contains(r#""id":2"#), "unexpected relay frame: {line}");
    drop(stdin);

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "relay failed\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let forwarded_prompt: serde_json::Value =
        serde_json::from_slice(&fs::read(&forwarded_prompt_log).unwrap()).unwrap();
    assert_eq!(
        forwarded_prompt["params"]["prompt"][0]["resource"]["uri"],
        format!("file://{}/README.md", lane_workdir.display())
    );
    assert_eq!(
        forwarded_prompt["params"]["prompt"][1]["uri"],
        format!("file://{}/src/lib.rs", lane_workdir.display())
    );
    assert_eq!(
        forwarded_prompt["params"]["prompt"][2]["uri"],
        "file:///outside/shared.md"
    );
    assert_eq!(
        forwarded_prompt["params"]["prompt"][3]["uri"],
        "https://example.com/context"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "hello\n",
        "the ACP agent must not edit the main workspace"
    );
    assert_eq!(
        fs::read_to_string(lane_workdir.join("README.md")).unwrap(),
        "changed through forwarded resource\n"
    );

    let db = Trail::open(temp.path()).unwrap();
    let status = db.lane_status("acp-resource-test").unwrap();
    assert!(status
        .changed_paths
        .iter()
        .any(|path| path.path == "README.md"));
    let mapping = db
        .try_lane_acp_session("sess_resource_stub")
        .unwrap()
        .unwrap();
    let session = db.show_lane_session(&mapping.trail_session_id).unwrap();
    let turn = db.show_lane_turn(&session.turns[0].turn_id).unwrap();
    assert!(turn
        .operations
        .iter()
        .any(|operation| operation.kind == OperationKind::LaneRecord));
}

#[cfg(target_os = "macos")]
#[test]
fn acp_relay_blocks_materialized_agent_writes_to_main_workspace() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let escaped_path = temp.path().join("ESCAPED.md");
    let stub_agent = temp.path().join("escaping-acp-agent.sh");
    fs::write(
        &stub_agent,
        format!(
            r#"#!/bin/sh
set -eu
IFS= read -r init
printf '%s\n' '{{"jsonrpc":"2.0","id":0,"result":{{"protocolVersion":1,"agentCapabilities":{{}}}}}}'
IFS= read -r session_new
printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"sessionId":"sess_escape_stub"}}}}'
IFS= read -r prompt
printf '%s\n' 'escaped' > "{}"
printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"stopReason":"end_turn"}}}}'
"#,
            escaped_path.display(),
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&stub_agent).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&stub_agent, permissions).unwrap();

    let mut child = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("acp")
        .arg("relay")
        .arg("--lane")
        .arg("acp-escape-test")
        .arg("--materialize")
        .arg("--provider")
        .arg("test-stub")
        .arg("--")
        .arg(&stub_agent)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":0,"method":"initialize","params":{{"protocolVersion":1}}}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"session/new","params":{{"cwd":"{}","mcpServers":[]}}}}"#,
        temp.path().display()
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{{"sessionId":"sess_escape_stub","prompt":[{{"type":"text","text":"escape"}}]}}}}"#
    )
    .unwrap();
    drop(stdin);

    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "relay unexpectedly allowed an ACP agent to write the main workspace"
    );
    assert!(
        stderr.contains("Operation not permitted") || stderr.contains("Permission denied"),
        "relay failed for an unexpected reason:\n{stderr}"
    );
    assert!(
        !escaped_path.exists(),
        "materialized ACP agent escaped its Trail lane"
    );
}

#[cfg(unix)]
#[test]
fn acp_relay_closes_failed_turn_on_upstream_crash() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let output =
        run_stub_acp_relay_scenario(temp.path(), "acp-crash", &["--crash-after-update"], false);
    assert!(
        !output.status.success(),
        "relay should fail on upstream crash\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let db = Trail::open(temp.path()).unwrap();
    let mapping = db
        .try_lane_acp_session("sess_stub_doctor")
        .unwrap()
        .unwrap();
    let session = db.show_lane_session(&mapping.trail_session_id).unwrap();
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

#[cfg(unix)]
#[test]
fn acp_relay_closes_failed_turn_on_malformed_upstream_json() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let output = run_stub_acp_relay_scenario(
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

    let db = Trail::open(temp.path()).unwrap();
    let mapping = db
        .try_lane_acp_session("sess_stub_doctor")
        .unwrap()
        .unwrap();
    let session = db.show_lane_session(&mapping.trail_session_id).unwrap();
    assert_eq!(session.turns[0].status, "failed");
    assert!(session.events.iter().any(|event| {
        event.event_type == "acp_relay_turn_closed"
            && event
                .payload
                .as_ref()
                .is_some_and(|payload| payload.to_string().contains("malformed JSON"))
    }));
}

#[cfg(unix)]
#[test]
fn acp_relay_truncates_oversized_assistant_capture() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let output = run_stub_acp_relay_scenario(
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

    let db = Trail::open(temp.path()).unwrap();
    let mapping = db
        .try_lane_acp_session("sess_stub_doctor")
        .unwrap()
        .unwrap();
    let session = db.show_lane_session(&mapping.trail_session_id).unwrap();
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

#[cfg(unix)]
#[test]
fn acp_relay_mirrors_permission_requests_to_approvals() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let output = run_stub_acp_relay_scenario(
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

    let db = Trail::open(temp.path()).unwrap();
    let approvals = db
        .list_lane_approvals(Some("acp-permission"), Some("pending"))
        .unwrap();
    assert!(approvals
        .iter()
        .any(|approval| approval.action == "acp_permission"
            && approval.summary == "approve diagnostic write"));
}

#[cfg(unix)]
#[test]
fn acp_relay_persists_assistant_messages_around_tool_events_in_order() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let output = run_stub_acp_relay_scenario(
        temp.path(),
        "acp-interleaved",
        &["--assistant-before-tool", "Before tool."],
        true,
    );
    assert!(
        output.status.success(),
        "relay should preserve assistant/tool interleaving\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let view = run_trail_json(temp.path(), &["agent", "view", "acp-interleaved"]);
    let turn = &view["transcript"]["turns"][0];
    let messages = turn["messages"].as_array().unwrap();
    let assistant_messages = messages
        .iter()
        .filter(|message| message["role"] == "assistant")
        .collect::<Vec<_>>();
    let assistant_bodies = assistant_messages
        .iter()
        .map(|message| message["body"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        assistant_bodies,
        vec!["Before tool.", "diagnostic complete"]
    );

    let first_assistant_id = assistant_messages[0]["message_id"].as_str().unwrap();
    let second_assistant_id = assistant_messages[1]["message_id"].as_str().unwrap();
    let events = turn["events"].as_array().unwrap();
    let first_message_idx = events
        .iter()
        .position(|event| {
            event["event_type"] == "message_added" && event["message_id"] == first_assistant_id
        })
        .unwrap();
    let tool_call_idx = events
        .iter()
        .position(|event| event["event_type"] == "tool_call")
        .unwrap();
    let tool_update_idx = events
        .iter()
        .position(|event| event["event_type"] == "tool_call_update")
        .unwrap();
    let second_message_idx = events
        .iter()
        .position(|event| {
            event["event_type"] == "message_added" && event["message_id"] == second_assistant_id
        })
        .unwrap();

    assert!(
        first_message_idx < tool_call_idx,
        "assistant text before a tool should be recorded before the tool call"
    );
    assert!(
        tool_update_idx < second_message_idx,
        "assistant text after a tool should be recorded after the tool update"
    );
}

#[cfg(unix)]
#[test]
fn acp_relay_records_cancel_requests() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let stub_agent = write_stub_acp_agent(
        temp.path(),
        "cancel-stub-acp-agent.sh",
        StubAcpAgentOptions {
            sleep_before_result_ms: Some(200),
            ..StubAcpAgentOptions::new("sess_stub_doctor")
        },
    );

    let mut child = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("acp")
        .arg("relay")
        .arg("--lane")
        .arg("acp-cancel")
        .arg("--materialize")
        .arg("--provider")
        .arg("test-stub")
        .arg("--")
        .arg(stub_agent)
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
        r#"{{"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{{"sessionId":"sess_stub_doctor","prompt":[{{"type":"text","text":"run diagnostic"}}]}}}}"#
    )
    .unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    assert!(line.contains("session/update"));
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":3,"method":"session/cancel","params":{{"sessionId":"sess_stub_doctor"}}}}"#
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

    let db = Trail::open(temp.path()).unwrap();
    let mapping = db
        .try_lane_acp_session("sess_stub_doctor")
        .unwrap()
        .unwrap();
    let session = db.show_lane_session(&mapping.trail_session_id).unwrap();
    assert!(session
        .events
        .iter()
        .any(|event| event.event_type == "acp_prompt_cancel_requested"));
}

#[cfg(unix)]
fn run_stub_acp_relay_scenario(
    workspace: &Path,
    lane: &str,
    test_agent_args: &[&str],
    expect_prompt_result: bool,
) -> std::process::Output {
    run_stub_acp_relay_scenario_with_session_id(
        workspace,
        lane,
        "sess_stub_doctor",
        test_agent_args,
        expect_prompt_result,
    )
}

#[cfg(unix)]
fn run_stub_acp_relay_scenario_with_session_id(
    workspace: &Path,
    lane: &str,
    session_id: &str,
    test_agent_args: &[&str],
    expect_prompt_result: bool,
) -> std::process::Output {
    let lane_workdir = workspace
        .canonicalize()
        .unwrap()
        .join(format!(".trail/worktrees/{lane}"));
    let mut options = StubAcpAgentOptions::new(session_id);
    options.lane_workdir = Some(&lane_workdir);
    let mut index = 0;
    while index < test_agent_args.len() {
        match test_agent_args[index] {
            "--crash-after-update" => options.crash_after_update = true,
            "--malformed-after-update" => options.malformed_after_update = true,
            "--request-permission" => options.request_permission = true,
            "--assistant-before-tool" => {
                index += 1;
                options.assistant_text_before_tool = Some(test_agent_args[index].to_string());
            }
            "--sleep-before-result-ms" => {
                index += 1;
                options.sleep_before_result_ms = Some(test_agent_args[index].parse().unwrap());
            }
            "--huge-message-bytes" => {
                index += 1;
                let bytes = test_agent_args[index].parse().unwrap();
                options.assistant_text = "x".repeat(bytes);
            }
            other => panic!("unsupported stub ACP arg {other}"),
        }
        index += 1;
    }
    let stub_agent = write_stub_acp_agent(workspace, &format!("{lane}-stub-acp-agent.sh"), options);
    let mut child = Command::new(trail_bin())
        .arg("--workspace")
        .arg(workspace)
        .arg("acp")
        .arg("relay")
        .arg("--lane")
        .arg(lane)
        .arg("--materialize")
        .arg("--provider")
        .arg("test-stub")
        .arg("--")
        .arg(stub_agent)
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

#[cfg(unix)]
#[test]
fn acp_relay_runs_three_synchronized_relays_on_distinct_lanes() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let workspace_a = temp.path().to_path_buf();
    let barrier_a = std::sync::Arc::clone(&barrier);
    let workspace_b = temp.path().to_path_buf();
    let barrier_b = std::sync::Arc::clone(&barrier);
    let workspace_c = temp.path().to_path_buf();
    let barrier_c = std::sync::Arc::clone(&barrier);
    let relay_a = thread::spawn(move || {
        barrier_a.wait();
        run_stub_acp_relay_scenario_with_session_id(
            &workspace_a,
            "acp-parallel-a",
            "sess_parallel_a",
            &["--sleep-before-result-ms", "100"],
            true,
        )
    });
    let relay_b = thread::spawn(move || {
        barrier_b.wait();
        run_stub_acp_relay_scenario_with_session_id(
            &workspace_b,
            "acp-parallel-b",
            "sess_parallel_b",
            &["--sleep-before-result-ms", "100"],
            true,
        )
    });
    let relay_c = thread::spawn(move || {
        barrier_c.wait();
        run_stub_acp_relay_scenario_with_session_id(
            &workspace_c,
            "acp-parallel-c",
            "sess_parallel_c",
            &["--sleep-before-result-ms", "100"],
            true,
        )
    });

    let output_a = relay_a.join().unwrap();
    let output_b = relay_b.join().unwrap();
    let output_c = relay_c.join().unwrap();
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
    assert!(
        output_c.status.success(),
        "relay c failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output_c.stdout),
        String::from_utf8_lossy(&output_c.stderr)
    );

    let db = Trail::open(temp.path()).unwrap();
    for (lane, session_id) in [
        ("acp-parallel-a", "sess_parallel_a"),
        ("acp-parallel-b", "sess_parallel_b"),
        ("acp-parallel-c", "sess_parallel_c"),
    ] {
        let mapping = db.try_lane_acp_session(session_id).unwrap().unwrap();
        let lane_details = db.lane_details(lane).unwrap();
        assert_eq!(mapping.lane_id, lane_details.record.lane_id);
        let session = db.show_lane_session(&mapping.trail_session_id).unwrap();
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
fn acp_relay_drains_delayed_terminal_frames_and_spill_before_finalizing() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let lane = "acp-finalization-order";
    let session_id = "sess_finalization_order";
    let lane_workdir = temp
        .path()
        .canonicalize()
        .unwrap()
        .join(format!(".trail/worktrees/{lane}"));
    let mut options = StubAcpAgentOptions::new(session_id);
    options.lane_workdir = Some(&lane_workdir);
    options.sleep_before_result_ms = Some(100);
    let stub_agent = write_stub_acp_agent(temp.path(), "finalization-order-agent.sh", options);

    let mut child = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .args([
            "acp",
            "relay",
            "--lane",
            lane,
            "--materialize",
            "--provider",
            "test-stub",
            "--",
        ])
        .arg(stub_agent)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = std::io::BufReader::new(child.stdout.take().unwrap());
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
        temp.path().display()
    )
    .unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    assert!(line.contains(session_id));

    let lock_path = temp.path().join(".trail/lock");
    fs::write(
        &lock_path,
        format!("pid={} created_at=0", std::process::id()),
    )
    .unwrap();
    let lock_remover = {
        let lock_path = lock_path.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(350));
            match fs::remove_file(lock_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("failed to release capture writer lock: {error}"),
            }
        })
    };
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{{"sessionId":"{session_id}","prompt":[{{"type":"text","text":"finish after drain"}}]}}}}"#
    )
    .unwrap();
    drop(stdin);

    let mut remaining_stdout = String::new();
    stdout.read_to_string(&mut remaining_stdout).unwrap();
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    let status = child.wait().unwrap();
    lock_remover.join().unwrap();
    assert!(
        status.success(),
        "relay failed\nstdout:\n{remaining_stdout}\nstderr:\n{stderr}"
    );
    assert!(remaining_stdout.contains(r#""id":2"#));

    let db = Trail::open(temp.path()).unwrap();
    let mapping = db.try_lane_acp_session(session_id).unwrap().unwrap();
    let session = db.show_lane_session(&mapping.trail_session_id).unwrap();
    assert_eq!(session.turns.len(), 1);
    assert_eq!(session.turns[0].status, "completed");
    assert!(session.messages.iter().any(|message| {
        message.role == "assistant" && message.body.contains("diagnostic complete")
    }));
}

#[cfg(unix)]
#[test]
fn acp_relay_waits_for_transient_workspace_writer_lock() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let stub_agent = temp.path().join("stub-acp-lock-agent.sh");
    fs::write(
        &stub_agent,
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
    let mut permissions = fs::metadata(&stub_agent).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&stub_agent, permissions).unwrap();

    let mut child = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("acp")
        .arg("relay")
        .arg("--lane")
        .arg("acp-lock-wait")
        .arg("--no-materialize")
        .arg("--provider")
        .arg("test-stub")
        .arg("--")
        .arg(&stub_agent)
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
    assert_eq!(init_response["result"]["_meta"]["trail"]["relay"], true);

    let lock_path = temp.path().join(".trail/lock");
    fs::write(
        &lock_path,
        format!("pid={} created_at=0", std::process::id()),
    )
    .unwrap();
    let lock_remover = {
        let lock_path = lock_path.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            match fs::remove_file(lock_path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("failed to release transient writer lock: {error}"),
            }
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

    let db = Trail::open(temp.path()).unwrap();
    let mapping = db.try_lane_acp_session("sess_wait").unwrap().unwrap();
    let session = db.show_lane_session(&mapping.trail_session_id).unwrap();
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

    let initialized = run_trail_json(
        temp.path(),
        &["init", "--working-tree", "--text-policy", "full"],
    );
    assert_eq!(initialized["branch"], "main");

    let opaque_limit = run_trail_json(
        temp.path(),
        &["config", "get", "text.opaque_text_max_bytes"],
    );
    assert_eq!(opaque_limit["value"], "67108864");

    let line_limit = run_trail_json(temp.path(), &["config", "get", "text.max_line_bytes"]);
    assert_eq!(line_limit["value"], "8388608");
}

#[test]
fn backup_create_verify_and_restore_roundtrip() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    fs::write(temp.path().join("README.md"), "hello\nbackup\n").unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    db.record(
        Some("main"),
        Some("prepare backup".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();
    let backup_parent = tempfile::tempdir().unwrap();
    let backup_path = backup_parent.path().join("trail-backup");
    let external_workdirs = tempfile::tempdir().unwrap();
    let external_workdir = external_workdirs.path().join("backup-bot");
    let spawned = db
        .spawn_lane_with_workdir(
            "backup-bot",
            Some("main"),
            true,
            None,
            None,
            Some(external_workdir.clone()),
        )
        .unwrap();
    let lane_head = db.lane_details("backup-bot").unwrap().branch;
    let manifest_path = PathBuf::from(spawned.workdir.unwrap())
        .join(".trail")
        .join("workdir-manifest.json");
    let mut legacy_manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    legacy_manifest["root_id"] = serde_json::Value::String("object_backup_pending_old".into());
    let legacy_manifest_bytes = serde_json::to_vec_pretty(&legacy_manifest).unwrap();
    fs::write(&manifest_path, &legacy_manifest_bytes).unwrap();
    let source_view_dir = temp.path().join(".trail/views/source-backup-view");
    let source_view_meta = source_view_dir.join("meta");
    fs::create_dir_all(&source_view_meta).unwrap();
    let source_checkpoint_path = source_view_meta.join("clean-checkpoint.json");
    let source_checkpoint_bytes = serde_json::to_vec_pretty(&serde_json::json!({
        "view_id": "source-backup-view",
        "operation": lane_head.head_change.0,
        "root_id": "object_backup_checkpoint_old",
        "journal_sequence": 0,
    }))
    .unwrap();
    fs::write(&source_checkpoint_path, &source_checkpoint_bytes).unwrap();
    let pending_conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
    pending_conn
        .execute(
            "INSERT INTO workspace_views \
             (view_id, lane_id, base_change, base_root, backend, mountpoint, \
              source_upper, generated_upper, scratch_upper, meta_dir, journal_path, \
              generation, checkpoint_seq, checkpoint_root, status, created_at, updated_at) \
             VALUES ('source-backup-view', ?1, ?2, ?3, 'test-cow', ?4, ?5, ?6, ?7, ?8, ?9, \
                     1, 0, ?3, 'unmounted', 1, 1)",
            rusqlite::params![
                lane_head.lane_id,
                lane_head.head_change.0,
                lane_head.head_root.0,
                external_workdir.to_string_lossy(),
                source_view_dir.join("source-upper").to_string_lossy(),
                source_view_dir.join("generated-upper").to_string_lossy(),
                source_view_dir.join("scratch-upper").to_string_lossy(),
                source_view_meta.to_string_lossy(),
                source_view_meta
                    .join("mutation-journal.jsonl")
                    .to_string_lossy(),
            ],
        )
        .unwrap();
    pending_conn
        .execute(
            "INSERT INTO pending_path_index_derived_repairs \
             (ref_name, repair_kind, old_root, new_root, new_change, created_at) \
             VALUES (?1, 'lane_manifest', ?2, ?3, ?4, 1)",
            rusqlite::params![
                lane_head.ref_name,
                "object_backup_pending_old",
                lane_head.head_root.0,
                lane_head.head_change.0,
            ],
        )
        .unwrap();
    pending_conn
        .execute(
            "INSERT INTO pending_path_index_derived_repairs \
             (ref_name, repair_kind, old_root, new_root, new_change, created_at) \
             VALUES (?1, 'workspace_checkpoint', ?2, ?3, ?4, 1)",
            rusqlite::params![
                lane_head.ref_name,
                "object_backup_checkpoint_old",
                lane_head.head_root.0,
                lane_head.head_change.0,
            ],
        )
        .unwrap();
    drop(pending_conn);
    let created = db.create_backup(&backup_path, false).unwrap();
    assert_eq!(created.branch, "main");
    assert!(created.sqlite_bytes > 0);
    let backed_up_workdir = backup_path.join("worktrees/backup-bot");
    fs::create_dir_all(backed_up_workdir.join(".trail")).unwrap();
    fs::copy(
        external_workdir.join("README.md"),
        backed_up_workdir.join("README.md"),
    )
    .unwrap();
    fs::copy(
        &manifest_path,
        backed_up_workdir.join(".trail/workdir-manifest.json"),
    )
    .unwrap();
    drop(db);

    let backup_conn = Connection::open(backup_path.join("index/trail.sqlite")).unwrap();
    let backed_up_pending: i64 = backup_conn
        .query_row(
            "SELECT COUNT(*) FROM pending_path_index_derived_repairs",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(backed_up_pending, 2);
    drop(backup_conn);

    let verified = run_trail_json(
        temp.path(),
        &["backup", "verify", backup_path.to_str().unwrap()],
    );
    assert_eq!(verified["valid"], true);
    assert_eq!(verified["branch"], "main");
    assert!(verified["checked_refs"].as_u64().unwrap() >= 2);

    let sdk_verified = Trail::verify_backup(&backup_path).unwrap();
    assert!(sdk_verified.valid, "{:?}", sdk_verified.errors);

    let restored = tempfile::tempdir().unwrap();
    let restored_report = run_trail_json(
        restored.path(),
        &["backup", "restore", backup_path.to_str().unwrap()],
    );
    assert_eq!(restored_report["branch"], "main");
    assert_eq!(restored_report["replaced_existing"], false);
    assert_eq!(restored_report["restored_trailignore"], true);
    assert_eq!(restored_report["rewritten_workdirs"], 1);

    let restored_db = Trail::open(restored.path()).unwrap();
    let restored_conn =
        Connection::open(restored.path().join(".trail/index/trail.sqlite")).unwrap();
    assert_eq!(
        restored_conn
            .query_row(
                "SELECT COUNT(*) FROM pending_path_index_derived_repairs",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    drop(restored_conn);
    assert_eq!(fs::read(&manifest_path).unwrap(), legacy_manifest_bytes);
    assert_eq!(
        fs::read(&source_checkpoint_path).unwrap(),
        source_checkpoint_bytes
    );
    let why = restored_db.why("README.md:2", Some("main")).unwrap();
    assert_eq!(why.current_text, "backup");
    let fsck = restored_db.fsck().unwrap();
    assert!(fsck.errors.is_empty(), "{:?}", fsck.errors);

    let lane = restored_db.lane_details("backup-bot").unwrap();
    assert!(restored_db
        .lane_workspace_view("backup-bot")
        .unwrap()
        .is_none());
    let workdir = lane.branch.workdir.as_ref().unwrap();
    let restored_db_dir = restored.path().canonicalize().unwrap().join(".trail");
    assert!(workdir.starts_with(&restored_db_dir.to_string_lossy().to_string()));
    assert!(PathBuf::from(workdir).is_dir());
    let restored_manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(PathBuf::from(workdir).join(".trail/workdir-manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(restored_manifest["root_id"], lane_head.head_root.0);
    let status = restored_db.lane_status("backup-bot").unwrap();
    assert_eq!(status.workdir_state, Some(WorktreeState::Clean));
}

#[test]
fn record_paths_records_only_selected_changes() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), "a1\n").unwrap();
    fs::write(temp.path().join("b.txt"), "b1\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    fs::write(temp.path().join("a.txt"), "a2\n").unwrap();
    fs::write(temp.path().join("b.txt"), "b2\n").unwrap();

    let recorded = run_trail_json(
        temp.path(),
        &["record", "--paths", "a.txt", "-m", "record only a"],
    );
    assert!(recorded["operation"].as_str().is_some());
    assert_eq!(recorded["changed_paths"].as_array().unwrap().len(), 1);
    assert_eq!(recorded["changed_paths"][0]["path"], "a.txt");

    let db = Trail::open(temp.path()).unwrap();
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    fs::remove_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("other.txt"), "two\n").unwrap();

    let recorded = run_trail_json(
        temp.path(),
        &["record", "--paths", "src", "-m", "record deleted src"],
    );
    assert!(recorded["operation"].as_str().is_some());
    assert_eq!(recorded["changed_paths"].as_array().unwrap().len(), 1);
    assert_eq!(recorded["changed_paths"][0]["path"], "src/lib.rs");
    assert_eq!(recorded["changed_paths"][0]["kind"], "Deleted");

    let db = Trail::open(temp.path()).unwrap();
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    fs::write(temp.path().join("src/lib.rs"), "pub fn lib() -> u8 { 1 }\n").unwrap();
    fs::write(temp.path().join("other.txt"), "two\n").unwrap();

    let recorded = run_trail_json(
        temp.path(),
        &["record", "--paths", "src", "-m", "record src only"],
    );
    assert!(recorded["operation"].as_str().is_some());
    assert_eq!(recorded["changed_paths"].as_array().unwrap().len(), 1);
    assert_eq!(recorded["changed_paths"][0]["path"], "src/lib.rs");
    assert_eq!(recorded["changed_paths"][0]["kind"], "Modified");

    let db = Trail::open(temp.path()).unwrap();
    let status = db.status(Some("main")).unwrap();
    assert_eq!(status.changed_paths.len(), 1);
    assert_eq!(status.changed_paths[0].path, "other.txt");
}

#[test]
fn record_paths_rejects_empty_selected_directory() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("empty")).unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let err = db
        .record_with_options(
            Some("main"),
            Some("record empty dir".to_string()),
            Actor::human(),
            trail::RecordOptions {
                paths: vec!["empty".to_string()],
                ..trail::RecordOptions::default()
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
    run_git(temp.path(), &["config", "user.email", "trail@example.com"]);
    run_git(temp.path(), &["config", "user.name", "Trail"]);
    fs::write(temp.path().join("a.txt"), "a1\n").unwrap();
    fs::write(temp.path().join("b.txt"), "b1\n").unwrap();
    run_git(temp.path(), &["add", "."]);
    run_git(temp.path(), &["commit", "-m", "initial"]);
    Trail::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();

    fs::write(temp.path().join("a.txt"), "a1\na2\n").unwrap();
    fs::remove_file(temp.path().join("b.txt")).unwrap();
    fs::write(temp.path().join("c.txt"), "c1\n").unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let status = db.status(Some("main")).unwrap();
    let status_paths = status
        .changed_paths
        .iter()
        .map(|path| (path.path.as_str(), path.kind.clone()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        status_paths.get("a.txt"),
        Some(&trail::FileChangeKind::Modified)
    );
    assert_eq!(
        status_paths.get("b.txt"),
        Some(&trail::FileChangeKind::Deleted)
    );
    assert_eq!(
        status_paths.get("c.txt"),
        Some(&trail::FileChangeKind::Added)
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
        .any(|path| path.path == "a.txt" && path.kind == trail::FileChangeKind::Modified));
    assert!(record
        .changed_paths
        .iter()
        .any(|path| path.path == "b.txt" && path.kind == trail::FileChangeKind::Deleted));
    assert!(record
        .changed_paths
        .iter()
        .any(|path| path.path == "c.txt" && path.kind == trail::FileChangeKind::Added));

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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
    let recorded = run_trail_json(
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

    let db = Trail::open(temp.path()).unwrap();
    let shown = db.show(operation).unwrap();
    match shown {
        ShowResult::Operation { value } => {
            assert_eq!(value.operation.kind, trail::OperationKind::ManualCheckpoint);
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
    let watched = run_trail_json(
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

    let db = Trail::open(temp.path()).unwrap();
    let shown = db.show(&operation).unwrap();
    match shown {
        ShowResult::Operation { value } => {
            assert_eq!(value.operation.kind, trail::OperationKind::WatchRecord);
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
fn ignore_cli_manages_trailignore_and_status() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let listed = run_trail_json(temp.path(), &["ignore", "list"]);
    assert!(listed["patterns"]
        .as_array()
        .unwrap()
        .iter()
        .any(|pattern| pattern["pattern"] == "*.p12"));

    let added = run_trail_json(temp.path(), &["ignore", "add", "notes.secret"]);
    assert_eq!(added["added"], true);
    let added_again = run_trail_json(temp.path(), &["ignore", "add", "notes.secret"]);
    assert_eq!(added_again["added"], false);

    fs::write(temp.path().join("notes.secret"), "secret\n").unwrap();
    let checked = run_trail_json(temp.path(), &["ignore", "check", "notes.secret"]);
    assert_eq!(checked["ignored"], true);
    assert_eq!(checked["source"], "workspace");

    let db = Trail::open(temp.path()).unwrap();
    let status = db.status(Some("main")).unwrap();
    assert!(!status
        .changed_paths
        .iter()
        .any(|path| path.path == "notes.secret"));
    drop(db);

    let removed = run_trail_json(temp.path(), &["ignore", "remove", "notes.secret"]);
    assert_eq!(removed["removed"], true);
    let checked = run_trail_json(temp.path(), &["ignore", "check", "notes.secret"]);
    assert_eq!(checked["ignored"], false);

    let db = Trail::open(temp.path()).unwrap();
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
                "path": ".trail/leak.txt",
                "content": "nope\n"
            }
        ]
    }))
    .unwrap();
    let err = apply_lane_patch_at_head(&mut db, "privacy-bot", internal).unwrap_err();
    assert!(matches!(err, Error::IgnoredPath(path) if path == ".trail/leak.txt"));
}

#[test]
fn lane_payload_secret_scan_rejects_patch_content_and_redacts_stored_payloads() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("secret-bot", Some("main"), false, None, None)
        .unwrap();
    let index_path = temp.path().join(".trail/index/trail.sqlite");
    let count_rows = |table: &str| -> i64 {
        Connection::open(&index_path)
            .unwrap()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    };

    let before_secret_content = (
        count_rows("objects"),
        count_rows("prolly_nodes"),
        count_rows("lane_turns"),
    );
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
    assert_eq!(
        (
            count_rows("objects"),
            count_rows("prolly_nodes"),
            count_rows("lane_turns"),
        ),
        before_secret_content
    );

    let before_secret_message = (
        count_rows("objects"),
        count_rows("prolly_nodes"),
        count_rows("lane_turns"),
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
    assert_eq!(
        (
            count_rows("objects"),
            count_rows("prolly_nodes"),
            count_rows("lane_turns"),
        ),
        before_secret_message
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
    let pem_message = db
        .add_lane_message(
            "secret-bot",
            "assistant",
            "before\n-----BEGIN PRIVATE KEY-----\nmessage-key-material\n-----END PRIVATE KEY-----\nafter",
            None,
        )
        .unwrap();
    let conn = Connection::open(&index_path).unwrap();
    let stored_message: String = conn
        .query_row(
            "SELECT body FROM messages WHERE message_id = ?1",
            [message.message_id.0.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(stored_message.contains("[REDACTED]"));
    assert!(!stored_message.contains("hunter2"));
    let stored_pem_message: String = conn
        .query_row(
            "SELECT body FROM messages WHERE message_id = ?1",
            [pem_message.message_id.0.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(stored_pem_message.contains("[REDACTED]"));
    assert!(stored_pem_message.contains("before"));
    assert!(stored_pem_message.contains("after"));
    assert!(!stored_pem_message.contains("message-key-material"));
    assert!(!stored_pem_message.contains("PRIVATE KEY"));

    let session = db
        .start_lane_session("secret-bot", Some("Secret scan".to_string()), None)
        .unwrap();
    let events_before_spaced_type = count_rows("lane_events");
    let err = db
        .add_lane_session_event(
            "secret-bot",
            &session.session.session_id,
            " tool_output",
            None,
        )
        .unwrap_err();
    assert!(
        matches!(err, Error::InvalidInput(ref message) if message.contains("leading or trailing whitespace")),
        "expected whitespace event type rejection, got {err:?}"
    );
    assert_eq!(count_rows("lane_events"), events_before_spaced_type);

    let events_before_secret_type = count_rows("lane_events");
    let err = db
        .add_lane_session_event(
            "secret-bot",
            &session.session.session_id,
            "password=hunter2",
            None,
        )
        .unwrap_err();
    assert!(
        matches!(err, Error::InvalidInput(ref message) if message.contains("secret scan rejected lane event type")),
        "expected secret event type rejection, got {err:?}"
    );
    assert_eq!(count_rows("lane_events"), events_before_secret_type);

    db.add_lane_session_event(
        "secret-bot",
        &session.session.session_id,
        "tool_output",
        Some(serde_json::json!({
            "api_key": "event-secret",
            "pem": "-----BEGIN PRIVATE KEY-----\nevent-key-material\n-----END PRIVATE KEY-----",
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
    assert!(!event_payload.contains("event-key-material"));
    assert!(!event_payload.contains("PRIVATE KEY"));
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
        "base_change": "change_stale",
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("policy-bot", Some("main"), false, None, None)
        .unwrap();

    for bad_path in [
        "CON",
        "CONIN$",
        "CONOUT$",
        "COM\u{00B9}.txt",
        "LPT\u{00B2}",
        "notes:ads.txt",
        "src\u{2215}lib.rs",
        "src\u{29F8}lib.rs",
        "docs/cafe\u{0301}.md",
    ] {
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

    for colliding_path in ["readme.md", "ＲＥＡＤＭＥ.md"] {
        let before = db
            .lane_details("policy-bot")
            .unwrap()
            .branch
            .head_change
            .clone();
        let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
        let objects_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM objects", [], |row| row.get(0))
            .unwrap();
        let prolly_nodes_before: i64 = conn
            .query_row("SELECT COUNT(*) FROM prolly_nodes", [], |row| row.get(0))
            .unwrap();
        let patch: PatchDocument = serde_json::from_value(serde_json::json!({
            "edits": [
                {"op": "write", "path": colliding_path, "content": "case collision\n"}
            ]
        }))
        .unwrap();
        let err = apply_lane_patch_at_head(&mut db, "policy-bot", patch).unwrap_err();
        assert!(
            matches!(err, Error::InvalidPath { ref reason, .. } if reason.contains("case-insensitive path collision")),
            "expected case-fold collision for {colliding_path}, got {err:?}"
        );
        assert_eq!(
            db.lane_details("policy-bot").unwrap().branch.head_change,
            before
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM objects", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            objects_before
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM prolly_nodes", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            prolly_nodes_before
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
fn lane_patch_allows_case_only_rename_when_final_root_is_safe() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("case-rename-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "rename", "from": "README.md", "to": "readme.md"}
        ]
    }))
    .unwrap();

    let report = apply_lane_patch_at_head(&mut db, "case-rename-bot", patch).unwrap();
    assert_eq!(report.changed_paths.len(), 1);
    assert_eq!(report.changed_paths[0].kind, trail::FileChangeKind::Renamed);
    assert_eq!(
        report.changed_paths[0].old_path.as_deref(),
        Some("README.md")
    );
    assert_eq!(report.changed_paths[0].path, "readme.md");
    assert_eq!(
        db.why("readme.md:1", Some("refs/lanes/case-rename-bot"))
            .unwrap()
            .current_text,
        "hello"
    );
}

#[test]
fn local_api_and_mcp_expose_ignore_controls() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let list_response = trail::server::handle_http_request(
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

    let add_response = trail::server::handle_http_request(
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
    let check_response = trail::server::handle_http_request(
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

    let guardrail_ignored = trail::server::handle_http_request(
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

    let guardrail_blocked = trail::server::handle_http_request(
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

    let remove_response = trail::server::handle_http_request(
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

    let tools = trail::mcp::handle_json_rpc(
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
        .any(|tool| tool["name"] == "trail.ignore_list"));
    assert!(tool_list
        .iter()
        .any(|tool| tool["name"] == "trail.ignore_add"));
    assert!(tool_list
        .iter()
        .any(|tool| tool["name"] == "trail.ignore_remove"));
    assert!(tool_list
        .iter()
        .any(|tool| tool["name"] == "trail.ignore_check"));
    assert!(tool_list
        .iter()
        .any(|tool| tool["name"] == "trail.guardrail_check"));

    let mcp_add = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "trail.ignore_add",
                "arguments": { "pattern": "mcp-visible.fixture" }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_add["result"]["isError"], false);
    assert_eq!(mcp_add["result"]["structuredContent"]["added"], true);

    let mcp_check = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "trail.ignore_check",
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

    let mcp_guardrail = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "trail.guardrail_check",
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

    let mcp_remove = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "trail.ignore_remove",
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("api-session-bot", Some("main"), false, None, None)
        .unwrap();

    let started = trail::server::handle_http_request(
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

    let current = trail::server::handle_http_request(
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

    let listed = trail::server::handle_http_request(
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

    let shown = trail::server::handle_http_request(
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

    let context = trail::server::handle_http_request(
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

    let cli_context = run_trail_json(
        temp.path(),
        &["session", "context", "session-api", "--limit", "1"],
    );
    assert_eq!(cli_context["session"]["session_id"], "session-api");
    assert_eq!(cli_context["message_count"], 2);
    assert_eq!(cli_context["recent_messages"][0]["role"], "assistant");

    let ended = trail::server::handle_http_request(
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

    let current = trail::server::handle_http_request(
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
    let tools = trail::mcp::handle_json_rpc(
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
        .any(|tool| tool["name"] == "trail.session_start"));
    assert!(tool_list
        .iter()
        .any(|tool| tool["name"] == "trail.session_current"));
    assert!(tool_list
        .iter()
        .any(|tool| tool["name"] == "trail.session_context"));
    assert!(tool_list
        .iter()
        .any(|tool| tool["name"] == "trail.session_end"));

    let mcp_start = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "trail.session_start",
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

    let mcp_current = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "trail.session_current",
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

    let mcp_show = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "trail.session_show",
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

    let mcp_context = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "trail.session_context",
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

    let mcp_end = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "trail.session_end",
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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

    let guardrail = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/guardrails/check",
            serde_json::json!({
                "lane": "approval-bot",
                "action": "shell.exec",
                "summary": "Run deployment smoke tests",
                "payload": {
                    "command": ["cargo", "test", "-p", "trail"],
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

    let requested = trail::server::handle_http_request(
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
                    "command": ["cargo", "test", "-p", "trail"],
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

    let pending_resume = trail::server::handle_http_request(
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

    let run_list = trail::server::handle_http_request(
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

    let cli_run_list = run_trail_json(
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

    let pending = trail::server::handle_http_request(
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

    let pending_guardrail = run_trail_json(
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

    let tools = trail::mcp::handle_json_rpc(
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
        "trail.approval_request",
        "trail.approval_list",
        "trail.approval_show",
        "trail.approval_decide",
        "trail.run_pause",
        "trail.run_list",
        "trail.run_show",
        "trail.run_resume",
    ] {
        assert!(tool_list.iter().any(|tool| tool["name"] == name), "{name}");
    }
    assert!(!tool_list
        .iter()
        .any(|tool| tool["name"] == "trail.merge_queue_add"));

    let mcp_run_show = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 20,
            "method": "tools/call",
            "params": {
                "name": "trail.run_show",
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

    let mcp_run_resource = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 21,
            "method": "resources/read",
            "params": {
                "uri": format!("trail://workspace/runs/{run_id}")
            }
        }),
    )
    .unwrap();
    let run_resource_text = mcp_run_resource["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    assert!(run_resource_text.contains(&run_id));

    let mcp_pause = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 22,
            "method": "tools/call",
            "params": {
                "name": "trail.run_pause",
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

    let mcp_run_list = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 23,
            "method": "tools/call",
            "params": {
                "name": "trail.run_list",
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

    let mcp_run_resume = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 240,
            "method": "tools/call",
            "params": {
                "name": "trail.run_resume",
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

    let mcp_show = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "trail.approval_show",
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

    let mcp_decide = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "trail.approval_decide",
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

    let resumed = trail::server::handle_http_request(
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

    let shown_run = trail::server::handle_http_request(
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

    let shown = trail::server::handle_http_request(
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

    let satisfied_guardrail = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/guardrails/check",
            serde_json::json!({
                "lane": "approval-bot",
                "action": "shell.exec",
                "summary": "Run deployment smoke tests",
                "payload": {
                    "command": ["cargo", "test", "-p", "trail"],
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

    let cli_satisfied = run_trail_json(
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let turn_response = trail::server::handle_http_request(
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

    let message_response = trail::server::handle_http_request(
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

    let event_response = trail::server::handle_http_request(
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

    let approval_response = trail::server::handle_http_request(
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

    let decision_response = trail::server::handle_http_request(
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
            "command": ["cargo", "test", "-p", "trail"]
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

    let api_response = trail::server::handle_http_request(
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

    let tools = trail::mcp::handle_json_rpc(
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
        .any(|tool| tool["name"] == "trail.event_list"));

    let mcp_events = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "trail.event_list",
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

    let cli_events = run_trail_json(
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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

    let http_start = trail::server::handle_http_request(
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

    let http_end = trail::server::handle_http_request(
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

    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
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

    let tools = trail::mcp::handle_json_rpc(
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
        "trail.span_start",
        "trail.span_end",
        "trail.span_list",
        "trail.span_summary",
        "trail.span_show",
    ] {
        assert!(tool_list.iter().any(|tool| tool["name"] == name), "{name}");
    }

    let mcp_start = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "trail.span_start",
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

    let mcp_end = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "trail.span_end",
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

    let cli_start = run_trail_json(
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

    let cli_end = run_trail_json(
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

    let cli_list = run_trail_json(
        temp.path(),
        &["lane", "trace", "list", "--turn", &turn_id, "--limit", "10"],
    );
    assert!(cli_list
        .as_array()
        .unwrap()
        .iter()
        .any(|span| span["span_id"] == child_span_id));
    let cli_summary = run_trail_json(
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

    let api_list = trail::server::handle_http_request(
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

    let api_summary = trail::server::handle_http_request(
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

    let mcp_summary = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "trail.span_summary",
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

    let mcp_show = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "trail.span_show",
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    fs::write(temp.path().join("id_rsa"), "PRIVATE\n").unwrap();
    fs::write(temp.path().join("client.p12"), "CERT\n").unwrap();

    let checked = run_trail_json(temp.path(), &["ignore", "check", "id_rsa"]);
    assert_eq!(checked["ignored"], true);
    assert_eq!(checked["source"], "hardcoded");

    let db = Trail::open(temp.path()).unwrap();
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let health = trail::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/health", serde_json::Value::Null),
    );
    assert_eq!(health.status, 200);

    let turn_response = trail::server::handle_http_request(
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

    let message_response = trail::server::handle_http_request(
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

    let event_response = trail::server::handle_http_request(
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

    let patch_response = trail::server::handle_http_request(
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

    let details_response = trail::server::handle_http_request(
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

    let end_response = trail::server::handle_http_request(
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();

    let bad_turn = trail::server::handle_http_request(
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

    let turn_response = trail::server::handle_http_request(
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

    let bad_message = trail::server::handle_http_request(
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

    let bad_patch = trail::server::handle_http_request(
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

    let mcp_bad_begin = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "trail.begin_turn",
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

    let mcp_bad_status = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "tools/call",
            "params": {
                "name": "trail.status",
                "arguments": {
                    "branch": "main",
                    "surprise": true
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_bad_status["result"]["isError"], true);
    assert!(mcp_bad_status["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("unknown field"));

    let mcp_bad_event_list = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 12,
            "method": "tools/call",
            "params": {
                "name": "trail.event_list",
                "arguments": {
                    "lane": "strict-mcp",
                    "limit": 10,
                    "surprise": true
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_bad_event_list["result"]["isError"], true);
    assert!(mcp_bad_event_list["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("unknown field"));

    let mcp_bad_conflict_show = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 13,
            "method": "tools/call",
            "params": {
                "name": "trail.conflict_show",
                "arguments": {
                    "conflict_set_id": "conflict_missing",
                    "limit": 10,
                    "surprise": true
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_bad_conflict_show["result"]["isError"], true);
    assert!(mcp_bad_conflict_show["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("unknown field"));

    let mcp_begin = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "trail.begin_turn",
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

    let mcp_bad_patch = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "trail.apply_patch",
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
fn external_patch_payloads_accept_explicit_empty_and_reject_missing_or_ambiguous_sources() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let http_turn = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lane/turns",
            serde_json::json!({
                "lane": "strict-http-patch",
                "branch": "main"
            }),
        ),
    );
    assert_eq!(http_turn.status, 201);
    let http_turn: LaneTurnStartReport = http_turn.body_json().unwrap();

    let http_missing_patch = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/lane/turns/{}/patches", http_turn.turn.turn_id),
            serde_json::json!({
                "message": "missing patch source"
            }),
        ),
    );
    assert_eq!(http_missing_patch.status, 400);
    let body: serde_json::Value = http_missing_patch.body_json().unwrap();
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("requires exactly one explicit edit source"));

    let http_empty_patch = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/lane/turns/{}/patches", http_turn.turn.turn_id),
            serde_json::json!({
                "message": "empty patch",
                "edits": []
            }),
        ),
    );
    assert_eq!(http_empty_patch.status, 200);
    let http_empty_report: LanePatchReport = http_empty_patch.body_json().unwrap();
    assert!(http_empty_report.changed_paths.is_empty());

    let http_ambiguous_patch = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/lane/turns/{}/patches", http_turn.turn.turn_id),
            serde_json::json!({
                "message": "ambiguous patch",
                "edits": [
                    {"op": "delete", "path": "old-http.md"}
                ],
                "files": [
                    {"type": "delete", "path": "new-http.md"}
                ]
            }),
        ),
    );
    assert_eq!(http_ambiguous_patch.status, 400);
    let body: serde_json::Value = http_ambiguous_patch.body_json().unwrap();
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("must use either `edits` or `files`"));

    let http_missing_base_patch = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes/strict-http-patch/patches",
            serde_json::json!({
                "message": "direct patch without fresh base",
                "files": [
                    {
                        "type": "add_text",
                        "path": "src/http_direct.rs",
                        "content": "pub fn http_direct() -> bool { true }\n"
                    }
                ]
            }),
        ),
    );
    assert_eq!(http_missing_base_patch.status, 409);
    let body: serde_json::Value = http_missing_base_patch.body_json().unwrap();
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("base_change"));

    let mcp_begin = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "trail.begin_turn",
                "arguments": {
                    "lane": "strict-mcp-patch",
                    "branch": "main"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_begin["result"]["isError"], false);
    let mcp_lane_id = mcp_begin["result"]["structuredContent"]["turn"]["lane_id"]
        .as_str()
        .unwrap()
        .to_string();
    let mcp_turn_id = mcp_begin["result"]["structuredContent"]["turn"]["turn_id"]
        .as_str()
        .unwrap()
        .to_string();

    let mcp_missing_patch = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "trail.apply_patch",
                "arguments": {
                    "turn_id": mcp_turn_id.clone(),
                    "message": "missing patch source"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_missing_patch["result"]["isError"], true);
    assert!(mcp_missing_patch["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("requires exactly one explicit edit source"));

    let mcp_empty_patch = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 20,
            "method": "tools/call",
            "params": {
                "name": "trail.apply_patch",
                "arguments": {
                    "turn_id": mcp_turn_id.clone(),
                    "message": "empty patch",
                    "edits": []
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_empty_patch["result"]["isError"], false);
    assert_eq!(
        mcp_empty_patch["result"]["structuredContent"]["changed_paths"],
        serde_json::json!([])
    );

    let mcp_ambiguous_patch = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "trail.apply_patch",
                "arguments": {
                    "turn_id": mcp_turn_id,
                    "message": "ambiguous patch",
                    "edits": [
                        {"op": "delete", "path": "old-mcp.md"}
                    ],
                    "files": [
                        {"type": "delete", "path": "new-mcp.md"}
                    ]
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_ambiguous_patch["result"]["isError"], true);
    assert!(mcp_ambiguous_patch["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("must use either `edits` or `files`"));

    let audits = db.list_external_mutation_audit(20).unwrap();
    assert!(audits.iter().any(|audit| {
        audit.surface == "http"
            && audit.command.contains("/patches")
            && audit.status == "error"
            && audit.status_code == Some(400)
            && audit.lane_id.as_deref() == Some(http_turn.turn.lane_id.as_str())
            && audit.target_ref.as_deref() == Some("refs/lanes/strict-http-patch")
            && audit
                .summary
                .as_ref()
                .and_then(|summary| summary["error"].as_str())
                .is_some_and(|message| {
                    message.contains("requires exactly one explicit edit source")
                })
    }));
    assert!(audits.iter().any(|audit| {
        audit.surface == "http"
            && audit.command == "POST /v1/lanes/strict-http-patch/patches"
            && audit.status == "error"
            && audit.status_code == Some(409)
            && audit.lane_id.as_deref() == Some(http_turn.turn.lane_id.as_str())
            && audit.target_ref.as_deref() == Some("refs/lanes/strict-http-patch")
            && audit
                .summary
                .as_ref()
                .and_then(|summary| summary["error"].as_str())
                .is_some_and(|message| message.contains("base_change"))
    }));
    assert!(audits.iter().any(|audit| {
        audit.surface == "mcp"
            && audit.command == "trail.apply_patch"
            && audit.status == "error"
            && audit.lane_id.as_deref() == Some(mcp_lane_id.as_str())
            && audit.target_ref.as_deref() == Some("refs/lanes/strict-mcp-patch")
            && audit
                .summary
                .as_ref()
                .and_then(|summary| summary["error"].as_str())
                .is_some_and(|message| message.contains("must use either `edits` or `files`"))
    }));
}

#[test]
fn external_http_and_mcp_mutations_emit_audit_events() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    assert!(db.list_external_mutation_audit(10).unwrap().is_empty());

    let http_turn = trail::server::handle_http_request(
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
    let http_turn: LaneTurnStartReport = http_turn.body_json().unwrap();

    let http_bad_turn = trail::server::handle_http_request(
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

    let mcp_read_only = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "trail.status",
                "arguments": {}
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_read_only["result"]["isError"], false);

    let mcp_set = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "trail.config_set",
                "arguments": {
                    "key": "lane.default_materialize",
                    "value": "false"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_set["result"]["isError"], false);

    let mcp_bad_set = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "trail.config_set",
                "arguments": {
                    "key": "lane.default_materialize",
                    "value": "true",
                    "unexpected": true
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_bad_set["result"]["isError"], true);

    let mcp_begin = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "trail.begin_turn",
                "arguments": {
                    "lane": "audit-mcp",
                    "branch": "main"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_begin["result"]["isError"], false);
    let mcp_lane_id = mcp_begin["result"]["structuredContent"]["turn"]["lane_id"]
        .as_str()
        .unwrap()
        .to_string();
    let mcp_turn_id = mcp_begin["result"]["structuredContent"]["turn"]["turn_id"]
        .as_str()
        .unwrap()
        .to_string();

    let mcp_patch = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "trail.apply_patch",
                "arguments": {
                    "turn_id": mcp_turn_id,
                    "message": "mcp audit patch",
                    "files": [
                        {
                            "type": "add_text",
                            "path": "src/mcp_audit.rs",
                            "content": "pub fn mcp_audit() -> bool { true }\n"
                        }
                    ]
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_patch["result"]["isError"], false);

    let mcp_bad_begin = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "trail.begin_turn",
                "arguments": {
                    "lane": "audit-mcp",
                    "branch": "main",
                    "unexpected": true
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_bad_begin["result"]["isError"], true);

    let mcp_bad_queue_add = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "trail.lane_merge_queue_add",
                "arguments": {
                    "lane": "audit-mcp",
                    "target": "main",
                    "unexpected": true
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_bad_queue_add["result"]["isError"], true);

    let audits = db.list_external_mutation_audit(20).unwrap();
    assert!(audits.iter().any(|audit| {
        audit.actor == "http:no-auth"
            && audit.surface == "http"
            && audit.command == "POST /v1/lane/turns"
            && audit.status == "ok"
            && audit.status_code == Some(201)
            && audit.target_ref.as_deref() == Some("refs/lanes/audit-http")
    }));
    assert!(audits.iter().any(|audit| {
        audit.actor == "http:no-auth"
            && audit.surface == "http"
            && audit.command == "POST /v1/lane/turns"
            && audit.status == "error"
            && audit.status_code == Some(400)
            && audit.lane_id.as_deref() == Some(http_turn.turn.lane_id.as_str())
            && audit.target_ref.as_deref() == Some("refs/lanes/audit-http")
            && audit
                .summary
                .as_ref()
                .and_then(|summary| summary["error"].as_str())
                .is_some_and(|message| message.contains("unknown field"))
    }));
    assert!(audits.iter().any(|audit| {
        audit.actor == "mcp:stdio"
            && audit.surface == "mcp"
            && audit.command == "trail.config_set"
            && audit.status == "ok"
    }));
    assert!(audits.iter().any(|audit| {
        audit.actor == "mcp:stdio"
            && audit.surface == "mcp"
            && audit.command == "trail.config_set"
            && audit.status == "error"
            && audit
                .summary
                .as_ref()
                .and_then(|summary| summary["error"].as_str())
                .is_some_and(|message| message.contains("unknown field"))
    }));
    assert!(audits.iter().any(|audit| {
        audit.actor == "mcp:stdio"
            && audit.surface == "mcp"
            && audit.command == "trail.begin_turn"
            && audit.status == "ok"
            && audit.lane_id.as_deref() == Some(mcp_lane_id.as_str())
            && audit.target_ref.as_deref() == Some("refs/lanes/audit-mcp")
    }));
    assert!(audits.iter().any(|audit| {
        audit.actor == "mcp:stdio"
            && audit.surface == "mcp"
            && audit.command == "trail.begin_turn"
            && audit.status == "error"
            && audit.lane_id.as_deref() == Some(mcp_lane_id.as_str())
            && audit.target_ref.as_deref() == Some("refs/lanes/audit-mcp")
            && audit
                .summary
                .as_ref()
                .and_then(|summary| summary["error"].as_str())
                .is_some_and(|message| message.contains("unknown field"))
    }));
    assert!(!audits.iter().any(|audit| {
        audit.actor == "mcp:stdio"
            && audit.command == "trail.begin_turn"
            && audit.status == "error"
            && audit.target_ref.as_deref() == Some("main")
    }));
    assert!(audits.iter().any(|audit| {
        audit.actor == "mcp:stdio"
            && audit.surface == "mcp"
            && audit.command == "trail.lane_merge_queue_add"
            && audit.status == "error"
            && audit.lane_id.as_deref() == Some(mcp_lane_id.as_str())
            && audit.target_ref.as_deref() == Some("refs/heads/main")
            && audit
                .summary
                .as_ref()
                .and_then(|summary| summary["error"].as_str())
                .is_some_and(|message| message.contains("unknown field"))
    }));
    assert!(!audits.iter().any(|audit| {
        audit.actor == "mcp:stdio"
            && audit.command == "trail.lane_merge_queue_add"
            && audit.status == "error"
            && audit.target_ref.as_deref() == Some("main")
    }));
    assert!(audits.iter().any(|audit| {
        audit.actor == "mcp:stdio"
            && audit.surface == "mcp"
            && audit.command == "trail.apply_patch"
            && audit.status == "ok"
            && audit.lane_id.as_deref() == Some(mcp_lane_id.as_str())
            && audit.target_ref.as_deref() == Some("refs/lanes/audit-mcp")
            && audit.change_id.is_some()
            && audit
                .summary
                .as_ref()
                .and_then(|summary| summary["change_id"].as_str())
                .is_some()
    }));
    assert!(!audits.iter().any(|audit| audit.command == "trail.status"));
}

#[test]
fn local_lane_http_api_replays_idempotent_mutation_requests() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

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

    let mut db = Trail::open(temp.path()).unwrap();
    let first = trail::server::handle_http_request(&mut db, &request);
    assert_eq!(first.status, 201);
    let first_turn: LaneTurnStartReport = first.body_json().unwrap();
    drop(db);

    let mut reopened = Trail::open(temp.path()).unwrap();
    let replayed = trail::server::handle_http_request(&mut reopened, &request);
    assert_eq!(replayed.status, 201);
    let replayed_turn: LaneTurnStartReport = replayed.body_json().unwrap();
    assert_eq!(replayed_turn.turn.turn_id, first_turn.turn.turn_id);
    assert_eq!(
        replayed_turn.session.session_id,
        first_turn.session.session_id
    );
    drop(reopened);

    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
    let turns: i64 = conn
        .query_row("SELECT COUNT(*) FROM lane_turns", [], |row| row.get(0))
        .unwrap();
    assert_eq!(turns, 1);
    drop(conn);

    let mut db = Trail::open(temp.path()).unwrap();
    let conflicting = trail::server::handle_http_request(
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

    let invalid_idempotency_body = serde_json::json!({
        "lane": "idempotent-api",
        "branch": "main",
        "session_title": "Invalid idempotency key"
    });
    let overlong_idempotency_key = "x".repeat(201);
    for key in ["", overlong_idempotency_key.as_str(), "key\twith-tab"] {
        let invalid_key = trail::server::handle_http_request(
            &mut db,
            &api_request_with_headers(
                "POST",
                "/v1/lane/turns",
                &[("Idempotency-Key", key)],
                invalid_idempotency_body.clone(),
            ),
        );
        assert_eq!(invalid_key.status, 400);
        let error: serde_json::Value = invalid_key.body_json().unwrap();
        assert!(error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Idempotency-Key"));
    }

    let auth = trail::server::ServerAuth::bearer("secret-token").unwrap();
    let auth_request = api_request_with_headers(
        "POST",
        "/v1/lane/turns",
        &[("Idempotency-Key", "auth-key-1")],
        serde_json::json!({
            "lane": "auth-idempotent-api",
            "branch": "main"
        }),
    );
    let missing_auth = trail::server::handle_http_request_with_auth(&mut db, &auth_request, &auth);
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
        trail::server::handle_http_request_with_auth(&mut db, &authorized_request, &auth);
    assert_eq!(authorized.status, 201);
    let unauthorized_replay =
        trail::server::handle_http_request_with_auth(&mut db, &auth_request, &auth);
    assert_eq!(unauthorized_replay.status, 401);

    let audits = db.list_external_mutation_audit(30).unwrap();
    let idempotent_turn_audits = audits
        .iter()
        .filter(|audit| {
            audit.actor == "http:no-auth"
                && audit.surface == "http"
                && audit.command == "POST /v1/lane/turns"
                && audit.status == "ok"
                && audit.status_code == Some(201)
                && audit.target_ref.as_deref() == Some("refs/lanes/idempotent-api")
        })
        .collect::<Vec<_>>();
    assert_eq!(idempotent_turn_audits.len(), 2);
    assert_eq!(
        idempotent_turn_audits
            .iter()
            .filter(|audit| {
                audit
                    .summary
                    .as_ref()
                    .and_then(|summary| summary["idempotency_replay"].as_bool())
                    == Some(true)
            })
            .count(),
        1
    );
    assert!(audits.iter().any(|audit| {
        audit.actor == "http:no-auth"
            && audit.surface == "http"
            && audit.command == "POST /v1/lane/turns"
            && audit.status == "error"
            && audit.status_code == Some(400)
            && audit
                .summary
                .as_ref()
                .and_then(|summary| summary["error"].as_str())
                .is_some_and(|message| message.contains("already used for a different request"))
    }));
    assert_eq!(
        audits
            .iter()
            .filter(|audit| {
                audit.actor == "http:no-auth"
                    && audit.surface == "http"
                    && audit.command == "POST /v1/lane/turns"
                    && audit.status == "error"
                    && audit.status_code == Some(400)
                    && audit.target_ref.as_deref() == Some("refs/lanes/idempotent-api")
                    && audit
                        .summary
                        .as_ref()
                        .and_then(|summary| summary["error"].as_str())
                        .is_some_and(|message| {
                            message.contains("Idempotency-Key")
                                && !message.contains("already used for a different request")
                        })
            })
            .count(),
        3
    );
    assert!(audits.iter().any(|audit| {
        audit.actor == "http:bearer"
            && audit.surface == "http"
            && audit.command == "POST /v1/lane/turns"
            && audit.status == "ok"
            && audit.status_code == Some(201)
            && audit.target_ref.as_deref() == Some("refs/lanes/auth-idempotent-api")
    }));
    assert_eq!(
        audits
            .iter()
            .filter(|audit| {
                audit.actor == "http:no-auth"
                    && audit.surface == "http"
                    && audit.command == "POST /v1/lane/turns"
                    && audit.status == "error"
                    && audit.status_code == Some(401)
            })
            .count(),
        2
    );
}

#[test]
fn local_api_and_mcp_patch_payloads_respect_ignore_policy() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.ignore_add("host-secret.txt").unwrap();

    let turn_response = trail::server::handle_http_request(
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

    let blocked = trail::server::handle_http_request(
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

    let tools = trail::mcp::handle_json_rpc(
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
        .find(|tool| tool["name"] == "trail.apply_patch")
        .unwrap();
    let apply_patch_schema = &apply_patch_schema["inputSchema"];
    assert_eq!(
        apply_patch_schema["properties"]["allow_ignored"]["type"],
        "boolean"
    );
    assert_eq!(
        apply_patch_schema["properties"]["allow_stale"]["type"],
        "boolean"
    );
    let edit_source_modes = apply_patch_schema["oneOf"].as_array().unwrap();
    assert_eq!(edit_source_modes.len(), 2);
    assert_eq!(
        edit_source_modes[0]["required"],
        serde_json::json!(["edits"])
    );
    assert_eq!(
        edit_source_modes[0]["not"]["required"],
        serde_json::json!(["files"])
    );
    assert_eq!(
        edit_source_modes[1]["required"],
        serde_json::json!(["files"])
    );
    assert_eq!(
        edit_source_modes[1]["not"]["required"],
        serde_json::json!(["edits"])
    );
    assert!(apply_patch_schema["properties"]["edits"]
        .get("minItems")
        .is_none());
    assert!(apply_patch_schema["properties"]["files"]
        .get("minItems")
        .is_none());
    let edit_variants = apply_patch_schema["properties"]["edits"]["items"]["oneOf"]
        .as_array()
        .unwrap();
    assert_eq!(edit_variants.len(), 5);
    for variant in edit_variants {
        assert_eq!(variant["additionalProperties"], false);
    }
    assert!(edit_variants[2]["required"]
        .as_array()
        .unwrap()
        .iter()
        .any(|field| field == "expected_text"));
    let file_variants = apply_patch_schema["properties"]["files"]["items"]["oneOf"]
        .as_array()
        .unwrap();
    assert_eq!(file_variants.len(), 5);
    for variant in file_variants {
        assert_eq!(variant["additionalProperties"], false);
    }
    let nested_line_edit = &file_variants[1]["properties"]["edits"]["items"]["oneOf"][0];
    assert_eq!(nested_line_edit["additionalProperties"], false);
    assert!(nested_line_edit["required"]
        .as_array()
        .unwrap()
        .iter()
        .any(|field| field == "expected_text"));

    let allowed = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "trail.apply_patch",
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let status_response = trail::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/status", serde_json::Value::Null),
    );
    assert_eq!(status_response.status, 200);
    let status: serde_json::Value = status_response.body_json().unwrap();
    assert_eq!(status["branch"], "main");

    let spawn_response = trail::server::handle_http_request(
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

    let lanes_response = trail::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/lanes", serde_json::Value::Null),
    );
    assert_eq!(lanes_response.status, 200);
    let lanes: serde_json::Value = lanes_response.body_json().unwrap();
    assert_eq!(lanes.as_array().unwrap().len(), 1);
    assert_eq!(lanes[0]["record"]["name"], "api-branch-lane");
    assert_eq!(lanes[0]["branch"]["ref_name"], "refs/lanes/api-branch-lane");

    let lane_status_response = trail::server::handle_http_request(
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

    let patch_response = trail::server::handle_http_request(
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
    let record_preview_response = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/lanes/{lane_id}/record"),
            serde_json::json!({ "preview": true }),
        ),
    );
    assert_eq!(record_preview_response.status, 200);
    let record_preview: serde_json::Value = record_preview_response.body_json().unwrap();
    assert_eq!(record_preview["clean"], false);
    assert!(record_preview["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "README.md"));
    assert!(record_preview["operation"].is_null());

    let sync_conflict = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/lanes/{lane_id}/sync-workdir"),
            serde_json::json!({}),
        ),
    );
    assert_eq!(sync_conflict.status, 409);

    let sync_response = trail::server::handle_http_request(
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

    let test_response = trail::server::handle_http_request(
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

    let diff_response = trail::server::handle_http_request(
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

    let contribution_response = trail::server::handle_http_request(
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

    let cli_contribution = run_trail_json(
        temp.path(),
        &["lane", "contribution", "api-branch-lane", "--limit", "5"],
    );
    assert_eq!(
        cli_contribution["status"]["lane"]["record"]["lane_id"],
        lane_id
    );

    let review_response = trail::server::handle_http_request(
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

    let cli_review = run_trail_json(
        temp.path(),
        &["lane", "review", "api-branch-lane", "--limit", "5"],
    );
    assert_eq!(cli_review["lane"]["record"]["lane_id"], lane_id);
    assert_eq!(cli_review["readiness"]["ready"], true);

    let cli_review_text = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .args(["lane", "review", "api-branch-lane", "--limit", "5"])
        .output()
        .unwrap();
    assert!(cli_review_text.status.success());
    let cli_review_stdout = String::from_utf8_lossy(&cli_review_text.stdout);
    assert!(
        cli_review_stdout.contains("Lane api-branch-lane is ready for review"),
        "{cli_review_stdout}"
    );
    assert!(cli_review_stdout.contains("Operations: 1"));
    assert!(cli_review_stdout.contains("Next:"));

    let readiness_response = trail::server::handle_http_request(
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

    let cli_readiness = run_trail_json(temp.path(), &["lane", "readiness", "api-branch-lane"]);
    assert_eq!(cli_readiness["lane"]["record"]["lane_id"], lane_id);
    assert_eq!(cli_readiness["ready"], true);

    let handoff_response = trail::server::handle_http_request(
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

    let cli_handoff = run_trail_json(
        temp.path(),
        &["lane", "handoff", "api-branch-lane", "--limit", "5"],
    );
    assert_eq!(cli_handoff["lane"]["record"]["lane_id"], lane_id);
    assert_eq!(cli_handoff["readiness"]["ready"], true);

    let remove_dirty_response = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "DELETE",
            &format!("/v1/lanes/{lane_id}"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(remove_dirty_response.status, 400);

    let merge_response = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/lanes/{lane_id}/merge"),
            serde_json::json!({
                "into": "main",
                "strategy": "line_id_aware",
                "direct": true
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

    let remove_response = trail::server::handle_http_request(
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
fn layered_workspace_reports_have_http_mcp_and_openapi_parity() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "baseline\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    let mode = if cfg!(target_os = "macos") {
        LaneWorkdirMode::NfsCow
    } else if cfg!(target_os = "windows") {
        LaneWorkdirMode::DokanCow
    } else {
        LaneWorkdirMode::FuseCow
    };
    db.spawn_lane_with_workdir_mode_paths_and_neighbors(
        "surface",
        Some("main"),
        mode,
        None,
        None,
        None,
        &[],
        false,
    )
    .unwrap();
    let view = db.lane_workspace_view("surface").unwrap().unwrap();
    fs::write(
        Path::new(&view.source_upper).join("surface.txt"),
        "surface parity\n",
    )
    .unwrap();

    let http_workspace = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/lanes/surface/workspace",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(http_workspace.status, 200);
    let http_workspace: serde_json::Value = http_workspace.body_json().unwrap();
    assert_eq!(http_workspace["view_id"], view.view_id);

    let http_space = trail::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/lanes/surface/space", serde_json::Value::Null),
    );
    assert_eq!(http_space.status, 200);
    let http_space: serde_json::Value = http_space.body_json().unwrap();
    assert_eq!(http_space["view_id"], view.view_id);

    let http_unmount = trail::server::handle_http_request(
        &mut db,
        &api_request("POST", "/v1/lanes/surface/unmount", serde_json::Value::Null),
    );
    assert_eq!(http_unmount.status, 200);
    assert_eq!(
        http_unmount.body_json::<serde_json::Value>().unwrap()["view_id"],
        view.view_id
    );

    let http_dependencies = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/lanes/surface/dependencies",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(http_dependencies.status, 200);
    assert!(http_dependencies
        .body_json::<serde_json::Value>()
        .unwrap()
        .as_array()
        .unwrap()
        .is_empty());

    let http_environment = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/lanes/surface/environment",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(http_environment.status, 200);
    assert!(http_environment
        .body_json::<serde_json::Value>()
        .unwrap()
        .as_array()
        .unwrap()
        .is_empty());

    let http_checkpoint = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes/surface/checkpoint",
            serde_json::json!({"message": "HTTP checkpoint"}),
        ),
    );
    assert_eq!(http_checkpoint.status, 200);
    let http_checkpoint: serde_json::Value = http_checkpoint.body_json().unwrap();
    assert_eq!(
        http_checkpoint["source_paths"],
        serde_json::json!(["surface.txt"])
    );

    let http_update = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes/surface/update",
            serde_json::json!({"from": "main"}),
        ),
    );
    assert_eq!(http_update.status, 200);
    let http_update: serde_json::Value = http_update.body_json().unwrap();
    assert_eq!(http_update["source_ref"], "refs/branches/main");
    assert_eq!(http_update["target_ref"], "refs/lanes/surface");

    let http_exec_error = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes/surface/exec",
            serde_json::json!({"command": []}),
        ),
    );
    assert_eq!(http_exec_error.status, 400);
    assert!(
        http_exec_error.body_json::<serde_json::Value>().unwrap()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("requires a command")
    );

    let http_sync_error = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes/surface/dependencies/sync",
            serde_json::json!({}),
        ),
    );
    assert_eq!(http_sync_error.status, 400);

    let http_environment_sync_error = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes/surface/environment/sync",
            serde_json::json!({"adapter": "trail/node@1"}),
        ),
    );
    assert_eq!(http_environment_sync_error.status, 400);

    let http_cache = trail::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/cache/layers", serde_json::Value::Null),
    );
    assert_eq!(http_cache.status, 200);
    assert!(http_cache
        .body_json::<serde_json::Value>()
        .unwrap()
        .as_array()
        .unwrap()
        .is_empty());
    let http_gc = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/cache/gc",
            serde_json::json!({"dry_run": true, "retention_secs": 0}),
        ),
    );
    assert_eq!(http_gc.status, 200);
    assert_eq!(
        http_gc.body_json::<serde_json::Value>().unwrap()["dry_run"],
        true
    );

    let mcp_call = |db: &mut Trail, id: u64, name: &str, arguments: serde_json::Value| {
        trail::mcp::handle_json_rpc(
            db,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments}
            }),
        )
        .unwrap()
    };
    let mcp_workspace = mcp_call(
        &mut db,
        1,
        "trail.lane_workspace",
        serde_json::json!({"lane": "surface"}),
    );
    assert_eq!(mcp_workspace["result"]["isError"], false);
    assert_eq!(
        mcp_workspace["result"]["structuredContent"]["view_id"],
        view.view_id
    );
    let mcp_space = mcp_call(
        &mut db,
        2,
        "trail.lane_space",
        serde_json::json!({"lane": "surface"}),
    );
    assert_eq!(
        mcp_space["result"]["structuredContent"]["view_id"],
        view.view_id
    );
    let mcp_dependencies = mcp_call(
        &mut db,
        3,
        "trail.deps_status",
        serde_json::json!({"lane": "surface"}),
    );
    assert!(mcp_dependencies["result"]["structuredContent"]
        .as_array()
        .unwrap()
        .is_empty());
    let mcp_environment = mcp_call(
        &mut db,
        32,
        "trail.env_status",
        serde_json::json!({"lane": "surface"}),
    );
    assert!(mcp_environment["result"]["structuredContent"]
        .as_array()
        .unwrap()
        .is_empty());
    let mcp_environment = mcp_call(
        &mut db,
        32,
        "trail.env_status",
        serde_json::json!({"lane": "surface"}),
    );
    assert!(mcp_environment["result"]["structuredContent"]
        .as_array()
        .unwrap()
        .is_empty());
    let mcp_unmount = mcp_call(
        &mut db,
        31,
        "trail.lane_unmount",
        serde_json::json!({"lane": "surface"}),
    );
    assert_eq!(
        mcp_unmount["result"]["structuredContent"]["view_id"],
        view.view_id
    );
    let mcp_checkpoint = mcp_call(
        &mut db,
        4,
        "trail.lane_checkpoint",
        serde_json::json!({"lane": "surface", "message": "MCP checkpoint"}),
    );
    assert_eq!(mcp_checkpoint["result"]["isError"], false);
    assert_eq!(
        mcp_checkpoint["result"]["structuredContent"]["root_id"],
        http_checkpoint["root_id"]
    );
    let mcp_cache = mcp_call(&mut db, 5, "trail.cache_list", serde_json::json!({}));
    assert!(mcp_cache["result"]["structuredContent"]
        .as_array()
        .unwrap()
        .is_empty());
    let mcp_gc = mcp_call(
        &mut db,
        6,
        "trail.cache_gc",
        serde_json::json!({"dry_run": true, "retention_secs": 0}),
    );
    assert_eq!(mcp_gc["result"]["structuredContent"]["dry_run"], true);
    let mcp_update = mcp_call(
        &mut db,
        7,
        "trail.lane_update",
        serde_json::json!({"lane": "surface", "source": "main"}),
    );
    assert_eq!(mcp_update["result"]["isError"], false);
    assert_eq!(
        mcp_update["result"]["structuredContent"]["source_ref"],
        "refs/branches/main"
    );

    let http_adapters = trail::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/environment/adapters", serde_json::Value::Null),
    );
    assert_eq!(http_adapters.status, 200);
    let http_adapters: serde_json::Value = http_adapters.body_json().unwrap();
    assert_eq!(http_adapters["contract_major"], 1);
    let adapter_identities = http_adapters["adapters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|adapter| adapter["canonical_identity"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        adapter_identities,
        BTreeSet::from([
            "trail/cargo-target-seed@1",
            "trail/cmake-build@1",
            "trail/command@1",
            "trail/go-vendor@1",
            "trail/node@1",
            "trail/oci-image@1",
            "trail/python-venv@1",
        ])
    );
    assert!(http_adapters["adapters"]
        .as_array()
        .unwrap()
        .iter()
        .any(|adapter| {
            adapter["canonical_identity"] == "trail/node@1"
                && adapter["discovery_markers"][0] == "package.json"
        }));
    assert!(http_adapters["adapters"]
        .as_array()
        .unwrap()
        .iter()
        .any(|adapter| {
            adapter["canonical_identity"] == "trail/cmake-build@1"
                && adapter["kind"] == "build"
                && adapter["discovery_markers"][0] == "CMakeLists.txt"
        }));
    let mcp_adapters = mcp_call(&mut db, 8, "trail.env_adapters", serde_json::json!({}));
    assert_eq!(mcp_adapters["result"]["isError"], false);
    assert_eq!(mcp_adapters["result"]["structuredContent"], http_adapters);

    let tools = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "tools/list",
            "params": {}
        }),
    )
    .unwrap();
    let tools = tools["result"]["tools"].as_array().unwrap();
    for (name, read_only, destructive, open_world) in [
        ("trail.lane_workspace", true, false, false),
        ("trail.lane_mount", false, false, false),
        ("trail.lane_unmount", false, false, false),
        ("trail.lane_checkpoint", false, false, false),
        ("trail.lane_update", false, false, false),
        ("trail.lane_exec", false, false, true),
        ("trail.deps_sync", false, false, true),
        ("trail.env_adapters", true, false, false),
        ("trail.env_status", true, false, false),
        ("trail.env_discover", true, false, false),
        ("trail.env_graph", true, false, false),
        ("trail.env_generation", true, false, false),
        ("trail.env_explain", true, false, false),
        ("trail.env_plan", true, false, false),
        ("trail.env_sync", false, false, true),
        ("trail.env_sync_all", false, false, true),
        ("trail.env_status", true, false, false),
        ("trail.env_sync", false, false, true),
        ("trail.cache_gc", false, true, false),
    ] {
        let tool = tools.iter().find(|tool| tool["name"] == name).unwrap();
        assert_eq!(tool["annotations"]["readOnlyHint"], read_only);
        assert_eq!(tool["annotations"]["destructiveHint"], destructive);
        assert_eq!(tool["annotations"]["openWorldHint"], open_world);
    }

    let openapi = trail::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/openapi.json", serde_json::Value::Null),
    );
    let openapi: serde_json::Value = openapi.body_json().unwrap();
    for path in [
        "/v1/environment/adapters",
        "/v1/lanes/{lane_or_id}/workspace",
        "/v1/lanes/{lane_or_id}/mount",
        "/v1/lanes/{lane_or_id}/unmount",
        "/v1/lanes/{lane_or_id}/checkpoint",
        "/v1/lanes/{lane_or_id}/update",
        "/v1/lanes/{lane_or_id}/space",
        "/v1/lanes/{lane_or_id}/exec",
        "/v1/lanes/{lane_or_id}/dependencies",
        "/v1/lanes/{lane_or_id}/dependencies/sync",
        "/v1/lanes/{lane_or_id}/environment",
        "/v1/lanes/{lane_or_id}/environment/discover",
        "/v1/lanes/{lane_or_id}/environment/graph",
        "/v1/lanes/{lane_or_id}/environment/generation",
        "/v1/lanes/{lane_or_id}/environment/explain",
        "/v1/lanes/{lane_or_id}/environment/plan",
        "/v1/lanes/{lane_or_id}/environment/sync",
        "/v1/lanes/{lane_or_id}/environment/sync-all",
        "/v1/cache/layers",
        "/v1/cache/gc",
    ] {
        assert!(
            openapi["paths"].get(path).is_some(),
            "missing OpenAPI path {path}"
        );
    }
}

#[test]
fn environment_graph_has_cli_http_mcp_and_openapi_parity() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("input.txt"), "graph\n").unwrap();
    fs::write(
        temp.path().join("trail.environment.toml"),
        r#"schema = "trail.environment/v1"

[[component]]
id = "graph.a"
adapter = "trail/command@1"
kind = "generated"
inputs = [{ path = "input.txt" }]
outputs = [{ source = "generated-a", target = ".trail-generated/a" }]
[component.build]
command = ["git", "--version"]

[[component]]
id = "graph.b"
adapter = "trail/command@1"
kind = "generated"
depends_on = ["graph.a"]
inputs = [{ path = "input.txt" }]
outputs = [{ source = "generated-b", target = ".trail-generated/b" }]
[component.build]
command = ["git", "--version"]
"#,
    )
    .unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane_with_workdir_mode_paths_and_neighbors(
        "graph",
        Some("main"),
        if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        },
        None,
        None,
        None,
        &[],
        false,
    )
    .unwrap();

    let rust = db.workspace_environment_graph("graph", None).unwrap();
    let expected = serde_json::to_value(&rust).unwrap();
    assert_eq!(rust.nodes.len(), 2);
    assert_eq!(rust.edges.len(), 1);
    assert_eq!(rust.nodes[0].component_id, "graph.a");
    assert_eq!(rust.nodes[1].component_id, "graph.b");
    assert_eq!(
        rust.edges[0].source_component_key,
        rust.nodes[0].component_key
    );

    let http = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/lanes/graph/environment/graph",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(http.status, 200);
    assert_eq!(http.body_json::<serde_json::Value>().unwrap(), expected);

    let mcp = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 41,
            "method": "tools/call",
            "params": {
                "name": "trail.env_graph",
                "arguments": {"lane": "graph"}
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp["result"]["isError"], false);
    assert_eq!(mcp["result"]["structuredContent"], expected);

    let cli = run_trail_json(temp.path(), &["env", "graph", "graph"]);
    assert_eq!(cli, expected);
    let openapi = trail::server::openapi_spec();
    assert_eq!(
        openapi["paths"]["/v1/lanes/{lane_or_id}/environment/graph"]["get"]["responses"]["200"]
            ["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/EnvironmentGraphReport"
    );
    assert!(db.list_workspace_layers().unwrap().is_empty());
}

#[test]
fn pinned_oci_metadata_has_cli_http_mcp_openapi_and_gc_parity() {
    let temp = tempfile::tempdir().unwrap();
    let digest = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    fs::write(
        temp.path().join("trail.oci.toml"),
        format!(
            "schema = \"trail.oci-images/v1\"\n\n[[image]]\nname = \"web\"\nreference = \"ghcr.io/example/web@{digest}\"\nplatform = \"linux/amd64\"\n"
        ),
    )
    .unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane_with_workdir_mode_paths_and_neighbors(
        "oci-surfaces",
        Some("main"),
        if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        },
        None,
        None,
        None,
        &[],
        false,
    )
    .unwrap();

    let http_plan = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/lanes/oci-surfaces/environment/plan?adapter=oci-image",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(http_plan.status, 200);
    let plan: serde_json::Value = http_plan.body_json().unwrap();
    assert_eq!(plan["component_id"], "oci-images");
    assert_eq!(plan["kind"], "external");
    assert!(plan["outputs"].as_array().unwrap().is_empty());
    assert!(plan["commands"].as_array().unwrap().is_empty());
    assert_eq!(plan["external_artifacts"][0]["digest"], digest);
    assert_eq!(
        plan["capabilities"]["sandbox"],
        "not-applicable-metadata-only"
    );
    assert_eq!(
        run_trail_json(
            temp.path(),
            &[
                "env",
                "plan",
                "oci-surfaces",
                "--adapter",
                "trail/oci-image@1",
            ],
        ),
        plan
    );

    let sync = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes/oci-surfaces/environment/sync",
            serde_json::json!({"adapter": "trail/oci-image@1"}),
        ),
    );
    assert_eq!(sync.status, 200);
    let sync: serde_json::Value = sync.body_json().unwrap();
    assert!(sync["layers"].as_array().unwrap().is_empty());
    assert!(sync["generation"]["components"][0]["outputs"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(sync["generation"]["components"][0]["layer_id"].is_null());
    assert!(sync["generation"]["components"][0]["mount_path"].is_null());
    assert_eq!(
        sync["generation"]["components"][0]["external_artifacts"][0]["reference"],
        format!("ghcr.io/example/web@{digest}")
    );
    assert_eq!(
        sync["generation"]["components"][0]["external_artifacts"][0]["cleanup_owner"],
        "external"
    );

    let mcp = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 42,
            "method": "tools/call",
            "params": {
                "name": "trail.env_generation",
                "arguments": {"lane": "oci-surfaces"}
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp["result"]["isError"], false);
    assert_eq!(mcp["result"]["structuredContent"], sync["generation"]);

    let gc = db.workspace_cache_gc(false, Some(0)).unwrap();
    assert!(gc
        .deleted
        .iter()
        .all(|entry| entry.kind != "external_artifact"));
    let active = db
        .active_environment_generation("oci-surfaces")
        .unwrap()
        .unwrap();
    assert_eq!(active.components[0].external_artifacts[0].digest, digest);
    assert!(db.list_workspace_layers().unwrap().is_empty());

    let openapi = trail::server::openapi_spec();
    assert!(openapi["components"]["schemas"]
        .get("EnvironmentExternalArtifactReport")
        .is_some());
    assert!(
        openapi["components"]["schemas"]["EnvironmentPlanReport"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field == "external_artifacts")
    );
}

#[test]
fn environment_sync_reuses_one_node_layer_across_http_and_mcp_parity() {
    if Command::new("npm").arg("--version").output().is_err()
        || Command::new("node").arg("--version").output().is_err()
    {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("package.json"),
        r#"{"name":"env-surface","version":"1.0.0","private":true}"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("package-lock.json"),
        r#"{"name":"env-surface","version":"1.0.0","lockfileVersion":3,"requires":true,"packages":{"":{"name":"env-surface","version":"1.0.0"}}}"#,
    )
    .unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    let mode = if cfg!(target_os = "macos") {
        LaneWorkdirMode::NfsCow
    } else if cfg!(target_os = "windows") {
        LaneWorkdirMode::DokanCow
    } else {
        LaneWorkdirMode::FuseCow
    };
    for lane in ["env-http", "env-mcp", "env-all-http", "env-all-mcp"] {
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            lane,
            Some("main"),
            mode.clone(),
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
    }

    let http_plan = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/lanes/env-http/environment/plan",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(http_plan.status, 200);
    let http_plan: serde_json::Value = http_plan.body_json().unwrap();
    assert_eq!(http_plan["component_id"], "node");
    assert_eq!(http_plan["capabilities"]["sandbox"], "trusted-builtin");
    assert!(http_plan["dependencies"].as_array().unwrap().is_empty());
    let mcp_plan = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 20,
            "method": "tools/call",
            "params": {
                "name": "trail.env_plan",
                "arguments": {"lane": "env-mcp"}
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_plan["result"]["isError"], false);
    assert_eq!(
        mcp_plan["result"]["structuredContent"]["component_key"],
        http_plan["component_key"]
    );

    let discovery = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/lanes/env-http/environment/discover",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(discovery.status, 200);
    let discovery: serde_json::Value = discovery.body_json().unwrap();
    assert_eq!(discovery["components"][0]["component_id"], "node");
    assert!(discovery["conflicts"].as_array().unwrap().is_empty());
    let mcp_discovery = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "tools/call",
            "params": {
                "name": "trail.env_discover",
                "arguments": {"lane": "env-mcp"}
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_discovery["result"]["isError"], false);
    assert_eq!(
        mcp_discovery["result"]["structuredContent"]["source_root"],
        discovery["source_root"]
    );

    let http = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes/env-http/environment/sync",
            serde_json::json!({"adapter": "trail/node@1"}),
        ),
    );
    assert_eq!(http.status, 200);
    let http_report: serde_json::Value = http.body_json().unwrap();
    let http_layer = &http_report["layers"][0];

    let mcp = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "trail.env_sync",
                "arguments": {"lane": "env-mcp", "adapter": "auto"}
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp["result"]["isError"], false);
    assert_eq!(
        mcp["result"]["structuredContent"]["layers"][0]["layer_id"],
        http_layer["layer_id"]
    );

    let status = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/lanes/env-http/environment",
            serde_json::Value::Null,
        ),
    );
    let status: serde_json::Value = status.body_json().unwrap();
    assert_eq!(status[0]["component"]["component_id"], "node");
    assert_eq!(status[0]["adapter"]["name"], "node");
    assert_eq!(status[0]["status"], "ready");
    assert_eq!(
        status[0]["adapter"]["distribution_digest"],
        "builtin:node-plan-v1"
    );

    let generation = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/lanes/env-http/environment/generation",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(generation.status, 200);
    let generation: serde_json::Value = generation.body_json().unwrap();
    assert_eq!(generation["generation_sequence"], 1);
    assert_eq!(generation["components"][0]["component_id"], "node");
    assert!(generation["components"][0]["dependencies"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        generation["components"][0]["layer_id"],
        http_layer["layer_id"]
    );
    let mcp_generation = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "trail.env_generation",
                "arguments": {"lane": "env-http"}
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_generation["result"]["isError"], false);
    assert_eq!(
        mcp_generation["result"]["structuredContent"]["generation_id"],
        generation["generation_id"]
    );

    let view = db.lane_workspace_view("env-http").unwrap().unwrap();
    fs::write(
        Path::new(&view.source_upper).join("package-lock.json"),
        r#"{"name":"env-surface","version":"1.0.1","lockfileVersion":3,"requires":true,"packages":{"":{"name":"env-surface","version":"1.0.1"}}}"#,
    )
    .unwrap();
    db.checkpoint_lane_workspace("env-http", Some("change lock".to_string()))
        .unwrap();
    db.lane_readiness("env-http").unwrap();
    let explained = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/lanes/env-http/environment/explain?component=node",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(explained.status, 200);
    let explained: serde_json::Value = explained.body_json().unwrap();
    assert_eq!(explained["status"], "stale");
    assert_eq!(explained["complete"], true);
    assert!(explained["changes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|change| {
            change["dimension"] == "input"
                && change["name"] == "package-lock.json"
                && change["change"] == "modified"
        }));
    let mcp_explained = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 21,
            "method": "tools/call",
            "params": {
                "name": "trail.env_explain",
                "arguments": {"lane": "env-http", "component": "node"}
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_explained["result"]["isError"], false);
    assert_eq!(mcp_explained["result"]["structuredContent"], explained);
    let cli_explained = run_trail_json(
        temp.path(),
        &["env", "explain", "env-http", "--component", "node"],
    );
    assert_eq!(cli_explained, explained);

    let all_http = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes/env-all-http/environment/sync-all",
            serde_json::json!({}),
        ),
    );
    assert_eq!(all_http.status, 200);
    let all_http: serde_json::Value = all_http.body_json().unwrap();
    assert_eq!(all_http["generation"]["generation_sequence"], 1);
    assert_eq!(all_http["layers"][0]["layer_id"], http_layer["layer_id"]);
    let all_mcp = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "trail.env_sync_all",
                "arguments": {"lane": "env-all-mcp"}
            }
        }),
    )
    .unwrap();
    assert_eq!(all_mcp["result"]["isError"], false);
    assert_eq!(
        all_mcp["result"]["structuredContent"]["layers"][0]["layer_id"],
        http_layer["layer_id"]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn writable_private_environment_sync_has_cli_http_mcp_and_openapi_parity() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("input.txt"), "private input\n").unwrap();
    fs::write(
        temp.path().join("trail.environment.toml"),
        r#"schema = "trail.environment/v1"

[environment]
default_network = "deny"
default_scripts = "deny"

[[component]]
id = "generated.private"
adapter = "trail/command@1"
root = "."
kind = "generated"

[[component.input]]
path = "input.txt"
role = "identity"
format = "bytes"

[component.build]
command = ["cp", "input.txt", "generated/copied.txt"]
cwd = "."
network = "deny"
scripts = "deny"

[[component.output]]
name = "private"
source = "generated"
target = ".trail-generated/private"
policy = "writable_private"
portability = "host"
"#,
    )
    .unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    for lane in ["private-http", "private-mcp", "private-cli"] {
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            lane,
            Some("main"),
            LaneWorkdirMode::NfsCow,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
    }

    let http = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes/private-http/environment/sync",
            serde_json::json!({"adapter": "trail/command@1"}),
        ),
    );
    assert_eq!(http.status, 200);
    let http: serde_json::Value = http.body_json().unwrap();
    assert!(http["layers"].as_array().unwrap().is_empty());
    let http_output = &http["generation"]["components"][0]["outputs"][0];
    assert_eq!(http_output["policy"], "writable_private");
    assert!(http_output["layer_id"].is_null());
    assert!(http_output["storage_identity"]
        .as_str()
        .unwrap()
        .starts_with("private_"));

    let mcp = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 31,
            "method": "tools/call",
            "params": {
                "name": "trail.env_sync",
                "arguments": {"lane": "private-mcp", "adapter": "trail/command@1"}
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp["result"]["isError"], false);
    let mcp = &mcp["result"]["structuredContent"];
    assert!(mcp["layers"].as_array().unwrap().is_empty());
    assert_eq!(
        mcp["generation"]["components"][0]["outputs"][0]["policy"],
        http_output["policy"]
    );
    assert!(mcp["generation"]["components"][0]["outputs"][0]["layer_id"].is_null());

    let cli = run_trail_json(
        temp.path(),
        &["env", "sync", "private-cli", "--adapter", "trail/command@1"],
    );
    assert!(cli["layers"].as_array().unwrap().is_empty());
    assert_eq!(
        cli["generation"]["components"][0]["outputs"][0]["policy"],
        "writable_private"
    );
    assert!(cli["generation"]["components"][0]["outputs"][0]["layer_id"].is_null());

    let openapi = trail::server::openapi_spec();
    assert_eq!(
        openapi["paths"]["/v1/lanes/{lane_or_id}/environment/sync"]["post"]["responses"]["200"]
            ["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/EnvironmentSyncReport"
    );
    assert_eq!(
        openapi["components"]["schemas"]["EnvironmentGenerationOutputReport"]["properties"]
            ["policy"]["enum"][1],
        "writable_private"
    );
    assert_eq!(
        openapi["components"]["schemas"]["EnvironmentPlanReport"]["properties"]["dependencies"]
            ["items"]["type"],
        "string"
    );
    assert_eq!(
        openapi["components"]["schemas"]["EnvironmentGenerationComponentReport"]["properties"]
            ["dependencies"]["items"]["$ref"],
        "#/components/schemas/EnvironmentGenerationDependencyReport"
    );
}

#[test]
fn environment_sync_reuses_one_node_layer_across_http_and_mcp() {
    if Command::new("npm").arg("--version").output().is_err()
        || Command::new("node").arg("--version").output().is_err()
    {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("package.json"),
        r#"{"name":"env-surface","version":"1.0.0","private":true}"#,
    )
    .unwrap();
    fs::write(
        temp.path().join("package-lock.json"),
        r#"{"name":"env-surface","version":"1.0.0","lockfileVersion":3,"requires":true,"packages":{"":{"name":"env-surface","version":"1.0.0"}}}"#,
    )
    .unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    let mode = if cfg!(target_os = "macos") {
        LaneWorkdirMode::NfsCow
    } else if cfg!(target_os = "windows") {
        LaneWorkdirMode::DokanCow
    } else {
        LaneWorkdirMode::FuseCow
    };
    for lane in ["env-http", "env-mcp"] {
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            lane,
            Some("main"),
            mode.clone(),
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
    }

    let http = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes/env-http/environment/sync",
            serde_json::json!({"adapter": "trail/node@1"}),
        ),
    );
    assert_eq!(http.status, 200);
    let http_report: serde_json::Value = http.body_json().unwrap();
    let http_layer = &http_report["layers"][0];

    let mcp = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "trail.env_sync",
                "arguments": {"lane": "env-mcp", "adapter": "auto"}
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp["result"]["isError"], false);
    assert_eq!(
        mcp["result"]["structuredContent"]["layers"][0]["layer_id"],
        http_layer["layer_id"]
    );

    let status = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/lanes/env-http/environment",
            serde_json::Value::Null,
        ),
    );
    let status: serde_json::Value = status.body_json().unwrap();
    assert_eq!(status[0]["component"]["component_id"], "node");
    assert_eq!(status[0]["adapter"]["name"], "node");
    assert_eq!(status[0]["status"], "ready");
    assert_eq!(
        status[0]["adapter"]["distribution_digest"],
        "builtin:node-plan-v1"
    );
}

#[test]
fn local_lane_http_api_can_require_bearer_token() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let auth = trail::server::ServerAuth::bearer("secret-token").unwrap();

    let health = trail::server::handle_http_request_with_auth(
        &mut db,
        &api_request("GET", "/v1/health", serde_json::Value::Null),
        &auth,
    );
    assert_eq!(health.status, 200);

    let cross_origin = trail::server::handle_http_request_with_auth(
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

    for origin in ["null", "http://localhost:999999"] {
        let malformed_origin = trail::server::handle_http_request_with_auth(
            &mut db,
            &api_request_with_headers(
                "GET",
                "/v1/health",
                &[("Origin", origin)],
                serde_json::Value::Null,
            ),
            &auth,
        );
        assert_eq!(malformed_origin.status, 403, "origin {origin}");
        let malformed_origin_body: serde_json::Value = malformed_origin.body_json().unwrap();
        assert!(
            malformed_origin_body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("local loopback origin"),
            "origin {origin}: {malformed_origin_body:?}"
        );
    }

    let missing = trail::server::handle_http_request_with_auth(
        &mut db,
        &api_request(
            "POST",
            "/v1/lane/turns",
            serde_json::json!({ "lane": "secure-lane", "branch": "main" }),
        ),
        &auth,
    );
    assert_eq!(missing.status, 401);

    let invalid = trail::server::handle_http_request_with_auth(
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

    let forbidden_origin = trail::server::handle_http_request_with_auth(
        &mut db,
        &api_request_with_headers(
            "POST",
            "/v1/lane/turns",
            &[
                ("Authorization", "Bearer secret-token"),
                ("Origin", "https://example.com"),
            ],
            serde_json::json!({ "lane": "forbidden-origin-lane", "branch": "main" }),
        ),
        &auth,
    );
    assert_eq!(forbidden_origin.status, 403);

    let bad_host_body =
        serde_json::to_vec(&serde_json::json!({ "lane": "bad-host-lane", "branch": "main" }))
            .unwrap();
    let bad_host_request = [
        format!(
            "POST /v1/lane/turns HTTP/1.1\r\nHost: example.com\r\nAuthorization: Bearer secret-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            bad_host_body.len()
        )
        .into_bytes(),
        bad_host_body,
    ]
    .concat();
    let forbidden_host =
        trail::server::handle_http_request_with_auth(&mut db, &bad_host_request, &auth);
    assert_eq!(forbidden_host.status, 403);
    let forbidden_host_body: serde_json::Value = forbidden_host.body_json().unwrap();
    assert!(forbidden_host_body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("local loopback host"));

    let missing_host_body = serde_json::to_vec(&serde_json::json!({
        "lane": "missing-host-lane",
        "branch": "main"
    }))
    .unwrap();
    let missing_host_request = [
        format!(
            "POST /v1/lane/turns HTTP/1.1\r\nAuthorization: Bearer secret-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            missing_host_body.len()
        )
        .into_bytes(),
        missing_host_body,
    ]
    .concat();
    let missing_host =
        trail::server::handle_http_request_with_auth(&mut db, &missing_host_request, &auth);
    assert_eq!(missing_host.status, 403);
    let missing_host_body: serde_json::Value = missing_host.body_json().unwrap();
    assert!(missing_host_body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("request host is missing"));

    let ok = trail::server::handle_http_request_with_auth(
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

    let local_origin = trail::server::handle_http_request_with_auth(
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

    let ipv6_loopback_origin = trail::server::handle_http_request_with_auth(
        &mut db,
        &api_request_with_headers(
            "POST",
            "/v1/lane/turns",
            &[
                ("Authorization", "Bearer secret-token"),
                ("Origin", "http://[::1]:8765"),
            ],
            serde_json::json!({ "lane": "ipv6-local-origin-lane", "branch": "main" }),
        ),
        &auth,
    );
    assert_eq!(ipv6_loopback_origin.status, 201);

    let second = trail::server::handle_http_request_with_auth(
        &mut db,
        &api_request_with_headers(
            "POST",
            "/v1/lane/turns",
            &[("X-Trail-Token", "secret-token")],
            serde_json::json!({ "lane": "other-secure-lane", "branch": "main" }),
        ),
        &auth,
    );
    assert_eq!(second.status, 201);

    let audits = db.list_external_mutation_audit(20).unwrap();
    assert!(audits.iter().any(|audit| {
        audit.actor == "http:no-auth"
            && audit.surface == "http"
            && audit.command == "POST /v1/lane/turns"
            && audit.status == "error"
            && audit.status_code == Some(401)
    }));
    assert!(audits.iter().any(|audit| {
        audit.actor == "http:bearer"
            && audit.surface == "http"
            && audit.command == "POST /v1/lane/turns"
            && audit.status == "error"
            && audit.status_code == Some(401)
    }));
    assert!(audits.iter().any(|audit| {
        audit.actor == "http:bearer"
            && audit.surface == "http"
            && audit.command == "POST /v1/lane/turns"
            && audit.status == "error"
            && audit.status_code == Some(403)
            && audit
                .summary
                .as_ref()
                .and_then(|summary| summary["error"].as_str())
                .is_some_and(|message| message.contains("local loopback origin"))
    }));
    assert!(audits.iter().any(|audit| {
        audit.actor == "http:bearer"
            && audit.surface == "http"
            && audit.command == "POST /v1/lane/turns"
            && audit.status == "error"
            && audit.status_code == Some(403)
            && audit
                .summary
                .as_ref()
                .and_then(|summary| summary["error"].as_str())
                .is_some_and(|message| message.contains("local loopback host"))
    }));
    assert!(audits.iter().any(|audit| {
        audit.actor == "http:bearer"
            && audit.surface == "http"
            && audit.command == "POST /v1/lane/turns"
            && audit.status == "error"
            && audit.status_code == Some(403)
            && audit
                .summary
                .as_ref()
                .and_then(|summary| summary["error"].as_str())
                .is_some_and(|message| message.contains("request host is missing"))
    }));
}

#[test]
fn local_lane_http_api_rejects_oversized_requests() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let raw = vec![b'x'; 16 * 1024 * 1024 + 1];
    let response = trail::server::handle_http_request(&mut db, &raw);
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let workspace = temp.path().to_path_buf();
    let handle = thread::spawn(move || {
        let mut db = Trail::open(workspace).unwrap();
        let rate_limit =
            trail::server::ServerRateLimit::per_window(1, Duration::from_secs(60)).unwrap();
        trail::server::serve_listener_with_auth_and_rate_limit(
            &mut db,
            listener,
            Some(2),
            trail::server::ServerAuth::disabled(),
            rate_limit,
        )
        .unwrap();
    });

    let request = b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    let first = raw_http_request(port, request);
    assert!(first.contains(" 200 "), "{first}");
    let second = raw_http_request(port, request);
    assert!(second.contains(" 429 "), "{second}");
    assert!(second.contains("\r\nRetry-After: "), "{second}");
    assert!(second.contains("rate limit exceeded"), "{second}");
    handle.join().unwrap();
}

#[test]
fn daemon_cli_rate_limit_options_apply_to_listener() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let port = free_loopback_port();
    let mut daemon = DaemonGuard {
        child: Command::new(trail_bin())
            .arg("--workspace")
            .arg(temp.path())
            .arg("--quiet")
            .arg("daemon")
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(port.to_string())
            .arg("--max-requests")
            .arg("3")
            .arg("--rate-limit-requests")
            .arg("2")
            .arg("--rate-limit-window-secs")
            .arg("60")
            .arg("--no-auth")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    };

    wait_for_daemon_health(port);
    let request = b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    let first = raw_http_request(port, request);
    assert!(first.contains(" 200 "), "{first}");
    let second = raw_http_request(port, request);
    assert!(second.contains(" 429 "), "{second}");
    assert!(second.contains("\r\nRetry-After: "), "{second}");
    assert!(second.contains("rate limit exceeded"), "{second}");
    wait_for_child_exit(&mut daemon.child);
}

#[test]
fn daemon_cli_connection_timeout_option_applies_to_listener() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let port = free_loopback_port();
    let mut daemon = DaemonGuard {
        child: Command::new(trail_bin())
            .arg("--workspace")
            .arg(temp.path())
            .arg("--quiet")
            .arg("daemon")
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(port.to_string())
            .arg("--max-requests")
            .arg("3")
            .arg("--connection-timeout-secs")
            .arg("1")
            .arg("--no-auth")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    };

    wait_for_daemon_health(port);
    let mut slow = TcpStream::connect(("127.0.0.1", port)).unwrap();
    slow.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    slow.write_all(b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\n")
        .unwrap();
    let mut timeout_response = String::new();
    slow.read_to_string(&mut timeout_response).unwrap();
    assert!(timeout_response.contains(" 408 "), "{timeout_response}");
    assert!(
        timeout_response.contains("timed out after 1 seconds"),
        "{timeout_response}"
    );

    let ok = raw_http_request(
        port,
        b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert!(ok.contains(" 200 "), "{ok}");
    wait_for_child_exit(&mut daemon.child);
}

#[test]
fn daemon_rejects_invalid_rate_limit_and_timeout_options() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    for (flag, expected) in [
        (
            "--rate-limit-requests",
            "max_requests must be greater than zero",
        ),
        (
            "--rate-limit-window-secs",
            "window must be greater than zero",
        ),
        (
            "--connection-timeout-secs",
            "--connection-timeout-secs must be greater than zero",
        ),
    ] {
        let output = Command::new(trail_bin())
            .arg("--workspace")
            .arg(temp.path())
            .arg("daemon")
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg("0")
            .arg("--once")
            .arg("--no-auth")
            .arg(flag)
            .arg("0")
            .output()
            .unwrap();

        assert!(!output.status.success(), "{flag} unexpectedly succeeded");
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(expected),
            "{flag} stderr did not contain {expected:?}: {stderr}"
        );
    }
}

#[test]
fn daemon_listener_times_out_slow_requests_without_exiting() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let workspace = temp.path().to_path_buf();
    let handle = thread::spawn(move || {
        let mut db = Trail::open(workspace).unwrap();
        trail::server::serve_listener_with_auth_rate_limit_and_timeout(
            &mut db,
            listener,
            Some(2),
            trail::server::ServerAuth::disabled(),
            trail::server::ServerRateLimit::disabled(),
            Duration::from_millis(150),
        )
        .unwrap();
    });

    let mut slow = TcpStream::connect(("127.0.0.1", port)).unwrap();
    slow.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    slow.write_all(b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\n")
        .unwrap();
    let mut timeout_response = String::new();
    slow.read_to_string(&mut timeout_response).unwrap();
    assert!(timeout_response.contains(" 408 "), "{timeout_response}");
    assert!(
        timeout_response.contains("HTTP request timed out"),
        "{timeout_response}"
    );

    let ok = raw_http_request(
        port,
        b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert!(ok.contains(" 200 "), "{ok}");
    handle.join().unwrap();
}

#[test]
fn daemon_no_auth_prints_loud_stderr_warning() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let port = free_loopback_port();
    let child = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("daemon")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--once")
        .arg("--no-auth")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    wait_for_daemon_health(port);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let daemon_url = format!("http://127.0.0.1:{port}");
    assert!(stdout.contains("Trail API daemon listening"), "{stdout}");
    assert!(
        stdout.contains(&format!("Endpoint: {daemon_url}")),
        "{stdout}"
    );
    assert!(stderr.contains("WARNING"), "{stderr}");
    assert!(stderr.contains("daemon auth is disabled"), "{stderr}");
    assert!(
        stderr.contains("Any local process can mutate this workspace"),
        "{stderr}"
    );
    assert!(stderr.contains(&daemon_url), "{stderr}");
    assert!(!stdout.contains("Daemon auth disabled"), "{stdout}");

    let quiet_port = free_loopback_port();
    let quiet_child = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("--quiet")
        .arg("daemon")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(quiet_port.to_string())
        .arg("--once")
        .arg("--no-auth")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    wait_for_daemon_health(quiet_port);
    let quiet_output = quiet_child.wait_with_output().unwrap();
    assert!(quiet_output.status.success());
    let quiet_stdout = String::from_utf8_lossy(&quiet_output.stdout);
    let quiet_stderr = String::from_utf8_lossy(&quiet_output.stderr);
    let quiet_daemon_url = format!("http://127.0.0.1:{quiet_port}");
    assert!(quiet_stdout.is_empty(), "{quiet_stdout}");
    assert!(quiet_stderr.contains("WARNING"), "{quiet_stderr}");
    assert!(
        quiet_stderr.contains("daemon auth is disabled"),
        "{quiet_stderr}"
    );
    assert!(quiet_stderr.contains(&quiet_daemon_url), "{quiet_stderr}");
}

#[test]
fn daemon_no_auth_rejects_non_loopback_listener() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let output = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("daemon")
        .arg("--host")
        .arg("0.0.0.0")
        .arg("--port")
        .arg("0")
        .arg("--once")
        .arg("--no-auth")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--no-auth requires a loopback --host"),
        "{stderr}"
    );
    assert!(stderr.contains("0.0.0.0"), "{stderr}");
}

#[test]
fn cli_daemon_url_routes_hot_lane_commands() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    fs::write(temp.path().join("NOTES.md"), "notes\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let port = free_loopback_port();
    let mut daemon = DaemonGuard {
        child: Command::new(trail_bin())
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
    let status = run_trail_json_daemon(temp.path(), &daemon_url, &["status"]);
    assert_eq!(status["branch"], "main");

    let spawn = run_trail_json_daemon(
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

    let list = run_trail_json_daemon(temp.path(), &daemon_url, &["lane", "list"]);
    assert!(list
        .as_array()
        .unwrap()
        .iter()
        .any(|lane| lane["record"]["name"] == "rpc-bot"));

    let show = run_trail_json_daemon(temp.path(), &daemon_url, &["lane", "show", "rpc-bot"]);
    assert_eq!(show["record"]["name"], "rpc-bot");
    assert_eq!(show["branch"]["ref_name"], "refs/lanes/rpc-bot");

    let no_workdir =
        run_trail_json_daemon(temp.path(), &daemon_url, &["lane", "workdir", "rpc-bot"]);
    assert!(no_workdir["workdir"].is_null());

    let claim = run_trail_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "claim", "rpc-bot", "README.md", "--ttl-secs", "120"],
    );
    assert_eq!(claim["claimed"], true);
    assert_eq!(claim["path"], "README.md");

    let lease = run_trail_json_daemon(
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
    let leases = run_trail_json_daemon(temp.path(), &daemon_url, &["lease", "list"]);
    assert!(leases
        .as_array()
        .unwrap()
        .iter()
        .any(|lease| lease["lease_id"] == lease_id));
    let released =
        run_trail_json_daemon(temp.path(), &daemon_url, &["lease", "release", &lease_id]);
    assert_eq!(released["lease_id"], lease_id);

    let session = run_trail_json_daemon(
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
        run_trail_json_daemon(temp.path(), &daemon_url, &["session", "current", "rpc-bot"]);
    assert!(current_session
        .as_array()
        .unwrap()
        .iter()
        .any(|report| report["session"]["session_id"] == session_id));
    let session_list = run_trail_json_daemon(
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
        run_trail_json_daemon(temp.path(), &daemon_url, &["session", "show", &session_id]);
    assert_eq!(session_show["session"]["session_id"], session_id);
    let session_context = run_trail_json_daemon(
        temp.path(),
        &daemon_url,
        &["session", "context", &session_id, "--limit", "5"],
    );
    assert_eq!(session_context["session"]["session_id"], session_id);

    let approval = run_trail_json_daemon(
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
    let approvals = run_trail_json_daemon(
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
    let approval_show = run_trail_json_daemon(
        temp.path(),
        &daemon_url,
        &["approvals", "show", &approval_id],
    );
    assert_eq!(approval_show["approval_id"], approval_id);
    let approval_decision = run_trail_json_daemon(
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
    let patch = run_trail_json_daemon(
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

    let read = run_trail_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "read", "rpc-bot", "README.md"],
    );
    assert_eq!(read["content"], "hello\nrpc\n");

    let diff = run_trail_json_daemon(temp.path(), &daemon_url, &["lane", "diff", "rpc-bot"]);
    assert!(diff["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "README.md"));

    let doctor = run_trail_json_daemon(temp.path(), &daemon_url, &["doctor"]);
    assert_eq!(doctor["status"], "ok");
    let why = run_trail_json_daemon(temp.path(), &daemon_url, &["why", "README.md:1"]);
    assert_eq!(why["path"], "README.md");
    let history = run_trail_json_daemon(temp.path(), &daemon_url, &["history", "README.md"]);
    assert!(history["file_history"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["path"] == "README.md"));
    let code_from = run_trail_json_daemon(temp.path(), &daemon_url, &["code-from", "rpc-bot"]);
    assert!(code_from["operations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|operation| operation["message"] == "daemon CLI patch"));

    let lane_timeline =
        run_trail_json_daemon(temp.path(), &daemon_url, &["lane", "timeline", "rpc-bot"]);
    assert!(lane_timeline
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["message"] == "daemon CLI patch"));
    let timeline = run_trail_json_daemon(
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
        run_trail_json_daemon(temp.path(), &daemon_url, &["lane", "readiness", "rpc-bot"]);
    assert_eq!(readiness["ready"], true);

    let contribution = run_trail_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "contribution", "rpc-bot"],
    );
    assert_eq!(contribution["status"]["lane"]["record"]["name"], "rpc-bot");

    let review = run_trail_json_daemon(temp.path(), &daemon_url, &["lane", "review", "rpc-bot"]);
    assert_eq!(review["lane"]["record"]["name"], "rpc-bot");
    assert_eq!(review["readiness"]["ready"], true);
    assert!(review["recent_operations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|operation| operation["message"] == "daemon CLI patch"));

    let gates = run_trail_json_daemon(temp.path(), &daemon_url, &["lane", "gates", "rpc-bot"]);
    assert_eq!(gates["lane"]["record"]["name"], "rpc-bot");

    let events = run_trail_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "events", "--lane", "rpc-bot"],
    );
    assert!(events
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["lane_id"] == spawn["lane_id"]));

    let turn = run_trail_json_daemon(
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

    let turn_message = run_trail_json_daemon(
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

    let turn_event = run_trail_json_daemon(
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
    let turn_patch = run_trail_json_daemon(
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

    let turn_details = run_trail_json_daemon(
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

    let trace_start = run_trail_json_daemon(
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

    let trace_end = run_trail_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "trace", "end", &span_id, "--status", "completed"],
    );
    assert_eq!(trace_end["span"]["status"], "completed");

    let trace_list = run_trail_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "trace", "list", "--turn", &turn_id, "--limit", "10"],
    );
    assert!(trace_list
        .as_array()
        .unwrap()
        .iter()
        .any(|span| span["span_id"] == span_id));

    let trace_summary = run_trail_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "trace", "summary", "--trace-id", &trace_id],
    );
    assert_eq!(trace_summary["trace_id"].as_str().unwrap(), trace_id);
    assert_eq!(trace_summary["span_count"], 1);
    assert_eq!(trace_summary["ended_span_count"], 1);

    let trace_show = run_trail_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "trace", "show", &span_id],
    );
    assert_eq!(trace_show["span_id"].as_str().unwrap(), span_id);
    assert_eq!(trace_show["trace_id"].as_str().unwrap(), trace_id);

    let turn_end = run_trail_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "turn", "end", &turn_id, "--status", "completed"],
    );
    assert_eq!(turn_end["turn"]["status"], "completed");

    let handoff = run_trail_json_daemon(temp.path(), &daemon_url, &["lane", "handoff", "rpc-bot"]);
    assert_eq!(handoff["lane"]["record"]["name"], "rpc-bot");

    fs::write(
        temp.path().join("NOTES.md"),
        "notes\nworkspace record through daemon\n",
    )
    .unwrap();
    let workspace_record = run_trail_json_daemon(
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
    let clean_status = run_trail_json_daemon(temp.path(), &daemon_url, &["status"]);
    assert_eq!(clean_status["worktree_state"], "Clean");

    let materialized = run_trail_json_daemon(
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
        run_trail_json_daemon(temp.path(), &daemon_url, &["lane", "workdir", "mat-rpc"]);
    assert_eq!(
        materialized_workdir_report["workdir"].as_str().unwrap(),
        materialized_workdir
    );
    fs::write(
        Path::new(materialized_workdir).join("README.md"),
        "hello\nrecorded through daemon\n",
    )
    .unwrap();
    let recorded = run_trail_json_daemon(
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
        run_trail_json_daemon(temp.path(), &daemon_url, &["lane", "status", "mat-rpc"]);
    assert_eq!(materialized_status["workdir_state"], "Clean");

    let merge = run_trail_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "merge", "rpc-bot", "--into", "main", "--dry-run"],
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let endpoint_path = temp.path().join(".trail/daemon.json");
    let port = free_loopback_port();
    let mut daemon = DaemonGuard {
        child: Command::new(trail_bin())
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

    let lanes = run_trail_json(temp.path(), &["lane", "list"]);
    assert!(lanes.as_array().unwrap().is_empty());
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
    let fallback = run_trail_json(temp.path(), &["lane", "list"]);
    assert!(fallback.as_array().unwrap().is_empty());
}

#[test]
fn local_api_and_cli_export_openapi_contract() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let cli = run_trail_json(temp.path(), &["api", "openapi"]);
    assert_eq!(cli["openapi"], "3.1.0");
    assert!(cli["paths"].get("/v1/openapi.json").is_some());
    assert!(cli["paths"].get("/v1/index/reconcile").is_some());
    assert_eq!(
        cli["paths"]["/v1/index/reconcile"]["post"]["responses"]["200"]["content"]
            ["application/json"]["schema"]["$ref"],
        "#/components/schemas/ChangeLedgerReconcileReport"
    );
    assert!(cli["paths"]["/v1/lanes"]["get"].is_object());
    assert!(cli["paths"]["/v1/lanes/{lane_or_id}"]["delete"].is_object());
    assert!(cli["paths"]
        .get("/v1/lanes/{lane_or_id}/read-file")
        .is_some());
    assert_eq!(
        cli["paths"]["/v1/lanes/{lane}/merge"]["post"]["operationId"],
        "laneMerge"
    );
    assert!(cli["paths"]
        .get("/v1/branches/{branch}/merge-lane")
        .is_none());
    assert_eq!(
        cli["components"]["schemas"]["LaneMergeRequest"]["required"],
        serde_json::json!(["into"])
    );
    assert!(cli["components"]["schemas"]
        .get("MergeLaneRequest")
        .is_none());
    assert!(cli["components"]["schemas"]["LaneReadFileRequest"].is_object());
    let workdir_modes = cli["components"]["schemas"]["SpawnLaneRequest"]["properties"]
        ["workdir_mode"]["enum"]
        .as_array()
        .unwrap();
    assert!(workdir_modes.iter().any(|mode| mode == "native-cow"));
    assert!(workdir_modes.iter().any(|mode| mode == "portable-copy"));
    assert!(workdir_modes.iter().any(|mode| mode == "fuse-cow"));
    assert!(workdir_modes.iter().any(|mode| mode == "dokan-cow"));
    assert!(!workdir_modes.iter().any(|mode| mode == "full-cow"));
    assert!(!workdir_modes.iter().any(|mode| mode == "overlay-cow"));
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

    let output_path = temp.path().join("trail.openapi.json");
    let output = Command::new(trail_bin())
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
        "trail api openapi --output failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let exported: serde_json::Value =
        serde_json::from_slice(&fs::read(&output_path).unwrap()).unwrap();
    assert_eq!(exported["info"]["title"], "Trail Local API");

    let mut db = Trail::open(temp.path()).unwrap();
    let response = trail::server::handle_http_request(
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
    let schemas = api["components"]["schemas"].as_object().unwrap();
    let loose_requests = schemas
        .iter()
        .filter_map(|(name, schema)| {
            if name.ends_with("Request")
                && schema["type"] == "object"
                && schema["additionalProperties"] != false
            {
                Some(name.as_str())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert!(
        loose_requests.is_empty(),
        "request schemas must reject unknown top-level fields: {loose_requests:?}"
    );
    let patch_request = schemas.get("PatchRequest").unwrap();
    let patch_request_modes = patch_request["oneOf"].as_array().unwrap();
    assert_eq!(patch_request_modes.len(), 2);
    assert_eq!(
        patch_request_modes[0]["required"],
        serde_json::json!(["edits"])
    );
    assert_eq!(
        patch_request_modes[0]["not"]["required"],
        serde_json::json!(["files"])
    );
    assert_eq!(
        patch_request_modes[1]["required"],
        serde_json::json!(["files"])
    );
    assert_eq!(
        patch_request_modes[1]["not"]["required"],
        serde_json::json!(["edits"])
    );
    assert!(patch_request["properties"]["edits"]
        .get("minItems")
        .is_none());
    assert!(patch_request["properties"]["files"]
        .get("minItems")
        .is_none());
    assert_eq!(
        patch_request["properties"]["edits"]["items"]["$ref"],
        "#/components/schemas/PatchEdit"
    );
    assert_eq!(
        patch_request["properties"]["files"]["items"]["$ref"],
        "#/components/schemas/ApiPatchFile"
    );
    let file_diff_summary = schemas.get("FileDiffSummary").unwrap();
    assert_eq!(file_diff_summary["additionalProperties"], false);
    assert!(file_diff_summary["required"]
        .as_array()
        .unwrap()
        .iter()
        .any(|field| field == "path"));
    assert_eq!(
        file_diff_summary["properties"]["kind"]["enum"],
        serde_json::json!(["Added", "Modified", "Deleted", "Renamed", "TypeChanged"])
    );
    let lane_refresh_preview = schemas.get("LaneRefreshPreviewReport").unwrap();
    assert_eq!(
        lane_refresh_preview["properties"]["changed_paths"]["items"]["$ref"],
        "#/components/schemas/FileDiffSummary"
    );
    assert_eq!(
        api["paths"]["/v1/lanes/{lane_or_id}/record"]["post"]["responses"]["200"]["content"]
            ["application/json"]["schema"]["$ref"],
        "#/components/schemas/LaneRecordWorkdirResponse"
    );
    let record_response = schemas.get("LaneRecordWorkdirResponse").unwrap();
    assert_eq!(
        record_response["oneOf"][0]["$ref"],
        "#/components/schemas/LaneRecordReport"
    );
    assert_eq!(
        record_response["oneOf"][1]["$ref"],
        "#/components/schemas/LaneRecordPreviewReport"
    );
    let record_preview = schemas.get("LaneRecordPreviewReport").unwrap();
    assert_eq!(record_preview["additionalProperties"], false);
    for field in [
        "changed_paths",
        "ignored_paths",
        "risky_paths",
        "oversized_files",
        "policy",
    ] {
        assert!(
            record_preview["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|required| required == field),
            "LaneRecordPreviewReport must require {field}"
        );
    }
    assert_eq!(
        record_preview["properties"]["changed_paths"]["items"]["$ref"],
        "#/components/schemas/FileDiffSummary"
    );
    assert_eq!(
        record_preview["properties"]["risky_paths"]["items"]["$ref"],
        "#/components/schemas/LaneWorkdirRisk"
    );
    let workdir_risk = schemas.get("LaneWorkdirRisk").unwrap();
    assert_eq!(workdir_risk["additionalProperties"], false);
    assert_eq!(
        workdir_risk["properties"]["kind"]["enum"],
        serde_json::json!([
            "nested_git",
            "nested_trail",
            "symlink",
            "hardlink",
            "external_mount"
        ])
    );
    assert_eq!(
        schemas.get("LaneRecordOversizedFile").unwrap()["required"],
        serde_json::json!(["path", "size_bytes", "limit_bytes"])
    );
    let conflict_merge_context = schemas.get("ConflictMergeContext").unwrap();
    for field in ["base_root", "target_root", "source_root"] {
        assert!(
            conflict_merge_context["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|required| required == field),
            "ConflictMergeContext must require {field}"
        );
        assert_eq!(
            conflict_merge_context["properties"][field]["type"],
            serde_json::json!("string")
        );
    }
    let conflict_path_explanation = schemas.get("ConflictPathExplanation").unwrap();
    assert!(conflict_path_explanation["required"]
        .as_array()
        .unwrap()
        .iter()
        .any(|field| field == "conflict_class"));
    assert_eq!(
        conflict_path_explanation["properties"]["conflict_class"]["enum"],
        serde_json::json!([
            "modify/modify",
            "delete/modify",
            "rename/modify",
            "binary",
            "mode",
            "same_insertion_gap"
        ])
    );
    assert_eq!(
        conflict_path_explanation["properties"]["known_resolutions"]["items"]["$ref"],
        "#/components/schemas/ConflictKnownResolution"
    );
    assert_eq!(
        schemas.get("PatchEdit").unwrap()["oneOf"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert_eq!(
        schemas.get("ApiPatchFile").unwrap()["oneOf"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert_eq!(
        schemas.get("ApiTextEdit").unwrap()["oneOf"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    for schema_name in [
        "PatchEditWrite",
        "PatchEditWriteBytes",
        "PatchEditReplaceLine",
        "PatchEditDelete",
        "PatchEditRename",
        "ApiPatchFileAddText",
        "ApiPatchFileModifyText",
        "ApiPatchFileWriteBytes",
        "ApiPatchFileDelete",
        "ApiPatchFileRename",
        "ApiTextEditModifyLine",
    ] {
        assert_eq!(
            schemas.get(schema_name).unwrap()["additionalProperties"],
            false,
            "{schema_name} must reject unknown fields"
        );
    }
    assert!(schemas.get("PatchEditReplaceLine").unwrap()["required"]
        .as_array()
        .unwrap()
        .iter()
        .any(|field| field == "expected_text"));
    assert!(schemas.get("ApiTextEditModifyLine").unwrap()["required"]
        .as_array()
        .unwrap()
        .iter()
        .any(|field| field == "expected_text"));
    let conflict_resolve = schemas.get("ConflictResolveRequest").unwrap();
    let conflict_resolve_modes = conflict_resolve["oneOf"].as_array().unwrap();
    assert_eq!(conflict_resolve_modes.len(), 2);
    assert_eq!(
        conflict_resolve_modes[0]["required"],
        serde_json::json!(["take"])
    );
    assert_eq!(
        conflict_resolve_modes[0]["not"]["required"],
        serde_json::json!(["manual"])
    );
    assert_eq!(
        conflict_resolve_modes[1]["required"],
        serde_json::json!(["manual"])
    );
    assert_eq!(
        conflict_resolve_modes[1]["not"]["required"],
        serde_json::json!(["take"])
    );
    assert_eq!(
        conflict_resolve["properties"]["manual"]["additionalProperties"],
        false
    );
    assert_eq!(
        conflict_resolve["properties"]["manual"]["properties"]["files"]["additionalProperties"]
            ["oneOf"][1]["additionalProperties"],
        false
    );

    let auth = trail::server::ServerAuth::bearer("secret-token").unwrap();
    let missing = trail::server::handle_http_request_with_auth(
        &mut db,
        &api_request("GET", "/v1/openapi.json", serde_json::Value::Null),
        &auth,
    );
    assert_eq!(missing.status, 401);

    let ok = trail::server::handle_http_request_with_auth(
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let started = run_trail_json(
        temp.path(),
        &[
            "lane", "turn", "start", "cli-lane", "--from", "main", "--title", "CLI turn",
        ],
    );
    let turn_id = started["turn"]["turn_id"].as_str().unwrap().to_string();
    assert_eq!(started["session"]["title"], "CLI turn");

    let message = run_trail_json(
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

    let event = run_trail_json(
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
    let event_update = run_trail_json(
        temp.path(),
        &[
            "lane",
            "turn",
            "event",
            &turn_id,
            "--event-type",
            "tool_call_update",
            "--payload-json",
            r#"{"tool":"cli.apply_patch","status":"completed"}"#,
        ],
    );
    assert_eq!(event_update["event"]["event_type"], "tool_call_update");

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
    let patch = run_trail_json(
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

    let details = run_trail_json(temp.path(), &["lane", "turn", "show", &turn_id]);
    assert_eq!(details["turn"]["status"], "patch_applied");
    assert_eq!(details["messages"][0]["body"], "Add a CLI turn note");
    assert!(details["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["event_type"] == "tool_call"));
    let event_types = details["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["event_type"].as_str().unwrap())
        .collect::<Vec<_>>();
    let tool_call_index = event_types
        .iter()
        .position(|event_type| *event_type == "tool_call")
        .unwrap();
    let tool_update_index = event_types
        .iter()
        .position(|event_type| *event_type == "tool_call_update")
        .unwrap();
    assert!(
        tool_call_index < tool_update_index,
        "same-second turn events should retain insertion order"
    );
    assert!(details["operations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|operation| operation["message"] == "add CLI turn note"));

    let ended = run_trail_json(
        temp.path(),
        &["lane", "turn", "end", &turn_id, "--status", "completed"],
    );
    assert_eq!(ended["turn"]["status"], "completed");

    let details = run_trail_json(temp.path(), &["lane", "turn", "show", &turn_id]);
    assert_eq!(details["turn"]["status"], "completed");
}

#[test]
fn mcp_stdio_tools_drive_lane_turn_workflow() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.config_set("lane.default_materialize", "true").unwrap();
    let init = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        }),
    )
    .unwrap();
    assert_eq!(init["result"]["serverInfo"]["name"], "trail");
    assert!(init["result"]["capabilities"]["tools"].is_object());
    assert!(init["result"]["capabilities"]["resources"].is_object());
    assert!(init["result"]["capabilities"]["prompts"].is_object());
    assert!(init["result"]["capabilities"]["completions"].is_object());

    let resources = trail::mcp::handle_json_rpc(
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
        .any(|resource| resource["uri"] == "trail://workspace/status"));
    assert!(resources_list
        .iter()
        .any(|resource| resource["uri"] == "trail://workspace/lane-merge-queue"));
    assert!(!resources_list
        .iter()
        .any(|resource| resource["uri"] == "trail://workspace/merge-queue"));
    assert!(resources_list
        .iter()
        .any(|resource| resource["uri"] == "trail://workspace/agent-tasks"));
    assert!(resources_list
        .iter()
        .any(|resource| { resource["uri"] == "trail://workspace/agent-tasks/latest/review" }));
    assert!(resources_list
        .iter()
        .any(|resource| { resource["uri"] == "trail://workspace/agent-tasks/latest/review-data" }));
    assert!(resources_list
        .iter()
        .any(|resource| { resource["uri"] == "trail://workspace/agent-tasks/latest/diagnose" }));
    assert!(resources_list
        .iter()
        .any(|resource| { resource["uri"] == "trail://workspace/agent-tasks/latest/confidence" }));
    assert!(resources_list
        .iter()
        .any(|resource| { resource["uri"] == "trail://workspace/agent-tasks/latest/test-plan" }));
    assert!(resources_list
        .iter()
        .any(|resource| { resource["uri"] == "trail://workspace/agent-tasks/latest/review-map" }));
    assert!(resources_list
        .iter()
        .any(|resource| { resource["uri"] == "trail://workspace/agent-tasks/latest/changes" }));
    assert!(resources_list
        .iter()
        .any(|resource| { resource["uri"] == "trail://workspace/agent-tasks/latest/timeline" }));
    assert!(resources_list
        .iter()
        .any(|resource| { resource["uri"] == "trail://workspace/agent-tasks/latest/focus" }));
    assert!(resources_list
        .iter()
        .any(|resource| { resource["uri"] == "trail://workspace/agent-tasks/latest/handoff" }));
    assert!(resources_list
        .iter()
        .any(|resource| resource["uri"] == "trail://docs/lane-workflows"));

    let resource_templates = trail::mcp::handle_json_rpc(
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
        .any(|template| template["uriTemplate"] == "trail://workspace/lanes/{lane}/status"));
    assert!(template_list.iter().any(|template| {
        template["uriTemplate"] == "trail://workspace/agent-tasks/{selector}/review"
    }));
    assert!(template_list.iter().any(|template| {
        template["uriTemplate"] == "trail://workspace/agent-tasks/{selector}/review-data"
    }));
    assert!(template_list.iter().any(|template| {
        template["uriTemplate"] == "trail://workspace/agent-tasks/{selector}/diagnose"
    }));
    assert!(template_list.iter().any(|template| {
        template["uriTemplate"] == "trail://workspace/agent-tasks/{selector}/confidence"
    }));
    assert!(template_list.iter().any(|template| {
        template["uriTemplate"] == "trail://workspace/agent-tasks/{selector}/test-plan"
    }));
    assert!(template_list.iter().any(|template| {
        template["uriTemplate"] == "trail://workspace/agent-tasks/{selector}/review-map"
    }));
    assert!(template_list.iter().any(|template| {
        template["uriTemplate"] == "trail://workspace/agent-tasks/{selector}/changes"
    }));
    assert!(template_list.iter().any(|template| {
        template["uriTemplate"] == "trail://workspace/agent-tasks/{selector}/timeline"
    }));
    assert!(template_list.iter().any(|template| {
        template["uriTemplate"] == "trail://workspace/agent-tasks/{selector}/files"
    }));
    assert!(template_list.iter().any(|template| {
        template["uriTemplate"] == "trail://workspace/agent-tasks/{selector}/report"
    }));
    assert!(template_list.iter().any(|template| {
        template["uriTemplate"] == "trail://workspace/agent-tasks/{selector}/handoff"
    }));
    assert!(template_list.iter().any(|template| {
        template["uriTemplate"] == "trail://workspace/agent-tasks/{selector}/focus"
    }));
    assert!(template_list
        .iter()
        .any(|template| template["uriTemplate"] == "trail://workspace/lanes/{lane}/review"));
    assert!(template_list
        .iter()
        .any(|template| template["uriTemplate"] == "trail://workspace/lanes/{lane}/contribution"));
    assert!(template_list
        .iter()
        .any(|template| template["uriTemplate"] == "trail://workspace/lanes/{lane}/gates"));
    assert!(template_list
        .iter()
        .any(|template| template["uriTemplate"] == "trail://workspace/lanes/{lane}/readiness"));
    assert!(template_list
        .iter()
        .any(|template| template["uriTemplate"] == "trail://workspace/lanes/{lane}/handoff"));
    assert!(template_list
        .iter()
        .any(|template| template["uriTemplate"] == "trail://workspace/turns/{turn_id}"));

    let status_resource = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/status"
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

    let docs_resource = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 12,
            "method": "resources/read",
            "params": {
                "uri": "trail://docs/lane-workflows"
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

    let missing_resource = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 13,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/missing"
            }
        }),
    )
    .unwrap();
    assert_eq!(missing_resource["error"]["code"], -32002);

    let prompts = trail::mcp::handle_json_rpc(
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
        .any(|prompt| prompt["name"] == "trail.lane_task"));
    assert!(prompt_list
        .iter()
        .any(|prompt| prompt["name"] == "trail.review_lane"));
    assert!(prompt_list
        .iter()
        .any(|prompt| prompt["name"] == "trail.resolve_conflict"));
    assert!(prompt_list
        .iter()
        .any(|prompt| prompt["name"] == "trail.review_agent"));
    assert!(prompt_list
        .iter()
        .any(|prompt| prompt["name"] == "trail.recover_agent"));
    assert!(prompt_list
        .iter()
        .any(|prompt| prompt["name"] == "trail.apply_agent"));

    let lane_prompt = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 15,
            "method": "prompts/get",
            "params": {
                "name": "trail.lane_task",
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
        "Safe Trail lane task workflow"
    );
    let prompt_messages = lane_prompt["result"]["messages"].as_array().unwrap();
    assert!(prompt_messages[0]["content"]["text"]
        .as_str()
        .unwrap()
        .contains("trail.begin_turn"));
    assert!(prompt_messages[0]["content"]["text"]
        .as_str()
        .unwrap()
        .contains("trail.lane_rewind"));
    assert!(prompt_messages[0]["content"]["text"]
        .as_str()
        .unwrap()
        .contains("mcp-lane"));
    assert!(prompt_messages
        .iter()
        .any(|message| message["content"]["type"] == "resource"));

    let review_prompt = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 151,
            "method": "prompts/get",
            "params": {
                "name": "trail.review_lane",
                "arguments": {
                    "lane": "mcp-lane"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        review_prompt["result"]["description"],
        "Trail lane review checklist"
    );
    assert!(review_prompt["result"]["messages"][0]["content"]["text"]
        .as_str()
        .unwrap()
        .contains("trail.lane_review"));

    let agent_review_prompt = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 152,
            "method": "prompts/get",
            "params": {
                "name": "trail.review_agent",
                "arguments": {
                    "selector": "latest"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_review_prompt["result"]["description"],
        "Trail agent task review workflow"
    );
    let agent_review_text = agent_review_prompt["result"]["messages"][0]["content"]["text"]
        .as_str()
        .unwrap();
    assert!(agent_review_text.contains("trail.agent_summary"));
    assert!(agent_review_text.contains("trail.agent_report"));
    assert!(agent_review_text.contains("trail.agent_handoff"));
    assert!(agent_review_text.contains("trail.agent_receipt"));
    assert!(agent_review_text.contains("trail.agent_pr"));
    assert!(agent_review_text.contains("trail.agent_guide"));
    assert!(agent_review_text.contains("trail.agent_board"));
    assert!(agent_review_text.contains("trail.agent_review_flow"));
    assert!(agent_review_text.contains("trail.agent_confidence"));
    assert!(agent_review_text.contains("trail.agent_test_plan"));
    assert!(agent_review_text.contains("trail.agent_focus"));
    assert!(agent_review_text.contains("trail.agent_diagnose"));
    assert!(agent_review_text.contains("trail.agent_changes"));
    assert!(agent_review_text.contains("trail.agent_delta"));
    assert!(agent_review_text.contains("trail.agent_new"));
    assert!(agent_review_text.contains("trail.agent_mark_reviewed"));
    assert!(agent_review_text.contains("trail.agent_archive"));
    assert!(agent_review_text.contains("trail.agent_unarchive"));
    assert!(agent_review_text.contains("trail.agent_change"));
    assert!(agent_review_text.contains("trail.agent_impact"));
    assert!(agent_review_text.contains("trail.agent_review_map"));
    assert!(agent_review_text.contains("trail.agent_tools"));
    assert!(agent_review_text.contains("trail.agent_timeline"));
    assert!(agent_review_text.contains("change cards"));
    assert!(agent_review_text.contains("trail.agent_files"));
    assert!(agent_review_text.contains("trail.agent_file"));

    let agent_recover_prompt = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 153,
            "method": "prompts/get",
            "params": {
                "name": "trail.recover_agent",
                "arguments": {}
            }
        }),
    )
    .unwrap();
    assert!(
        agent_recover_prompt["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap()
            .contains("trail.agent_diagnose")
    );
    assert!(
        agent_recover_prompt["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap()
            .contains("trail.agent_checkpoints")
    );

    let agent_apply_prompt = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 154,
            "method": "prompts/get",
            "params": {
                "name": "trail.apply_agent",
                "arguments": {}
            }
        }),
    )
    .unwrap();
    assert!(
        agent_apply_prompt["result"]["messages"][0]["content"]["text"]
            .as_str()
            .unwrap()
            .contains("trail.agent_apply")
    );

    let missing_prompt_argument = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 16,
            "method": "prompts/get",
            "params": {
                "name": "trail.resolve_conflict",
                "arguments": {}
            }
        }),
    )
    .unwrap();
    assert_eq!(missing_prompt_argument["error"]["code"], -32602);

    let branch_completion = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 18,
            "method": "completion/complete",
            "params": {
                "ref": {
                    "type": "ref/prompt",
                    "name": "trail.lane_task"
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

    let agent_selector_completion = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 181,
            "method": "completion/complete",
            "params": {
                "ref": {
                    "type": "ref/prompt",
                    "name": "trail.review_agent"
                },
                "argument": {
                    "name": "selector",
                    "value": "lat"
                }
            }
        }),
    )
    .unwrap();
    assert!(agent_selector_completion["result"]["completion"]["values"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value.as_str() == Some("latest")));

    let missing_completion_prompt = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 19,
            "method": "completion/complete",
            "params": {
                "ref": {
                    "type": "ref/prompt",
                    "name": "trail.missing"
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

    let list = trail::mcp::handle_json_rpc(
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
    assert!(tools.iter().any(|tool| tool["name"] == "trail.begin_turn"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.lane_spawn"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.lane_list"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.lane_review"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.lane_contribution"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.gate_history"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.lane_readiness"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.lane_refresh_preview"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.lane_handoff"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.guardrail_check"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_status"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_inbox"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_board"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_review_flow"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_review_data"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_next"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_guide"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_ask"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_view"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_brief"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_summary"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_validate"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_test_plan"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_diagnose"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_report"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_handoff"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_receipt"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_pr"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_story"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_tools"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_impact"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_review_map"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_risk"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_confidence"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_ready"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_workdir"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_changes"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_delta"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_new"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_mark_reviewed"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_mark_file_reviewed"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_archive"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_unarchive"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_change"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_timeline"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_files"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_file"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_checkpoints"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_why"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_turn"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_compare"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_test"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_eval"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_diff"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_review"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_focus"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_apply"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.agent_rewind"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.agent_undo"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.lane_remove"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.lane_rewind"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.apply_patch"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.run_test"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.read_file"));
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "trail.sync_workdir"));
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
    assert!(!tool_annotation("trail.status", "readOnlyHint"));
    assert!(!tool_annotation("trail.status", "destructiveHint"));
    assert!(tool_annotation("trail.status", "idempotentHint"));
    assert!(!tool_annotation("trail.apply_patch", "readOnlyHint"));
    assert!(tool_annotation("trail.apply_patch", "destructiveHint"));
    assert!(tool_annotation("trail.lane_rewind", "destructiveHint"));
    assert!(tool_annotation(
        "trail.lane_merge_queue_run",
        "destructiveHint"
    ));
    assert!(tool_annotation("trail.run_test", "openWorldHint"));
    assert!(tool_annotation("trail.guardrail_check", "readOnlyHint"));
    assert!(tool_annotation("trail.lane_review", "readOnlyHint"));
    assert!(tool_annotation("trail.gate_history", "readOnlyHint"));
    assert!(tool_annotation(
        "trail.lane_refresh_preview",
        "readOnlyHint"
    ));
    assert!(tool_annotation("trail.agent_next", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_guide", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_inbox", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_board", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_review_flow", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_review_data", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_brief", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_summary", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_validate", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_test_plan", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_diagnose", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_report", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_handoff", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_receipt", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_pr", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_story", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_tools", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_impact", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_review_map", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_risk", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_confidence", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_ready", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_workdir", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_changes", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_delta", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_new", "readOnlyHint"));
    assert!(!tool_annotation(
        "trail.agent_mark_reviewed",
        "readOnlyHint"
    ));
    assert!(!tool_annotation(
        "trail.agent_mark_reviewed",
        "destructiveHint"
    ));
    assert!(!tool_annotation(
        "trail.agent_mark_file_reviewed",
        "readOnlyHint"
    ));
    assert!(!tool_annotation(
        "trail.agent_mark_file_reviewed",
        "destructiveHint"
    ));
    assert!(!tool_annotation("trail.agent_archive", "readOnlyHint"));
    assert!(!tool_annotation("trail.agent_archive", "destructiveHint"));
    assert!(!tool_annotation("trail.agent_unarchive", "readOnlyHint"));
    assert!(!tool_annotation("trail.agent_unarchive", "destructiveHint"));
    assert!(tool_annotation("trail.agent_change", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_timeline", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_files", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_file", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_checkpoints", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_why", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_turn", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_compare", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_test", "openWorldHint"));
    assert!(tool_annotation("trail.agent_eval", "openWorldHint"));
    assert!(tool_annotation("trail.agent_diff", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_focus", "readOnlyHint"));
    assert!(tool_annotation("trail.agent_apply", "destructiveHint"));
    assert!(tool_annotation("trail.agent_rewind", "destructiveHint"));
    assert!(tool_annotation("trail.agent_undo", "destructiveHint"));

    let workdir_spawn = db
        .spawn_lane(
            "agent-mcp-workdir",
            Some("main"),
            true,
            Some("claude-code".to_string()),
            None,
        )
        .unwrap();
    let agent_workdir = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 243,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_workdir",
                "arguments": {
                    "selector": "agent-mcp-workdir"
                }
            }
        }),
    )
    .unwrap();
    let structured_workdir = agent_workdir["result"]["structuredContent"]["workdir"]
        .as_str()
        .unwrap();
    assert_eq!(Some(structured_workdir), workdir_spawn.workdir.as_deref());
    assert!(agent_workdir["result"]["structuredContent"]["cd_command"]
        .as_str()
        .unwrap()
        .starts_with("cd "));
    let agent_ask_workdir = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 244,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp-workdir",
                    "question": "where is the workdir"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_workdir["result"]["structuredContent"]["intent"],
        "workdir"
    );
    assert_eq!(
        agent_ask_workdir["result"]["structuredContent"]["tool"],
        "trail.agent_workdir"
    );
    assert_eq!(
        agent_ask_workdir["result"]["structuredContent"]["report"]["workdir"],
        structured_workdir
    );
    let agent_test = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 248,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_test",
                "arguments": {
                    "selector": "agent-mcp-workdir",
                    "command": ["sh", "-c", "pwd"],
                    "suite": "mcp-smoke"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_test["result"]["structuredContent"]["workdir"]
            .as_str()
            .unwrap(),
        structured_workdir
    );
    assert_eq!(agent_test["result"]["structuredContent"]["kind"], "test");
    assert_eq!(agent_test["result"]["structuredContent"]["success"], true);
    let agent_eval = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 249,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_eval",
                "arguments": {
                    "selector": "agent-mcp-workdir",
                    "command": ["sh", "-c", "exit 0"],
                    "suite": "mcp-quality",
                    "score": 1.0,
                    "threshold": 0.5
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_eval["result"]["structuredContent"]["workdir"]
            .as_str()
            .unwrap(),
        structured_workdir
    );
    assert_eq!(agent_eval["result"]["structuredContent"]["kind"], "eval");
    assert_eq!(agent_eval["result"]["structuredContent"]["success"], true);

    db.spawn_lane(
        "agent-mcp",
        Some("main"),
        false,
        Some("claude-code".to_string()),
        None,
    )
    .unwrap();
    let agent_patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "agent mcp readme",
        "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nfrom agent mcp\n"}
        ]
    }))
    .unwrap();
    let agent_turn = db
        .begin_lane_turn(
            "agent-mcp",
            None,
            Some("agent mcp transcript".to_string()),
            None,
        )
        .unwrap()
        .turn
        .turn_id;
    db.add_lane_turn_message(&agent_turn, "user", "edit README from MCP agent")
        .unwrap();
    let agent_operation = db
        .apply_lane_turn_patch(&agent_turn, agent_patch)
        .unwrap()
        .operation;
    db.add_lane_turn_message(&agent_turn, "assistant", "updated README")
        .unwrap();
    db.end_lane_turn(&agent_turn, "completed").unwrap();
    let agent_next = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 24,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_next",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_next["result"]["structuredContent"]["primary"]["command"],
        "trail agent new agent-mcp"
    );
    let agent_guide = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 278,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_guide",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_guide["result"]["structuredContent"]["primary"]["command"],
        "trail agent new agent-mcp"
    );
    assert!(agent_guide["result"]["structuredContent"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|step| step["command"]
            .as_str()
            .unwrap()
            .contains("trail agent ask --selector agent-mcp")));
    assert!(agent_guide["result"]["structuredContent"]["concepts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|concept| concept["name"] == "Agent task"));
    let agent_ask_next = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 277,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "what should I do next"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_next["result"]["structuredContent"]["intent"],
        "next"
    );
    assert_eq!(
        agent_ask_next["result"]["structuredContent"]["tool"],
        "trail.agent_next"
    );
    assert_eq!(
        agent_ask_next["result"]["structuredContent"]["report"]["primary"]["command"],
        agent_next["result"]["structuredContent"]["primary"]["command"]
    );
    assert_eq!(
        agent_ask_next["result"]["structuredContent"]["read_only"],
        true
    );
    let agent_ask_guide = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 279,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "help me use Trail"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_guide["result"]["structuredContent"]["intent"],
        "guide"
    );
    assert_eq!(
        agent_ask_guide["result"]["structuredContent"]["tool"],
        "trail.agent_guide"
    );
    assert!(
        agent_ask_guide["result"]["structuredContent"]["report"]["concepts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|concept| concept["name"] == "Changes")
    );
    let agent_ask_tools = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 282,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "what tools were used"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_tools["result"]["structuredContent"]["intent"],
        "tools"
    );
    assert_eq!(
        agent_ask_tools["result"]["structuredContent"]["tool"],
        "trail.agent_tools"
    );
    assert_eq!(
        agent_ask_tools["result"]["structuredContent"]["report"]["task"]["lane"],
        "agent-mcp"
    );
    assert_eq!(
        agent_ask_tools["result"]["structuredContent"]["report"]["total_tool_events"],
        0
    );
    let agent_ask_transcript = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 286,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "show transcript"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_transcript["result"]["structuredContent"]["intent"],
        "view"
    );
    assert_eq!(
        agent_ask_transcript["result"]["structuredContent"]["tool"],
        "trail.agent_view"
    );
    assert_eq!(
        agent_ask_transcript["result"]["structuredContent"]["report"]["transcript"]["turns"][0]
            ["messages"][0]["body"],
        "edit README from MCP agent"
    );
    let agent_inbox = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 247,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_inbox",
                "arguments": {}
            }
        }),
    )
    .unwrap();
    assert_eq!(agent_inbox["result"]["structuredContent"]["total"], 2);
    assert!(agent_inbox["result"]["structuredContent"]["groups"]
        .as_array()
        .unwrap()
        .iter()
        .any(|group| group["key"] == "ready"));
    assert_eq!(
        agent_inbox["result"]["structuredContent"]["items"][0]["attention"],
        "unreviewed"
    );
    assert_eq!(
        agent_inbox["result"]["structuredContent"]["items"][0]["review_first"]["path"],
        "README.md"
    );
    assert!(
        agent_inbox["result"]["structuredContent"]["items"][0]["next"]["command"]
            .as_str()
            .unwrap()
            .contains("agent new")
    );
    let agent_board = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2471,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_board",
                "arguments": {}
            }
        }),
    )
    .unwrap();
    assert_eq!(agent_board["result"]["structuredContent"]["total"], 2);
    assert_eq!(
        agent_board["result"]["structuredContent"]["columns"][0]["key"],
        "needs_review"
    );
    assert!(
        agent_board["result"]["structuredContent"]["columns"][0]["items"][0]["next"]["command"]
            .as_str()
            .unwrap()
            .contains("agent new")
    );
    let agent_ask_board = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2472,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "show agent board"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_board["result"]["structuredContent"]["intent"],
        "board"
    );
    assert_eq!(
        agent_ask_board["result"]["structuredContent"]["tool"],
        "trail.agent_board"
    );
    assert_eq!(
        agent_ask_board["result"]["structuredContent"]["routed_command"],
        "trail agent board"
    );
    let agent_ask_inbox = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 248,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "what needs attention"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_inbox["result"]["structuredContent"]["intent"],
        "inbox"
    );
    assert_eq!(
        agent_ask_inbox["result"]["structuredContent"]["tool"],
        "trail.agent_inbox"
    );
    assert_eq!(
        agent_ask_inbox["result"]["structuredContent"]["routed_command"],
        "trail agent inbox"
    );
    assert_eq!(
        agent_ask_inbox["result"]["structuredContent"]["report"]["total"],
        agent_inbox["result"]["structuredContent"]["total"]
    );
    assert_eq!(
        agent_ask_inbox["result"]["structuredContent"]["report"]["items"][0]["review_first"]
            ["path"],
        "README.md"
    );
    let agent_brief = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 242,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_brief",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert!(agent_brief["result"]["structuredContent"]["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "README.md"));
    assert!(
        agent_brief["result"]["structuredContent"]["risk"]["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason["code"] == "missing_latest_test")
    );
    let agent_report = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 251,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_report",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert!(agent_report["result"]["structuredContent"]["markdown"]
        .as_str()
        .unwrap()
        .contains("# Agent Task Report"));
    assert!(
        agent_report["result"]["structuredContent"]["risk"]["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason["code"] == "missing_latest_test")
    );
    let agent_handoff = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2511,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_handoff",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert!(agent_handoff["result"]["structuredContent"]["markdown"]
        .as_str()
        .unwrap()
        .contains("# Agent Task Handoff"));
    assert!(agent_handoff["result"]["structuredContent"]["markdown"]
        .as_str()
        .unwrap()
        .contains("## Receiver Next Step"));
    assert_eq!(
        agent_handoff["result"]["structuredContent"]["transcript_turns"],
        1
    );
    let agent_receipt = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 252,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_receipt",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert!(agent_receipt["result"]["structuredContent"]["markdown"]
        .as_str()
        .unwrap()
        .contains("# Agent Task Receipt"));
    assert_eq!(
        agent_receipt["result"]["structuredContent"]["validation"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let agent_pr = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 262,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_pr",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert!(agent_pr["result"]["structuredContent"]["title"]
        .as_str()
        .unwrap()
        .starts_with("Apply "));
    assert!(agent_pr["result"]["structuredContent"]["body"]
        .as_str()
        .unwrap()
        .contains("## Trail Review"));
    let agent_ask_receipt = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 310,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "give me a summary to share"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_receipt["result"]["structuredContent"]["intent"],
        "receipt"
    );
    assert_eq!(
        agent_ask_receipt["result"]["structuredContent"]["tool"],
        "trail.agent_receipt"
    );
    assert!(
        agent_ask_receipt["result"]["structuredContent"]["report"]["markdown"]
            .as_str()
            .unwrap()
            .contains("# Agent Task Receipt")
    );
    let agent_ask_handoff = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3101,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "handoff this to another agent"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_handoff["result"]["structuredContent"]["intent"],
        "handoff"
    );
    assert_eq!(
        agent_ask_handoff["result"]["structuredContent"]["tool"],
        "trail.agent_handoff"
    );
    assert!(
        agent_ask_handoff["result"]["structuredContent"]["report"]["markdown"]
            .as_str()
            .unwrap()
            .contains("# Agent Task Handoff")
    );
    let agent_ask_pr = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 311,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "what should I put in the PR"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(agent_ask_pr["result"]["structuredContent"]["intent"], "pr");
    assert_eq!(
        agent_ask_pr["result"]["structuredContent"]["tool"],
        "trail.agent_pr"
    );
    assert!(
        agent_ask_pr["result"]["structuredContent"]["report"]["title"]
            .as_str()
            .unwrap()
            .starts_with("Apply ")
    );
    let agent_ask_merge_pr = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 312,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "can I merge the PR"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_merge_pr["result"]["structuredContent"]["intent"],
        "ready"
    );
    assert_eq!(
        agent_ask_merge_pr["result"]["structuredContent"]["tool"],
        "trail.agent_ready"
    );
    let agent_ask_commit_message = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 313,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "what commit message should I use"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_commit_message["result"]["structuredContent"]["intent"],
        "ready"
    );
    assert_eq!(
        agent_ask_commit_message["result"]["structuredContent"]["tool"],
        "trail.agent_ready"
    );
    assert!(
        agent_ask_commit_message["result"]["structuredContent"]["report"]["default_apply_message"]
            .as_str()
            .unwrap()
            .starts_with("Apply agent task")
    );
    let agent_summary = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 265,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_summary",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert!(agent_summary["result"]["structuredContent"]["summary"]
        .as_str()
        .unwrap()
        .contains("changed file"));
    assert!(
        agent_summary["result"]["structuredContent"]["receipt_markdown"]
            .as_str()
            .unwrap()
            .contains("# Agent Task Receipt")
    );
    assert!(agent_summary["result"]["structuredContent"]["pr_body"]
        .as_str()
        .unwrap()
        .contains("## Trail Review"));
    let agent_diagnose = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 267,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_diagnose",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_diagnose["result"]["structuredContent"]["status"],
        "git_blocked"
    );
    assert_eq!(
        agent_diagnose["result"]["structuredContent"]["severity"],
        "high"
    );
    assert!(
        agent_diagnose["result"]["structuredContent"]["likely_issue"]
            .as_str()
            .unwrap()
            .contains("requires a Git working tree")
    );
    assert!(
        agent_diagnose["result"]["structuredContent"]["recovery_options"]
            .as_array()
            .unwrap()
            .iter()
            .any(|suggestion| suggestion["command"]
                .as_str()
                .unwrap()
                .contains("turn-diff"))
    );
    let agent_review = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 254,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_review",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_review["result"]["structuredContent"]["priorities"][0]["change"]["path"],
        "README.md"
    );
    assert!(
        agent_review["result"]["structuredContent"]["priorities"][0]["why_command"]
            .as_str()
            .unwrap()
            .contains("agent why")
    );
    assert!(
        agent_review["result"]["structuredContent"]["next"]["command"]
            .as_str()
            .unwrap()
            .contains("agent validate")
    );
    let agent_dashboard = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 358,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_dashboard",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_dashboard["result"]["structuredContent"]["task"]["lane"],
        "agent-mcp"
    );
    assert_eq!(
        agent_dashboard["result"]["structuredContent"]["focus"]["path"],
        "README.md"
    );
    assert_eq!(
        agent_dashboard["result"]["structuredContent"]["validation"]["status"],
        "missing_test"
    );
    let agent_review_flow = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 360,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_review_flow",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_review_flow["result"]["structuredContent"]["task"]["lane"],
        "agent-mcp"
    );
    assert_eq!(
        agent_review_flow["result"]["structuredContent"]["review_status"],
        "unreviewed"
    );
    assert_eq!(
        agent_review_flow["result"]["structuredContent"]["steps"][0]["label"],
        "Inspect changes"
    );
    assert_eq!(
        agent_review_flow["result"]["structuredContent"]["steps"][0]["state"],
        "current"
    );
    let agent_ask_review_flow = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 361,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "walk me through review"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_review_flow["result"]["structuredContent"]["intent"],
        "review_flow"
    );
    assert_eq!(
        agent_ask_review_flow["result"]["structuredContent"]["tool"],
        "trail.agent_review_flow"
    );
    assert_eq!(
        agent_ask_review_flow["result"]["structuredContent"]["report"]["steps"][0]["label"],
        "Inspect changes"
    );
    let agent_ask_dashboard = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 359,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "show dashboard"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_dashboard["result"]["structuredContent"]["intent"],
        "dashboard"
    );
    assert_eq!(
        agent_ask_dashboard["result"]["structuredContent"]["tool"],
        "trail.agent_dashboard"
    );
    assert_eq!(
        agent_ask_dashboard["result"]["structuredContent"]["report"]["focus"]["path"],
        "README.md"
    );
    let agent_focus = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 258,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_focus",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_focus["result"]["structuredContent"]["path"],
        "README.md"
    );
    assert_eq!(
        agent_focus["result"]["structuredContent"]["diff"]["file_filter"],
        "README.md"
    );
    assert!(agent_focus["result"]["structuredContent"]["open_path"].is_null());
    assert!(agent_focus["result"]["structuredContent"]["open_command"].is_null());
    let agent_ask_focus_file = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 292,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "what file should I review first"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_focus_file["result"]["structuredContent"]["intent"],
        "focus"
    );
    assert_eq!(
        agent_ask_focus_file["result"]["structuredContent"]["tool"],
        "trail.agent_focus"
    );
    assert_eq!(
        agent_ask_focus_file["result"]["structuredContent"]["report"]["path"],
        "README.md"
    );
    let agent_ask_open_file = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 322,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "what file should I open"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_open_file["result"]["structuredContent"]["intent"],
        "focus"
    );
    assert_eq!(
        agent_ask_open_file["result"]["structuredContent"]["tool"],
        "trail.agent_focus"
    );
    assert_eq!(
        agent_ask_open_file["result"]["structuredContent"]["report"]["path"],
        "README.md"
    );
    assert_eq!(
        agent_ask_open_file["result"]["structuredContent"]["report"]["open_path"],
        agent_focus["result"]["structuredContent"]["open_path"]
    );
    assert_eq!(
        agent_ask_open_file["result"]["structuredContent"]["report"]["open_command"],
        agent_focus["result"]["structuredContent"]["open_command"]
    );
    let agent_ask_look_first = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 293,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "where should I look first"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_look_first["result"]["structuredContent"]["intent"],
        "focus"
    );
    assert_eq!(
        agent_ask_look_first["result"]["structuredContent"]["tool"],
        "trail.agent_focus"
    );
    assert_eq!(
        agent_ask_look_first["result"]["structuredContent"]["report"]["path"],
        "README.md"
    );
    let agent_review_resource = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 255,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/agent-tasks/agent-mcp/review"
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_review_resource["result"]["contents"][0]["mimeType"],
        "application/json"
    );
    let agent_review_resource_text = agent_review_resource["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let agent_review_resource_json: serde_json::Value =
        serde_json::from_str(agent_review_resource_text).unwrap();
    assert_eq!(
        agent_review_resource_json["priorities"][0]["change"]["path"],
        "README.md"
    );
    let agent_focus_resource = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 259,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/agent-tasks/agent-mcp/focus"
            }
        }),
    )
    .unwrap();
    let agent_focus_resource_text = agent_focus_resource["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let agent_focus_resource_json: serde_json::Value =
        serde_json::from_str(agent_focus_resource_text).unwrap();
    assert_eq!(agent_focus_resource_json["path"], "README.md");
    assert!(agent_focus_resource_json["open_path"].is_null());
    let agent_confidence_resource = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2591,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/agent-tasks/agent-mcp/confidence"
            }
        }),
    )
    .unwrap();
    let agent_confidence_resource_text = agent_confidence_resource["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let agent_confidence_resource_json: serde_json::Value =
        serde_json::from_str(agent_confidence_resource_text).unwrap();
    assert_eq!(agent_confidence_resource_json["verdict"], "review");
    assert_eq!(
        agent_confidence_resource_json["ready"]["status"],
        "git_blocked"
    );
    let agent_test_plan_resource = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 25911,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/agent-tasks/agent-mcp/test-plan"
            }
        }),
    )
    .unwrap();
    let agent_test_plan_resource_text = agent_test_plan_resource["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let agent_test_plan_resource_json: serde_json::Value =
        serde_json::from_str(agent_test_plan_resource_text).unwrap();
    assert_eq!(agent_test_plan_resource_json["status"], "needs_test");
    assert_eq!(
        agent_test_plan_resource_json["validation"]["status"],
        "missing_test"
    );
    let agent_review_map_resource = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2592,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/agent-tasks/agent-mcp/review-map"
            }
        }),
    )
    .unwrap();
    let agent_review_map_resource_text = agent_review_map_resource["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let agent_review_map_resource_json: serde_json::Value =
        serde_json::from_str(agent_review_map_resource_text).unwrap();
    assert_eq!(agent_review_map_resource_json["areas"][0]["key"], "docs");
    assert_eq!(
        agent_review_map_resource_json["areas"][0]["files"][0]["path"],
        "README.md"
    );
    let agent_files_resource = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 256,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/agent-tasks/agent-mcp/files"
            }
        }),
    )
    .unwrap();
    let agent_files_resource_text = agent_files_resource["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let agent_files_resource_json: serde_json::Value =
        serde_json::from_str(agent_files_resource_text).unwrap();
    assert_eq!(
        agent_files_resource_json["files"][0]["change"]["path"],
        "README.md"
    );
    let agent_report_resource = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 257,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/agent-tasks/agent-mcp/report"
            }
        }),
    )
    .unwrap();
    let agent_report_resource_text = agent_report_resource["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let agent_report_resource_json: serde_json::Value =
        serde_json::from_str(agent_report_resource_text).unwrap();
    assert!(agent_report_resource_json["markdown"]
        .as_str()
        .unwrap()
        .contains("# Agent Task Report"));
    let agent_handoff_resource = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2571,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/agent-tasks/agent-mcp/handoff"
            }
        }),
    )
    .unwrap();
    let agent_handoff_resource_text = agent_handoff_resource["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let agent_handoff_resource_json: serde_json::Value =
        serde_json::from_str(agent_handoff_resource_text).unwrap();
    assert!(agent_handoff_resource_json["markdown"]
        .as_str()
        .unwrap()
        .contains("# Agent Task Handoff"));
    let agent_receipt_resource = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 263,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/agent-tasks/agent-mcp/receipt"
            }
        }),
    )
    .unwrap();
    let agent_receipt_resource_text = agent_receipt_resource["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let agent_receipt_resource_json: serde_json::Value =
        serde_json::from_str(agent_receipt_resource_text).unwrap();
    assert!(agent_receipt_resource_json["markdown"]
        .as_str()
        .unwrap()
        .contains("# Agent Task Receipt"));
    let agent_pr_resource = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 264,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/agent-tasks/agent-mcp/pr"
            }
        }),
    )
    .unwrap();
    let agent_pr_resource_text = agent_pr_resource["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let agent_pr_resource_json: serde_json::Value =
        serde_json::from_str(agent_pr_resource_text).unwrap();
    assert!(agent_pr_resource_json["body"]
        .as_str()
        .unwrap()
        .contains("## Summary"));
    let agent_summary_resource = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 266,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/agent-tasks/agent-mcp/summary"
            }
        }),
    )
    .unwrap();
    let agent_summary_resource_text = agent_summary_resource["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let agent_summary_resource_json: serde_json::Value =
        serde_json::from_str(agent_summary_resource_text).unwrap();
    assert!(agent_summary_resource_json["pr_title"]
        .as_str()
        .unwrap()
        .starts_with("Apply "));
    let agent_diagnose_resource = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 268,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/agent-tasks/agent-mcp/diagnose"
            }
        }),
    )
    .unwrap();
    let agent_diagnose_resource_text = agent_diagnose_resource["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let agent_diagnose_resource_json: serde_json::Value =
        serde_json::from_str(agent_diagnose_resource_text).unwrap();
    assert_eq!(agent_diagnose_resource_json["status"], "git_blocked");
    assert!(agent_diagnose_resource_json["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().unwrap().contains("Git preflight failed")));
    let agent_story = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 244,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_story",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert!(agent_story["result"]["structuredContent"]["summary"]
        .as_str()
        .unwrap()
        .contains("`README.md`"));
    assert!(agent_story["result"]["structuredContent"]["changed_files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "README.md"));
    let agent_risk = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 245,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_risk",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert!(agent_risk["result"]["structuredContent"]["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .any(|reason| reason["code"] == "missing_latest_test"));
    let agent_impact = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2447,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_impact",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_impact["result"]["structuredContent"]["task"]["lane"],
        "agent-mcp"
    );
    assert_eq!(
        agent_impact["result"]["structuredContent"]["areas"][0]["key"],
        "docs"
    );
    assert_eq!(
        agent_impact["result"]["structuredContent"]["areas"][0]["changed_paths"][0]["path"],
        "README.md"
    );
    let agent_ask_impact = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 24471,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "what is the blast radius"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_impact["result"]["structuredContent"]["intent"],
        "impact"
    );
    assert_eq!(
        agent_ask_impact["result"]["structuredContent"]["tool"],
        "trail.agent_impact"
    );
    assert_eq!(
        agent_ask_impact["result"]["structuredContent"]["report"]["areas"][0]["key"],
        "docs"
    );
    let agent_review_map = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 24472,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_review_map",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_review_map["result"]["structuredContent"]["task"]["lane"],
        "agent-mcp"
    );
    assert_eq!(
        agent_review_map["result"]["structuredContent"]["areas"][0]["key"],
        "docs"
    );
    assert_eq!(
        agent_review_map["result"]["structuredContent"]["areas"][0]["files"][0]["path"],
        "README.md"
    );
    let agent_ask_review_map = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 24473,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "show review map"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_review_map["result"]["structuredContent"]["intent"],
        "review_map"
    );
    assert_eq!(
        agent_ask_review_map["result"]["structuredContent"]["tool"],
        "trail.agent_review_map"
    );
    assert_eq!(
        agent_ask_review_map["result"]["structuredContent"]["report"]["areas"][0]["files"][0]
            ["path"],
        "README.md"
    );
    let agent_tools = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2448,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_tools",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_tools["result"]["structuredContent"]["task"]["lane"],
        "agent-mcp"
    );
    assert_eq!(
        agent_tools["result"]["structuredContent"]["total_tool_events"],
        0
    );
    assert!(agent_tools["result"]["structuredContent"]["tools"]
        .as_array()
        .unwrap()
        .is_empty());
    let agent_confidence = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2451,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_confidence",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_confidence["result"]["structuredContent"]["task"]["lane"],
        "agent-mcp"
    );
    assert_eq!(
        agent_confidence["result"]["structuredContent"]["verdict"],
        "review"
    );
    assert_eq!(
        agent_confidence["result"]["structuredContent"]["ready"]["status"],
        "git_blocked"
    );
    assert!(agent_confidence["result"]["structuredContent"]["factors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|factor| factor["name"] == "apply_preflight" && factor["state"] == "block"));
    let agent_ask_confidence = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2452,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "final check, am I good?"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_confidence["result"]["structuredContent"]["intent"],
        "confidence"
    );
    assert_eq!(
        agent_ask_confidence["result"]["structuredContent"]["tool"],
        "trail.agent_confidence"
    );
    assert_eq!(
        agent_ask_confidence["result"]["structuredContent"]["report"]["verdict"],
        "review"
    );
    let agent_ask_red_flags = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 317,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "any red flags"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_red_flags["result"]["structuredContent"]["intent"],
        "risk"
    );
    assert_eq!(
        agent_ask_red_flags["result"]["structuredContent"]["tool"],
        "trail.agent_risk"
    );
    assert_eq!(
        agent_ask_red_flags["result"]["structuredContent"]["report"]["level"],
        agent_risk["result"]["structuredContent"]["level"]
    );
    let agent_ready = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 246,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ready",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(agent_ready["result"]["structuredContent"]["ready"], false);
    assert_eq!(
        agent_ready["result"]["structuredContent"]["status"],
        "git_blocked"
    );
    assert!(agent_ready["result"]["structuredContent"]["apply_error"]
        .as_str()
        .unwrap()
        .contains("requires a Git working tree"));
    let agent_ask_ready = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 278,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "is it safe to land"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_ready["result"]["structuredContent"]["intent"],
        "ready"
    );
    assert_eq!(
        agent_ask_ready["result"]["structuredContent"]["tool"],
        "trail.agent_ready"
    );
    assert_eq!(
        agent_ask_ready["result"]["structuredContent"]["report"]["status"],
        agent_ready["result"]["structuredContent"]["status"]
    );
    let agent_ask_why_apply = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 294,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "why can't I apply"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_why_apply["result"]["structuredContent"]["intent"],
        "ready"
    );
    assert_eq!(
        agent_ask_why_apply["result"]["structuredContent"]["tool"],
        "trail.agent_ready"
    );
    assert_eq!(
        agent_ask_why_apply["result"]["structuredContent"]["report"]["status"],
        agent_ready["result"]["structuredContent"]["status"]
    );
    let agent_ask_blocking = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 295,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "what is blocking this task"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_blocking["result"]["structuredContent"]["intent"],
        "diagnose"
    );
    assert_eq!(
        agent_ask_blocking["result"]["structuredContent"]["tool"],
        "trail.agent_diagnose"
    );
    assert_eq!(
        agent_ask_blocking["result"]["structuredContent"]["report"]["status"],
        agent_diagnose["result"]["structuredContent"]["status"]
    );
    let agent_ask_failed = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 296,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "why did it fail"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_failed["result"]["structuredContent"]["intent"],
        "diagnose"
    );
    assert_eq!(
        agent_ask_failed["result"]["structuredContent"]["tool"],
        "trail.agent_diagnose"
    );
    assert_eq!(
        agent_ask_failed["result"]["structuredContent"]["report"]["status"],
        agent_diagnose["result"]["structuredContent"]["status"]
    );
    let agent_ask_wrong = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 297,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "what went wrong"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_wrong["result"]["structuredContent"]["intent"],
        "diagnose"
    );
    assert_eq!(
        agent_ask_wrong["result"]["structuredContent"]["tool"],
        "trail.agent_diagnose"
    );
    let agent_validate = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 279,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_validate",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_validate["result"]["structuredContent"]["status"],
        "missing_test"
    );
    assert!(
        agent_validate["result"]["structuredContent"]["next"]["command"]
            .as_str()
            .unwrap()
            .contains("agent test")
    );
    let agent_test_plan = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2791,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_test_plan",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_test_plan["result"]["structuredContent"]["status"],
        "needs_test"
    );
    assert!(agent_test_plan["result"]["structuredContent"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|step| step["kind"] == "test"
            && step["required"] == true
            && step["command"].as_str().unwrap().contains("agent test")));
    let agent_ask_validate = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 280,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "what tests should I run"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_validate["result"]["structuredContent"]["intent"],
        "test_plan"
    );
    assert_eq!(
        agent_ask_validate["result"]["structuredContent"]["tool"],
        "trail.agent_test_plan"
    );
    assert_eq!(
        agent_ask_validate["result"]["structuredContent"]["report"]["steps"],
        agent_test_plan["result"]["structuredContent"]["steps"]
    );
    let agent_ask_test_this = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 314,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "how should I test this"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_test_this["result"]["structuredContent"]["intent"],
        "test_plan"
    );
    assert_eq!(
        agent_ask_test_this["result"]["structuredContent"]["tool"],
        "trail.agent_test_plan"
    );
    assert_eq!(
        agent_ask_test_this["result"]["structuredContent"]["report"]["steps"],
        agent_test_plan["result"]["structuredContent"]["steps"]
    );
    let agent_ask_tests_pass = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 315,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "did tests pass"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_tests_pass["result"]["structuredContent"]["intent"],
        "validate"
    );
    assert_eq!(
        agent_ask_tests_pass["result"]["structuredContent"]["tool"],
        "trail.agent_validate"
    );
    assert_eq!(
        agent_ask_tests_pass["result"]["structuredContent"]["report"]["status"],
        agent_validate["result"]["structuredContent"]["status"]
    );
    let agent_changes = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 21,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_changes",
                "arguments": {
                    "selector": "agent-mcp",
                    "by_operation": true
                }
            }
        }),
    )
    .unwrap();
    assert!(
        agent_changes["result"]["structuredContent"]["groups"][0]["changed_paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path["path"] == "README.md")
    );
    assert_eq!(
        agent_changes["result"]["structuredContent"]["cards"][0]["key"],
        "docs"
    );
    assert!(
        agent_changes["result"]["structuredContent"]["next"]["command"]
            .as_str()
            .unwrap()
            .contains("agent change")
    );
    assert!(
        agent_changes["result"]["structuredContent"]["cards"][0]["review_command"]
            .as_str()
            .unwrap()
            .contains("agent change")
    );
    assert!(
        agent_changes["result"]["structuredContent"]["cards"][0]["focus_command"]
            .as_str()
            .unwrap()
            .contains("agent focus")
    );
    let agent_file_changes = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 319,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_changes",
                "arguments": {
                    "selector": "agent-mcp",
                    "by-file": true
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_file_changes["result"]["structuredContent"]["grouping"],
        "file"
    );
    assert_eq!(
        agent_file_changes["result"]["structuredContent"]["cards"][0]["key"],
        "README.md"
    );
    assert!(
        agent_file_changes["result"]["structuredContent"]["cards"][0]["review_command"]
            .as_str()
            .unwrap()
            .contains("agent file")
    );
    let agent_review_data = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 322,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_review_data",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_review_data["result"]["structuredContent"]["total_files"],
        1
    );
    assert_eq!(
        agent_review_data["result"]["structuredContent"]["needs_review_files"],
        1
    );
    assert_eq!(
        agent_review_data["result"]["structuredContent"]["changes_by_file"]["grouping"],
        "file"
    );
    assert_eq!(
        agent_review_data["result"]["structuredContent"]["focus"]["path"],
        "README.md"
    );
    let mcp_review_actions = agent_review_data["result"]["structuredContent"]["actions"]
        .as_array()
        .unwrap();
    assert!(mcp_review_actions.iter().any(|action| {
        action["id"] == "mark_focus_file_reviewed"
            && action["enabled"] == true
            && action["mcp_tool"] == "trail.agent_mark_file_reviewed"
            && action["mcp_arguments"]["selector"] == "agent-mcp"
            && action["mcp_arguments"]["path"] == "README.md"
    }));
    assert!(mcp_review_actions.iter().any(|action| {
        action["id"] == "apply_task"
            && action["safety"] == "destructive"
            && action["requires_confirmation"] == true
            && action["mcp_arguments"]["selector"] == "agent-mcp"
            && action["mcp_arguments"]["dry-run"] == false
    }));
    let agent_ask_changes_by_file = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 320,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "show changes by file"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_changes_by_file["result"]["structuredContent"]["intent"],
        "changes"
    );
    assert_eq!(
        agent_ask_changes_by_file["result"]["structuredContent"]["tool"],
        "trail.agent_changes"
    );
    assert_eq!(
        agent_ask_changes_by_file["result"]["structuredContent"]["report"]["grouping"],
        "file"
    );
    let agent_ask_review_data = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 323,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "show editor panel data"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_review_data["result"]["structuredContent"]["intent"],
        "review_data"
    );
    assert_eq!(
        agent_ask_review_data["result"]["structuredContent"]["tool"],
        "trail.agent_review_data"
    );
    assert_eq!(
        agent_ask_review_data["result"]["structuredContent"]["report"]["total_files"],
        1
    );
    assert!(
        agent_ask_review_data["result"]["structuredContent"]["report"]["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["id"] == "apply_dry_run")
    );
    let agent_ask_actions = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 324,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "show actions"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_actions["result"]["structuredContent"]["intent"],
        "actions"
    );
    assert_eq!(
        agent_ask_actions["result"]["structuredContent"]["tool"],
        "trail.agent_review_data"
    );
    assert!(
        agent_ask_actions["result"]["structuredContent"]["report"]["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["id"] == "mark_focus_file_reviewed")
    );
    let agent_ask_risky_files = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 321,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "which files are risky"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_risky_files["result"]["structuredContent"]["intent"],
        "changes"
    );
    assert_eq!(
        agent_ask_risky_files["result"]["structuredContent"]["tool"],
        "trail.agent_changes"
    );
    assert_eq!(
        agent_ask_risky_files["result"]["structuredContent"]["report"]["grouping"],
        "file"
    );
    let agent_delta = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 273,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_delta",
                "arguments": {
                    "selector": "agent-mcp",
                    "file": "README.md",
                    "patch": true
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(agent_delta["result"]["structuredContent"]["mode"], "turn");
    assert_eq!(
        agent_delta["result"]["structuredContent"]["file_filter"],
        "README.md"
    );
    assert_eq!(agent_delta["result"]["structuredContent"]["matched"], true);
    assert!(
        agent_delta["result"]["structuredContent"]["diff"]["diff"]["files"][0]["patch"]
            .as_str()
            .unwrap()
            .contains("from agent mcp")
    );
    let agent_ask_delta = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 279,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "what just changed"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_delta["result"]["structuredContent"]["intent"],
        "delta"
    );
    assert_eq!(
        agent_ask_delta["result"]["structuredContent"]["tool"],
        "trail.agent_delta"
    );
    assert_eq!(
        agent_ask_delta["result"]["structuredContent"]["report"]["mode"],
        "turn"
    );
    let agent_ask_prompt_delta = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 288,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "what changed in the last prompt"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_prompt_delta["result"]["structuredContent"]["intent"],
        "delta"
    );
    assert_eq!(
        agent_ask_prompt_delta["result"]["structuredContent"]["tool"],
        "trail.agent_delta"
    );
    assert_eq!(
        agent_ask_prompt_delta["result"]["structuredContent"]["report"]["changed_paths"][0]["path"],
        "README.md"
    );
    let agent_ask_prompt_file_delta = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 289,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "what changed in README.md in the last prompt"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_prompt_file_delta["result"]["structuredContent"]["intent"],
        "delta"
    );
    assert_eq!(
        agent_ask_prompt_file_delta["result"]["structuredContent"]["tool"],
        "trail.agent_delta"
    );
    assert_eq!(
        agent_ask_prompt_file_delta["result"]["structuredContent"]["report"]["file_filter"],
        "README.md"
    );
    assert_eq!(
        agent_ask_prompt_file_delta["result"]["structuredContent"]["report"]["matched"],
        true
    );
    let agent_ask_file_patch = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 281,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "show patch for README.md"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_file_patch["result"]["structuredContent"]["intent"],
        "file"
    );
    assert_eq!(
        agent_ask_file_patch["result"]["structuredContent"]["tool"],
        "trail.agent_file"
    );
    assert!(
        agent_ask_file_patch["result"]["structuredContent"]["routed_command"]
            .as_str()
            .unwrap()
            .contains("--patch")
    );
    assert!(
        agent_ask_file_patch["result"]["structuredContent"]["report"]["diff"]["diff"]["files"][0]
            ["patch"]
            .as_str()
            .unwrap()
            .contains("from agent mcp")
    );
    let agent_ask_turn_diff = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 282,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "show turn diff for README.md"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_turn_diff["result"]["structuredContent"]["intent"],
        "turn_diff"
    );
    assert_eq!(
        agent_ask_turn_diff["result"]["structuredContent"]["tool"],
        "trail.agent_diff"
    );
    assert!(
        agent_ask_turn_diff["result"]["structuredContent"]["routed_command"]
            .as_str()
            .unwrap()
            .contains("agent turn-diff")
    );
    assert_eq!(
        agent_ask_turn_diff["result"]["structuredContent"]["report"]["target_kind"],
        "turn"
    );
    assert_eq!(
        agent_ask_turn_diff["result"]["structuredContent"]["report"]["file_filter"],
        "README.md"
    );
    assert!(
        agent_ask_turn_diff["result"]["structuredContent"]["report"]["diff"]["files"][0]["patch"]
            .as_str()
            .unwrap()
            .contains("from agent mcp")
    );
    let agent_ask_task_diff = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 316,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "show me the diff"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_task_diff["result"]["structuredContent"]["intent"],
        "diff"
    );
    assert_eq!(
        agent_ask_task_diff["result"]["structuredContent"]["tool"],
        "trail.agent_diff"
    );
    assert!(
        agent_ask_task_diff["result"]["structuredContent"]["routed_command"]
            .as_str()
            .unwrap()
            .contains("agent diff")
    );
    assert_eq!(
        agent_ask_task_diff["result"]["structuredContent"]["report"]["target_kind"],
        "task"
    );
    assert!(
        agent_ask_task_diff["result"]["structuredContent"]["report"]["diff"]["files"][0]["patch"]
            .as_str()
            .unwrap()
            .contains("from agent mcp")
    );
    let agent_ask_review_plan = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 283,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "what should I review"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_review_plan["result"]["structuredContent"]["intent"],
        "review"
    );
    assert_eq!(
        agent_ask_review_plan["result"]["structuredContent"]["tool"],
        "trail.agent_review"
    );
    assert!(
        agent_ask_review_plan["result"]["structuredContent"]["routed_command"]
            .as_str()
            .unwrap()
            .contains("agent review-plan")
    );
    assert_eq!(
        agent_ask_review_plan["result"]["structuredContent"]["report"]["priorities"][0]["change"]
            ["path"],
        "README.md"
    );
    let agent_ask_open_review = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 290,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "open review"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_open_review["result"]["structuredContent"]["intent"],
        "review"
    );
    assert_eq!(
        agent_ask_open_review["result"]["structuredContent"]["tool"],
        "trail.agent_review"
    );
    assert!(
        agent_ask_open_review["result"]["structuredContent"]["routed_command"]
            .as_str()
            .unwrap()
            .contains("agent review-plan")
    );
    let agent_ask_start_review = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 291,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "start review"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_start_review["result"]["structuredContent"]["intent"],
        "review"
    );
    assert_eq!(
        agent_ask_start_review["result"]["structuredContent"]["tool"],
        "trail.agent_review"
    );
    let agent_new = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 274,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_new",
                "arguments": {
                    "selector": "agent-mcp",
                    "file": "README.md",
                    "patch": true
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_new["result"]["structuredContent"]["status"],
        "unreviewed"
    );
    assert_eq!(
        agent_new["result"]["structuredContent"]["file_filter"],
        "README.md"
    );
    assert!(
        agent_new["result"]["structuredContent"]["diff"]["diff"]["files"][0]["patch"]
            .as_str()
            .unwrap()
            .contains("from agent mcp")
    );
    let agent_mark_file_reviewed = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2741,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_mark_file_reviewed",
                "arguments": {
                    "selector": "agent-mcp",
                    "path": "README.md",
                    "note": "mcp file reviewed"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_mark_file_reviewed["result"]["structuredContent"]["marker"]["path"],
        "README.md"
    );
    assert_eq!(
        agent_mark_file_reviewed["result"]["structuredContent"]["marker"]["note"],
        "mcp file reviewed"
    );
    let agent_review_map_after_file = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2742,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_review_map",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_review_map_after_file["result"]["structuredContent"]["areas"][0]["files"][0]["state"],
        "reviewed"
    );
    assert_eq!(
        agent_review_map_after_file["result"]["structuredContent"]["areas"][0]["files"][0]
            ["reviewed"]["note"],
        "mcp file reviewed"
    );
    let agent_mark_reviewed = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 275,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_mark_reviewed",
                "arguments": {
                    "selector": "agent-mcp",
                    "note": "mcp reviewed"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_mark_reviewed["result"]["structuredContent"]["marker"]["note"],
        "mcp reviewed"
    );
    let agent_new_after_review = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 276,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_new",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_new_after_review["result"]["structuredContent"]["status"],
        "up_to_date"
    );
    assert!(
        agent_new_after_review["result"]["structuredContent"]["changed_paths"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let agent_change = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 271,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_change",
                "arguments": {
                    "selector": "agent-mcp",
                    "card": "docs",
                    "patch": true
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_change["result"]["structuredContent"]["card"]["key"],
        "docs"
    );
    assert_eq!(
        agent_change["result"]["structuredContent"]["files"][0]["change"]["path"],
        "README.md"
    );
    assert!(
        agent_change["result"]["structuredContent"]["diffs"][0]["diff"]["files"][0]["patch"]
            .as_str()
            .unwrap()
            .contains("from agent mcp")
    );
    let agent_file = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 272,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_file",
                "arguments": {
                    "selector": "agent-mcp",
                    "path": "README.md",
                    "patch": true
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_file["result"]["structuredContent"]["path"],
        "README.md"
    );
    assert_eq!(agent_file["result"]["structuredContent"]["matched"], true);
    assert_eq!(
        agent_file["result"]["structuredContent"]["change_cards"][0]["key"],
        "docs"
    );
    assert!(
        agent_file["result"]["structuredContent"]["diff"]["diff"]["files"][0]["patch"]
            .as_str()
            .unwrap()
            .contains("from agent mcp")
    );
    let agent_timeline = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 269,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_timeline",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_timeline["result"]["structuredContent"]["mode"],
        "turn"
    );
    assert_eq!(
        agent_timeline["result"]["structuredContent"]["items"][0]["kind"],
        "turn"
    );
    assert!(
        agent_timeline["result"]["structuredContent"]["items"][0]["view_command"]
            .as_str()
            .unwrap()
            .contains("agent turn")
    );
    assert!(
        agent_timeline["result"]["structuredContent"]["items"][0]["diff_command"]
            .as_str()
            .unwrap()
            .contains("--turn 1")
    );
    let agent_changes_resource = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 260,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/agent-tasks/agent-mcp/changes"
            }
        }),
    )
    .unwrap();
    let agent_changes_resource_text = agent_changes_resource["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let agent_changes_resource_json: serde_json::Value =
        serde_json::from_str(agent_changes_resource_text).unwrap();
    assert_eq!(agent_changes_resource_json["cards"][0]["key"], "docs");
    let agent_review_data_resource = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 261,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/agent-tasks/agent-mcp/review-data"
            }
        }),
    )
    .unwrap();
    let agent_review_data_resource_text = agent_review_data_resource["result"]["contents"][0]
        ["text"]
        .as_str()
        .unwrap();
    let agent_review_data_resource_json: serde_json::Value =
        serde_json::from_str(agent_review_data_resource_text).unwrap();
    assert_eq!(agent_review_data_resource_json["total_files"], 1);
    assert_eq!(
        agent_review_data_resource_json["changes_by_file"]["grouping"],
        "file"
    );
    assert!(agent_review_data_resource_json["actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action["id"] == "show_review_map"
            && action["mcp_tool"] == "trail.agent_review_map"
            && action["mcp_arguments"]["selector"] == "agent-mcp"));
    let agent_timeline_resource = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 270,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/agent-tasks/agent-mcp/timeline"
            }
        }),
    )
    .unwrap();
    let agent_timeline_resource_text = agent_timeline_resource["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let agent_timeline_resource_json: serde_json::Value =
        serde_json::from_str(agent_timeline_resource_text).unwrap();
    assert_eq!(agent_timeline_resource_json["mode"], "turn");
    assert_eq!(agent_timeline_resource_json["items"][0]["kind"], "turn");
    let agent_turn_tool = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 261,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_turn",
                "arguments": {
                    "selector": "agent-mcp",
                    "turn": "1",
                    "file": "README.md",
                    "patch": true
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(agent_turn_tool["result"]["structuredContent"]["index"], 1);
    assert_eq!(
        agent_turn_tool["result"]["structuredContent"]["changed_paths"][0]["path"],
        "README.md"
    );
    assert!(
        agent_turn_tool["result"]["structuredContent"]["diff"]["diff"]["files"][0]["patch"]
            .as_str()
            .unwrap()
            .contains("from agent mcp")
    );
    let agent_ask_last_prompt = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 287,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "last prompt"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_last_prompt["result"]["structuredContent"]["intent"],
        "turn"
    );
    assert_eq!(
        agent_ask_last_prompt["result"]["structuredContent"]["tool"],
        "trail.agent_turn"
    );
    assert_eq!(
        agent_ask_last_prompt["result"]["structuredContent"]["report"]["index"],
        1
    );
    assert_eq!(
        agent_ask_last_prompt["result"]["structuredContent"]["report"]["changed_paths"][0]["path"],
        "README.md"
    );
    let agent_files = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 253,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_files",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_files["result"]["structuredContent"]["files"][0]["change"]["path"],
        "README.md"
    );
    assert_eq!(
        agent_files["result"]["structuredContent"]["files"][0]["touched_by"][0]["kind"],
        "turn"
    );
    assert!(
        agent_files["result"]["structuredContent"]["files"][0]["why_command"]
            .as_str()
            .unwrap()
            .contains("agent why")
    );
    let agent_ask_agent_change = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 300,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "what did the agent change"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_agent_change["result"]["structuredContent"]["intent"],
        "files"
    );
    assert_eq!(
        agent_ask_agent_change["result"]["structuredContent"]["tool"],
        "trail.agent_files"
    );
    assert_eq!(
        agent_ask_agent_change["result"]["structuredContent"]["report"]["files"][0]["change"]
            ["path"],
        "README.md"
    );
    let agent_ask_files_touched = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 301,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "what files did it touch"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_files_touched["result"]["structuredContent"]["intent"],
        "files"
    );
    assert_eq!(
        agent_ask_files_touched["result"]["structuredContent"]["tool"],
        "trail.agent_files"
    );
    assert_eq!(
        agent_ask_files_touched["result"]["structuredContent"]["report"]["files"][0]["change"]
            ["path"],
        "README.md"
    );
    let agent_checkpoints = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 252,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_checkpoints",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_checkpoints["result"]["structuredContent"]["entries"][0]["kind"],
        "turn"
    );
    assert_eq!(
        agent_checkpoints["result"]["structuredContent"]["entries"][0]["checkpoint_target"],
        "turn:1"
    );
    assert!(
        agent_checkpoints["result"]["structuredContent"]["entries"][0]["rewind_before_command"]
            .as_str()
            .unwrap()
            .contains("agent rewind")
    );
    let agent_why = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 246,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_why",
                "arguments": {
                    "selector": "agent-mcp",
                    "path": "README.md"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_why["result"]["structuredContent"]["path"],
        "README.md"
    );
    assert_eq!(agent_why["result"]["structuredContent"]["matched"], true);
    assert!(
        agent_why["result"]["structuredContent"]["groups"][0]["changed_paths"]
            .as_array()
            .unwrap()
            .iter()
            .all(|path| path["path"] == "README.md")
    );
    let agent_ask_why = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 280,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "explain README.md"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_why["result"]["structuredContent"]["intent"],
        "why"
    );
    assert_eq!(
        agent_ask_why["result"]["structuredContent"]["tool"],
        "trail.agent_why"
    );
    assert_eq!(
        agent_ask_why["result"]["structuredContent"]["report"]["path"],
        "README.md"
    );
    let agent_ask_prompt_provenance = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 285,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_ask",
                "arguments": {
                    "selector": "agent-mcp",
                    "question": "which prompt changed README.md"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_ask_prompt_provenance["result"]["structuredContent"]["intent"],
        "why"
    );
    assert_eq!(
        agent_ask_prompt_provenance["result"]["structuredContent"]["tool"],
        "trail.agent_why"
    );
    assert_eq!(
        agent_ask_prompt_provenance["result"]["structuredContent"]["report"]["groups"],
        agent_why["result"]["structuredContent"]["groups"]
    );
    let agent_diff = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 22,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_diff",
                "arguments": {
                    "selector": "agent-mcp",
                    "operation": agent_operation.0,
                    "file": "README.md",
                    "patch": true
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_diff["result"]["structuredContent"]["file_filter"],
        "README.md"
    );
    assert_eq!(
        agent_diff["result"]["structuredContent"]["diff"]["files"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        agent_diff["result"]["structuredContent"]["diff"]["files"][0]["patch"]
            .as_str()
            .unwrap()
            .contains("from agent mcp")
    );
    let agent_compare = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 250,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_compare",
                "arguments": {
                    "left": "agent-mcp-workdir",
                    "right": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(
        agent_compare["result"]["structuredContent"]["shared_paths"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert!(
        agent_compare["result"]["structuredContent"]["right_only_paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path["path"] == "README.md")
    );
    assert!(
        agent_compare["result"]["structuredContent"]["recommendation"]["command"]
            .as_str()
            .unwrap()
            .contains("agent review")
            || agent_compare["result"]["structuredContent"]["recommendation"]["command"]
                .as_str()
                .unwrap()
                .contains("agent apply")
            || agent_compare["result"]["structuredContent"]["recommendation"]["command"]
                .as_str()
                .unwrap()
                .contains("agent land")
    );
    let agent_rewind = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 23,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_rewind",
                "arguments": {
                    "selector": "agent-mcp",
                    "to": "before-last-turn"
                }
            }
        }),
    )
    .unwrap();
    assert!(agent_rewind["result"]["structuredContent"]["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "README.md"));

    let agent_patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "agent mcp readme again",
        "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nfrom agent undo\n"}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "agent-mcp", agent_patch).unwrap();
    let agent_undo = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 241,
            "method": "tools/call",
            "params": {
                "name": "trail.agent_undo",
                "arguments": {
                    "selector": "agent-mcp"
                }
            }
        }),
    )
    .unwrap();
    assert!(agent_undo["result"]["structuredContent"]["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "README.md"));

    let spawned = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 20,
            "method": "tools/call",
            "params": {
                "name": "trail.lane_spawn",
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

    let lane_list = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 21,
            "method": "tools/call",
            "params": {
                "name": "trail.lane_list",
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

    let lane_show = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 22,
            "method": "tools/call",
            "params": {
                "name": "trail.lane_show",
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

    let lane_status = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 23,
            "method": "tools/call",
            "params": {
                "name": "trail.lane_status",
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

    let templated_lane_status = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 25,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/lanes/mcp-lifecycle/status"
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

    let review = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 251,
            "method": "tools/call",
            "params": {
                "name": "trail.lane_review",
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

    let templated_review = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 252,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/lanes/mcp-lifecycle/review"
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

    let contribution = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 26,
            "method": "tools/call",
            "params": {
                "name": "trail.lane_contribution",
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

    let templated_contribution = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 27,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/lanes/mcp-lifecycle/contribution"
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

    let readiness = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 29,
            "method": "tools/call",
            "params": {
                "name": "trail.lane_readiness",
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

    let templated_readiness = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 30,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/lanes/mcp-lifecycle/readiness"
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

    let handoff = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 31,
            "method": "tools/call",
            "params": {
                "name": "trail.lane_handoff",
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

    let templated_handoff = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 32,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/lanes/mcp-lifecycle/handoff"
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

    let lane_completion = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 28,
            "method": "completion/complete",
            "params": {
                "ref": {
                    "type": "ref/resource",
                    "uri": "trail://workspace/lanes/{lane}/handoff"
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

    let lane_remove = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 24,
            "method": "tools/call",
            "params": {
                "name": "trail.lane_remove",
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

    let begin = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "trail.begin_turn",
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

    let templated_turn = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 31,
            "method": "resources/read",
            "params": {
                "uri": format!("trail://workspace/turns/{turn_id}")
            }
        }),
    )
    .unwrap();
    let templated_turn_text = templated_turn["result"]["contents"][0]["text"]
        .as_str()
        .unwrap();
    let templated_turn_json: serde_json::Value = serde_json::from_str(templated_turn_text).unwrap();
    assert_eq!(templated_turn_json["turn"]["turn_id"], turn_id);

    let turn_completion = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 32,
            "method": "completion/complete",
            "params": {
                "ref": {
                    "type": "ref/resource",
                    "uri": "trail://workspace/turns/{turn_id}"
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

    let event = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "trail.add_event",
                "arguments": {
                    "turn_id": turn_id.clone(),
                    "event_type": "tool_call",
                    "payload": {
                        "tool": "trail.apply_patch",
                        "status": "planned"
                    }
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(event["result"]["isError"], false);

    let patch = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "trail.apply_patch",
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
    let sync = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "trail.sync_workdir",
                "arguments": {
                    "lane": "mcp-lane",
                    "force": true
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(sync["result"]["isError"], false, "{sync:#}");
    assert_eq!(sync["result"]["structuredContent"]["forced"], true);

    let test = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "trail.run_test",
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

    let show = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 8,
            "method": "tools/call",
            "params": {
                "name": "trail.show_turn",
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

    let active_handoff = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 33,
            "method": "tools/call",
            "params": {
                "name": "trail.lane_handoff",
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

    let mut stdio_db = Trail::open(temp.path()).unwrap();
    let mut output = Vec::new();
    trail::mcp::serve_stdio(
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
fn mcp_status_refreshes_index_while_status_resource_remains_read_only() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), "a1\n").unwrap();
    fs::write(temp.path().join("b.txt"), "b1\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
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

    let status = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "trail.status",
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

    assert_eq!(count_rows("worktree_file_index"), index_before + 1);
    assert_eq!(count_rows("objects"), objects_before);
    assert_eq!(count_rows("prolly_nodes"), prolly_nodes_before);

    let resource_status = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/status"
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

    assert_eq!(count_rows("worktree_file_index"), index_before + 1);
    assert_eq!(count_rows("objects"), objects_before);
    assert_eq!(count_rows("prolly_nodes"), prolly_nodes_before);
}

#[test]
fn changed_path_activation_is_checked_and_linux_macos_only() {
    let evidence = trail::test_support::changed_path_activation_evidence().unwrap();
    assert!(evidence["schema_hard_cutover"].as_bool().unwrap());
    assert!(evidence["producer_inventory_complete"].as_bool().unwrap());
    assert!(evidence["crash_matrix"].as_bool().unwrap());
    assert!(evidence["corruption_matrix"].as_bool().unwrap());
    assert!(evidence["scale_gates"].as_bool().unwrap());
    assert_eq!(
        trail::test_support::changed_path_production_authority_default(),
        cfg!(any(target_os = "linux", target_os = "macos"))
    );
    assert!(!trail::test_support::changed_path_authority_enabled_for("windows").unwrap());
}

#[test]
fn mcp_stdio_reports_parse_errors_and_continues() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let mut output = Vec::new();
    trail::mcp::serve_stdio(
        &mut db,
        std::io::Cursor::new(
            br#"{not-json
{"jsonrpc":"1.0","id":6,"method":"ping","params":{}}
{"jsonrpc":"2.0","id":{"bad":true},"method":"ping","params":{}}
{"jsonrpc":"2.0","id":8,"method":"ping","params":"bad"}
{"jsonrpc":"2.0","method":"ping","params":{}}
{}
{"jsonrpc":"2.0","id":7,"method":"tools/list","params":{}}
"#,
        ),
        &mut output,
    )
    .unwrap();
    let output = String::from_utf8(output).unwrap();
    let responses = output
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 6);
    assert_eq!(responses[0]["id"], serde_json::Value::Null);
    assert_eq!(responses[0]["error"]["code"], -32700);
    assert!(responses[0]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("parse error"));
    assert_eq!(responses[1]["id"], 6);
    assert_eq!(responses[1]["error"]["code"], -32600);
    assert!(responses[1]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("invalid JSON-RPC request"));
    assert_eq!(responses[2]["id"], serde_json::Value::Null);
    assert_eq!(responses[2]["error"]["code"], -32600);
    assert!(responses[2]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("invalid JSON-RPC request"));
    assert_eq!(responses[3]["id"], 8);
    assert_eq!(responses[3]["error"]["code"], -32600);
    assert!(responses[3]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("invalid JSON-RPC request"));
    assert_eq!(responses[4]["id"], serde_json::Value::Null);
    assert_eq!(responses[4]["error"]["code"], -32600);
    assert!(responses[4]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("invalid JSON-RPC request"));
    assert_eq!(responses[5]["id"], 7);
    assert!(responses[5]["result"]["tools"].is_array());
}

#[test]
fn config_api_lists_sets_persists_and_validates_keys() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let entries = db.config_entries();
    assert!(entries
        .iter()
        .any(|entry| entry.key == "workspace.id" && entry.read_only));
    assert_eq!(
        db.config_get("recording.ignore_gitignored").unwrap().value,
        "true"
    );

    let http_list = trail::server::handle_http_request(
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
    assert!(http_entries
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["key"] == "storage.prolly_backend"));
    assert!(http_entries
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["key"] == "storage.slatedb_s3_endpoint"));

    let http_get = trail::server::handle_http_request(
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

    let http_set = trail::server::handle_http_request(
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

    let mcp_tools = trail::mcp::handle_json_rpc(
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
    assert!(tools.iter().any(|tool| tool["name"] == "trail.config_list"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.config_get"));
    assert!(tools.iter().any(|tool| tool["name"] == "trail.config_set"));

    let mcp_get = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "trail.config_get",
                "arguments": { "key": "text.preserve_similarity" }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_get["result"]["isError"], false);
    assert_eq!(mcp_get["result"]["structuredContent"]["value"], "0.55");

    let mcp_set = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "trail.config_set",
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
    let path_set = db
        .config_set("storage.slatedb_path", "/custom/prolly/")
        .unwrap();
    assert_eq!(path_set.new_value, "custom/prolly");
    let endpoint_set = db
        .config_set("storage.slatedb_s3_endpoint", "http://localhost:9001/")
        .unwrap();
    assert_eq!(endpoint_set.new_value, "http://localhost:9001");
    let bucket_set = db
        .config_set("storage.slatedb_s3_bucket", "test-bucket")
        .unwrap();
    assert_eq!(bucket_set.new_value, "test-bucket");
    let allow_http_set = db
        .config_set("storage.slatedb_s3_allow_http", "no")
        .unwrap();
    assert_eq!(allow_http_set.new_value, "false");
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
    let mut reopened = Trail::open(temp.path()).unwrap();
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
    assert_eq!(reopened.config().storage.prolly_backend, "sqlite");
    assert_eq!(reopened.config().storage.slatedb_path, "custom/prolly");
    assert_eq!(
        reopened.config().storage.slatedb_s3_endpoint,
        "http://localhost:9001"
    );
    assert_eq!(reopened.config().storage.slatedb_s3_bucket, "test-bucket");
    assert!(!reopened.config().storage.slatedb_s3_allow_http);

    let err = reopened
        .config_set("recording.ignore_gitignored", "sometimes")
        .unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)));

    let err = reopened
        .config_set("storage.prolly_backend", "slatedb")
        .unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)));

    let err = reopened
        .config_set("storage.slatedb_path", "///")
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

    let mut reopened = Trail::open(temp.path()).unwrap();
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
    let init = Trail::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
        .any(|path| path.path == "README.md" && path.kind == trail::FileChangeKind::Modified));
    assert!(report
        .changed_paths
        .iter()
        .any(|path| path.path == "src/lib.rs" && path.kind == trail::FileChangeKind::Added));
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
            assert_eq!(value.operation.kind, trail::OperationKind::GitImport);
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
    let no_change_mapping = no_change.mapping.as_ref().unwrap();
    assert_eq!(no_change_mapping.direction, "import");
    assert_eq!(no_change_mapping.crab_change, imported_change);
    assert_eq!(no_change_mapping.crab_root, no_change.root_id);
    assert!(no_change_mapping.git_dirty);
    assert!(no_change.changed_paths.is_empty());

    fs::remove_file(temp.path().join("src/lib.rs")).unwrap();
    let deleted = db
        .git_import_update(Some("main"), Some("remove tracked lib".to_string()))
        .unwrap();
    assert!(deleted.operation.is_some());
    assert!(deleted
        .changed_paths
        .iter()
        .any(|path| { path.path == "src/lib.rs" && path.kind == trail::FileChangeKind::Deleted }));
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
        .any(|path| path.path == "scratch.txt" && path.kind == trail::FileChangeKind::Added));
    assert!(!clean_after_delete
        .changed_paths
        .iter()
        .any(|path| path.path == "src/lib.rs"));

    let mappings = db.git_mappings(10).unwrap();
    assert_eq!(mappings.len(), 4);
    assert_eq!(mappings[0].crab_change, deleted.operation.unwrap());
    assert_eq!(mappings[1].crab_change, imported_change);
    assert_eq!(mappings[2].crab_change, imported_change);
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

    let init = Trail::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();
    assert_eq!(init.imported.files, 1);
    assert_eq!(init.imported.text, 1);

    let db = Trail::open(temp.path()).unwrap();
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
    run_git(temp.path(), &["config", "user.email", "trail@example.test"]);
    run_git(temp.path(), &["config", "user.name", "Trail Test"]);
    fs::write(temp.path().join("README.md"), "one\ntwo\n").unwrap();
    run_git(temp.path(), &["add", "README.md"]);
    run_git(temp.path(), &["commit", "-m", "initial"]);
    let git_head = git_output(temp.path(), &["rev-parse", "HEAD"]);

    let init = Trail::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();
    fs::write(temp.path().join("README.md"), "one\ntwo\nthree\n").unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
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
    let exported = run_trail_json(
        temp.path(),
        &["git", "export", &range, "-m", "Export Trail change"],
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

    let db = Trail::open(temp.path()).unwrap();
    let mappings = db.git_mappings(10).unwrap();
    assert_eq!(mappings[0].direction, "export");
    assert_eq!(mappings[0].git_head.as_deref(), Some(commit));
    assert_eq!(mappings[0].crab_change, exported_change);
}

#[test]
fn git_export_uses_clean_head_mapping_for_delta_commit() {
    if !git_available() {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    run_git(temp.path(), &["init"]);
    run_git(temp.path(), &["config", "user.email", "trail@example.test"]);
    run_git(temp.path(), &["config", "user.name", "Trail Test"]);
    fs::write(temp.path().join("README.md"), "one\ntwo\n").unwrap();
    fs::write(temp.path().join("notes.md"), "alpha\n").unwrap();
    run_git(temp.path(), &["add", "README.md", "notes.md"]);
    run_git(temp.path(), &["commit", "-m", "initial"]);
    let git_head = git_output(temp.path(), &["rev-parse", "HEAD"]);

    let init = Trail::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    let mapping = db.git_mappings(1).unwrap().pop().unwrap();
    assert!(!mapping.git_dirty);
    assert_eq!(mapping.git_head.as_deref(), Some(git_head.as_str()));

    fs::write(temp.path().join("README.md"), "one\nTWO\n").unwrap();
    fs::remove_file(temp.path().join("notes.md")).unwrap();
    let record = db
        .record(
            Some("main"),
            Some("rewrite readme and remove notes".to_string()),
            Actor::human(),
            false,
        )
        .unwrap();
    let exported_change = record.operation.unwrap();
    drop(db);
    run_git(temp.path(), &["checkout", "--", "README.md", "notes.md"]);
    assert!(git_output(
        temp.path(),
        &["status", "--porcelain", "--untracked-files=no"]
    )
    .is_empty());

    let range = format!("{}..{}", init.operation.0, exported_change.0);
    let exported = run_trail_json(
        temp.path(),
        &["git", "export", &range, "-m", "Export mapped delta"],
    );
    let commit = exported["commit"].as_str().unwrap();

    assert_eq!(exported["parent"], git_head);
    assert_eq!(
        git_output(temp.path(), &["show", &format!("{commit}:README.md")]),
        "one\nTWO"
    );
    let missing = Command::new("git")
        .arg("-C")
        .arg(temp.path())
        .args(["show", &format!("{commit}:notes.md")])
        .output()
        .unwrap();
    assert!(!missing.status.success());
}

#[test]
fn git_export_seeds_clean_mapping_from_worktree_index_for_delta_commit() {
    if !git_available() {
        return;
    }

    let temp = tempfile::tempdir().unwrap();
    run_git(temp.path(), &["init"]);
    run_git(temp.path(), &["config", "user.email", "trail@example.test"]);
    run_git(temp.path(), &["config", "user.name", "Trail Test"]);
    fs::write(temp.path().join("README.md"), "one\n").unwrap();
    run_git(temp.path(), &["add", "README.md"]);
    run_git(temp.path(), &["commit", "-m", "initial"]);

    fs::write(temp.path().join("README.md"), "one\ntwo\n").unwrap();
    let init = Trail::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    let dirty_mapping = db.git_mappings(1).unwrap().pop().unwrap();
    assert!(dirty_mapping.git_dirty);

    run_git(temp.path(), &["add", "README.md"]);
    run_git(temp.path(), &["commit", "-m", "commit imported root"]);
    let git_head = git_output(temp.path(), &["rev-parse", "HEAD"]);
    assert!(git_output(
        temp.path(),
        &["status", "--porcelain", "--untracked-files=no"]
    )
    .is_empty());

    db.spawn_lane("delta-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "lane adds export line",
        "edits": [
            {"op": "write", "path": "README.md", "content": "one\ntwo\nthree\n"}
        ]
    }))
    .unwrap();
    let applied = apply_lane_patch_at_head(&mut db, "delta-bot", patch).unwrap();
    let range = format!("{}..{}", init.operation.0, applied.operation.0);
    let exported = db
        .git_export_commit(&range, "Export index-verified delta")
        .unwrap();

    assert_eq!(exported.parent.as_deref(), Some(git_head.as_str()));
    assert_eq!(
        git_output(
            temp.path(),
            &["show", &format!("{}:README.md", exported.commit)]
        ),
        "one\ntwo\nthree"
    );
    let mappings = db.git_mappings(10).unwrap();
    assert!(mappings.iter().any(|mapping| {
        mapping.direction == "verify-index"
            && mapping.git_head.as_deref() == Some(git_head.as_str())
            && mapping.crab_root == init.root_id
    }));
}

#[test]
fn same_position_rewrite_preserves_line_identity() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "one\ntwo\nthree\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
        .any(|entry| entry.kind == trail::LineChangeKind::Modified));

    let line_id = before.line_id.alias();
    let cli_by_line = run_trail_json(temp.path(), &["why", "--line-id", &line_id, "--at", "main"]);
    assert_eq!(cli_by_line["current_text"], "lane rewrote this line");
    let cli_at_root = run_trail_json(
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

    let http = trail::server::handle_http_request(
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

    let mcp = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "trail.why",
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
    let init = Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
    assert_eq!(diff.files[0].kind, trail::FileChangeKind::Modified);
    assert!(diff.files[0].patch.as_ref().unwrap().contains("+TWO"));
    assert!(diff.files[1].patch.as_ref().unwrap().contains("+BETA"));
    assert_eq!(diff.files[0].line_changes.len(), 1);
    assert_eq!(diff.files[0].line_changes[0].line_id, before.line_id);

    let root_range = format!("{}..{}", init.root_id.0, record.root_id.0);
    let root_diff = db.diff_roots(&root_range, false, true).unwrap();
    assert_eq!(root_diff.from, init.root_id.0);
    assert_eq!(root_diff.files[0].line_changes[0].line_id, before.line_id);

    let cli = run_trail_json(temp.path(), &["diff", &range, "--show-line-ids"]);
    assert_eq!(
        cli["files"][0]["line_changes"][0]["line_id"]["origin_change"],
        before.line_id.origin_change.0
    );

    let api_response = trail::server::handle_http_request(
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

    let mcp = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "trail.diff",
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

    let cli_dirty = run_trail_json(temp.path(), &["diff", "--dirty", "--show-line-ids"]);
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let db = Trail::open(temp.path()).unwrap();
    let status = db.status(Some("main")).unwrap();
    let root_object = db.inspect_object(&status.head.root_id.0).unwrap();
    assert_eq!(root_object.info.kind, trail::WORKTREE_ROOT_KIND);
    assert_eq!(root_object.summary["file_count"], 2);

    let root = db.inspect_root(&status.head.root_id.0).unwrap();
    assert_eq!(root.files.len(), 2);
    let readme = root
        .files
        .iter()
        .find(|file| file.path == "README.md")
        .unwrap();
    assert_eq!(readme.kind, trail::FileKind::Text);
    let text_object = db.inspect_object(&readme.content_object.0).unwrap();
    assert_eq!(text_object.info.kind, trail::TEXT_CONTENT_KIND);
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
    Trail::init_with_text_policy(
        temp.path(),
        "main",
        InitImportMode::WorkingTree,
        false,
        Some("full"),
    )
    .unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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

    let cli_range = run_trail_json(
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

    fs::write(temp.path().join("README.md"), "hello\ntrail\n").unwrap();
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

    let cli_diff = run_trail_json(
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let created = db
        .create_anchor("note.txt:2", "important line", Some("main"))
        .unwrap();
    assert!(created.anchor.id.0.starts_with("anchor_"));
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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

    let why = trail::server::handle_http_request(
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
        "line_{}:{}",
        why["line_id"]["origin_change"]
            .as_str()
            .unwrap()
            .strip_prefix("change_")
            .unwrap(),
        why["line_id"]["local_seq"].as_u64().unwrap()
    );

    let history = trail::server::handle_http_request(
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

    let tools = trail::mcp::handle_json_rpc(
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
        "trail.why",
        "trail.history",
        "trail.code_from",
        "trail.anchor_create",
        "trail.anchor_list",
        "trail.anchor_resolve",
        "trail.anchor_delete",
    ] {
        assert!(tool_list.iter().any(|tool| tool["name"] == name), "{name}");
    }

    let mcp_why = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "trail.why",
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

    let mcp_code_from = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "trail.code_from",
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

    let created = trail::server::handle_http_request(
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

    let mcp_list = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "trail.anchor_list",
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

    let resolved = trail::server::handle_http_request(
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

    let mcp_resolved = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "trail.anchor_resolve",
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

    let mcp_delete = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "trail.anchor_delete",
                "arguments": {
                    "anchor_id": anchor_id
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_delete["result"]["isError"], false);

    let anchors = trail::server::handle_http_request(
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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

    let checkpoint_selector = branch.from.checkpoint_alias();
    let checkpoint_preview = db
        .checkout_with_options(&checkpoint_selector, true, true, None, false)
        .unwrap();
    assert!(checkpoint_preview.dry_run);
    assert_eq!(checkpoint_preview.change_id, branch.from);
    assert_eq!(checkpoint_preview.root_id, branch.root_id);

    let raw_root = branch.root_id.0.clone();
    drop(db);

    let cli_root_checkout = run_trail_json(temp.path(), &["checkout", &raw_root, "--force"]);
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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

    let cli_dry_run = run_trail_json(temp.path(), &["checkout", &branch.from.0, "--dry-run"]);
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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

    let checked_out = run_trail_json(temp.path(), &["checkout", &branch.from.0, "--record-dirty"]);
    let recorded_dirty = checked_out["recorded_dirty"].as_str().unwrap().to_string();
    assert_eq!(checked_out["change_id"], branch.from.0);
    assert_eq!(
        fs::read_to_string(temp.path().join("note.txt")).unwrap(),
        "one\n"
    );

    let mut db = Trail::open(temp.path()).unwrap();
    assert_eq!(
        db.status(Some("main")).unwrap().head.change_id.0,
        recorded_dirty
    );
    match db.show(&recorded_dirty).unwrap() {
        ShowResult::Operation { value } => {
            let expected_message =
                format!("Record dirty worktree before checkout `{}`", branch.from.0);
            assert_eq!(value.operation.kind, trail::OperationKind::Checkout);
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
    assert!(!temp.path().join(".trail/refs/branches/main").exists());
    assert!(temp.path().join(".trail/refs/branches/trunk").exists());

    let deleted = db.delete_branch("scratch").unwrap();
    assert_eq!(deleted.name, "scratch");
    assert!(!temp.path().join(".trail/refs/branches/scratch").exists());
    let err = db.delete_branch("trunk").unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)));
}

#[test]
fn timeline_branch_scope_accepts_command_flag_and_ref_aliases() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("note.txt"), "one\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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

    let scratch_timeline = run_trail_json(
        temp.path(),
        &["timeline", "--branch", "branch:scratch", "--limit", "10"],
    );
    let scratch_entries = scratch_timeline.as_array().unwrap();
    assert_eq!(scratch_entries.len(), 1);
    assert_eq!(scratch_entries[0]["change_id"], scratch_record.0);

    let main_timeline = run_trail_json(temp.path(), &["timeline", "--branch", "main"]);
    let main_entries = main_timeline.as_array().unwrap();
    assert!(main_entries
        .iter()
        .any(|entry| entry["change_id"] == main_record.0));
}

#[test]
fn lane_patch_can_merge_into_main() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
        &fs::read_to_string(workdir.join(".trail/workdir-manifest.json")).unwrap(),
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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

    let cli = run_trail_json(
        temp.path(),
        &[
            "lane",
            "merge",
            "doc-bot",
            "--strategy",
            "line-id-aware",
            "--dry-run",
        ],
    );
    assert_eq!(cli["dry_run"], true);
    assert_eq!(cli["changed_paths"][0]["path"], "README.md");

    let api_response = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes/doc-bot/merge",
            serde_json::json!({
                "into": "main",
                "dry_run": true
            }),
        ),
    );
    assert_eq!(api_response.status, 200);
    let api: serde_json::Value = api_response.body_json().unwrap();
    assert_eq!(api["dry_run"], true);

    let legacy_response = trail::server::handle_http_request(
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
    assert_eq!(legacy_response.status, 400);

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
fn user_facing_lane_merge_prefers_queue_for_default_branch() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("cli-bot", Some("main"), false, None, None)
        .unwrap();
    let cli_patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write", "path": "docs/cli.md", "content": "cli\n"}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "cli-bot", cli_patch).unwrap();

    let blocked = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .arg("--json")
        .args(["lane", "merge", "cli-bot", "--into", "main"])
        .output()
        .unwrap();
    assert!(!blocked.status.success());
    let blocked_stderr: serde_json::Value = serde_json::from_slice(&blocked.stderr).unwrap();
    assert!(blocked_stderr["error"]["message"]
        .as_str()
        .unwrap()
        .contains("lane merge-queue add cli-bot --into main"));
    assert_eq!(db.status(Some("main")).unwrap().changed_paths.len(), 0);

    let direct = run_trail_json(
        temp.path(),
        &["lane", "merge", "cli-bot", "--into", "main", "--direct"],
    );
    assert_eq!(direct["dry_run"], false);
    assert_eq!(direct["changed_paths"][0]["path"], "docs/cli.md");

    db.spawn_lane("api-bot", Some("main"), false, None, None)
        .unwrap();
    let api_patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write", "path": "docs/api.md", "content": "api\n"}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "api-bot", api_patch).unwrap();
    let blocked_api = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes/api-bot/merge",
            serde_json::json!({ "into": "main" }),
        ),
    );
    assert_eq!(blocked_api.status, 400);
    let blocked_api_body: serde_json::Value = blocked_api.body_json().unwrap();
    assert!(blocked_api_body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("lane merge-queue add api-bot --into main"));

    let direct_api = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes/api-bot/merge",
            serde_json::json!({ "into": "main", "direct": true }),
        ),
    );
    assert_eq!(direct_api.status, 200);
    let direct_api_body: serde_json::Value = direct_api.body_json().unwrap();
    assert_eq!(direct_api_body["changed_paths"][0]["path"], "docs/api.md");
}

#[test]
fn lane_diff_cli_renders_scannable_overview() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("docs")).unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nlane\nworld\n"},
            {"op": "write", "path": "docs/guide.md", "content": "# Guide\n"}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "doc-bot", patch).unwrap();
    drop(db);

    let output = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .args(["lane", "diff", "doc-bot", "--stat"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "lane diff failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Lane diff: doc-bot"));
    assert!(stdout.contains("M  README.md  +1 -0"), "{stdout}");
    assert!(stdout.contains("A  docs/guide.md  +1 -0"), "{stdout}");
    assert!(stdout.contains("2 files changed, 2 insertions, 0 deletions"));
}

#[test]
fn layered_lane_update_preserves_lane_changes_and_advances_its_pinned_base() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "baseline\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    let mode = if cfg!(target_os = "macos") {
        LaneWorkdirMode::NfsCow
    } else if cfg!(target_os = "windows") {
        LaneWorkdirMode::DokanCow
    } else {
        LaneWorkdirMode::FuseCow
    };
    db.spawn_lane_with_workdir_mode_paths_and_neighbors(
        "updatable",
        Some("main"),
        mode,
        None,
        None,
        None,
        &[],
        false,
    )
    .unwrap();
    let view_before = db.lane_workspace_view("updatable").unwrap().unwrap();
    fs::write(
        Path::new(&view_before.source_upper).join("lane-only.txt"),
        "lane\n",
    )
    .unwrap();
    db.checkpoint_lane_workspace("updatable", Some("lane change".to_string()))
        .unwrap();

    fs::write(temp.path().join("main-only.txt"), "main\n").unwrap();
    let main_record = db
        .record(
            Some("main"),
            Some("main change".to_string()),
            Actor::human(),
            false,
        )
        .unwrap();
    let preview = db.preview_lane_refresh("updatable", "main").unwrap();
    assert!(!preview.clean);
    assert!(!preview.conflicted);
    assert!(preview
        .changed_paths
        .iter()
        .any(|path| path.path == "main-only.txt"));

    let update = db
        .update_layered_lane_from("updatable", "main", false)
        .unwrap();
    assert!(update.conflicts.is_empty());
    assert!(update
        .changed_paths
        .iter()
        .any(|path| path.path == "main-only.txt"));
    let details = db.lane_details("updatable").unwrap();
    assert_eq!(details.branch.base_change, main_record.operation.unwrap());
    assert_eq!(details.branch.head_change, update.operation);
    let view_after = db.lane_workspace_view("updatable").unwrap().unwrap();
    assert_eq!(view_after.generation, view_before.generation + 1);
    assert_eq!(view_after.base_change, update.operation);
    assert_eq!(view_after.base_root, update.root_id);
    assert_eq!(
        db.read_lane_file("updatable", "lane-only.txt", false, false, false)
            .unwrap()
            .content,
        "lane\n"
    );
    assert_eq!(
        db.read_lane_file("updatable", "main-only.txt", false, false, false)
            .unwrap()
            .content,
        "main\n"
    );
    assert!(db.preview_lane_refresh("updatable", "main").unwrap().clean);
}

#[test]
fn lane_refresh_preview_reports_target_changes_and_conflicts_without_mutating() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("docs")).unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nlane\n"}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "doc-bot", patch).unwrap();
    let lane_head_before = db.lane_details("doc-bot").unwrap().branch.head_change;

    fs::write(temp.path().join("docs/guide.md"), "# Guide\n").unwrap();
    db.record(
        Some("main"),
        Some("main guide".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();

    let preview = db.preview_lane_refresh("doc-bot", "main").unwrap();
    assert_eq!(preview.operations_behind, Some(1));
    assert!(!preview.clean);
    assert!(!preview.conflicted);
    assert!(preview
        .changed_paths
        .iter()
        .any(|path| path.path == "docs/guide.md"));
    assert!(preview.conflicts.is_empty());
    assert_eq!(
        db.lane_details("doc-bot").unwrap().branch.head_change,
        lane_head_before
    );
    assert!(db.list_conflicts().unwrap().is_empty());

    let api_response = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/lanes/doc-bot/refresh-preview?target=main",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(api_response.status, 200);
    let api: serde_json::Value = api_response.body_json().unwrap();
    assert_eq!(api["operations_behind"], 1);
    assert_eq!(api["conflicted"], false);
    assert!(api["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["path"] == "docs/guide.md"));

    fs::write(temp.path().join("README.md"), "hello\nmain\n").unwrap();
    db.record(
        Some("main"),
        Some("main readme".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();

    let conflicted = db.preview_lane_refresh("doc-bot", "main").unwrap();
    assert_eq!(conflicted.operations_behind, Some(2));
    assert!(conflicted.conflicted);
    assert_eq!(
        conflicted.conflicts,
        vec!["both changed `README.md` differently"]
    );
    assert_eq!(
        db.lane_details("doc-bot").unwrap().branch.head_change,
        lane_head_before
    );
    assert!(db.list_conflicts().unwrap().is_empty());
    let mcp_preview = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "trail.lane_refresh_preview",
                "arguments": {
                    "lane": "doc-bot",
                    "target": "main"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_preview["result"]["isError"], false);
    assert_eq!(
        mcp_preview["result"]["structuredContent"]["operations_behind"],
        2
    );
    assert_eq!(
        mcp_preview["result"]["structuredContent"]["conflicted"],
        true
    );
    assert_eq!(
        mcp_preview["result"]["structuredContent"]["conflicts"][0],
        "both changed `README.md` differently"
    );
    assert_eq!(
        db.lane_details("doc-bot").unwrap().branch.head_change,
        lane_head_before
    );
    assert!(db.list_conflicts().unwrap().is_empty());
    drop(db);

    let cli = run_trail_json(temp.path(), &["lane", "refresh-preview", "doc-bot"]);
    assert_eq!(cli["operations_behind"], 2);
    assert_eq!(cli["conflicted"], true);
    assert_eq!(cli["conflicts"][0], "both changed `README.md` differently");
}

#[test]
fn lane_status_surfaces_stale_base_lag() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("docs")).unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("doc-bot", Some("main"), false, None, None)
        .unwrap();

    fs::write(temp.path().join("docs/one.md"), "one\n").unwrap();
    db.record(
        Some("main"),
        Some("main one".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();
    fs::write(temp.path().join("docs/two.md"), "two\n").unwrap();
    db.record(
        Some("main"),
        Some("main two".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();
    let main_head = db.status(Some("main")).unwrap().head.change_id;

    let status = db.lane_status("doc-bot").unwrap();
    let base_status = status.base_status.as_ref().unwrap();
    assert_eq!(base_status.target_branch, "main");
    assert_eq!(base_status.target_ref, "refs/branches/main");
    assert_eq!(base_status.target_change, main_head);
    assert_eq!(base_status.lane_base_change, spawned.base_change);
    assert_eq!(base_status.operations_behind, Some(2));
    assert!(base_status.stale);

    let readiness = db.lane_readiness("doc-bot").unwrap();
    let stale_warning = readiness
        .warnings
        .iter()
        .find(|issue| issue.code == "stale_lane_base")
        .expect("missing stale base warning");
    assert!(stale_warning
        .message
        .contains("lane started 2 operations behind `main`"));
    assert_eq!(
        stale_warning.details.as_ref().unwrap()["operations_behind"],
        serde_json::json!(2)
    );

    let api_status = trail::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/lanes/doc-bot/status", serde_json::Value::Null),
    );
    assert_eq!(api_status.status, 200);
    let api_status: serde_json::Value = api_status.body_json().unwrap();
    assert_eq!(api_status["base_status"]["operations_behind"], 2);
    assert_eq!(api_status["base_status"]["stale"], true);
    drop(db);

    let cli_json = run_trail_json(temp.path(), &["lane", "status", "doc-bot"]);
    assert_eq!(cli_json["base_status"]["target_branch"], "main");
    assert_eq!(cli_json["base_status"]["operations_behind"], 2);
    assert_eq!(cli_json["base_status"]["stale"], true);

    let cli_text = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .args(["lane", "status", "doc-bot"])
        .output()
        .unwrap();
    assert!(cli_text.status.success());
    let stdout = String::from_utf8_lossy(&cli_text.stdout);
    assert!(
        stdout.contains("Base: 2 operation(s) behind main"),
        "{stdout}"
    );
}

#[test]
fn merge_dry_run_reports_conflicts_without_opening_conflict_state() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "one\ntwo\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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

    let cli = run_trail_json(temp.path(), &["lane", "merge", "doc-bot", "--dry-run"]);
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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

    let response = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes/api-bot/merge",
            serde_json::json!({ "into": "main", "direct": true }),
        ),
    );
    assert_eq!(response.status, 409);
    let body: serde_json::Value = response.body_json().unwrap();
    assert_eq!(body["error"]["code"], "MERGE_CONFLICT");
    assert_eq!(body["error"]["exit"], 6);
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("conflict_"));

    let conflicts = trail::server::handle_http_request(
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let why = db.why("README.md:2", Some("refs/lanes/doc-bot")).unwrap();
    let line_id = why.line_id.alias();
    let missing_expected: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "line-id patch without guard",
        "edits": [{
            "op": "replace_line",
            "path": "README.md",
            "line_id": line_id.clone(),
            "new_text": "unguarded lane two"
        }]
    }))
    .unwrap();
    let err = apply_lane_patch_at_head(&mut db, "doc-bot", missing_expected).unwrap_err();
    assert!(matches!(err, Error::PatchRejected(_)));
    assert!(err.to_string().contains("requires expected_text"));
    let unchanged = db.why("README.md:2", Some("refs/lanes/doc-bot")).unwrap();
    assert_eq!(unchanged.current_text, "two");

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
                trail::LineChangeKind::Modified
            );
        }
        other => panic!("expected operation, got {other:?}"),
    }

    let stale_patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [{
            "op": "replace_line",
            "path": "README.md",
            "line_id": why.line_id.alias(),
            "expected_text": "two",
            "new_text": "stale"
        }]
    }))
    .unwrap();
    let err = apply_lane_patch_at_head(&mut db, "doc-bot", stale_patch).unwrap_err();
    assert!(matches!(err, Error::PatchRejected(_)));
    assert!(err.to_string().contains("expected text mismatch"), "{err}");
}

#[test]
fn lane_patch_rejected_line_id_batch_rolls_back_candidate_objects() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "one\ntwo\nthree\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("atomic-line-bot", Some("main"), false, None, None)
        .unwrap();
    let first = db
        .why("README.md:1", Some("refs/lanes/atomic-line-bot"))
        .unwrap();
    let second = db
        .why("README.md:2", Some("refs/lanes/atomic-line-bot"))
        .unwrap();
    let first_line_id = first.line_id.alias();
    let second_line_id = second.line_id.alias();

    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
    let count_rows = |table: &str| -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    };
    let objects_before = count_rows("objects");
    let prolly_nodes_before = count_rows("prolly_nodes");

    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "atomic stale line-id batch",
        "edits": [
            {
                "op": "replace_line",
                "path": "README.md",
                "line_id": first_line_id,
                "expected_text": "one",
                "new_text": "changed one"
            },
            {
                "op": "replace_line",
                "path": "README.md",
                "line_id": second_line_id,
                "expected_text": "stale two",
                "new_text": "changed two"
            }
        ]
    }))
    .unwrap();
    let err = apply_lane_patch_at_head(&mut db, "atomic-line-bot", patch).unwrap_err();
    assert!(matches!(err, Error::PatchRejected(_)));
    assert!(err.to_string().contains("expected text mismatch"), "{err}");

    assert_eq!(count_rows("objects"), objects_before);
    assert_eq!(count_rows("prolly_nodes"), prolly_nodes_before);
    let unchanged_first = db
        .why("README.md:1", Some("refs/lanes/atomic-line-bot"))
        .unwrap();
    let unchanged_second = db
        .why("README.md:2", Some("refs/lanes/atomic-line-bot"))
        .unwrap();
    assert_eq!(unchanged_first.current_text, "one");
    assert_eq!(unchanged_second.current_text, "two");
    assert_eq!(unchanged_first.line_id, first.line_id);
    assert_eq!(unchanged_second.line_id, second.line_id);
}

#[test]
fn lane_patch_rejects_foreign_session_before_candidate_objects() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("patch-session-bot", Some("main"), false, None, None)
        .unwrap();
    db.spawn_lane("other-session-bot", Some("main"), false, None, None)
        .unwrap();
    let other_turn = db
        .begin_lane_turn(
            "other-session-bot",
            None,
            Some("foreign session".to_string()),
            None,
        )
        .unwrap();

    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
    let count_rows = |table: &str| -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    };
    let objects_before = count_rows("objects");
    let prolly_nodes_before = count_rows("prolly_nodes");

    let patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "session_id": other_turn.session.session_id,
        "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nforeign session\n"}
        ]
    }))
    .unwrap();
    let err = apply_lane_patch_at_head(&mut db, "patch-session-bot", patch).unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)));
    assert!(err.to_string().contains("belongs to another lane"));

    assert_eq!(count_rows("objects"), objects_before);
    assert_eq!(count_rows("prolly_nodes"), prolly_nodes_before);
    let unchanged = db
        .why("README.md:1", Some("refs/lanes/patch-session-bot"))
        .unwrap();
    assert_eq!(unchanged.current_text, "hello");
}

#[test]
fn lane_patch_replace_line_fuzzes_batch_expected_text_edits() {
    let temp = tempfile::tempdir().unwrap();
    let original = (1..=16)
        .map(|idx| format!("line-{idx:02}\n"))
        .collect::<String>();
    fs::write(temp.path().join("README.md"), original).unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
        let line_id = why.line_id.alias();
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
        path.kind == trail::FileChangeKind::Renamed
            && path.old_path.as_deref() == Some("README.md")
            && path.path == "docs/README.md"
    }));
    assert!(applied
        .changed_paths
        .iter()
        .any(|path| { path.kind == trail::FileChangeKind::Deleted && path.path == "old.txt" }));
    assert!(applied
        .changed_paths
        .iter()
        .any(|path| { path.kind == trail::FileChangeKind::Added && path.path == "src/new.rs" }));

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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("rewind-bot", Some("main"), true, None, None)
        .unwrap();
    let base_change = spawned.base_change;
    let workdir = PathBuf::from(spawned.workdir.unwrap());
    fs::write(workdir.join("README.md"), "bad workdir\n").unwrap();
    drop(db);

    let cli = run_trail_json(
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

    let db = Trail::open(temp.path()).unwrap();
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
                trail::OperationKind::LaneRewind
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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

    let response = trail::server::handle_http_request(
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
    let mcp = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "trail.lane_rewind",
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
    assert!(!temp.path().join(".trail/refs/lanes/doc-bot").exists());
    if let Some(workdir) = removed.removed_workdir {
        assert!(!std::path::Path::new(&workdir).exists());
    }
    assert_eq!(db.lane_details("doc-bot").unwrap().branch.status, "removed");
    drop(db);
    let reopened = Trail::open(temp.path()).unwrap();
    assert_eq!(
        reopened.lane_details("doc-bot").unwrap().branch.status,
        "removed"
    );
    drop(reopened);

    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("doc-bot", Some("main"), true, None, None)
        .unwrap();
    drop(db);

    let tested = run_trail_json(
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

    let mut db = Trail::open(temp.path()).unwrap();
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
    let evaled = run_trail_json(
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

    let mut db = Trail::open(temp.path()).unwrap();
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

    let api_eval = trail::server::handle_http_request(
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

    let mcp_eval = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "trail.run_eval",
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

    let api_gates = trail::server::handle_http_request(
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

    let mcp_gates = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "trail.gate_history",
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

    let mcp_gate_resource = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "resources/read",
            "params": {
                "uri": "trail://workspace/lanes/doc-bot/gates"
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
    let cli_gates = run_trail_json(
        temp.path(),
        &["lane", "gates", "doc-bot", "--kind", "eval", "--limit", "2"],
    );
    assert_eq!(cli_gates["kind"], "eval");
    assert_eq!(cli_gates["gates"].as_array().unwrap().len(), 2);
    assert_eq!(cli_gates["gates"][0]["suite"], "nightly");

    let mut db = Trail::open(temp.path()).unwrap();
    let gc = db.gc(false).unwrap();
    assert!(gc.errors.is_empty(), "{:?}", gc.errors);
    assert!(db.inspect_object(&stdout_object).is_ok());
    assert!(db.inspect_object(&stderr_object).is_ok());
}

#[test]
fn lane_sessions_track_messages_patches_and_turns() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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

    let http_timeline = trail::server::handle_http_request(
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

    let mcp_tools = trail::mcp::handle_json_rpc(
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
        .any(|tool| tool["name"] == "trail.timeline"));

    let mcp_timeline = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "trail.timeline",
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

    let cli_session_timeline = run_trail_json(
        temp.path(),
        &["timeline", "--session", "session-docs", "--limit", "5"],
    );
    assert_eq!(cli_session_timeline.as_array().unwrap().len(), 1);
    assert_eq!(cli_session_timeline[0]["change_id"], applied.operation.0);

    let cli_lane_timeline = run_trail_json(
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
        .any(|entry| entry.kind == trail::OperationKind::LaneRecord));

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
fn lane_workdir_record_rejects_case_fold_collision_before_candidate_objects() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("record-case-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = PathBuf::from(spawned.workdir.unwrap());
    fs::write(workdir.join("ＲＥＡＤＭＥ.md"), "case collision\n").unwrap();

    let preview = db.preview_lane_workdir_record("record-case-bot").unwrap();
    assert!(!preview.policy.allowed);
    assert!(preview
        .policy
        .error
        .as_deref()
        .is_some_and(|message| message.contains("case-insensitive path collision")));

    let before = db
        .lane_details("record-case-bot")
        .unwrap()
        .branch
        .head_change
        .clone();
    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
    let count_rows = |table: &str| -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    };
    let objects_before = count_rows("objects");
    let prolly_nodes_before = count_rows("prolly_nodes");

    let err = db
        .record_lane_workdir("record-case-bot", Some("record case collision".to_string()))
        .unwrap_err();
    assert!(
        matches!(err, Error::InvalidPath { ref reason, .. } if reason.contains("case-insensitive path collision")),
        "expected case-fold collision, got {err:?}"
    );
    assert_eq!(
        db.lane_details("record-case-bot")
            .unwrap()
            .branch
            .head_change,
        before
    );
    assert_eq!(count_rows("objects"), objects_before);
    assert_eq!(count_rows("prolly_nodes"), prolly_nodes_before);
}

#[test]
fn lane_workdir_record_rejects_secret_message_before_candidate_objects() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("record-secret-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = PathBuf::from(spawned.workdir.unwrap());
    fs::write(
        workdir.join("README.md"),
        "hello\nsecret message preflight\n",
    )
    .unwrap();

    let before = db
        .lane_details("record-secret-bot")
        .unwrap()
        .branch
        .head_change
        .clone();
    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
    let count_rows = |table: &str| -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    };
    let objects_before = count_rows("objects");
    let prolly_nodes_before = count_rows("prolly_nodes");
    let turns_before = count_rows("lane_turns");

    let err = db
        .record_lane_workdir("record-secret-bot", Some("password=hunter2".to_string()))
        .unwrap_err();
    assert!(
        matches!(err, Error::PatchRejected(ref message) if message.contains("secret scan rejected lane record message")),
        "expected secret-scan rejection, got {err:?}"
    );
    assert_eq!(
        db.lane_details("record-secret-bot")
            .unwrap()
            .branch
            .head_change,
        before
    );
    assert_eq!(count_rows("objects"), objects_before);
    assert_eq!(count_rows("prolly_nodes"), prolly_nodes_before);
    assert_eq!(count_rows("lane_turns"), turns_before);
}

#[test]
fn lane_workdir_record_rejects_foreign_session_before_candidate_objects() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("record-session-bot", Some("main"), true, None, None)
        .unwrap();
    db.spawn_lane("other-record-session-bot", Some("main"), false, None, None)
        .unwrap();
    let other_turn = db
        .begin_lane_turn(
            "other-record-session-bot",
            None,
            Some("foreign record session".to_string()),
            None,
        )
        .unwrap();
    let workdir = PathBuf::from(spawned.workdir.unwrap());
    fs::write(workdir.join("README.md"), "hello\nforeign record session\n").unwrap();

    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
    conn.execute(
        "UPDATE lane_branches SET session_id = ?1 WHERE lane_id = ?2",
        rusqlite::params![other_turn.session.session_id, spawned.lane_id],
    )
    .unwrap();
    let count_rows = |table: &str| -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    };
    let objects_before = count_rows("objects");
    let prolly_nodes_before = count_rows("prolly_nodes");

    let err = db
        .record_lane_workdir(
            "record-session-bot",
            Some("foreign session record".to_string()),
        )
        .unwrap_err();
    assert!(matches!(err, Error::InvalidInput(_)));
    assert!(err.to_string().contains("belongs to another lane"));

    assert_eq!(count_rows("objects"), objects_before);
    assert_eq!(count_rows("prolly_nodes"), prolly_nodes_before);
    let unchanged = db
        .why("README.md:1", Some("refs/lanes/record-session-bot"))
        .unwrap();
    assert_eq!(unchanged.current_text, "hello");
}

#[test]
fn materialized_lane_status_and_record_handle_workdir_renames() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("rename-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = PathBuf::from(spawned.workdir.unwrap());
    let before_line = db
        .why("README.md:1", Some("refs/lanes/rename-bot"))
        .unwrap()
        .line_id;
    fs::create_dir_all(workdir.join("docs")).unwrap();
    fs::rename(workdir.join("README.md"), workdir.join("docs/README.md")).unwrap();

    let status = db.lane_status("rename-bot").unwrap();
    assert_eq!(status.workdir_state, Some(WorktreeState::DirtyUntracked));
    let changed = status
        .workdir_changed_paths
        .iter()
        .map(|path| (path.path.as_str(), path.kind.clone()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        changed.get("README.md"),
        Some(&trail::FileChangeKind::Deleted)
    );
    assert_eq!(
        changed.get("docs/README.md"),
        Some(&trail::FileChangeKind::Added)
    );

    let preview = db.preview_lane_workdir_record("rename-bot").unwrap();
    let preview_paths = preview
        .changed_paths
        .iter()
        .map(|path| (path.path.as_str(), path.kind.clone()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        preview_paths.get("README.md"),
        Some(&trail::FileChangeKind::Deleted)
    );
    assert_eq!(
        preview_paths.get("docs/README.md"),
        Some(&trail::FileChangeKind::Added)
    );

    let recorded = db
        .record_lane_workdir("rename-bot", Some("record rename".to_string()))
        .unwrap();
    assert!(recorded.operation.is_some());
    assert_eq!(recorded.changed_paths.len(), 1);
    assert_eq!(
        recorded.changed_paths[0].kind,
        trail::FileChangeKind::Renamed
    );
    assert_eq!(recorded.changed_paths[0].path, "docs/README.md");
    assert_eq!(
        recorded.changed_paths[0].old_path.as_deref(),
        Some("README.md")
    );
    assert_eq!(
        db.why("docs/README.md:1", Some("refs/lanes/rename-bot"))
            .unwrap()
            .line_id,
        before_line
    );
    assert_eq!(
        db.lane_status("rename-bot").unwrap().workdir_state,
        Some(WorktreeState::Clean)
    );

    db.merge_lane("rename-bot", "main").unwrap();
    db.checkout("main", true).unwrap();
    assert!(!temp.path().join("README.md").exists());
    assert_eq!(
        fs::read_to_string(temp.path().join("docs/README.md")).unwrap(),
        "hello\n"
    );
}

#[test]
fn materialized_lane_record_allows_case_only_rename_when_final_root_is_safe() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("record-case-rename", Some("main"), true, None, None)
        .unwrap();
    let workdir = PathBuf::from(spawned.workdir.unwrap());
    fs::rename(workdir.join("README.md"), workdir.join("rename-staging")).unwrap();
    fs::rename(workdir.join("rename-staging"), workdir.join("readme.md")).unwrap();

    let preview = db
        .preview_lane_workdir_record("record-case-rename")
        .unwrap();
    assert!(preview.policy.allowed, "{:?}", preview.policy.error);
    let recorded = db
        .record_lane_workdir(
            "record-case-rename",
            Some("record case-only rename".to_string()),
        )
        .unwrap();
    assert_eq!(recorded.changed_paths.len(), 1);
    assert_eq!(
        recorded.changed_paths[0].kind,
        trail::FileChangeKind::Renamed
    );
    assert_eq!(
        recorded.changed_paths[0].old_path.as_deref(),
        Some("README.md")
    );
    assert_eq!(recorded.changed_paths[0].path, "readme.md");
}

#[test]
fn lane_workdir_record_preview_reports_risks_and_oversized_files() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.config_set("lane.max_patch_file_bytes", "12").unwrap();
    let spawned = db
        .spawn_lane("doc-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = PathBuf::from(spawned.workdir.unwrap());
    fs::write(workdir.join("README.md"), "hello\nworkdir\n").unwrap();
    fs::write(workdir.join(".gitignore"), "ignored.log\n").unwrap();
    fs::write(workdir.join("ignored.log"), "ignored\n").unwrap();
    fs::create_dir_all(workdir.join(".git")).unwrap();
    fs::write(workdir.join(".git/config"), "[core]\n").unwrap();
    fs::create_dir_all(workdir.join("nested/.trail")).unwrap();
    fs::write(workdir.join("nested/.trail/config"), "metadata\n").unwrap();
    #[cfg(unix)]
    {
        fs::write(workdir.join("hardlink-source.txt"), "linked\n").unwrap();
        fs::hard_link(
            workdir.join("hardlink-source.txt"),
            workdir.join("hardlink-copy.txt"),
        )
        .unwrap();
        std::os::unix::fs::symlink("README.md", workdir.join("readme-link.md")).unwrap();
    }

    let head_before = db.lane_details("doc-bot").unwrap().branch.head_change;
    let preview = db.preview_lane_workdir_record("doc-bot").unwrap();
    assert!(!preview.clean);
    assert!(preview
        .changed_paths
        .iter()
        .any(|path| path.path == "README.md"));
    assert!(preview
        .ignored_paths
        .iter()
        .any(|path| path.path == "ignored.log" && path.source == "workdir"));
    assert!(preview
        .risky_paths
        .iter()
        .any(|path| path.path == ".git" && path.kind == "nested_git"));
    assert!(preview
        .risky_paths
        .iter()
        .any(|path| path.kind == "nested_trail" && path.path.starts_with("nested/.trail")));
    #[cfg(unix)]
    {
        assert!(preview
            .risky_paths
            .iter()
            .any(|path| path.path == "readme-link.md" && path.kind == "symlink"));
        assert!(preview.risky_paths.iter().any(|path| {
            path.kind == "hardlink"
                && matches!(
                    path.path.as_str(),
                    "hardlink-source.txt" | "hardlink-copy.txt"
                )
        }));
    }
    assert_eq!(preview.oversized_files.len(), 1);
    assert_eq!(preview.oversized_files[0].path, "README.md");
    assert!(!preview.policy.allowed);
    assert!(preview
        .policy
        .error
        .as_deref()
        .unwrap()
        .contains("lane.max_patch_file_bytes"));
    assert_eq!(
        db.lane_details("doc-bot").unwrap().branch.head_change,
        head_before
    );

    let cli_preview = run_trail_json(temp.path(), &["lane", "record", "doc-bot", "--preview"]);
    assert_eq!(cli_preview["clean"], false);
    assert!(cli_preview["risky_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["kind"] == "nested_git"));
    assert!(cli_preview["risky_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path["kind"] == "nested_trail"));
    #[cfg(unix)]
    {
        assert!(cli_preview["risky_paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path["kind"] == "symlink"));
        assert!(cli_preview["risky_paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path["kind"] == "hardlink"));
    }
    assert_eq!(
        db.lane_details("doc-bot").unwrap().branch.head_change,
        head_before
    );

    let err = db
        .record_lane_workdir("doc-bot", Some("record oversized".to_string()))
        .unwrap_err();
    assert!(
        matches!(err, Error::PatchRejected(message) if message.contains("lane.max_patch_file_bytes"))
    );
    assert_eq!(
        db.lane_details("doc-bot").unwrap().branch.head_change,
        head_before
    );
}

#[test]
fn lane_workdir_local_ignore_cannot_hide_materialized_tracked_files() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("ignore-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = PathBuf::from(spawned.workdir.unwrap());
    fs::write(workdir.join(".gitignore"), "README.md\n").unwrap();

    let ignore_record = db
        .record_lane_workdir("ignore-bot", Some("add local ignore".to_string()))
        .unwrap();
    assert!(ignore_record.operation.is_some());
    assert!(ignore_record
        .changed_paths
        .iter()
        .any(|path| path.path == ".gitignore"));
    assert_eq!(
        db.lane_status("ignore-bot").unwrap().workdir_state,
        Some(WorktreeState::Clean)
    );
    let clean_manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(workdir.join(".trail/workdir-manifest.json")).unwrap(),
    )
    .unwrap();
    assert!(clean_manifest["files"].get("README.md").is_some());
    assert!(clean_manifest["files"].get(".gitignore").is_some());

    fs::write(workdir.join("README.md"), "hello\nstill tracked\n").unwrap();
    let dirty = db.lane_status("ignore-bot").unwrap();
    assert_eq!(dirty.workdir_state, Some(WorktreeState::DirtyTracked));
    assert!(dirty
        .workdir_changed_paths
        .iter()
        .any(|path| path.path == "README.md" && path.kind == trail::FileChangeKind::Modified));

    let readme_record = db
        .record_lane_workdir(
            "ignore-bot",
            Some("record ignored tracked file".to_string()),
        )
        .unwrap();
    assert!(readme_record.operation.is_some());
    assert!(readme_record
        .changed_paths
        .iter()
        .any(|path| path.path == "README.md"));

    db.merge_lane("ignore-bot", "main").unwrap();
    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "hello\nstill tracked\n"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join(".gitignore")).unwrap(),
        "README.md\n"
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let workdir_parent = tempfile::tempdir().unwrap();
    let default_spawn = run_trail_json(
        temp.path(),
        &["lane", "spawn", "default-bot", "--from", "main"],
    );
    assert!(default_spawn["workdir"].is_null());
    assert_eq!(default_spawn["requested_workdir_mode"], "virtual");
    assert_eq!(default_spawn["workdir_mode"], "virtual");
    assert_eq!(default_spawn["workdir_backend"], "virtual");
    assert_eq!(default_spawn["sparse_paths"].as_array().unwrap().len(), 0);
    assert_eq!(default_spawn["transparent_cow_available"], false);

    let cli_workdir = workdir_parent.path().join("cli-bot");
    let cli_spawn = run_trail_json(
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
    assert_eq!(cli_spawn["requested_workdir_mode"], "auto");
    assert_eq!(cli_spawn["workdir_mode"], "native-cow");
    assert_eq!(cli_spawn["workdir_backend"], "clone");
    assert_eq!(cli_spawn["materialization"]["copied_files"], 0);
    assert!(
        cli_spawn["materialization"]["cloned_files"]
            .as_u64()
            .unwrap()
            > 0
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

    let native_workdir = workdir_parent.path().join("native-bot");
    let native_spawn = run_trail_json(
        temp.path(),
        &[
            "lane",
            "spawn",
            "native-bot",
            "--from",
            "main",
            "--workdir-mode",
            "native-cow",
            "--workdir",
            native_workdir.to_str().unwrap(),
        ],
    );
    assert_eq!(native_spawn["requested_workdir_mode"], "native-cow");
    assert_eq!(native_spawn["workdir_mode"], "native-cow");
    assert_eq!(native_spawn["workdir_backend"], "clone");
    assert_eq!(native_spawn["materialization"]["copied_files"], 0);

    let portable_workdir = workdir_parent.path().join("portable-bot");
    let portable_spawn = run_trail_json(
        temp.path(),
        &[
            "lane",
            "spawn",
            "portable-bot",
            "--from",
            "main",
            "--workdir-mode",
            "portable-copy",
            "--workdir",
            portable_workdir.to_str().unwrap(),
        ],
    );
    assert_eq!(portable_spawn["requested_workdir_mode"], "portable-copy");
    assert_eq!(portable_spawn["workdir_mode"], "portable-copy");
    assert_eq!(portable_spawn["workdir_backend"], "clone");
    assert_eq!(portable_spawn["materialization"]["copied_files"], 0);

    let headless_spawn = run_trail_json(
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
    assert_eq!(headless_spawn["workdir_mode"], "virtual");

    let no_materialize_spawn = run_trail_json(
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
    assert_eq!(no_materialize_spawn["workdir_mode"], "virtual");

    #[cfg(any(target_os = "linux", all(target_os = "macos", feature = "macfuse")))]
    {
        let fuse_spawn = run_trail_json(
            temp.path(),
            &[
                "lane",
                "spawn",
                "fuse-bot",
                "--from",
                "main",
                "--workdir-mode",
                "fuse-cow",
            ],
        );
        assert_eq!(fuse_spawn["workdir_mode"], "fuse-cow");
        assert_eq!(fuse_spawn["workdir_backend"], "fuse");
        assert_eq!(fuse_spawn["transparent_cow_available"], true);
        assert_eq!(fuse_spawn["sparse_paths"].as_array().unwrap().len(), 0);
        let fuse_workdir = PathBuf::from(fuse_spawn["workdir"].as_str().unwrap());
        assert!(fuse_workdir.is_dir());
        assert!(fs::read_dir(&fuse_workdir).unwrap().next().is_none());
        assert!(!fuse_workdir.join("README.md").exists());
        let db = Trail::open(temp.path()).unwrap();
        let view = db.lane_workspace_view("fuse-bot").unwrap().unwrap();
        assert!(Path::new(&view.source_upper).is_dir());
        assert!(Path::new(&view.meta_dir).is_dir());
    }

    #[cfg(target_os = "windows")]
    {
        let dokan_spawn = run_trail_json(
            temp.path(),
            &[
                "lane",
                "spawn",
                "dokan-bot",
                "--from",
                "main",
                "--workdir-mode",
                "dokan-cow",
            ],
        );
        assert_eq!(dokan_spawn["workdir_mode"], "dokan-cow");
        assert_eq!(dokan_spawn["workdir_backend"], "dokan");
        assert_eq!(dokan_spawn["transparent_cow_available"], true);
    }

    #[cfg(target_os = "macos")]
    {
        let nfs_spawn = run_trail_json(
            temp.path(),
            &[
                "lane",
                "spawn",
                "nfs-bot",
                "--from",
                "main",
                "--workdir-mode",
                "nfs-cow",
            ],
        );
        assert_eq!(nfs_spawn["workdir_mode"], "nfs-cow");
        assert_eq!(nfs_spawn["workdir_backend"], "nfs");
        assert_eq!(nfs_spawn["transparent_cow_available"], true);
        let db = Trail::open(temp.path()).unwrap();
        let view = db.lane_workspace_view("nfs-bot").unwrap().unwrap();
        assert!(Path::new(&view.source_upper).is_dir());
        assert!(Path::new(&view.meta_dir).is_dir());
    }

    let mut db = Trail::open(temp.path()).unwrap();
    assert_eq!(
        db.lane_status("cli-bot").unwrap().workdir_state,
        Some(WorktreeState::Clean)
    );
    let persisted_workdir = db.lane_workdir("cli-bot").unwrap();
    assert_eq!(
        persisted_workdir.requested_workdir_mode,
        LaneWorkdirMode::Auto
    );
    assert_eq!(persisted_workdir.workdir_mode, LaneWorkdirMode::NativeCow);
    assert_eq!(
        persisted_workdir.workdir_backend,
        Some(WorkdirBackend::Clone)
    );
    assert!(persisted_workdir.materialization.is_some());

    let sparse_spawn = run_trail_json(
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
    assert_eq!(sparse_spawn["workdir_mode"], "sparse");
    assert_eq!(sparse_spawn["workdir_backend"], "clone");
    assert_eq!(sparse_spawn["materialization"]["cloned_files"], 1);
    assert_eq!(sparse_spawn["materialization"]["copied_files"], 0);
    assert_eq!(
        sparse_spawn["sparse_paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|path| path.as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["README.md"]
    );
    let sparse_workdir = PathBuf::from(sparse_spawn["workdir"].as_str().unwrap());
    assert!(sparse_workdir.join("README.md").is_file());
    assert!(!sparse_workdir.join("src/lib.rs").exists());
    assert!(!sparse_workdir.join("src/claimed.rs").exists());
    let sparse_clean = db.lane_status("sparse-bot").unwrap();
    assert_eq!(sparse_clean.workdir_state, Some(WorktreeState::Clean));
    assert!(sparse_clean.workdir_changed_paths.is_empty());
    let claimed_hydration = run_trail_json(
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
    let sparse_read = run_trail_json(
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
    let sparse_hydrate = run_trail_json(
        temp.path(),
        &["lane", "hydrate", "sparse-bot", "src/lib.rs"],
    );
    assert_eq!(sparse_hydrate["forced"], false);
    assert!(sparse_workdir.join("src/lib.rs").is_file());
    let sparse_read_hydrate = run_trail_json(
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
    let sparse_dir_spawn = run_trail_json(
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
        trail::FileChangeKind::Deleted
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
        trail::FileChangeKind::Deleted
    );
    assert_eq!(
        db.lane_status("sparse-dir-bot").unwrap().workdir_state,
        Some(WorktreeState::Clean)
    );
    let sparse_neighbor_spawn = run_trail_json(
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
    let hydrated = run_trail_json(
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
    let sparse_manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(sparse_workdir.join(".trail/sparse-selection.json")).unwrap(),
    )
    .unwrap();
    let sparse_manifest_paths = sparse_manifest["materialized_paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| path.as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert!(sparse_manifest_paths.contains("README.md"));
    assert!(sparse_manifest_paths.contains("src/lib.rs"));
    let clean_manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(sparse_workdir.join(".trail/workdir-manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        clean_manifest["root_id"],
        sparse_hydrated.lane.branch.head_root.0
    );
    assert!(clean_manifest["files"].get("src/lib.rs").is_some());
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
    fs::remove_file(sparse_workdir.join(".trail/workdir-manifest.json")).unwrap();
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
    let api_response = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes",
            serde_json::json!({
                "name": "api-bot",
                "from_ref": "main",
                "workdir_mode": "native-cow",
                "workdir": api_workdir
            }),
        ),
    );
    assert_eq!(api_response.status, 201);
    let api_spawn: serde_json::Value = api_response.body_json().unwrap();
    assert_eq!(api_spawn["workdir_mode"], "native-cow");
    assert_eq!(
        PathBuf::from(api_spawn["workdir"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        api_workdir.canonicalize().unwrap()
    );
    assert!(api_workdir.join("README.md").is_file());

    let api_sparse_workdir = workdir_parent.path().join("api-sparse-bot");
    let api_sparse_response = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes",
            serde_json::json!({
                "name": "api-sparse-bot",
                "from_ref": "main",
                "workdir_mode": "sparse",
                "workdir": api_sparse_workdir,
                "paths": ["README.md"]
            }),
        ),
    );
    assert_eq!(api_sparse_response.status, 201);
    let api_sparse_spawn: serde_json::Value = api_sparse_response.body_json().unwrap();
    assert_eq!(api_sparse_spawn["workdir_mode"], "sparse");
    assert!(api_sparse_workdir.join("README.md").is_file());
    assert!(!api_sparse_workdir.join("src/lib.rs").exists());
    let api_sparse_lane = api_sparse_spawn["lane_id"].as_str().unwrap();
    let api_sparse_read = trail::server::handle_http_request(
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
    let api_sparse_read_hydrate = trail::server::handle_http_request(
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
    let api_sparse_sync = trail::server::handle_http_request(
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
    let mcp_spawn = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "trail.lane_spawn",
                "arguments": {
                    "name": "mcp-bot",
                    "from_ref": "main",
                    "workdir_mode": "native-cow",
                    "workdir": mcp_workdir
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_spawn["result"]["isError"], false);
    assert_eq!(
        mcp_spawn["result"]["structuredContent"]["workdir_mode"],
        "native-cow"
    );
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
    let mcp_read = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "trail.read_file",
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

    db.config_set("lane.worktrees_dir", ".trail/custom-worktrees")
        .unwrap();
    let configured = db
        .spawn_lane("configured-bot", Some("main"), true, None, None)
        .unwrap();
    let configured_workdir = PathBuf::from(configured.workdir.unwrap());
    assert!(configured_workdir.ends_with(".trail/custom-worktrees/configured-bot"));
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

#[cfg(unix)]
#[test]
fn sparse_lane_path_sync_rolls_back_hydrated_files_when_manifest_update_fails() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(
        temp.path().join("src/lib.rs"),
        "pub fn answer() -> u8 { 42 }\n",
    )
    .unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let workdir_parent = tempfile::tempdir().unwrap();
    let sparse_workdir = workdir_parent.path().join("sparse-rollback");
    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane_with_workdir_paths(
        "sparse-rollback",
        Some("main"),
        true,
        None,
        None,
        Some(sparse_workdir.clone()),
        &["README.md".to_string()],
    )
    .unwrap();
    assert!(sparse_workdir.join("README.md").is_file());
    assert!(!sparse_workdir.join("src/lib.rs").exists());

    let sparse_manifest_path = sparse_workdir.join(".trail/sparse-selection.json");
    let clean_manifest_path = sparse_workdir.join(".trail/workdir-manifest.json");
    let sparse_manifest_before = fs::read(&sparse_manifest_path).unwrap();
    let clean_manifest_before = fs::read(&clean_manifest_path).unwrap();

    trail::test_support::set_sparse_selection_write_failure_for_current_thread(true);
    let sync_result =
        db.sync_lane_workdir_with_paths("sparse-rollback", false, &["src/lib.rs".to_string()]);
    trail::test_support::set_sparse_selection_write_failure_for_current_thread(false);

    let err = sync_result.unwrap_err();
    assert!(matches!(err, Error::Io(_)));
    assert!(!sparse_workdir.join("src/lib.rs").exists());
    assert_eq!(
        fs::read(&sparse_manifest_path).unwrap(),
        sparse_manifest_before
    );
    assert_eq!(
        fs::read(&clean_manifest_path).unwrap(),
        clean_manifest_before
    );
    let status = db.lane_status("sparse-rollback").unwrap();
    assert_eq!(status.workdir_state, Some(WorktreeState::Clean));
    assert!(status.workdir_changed_paths.is_empty());
}

#[test]
fn sparse_lane_path_sync_recovers_when_sparse_manifest_is_missing() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(
        temp.path().join("src/lib.rs"),
        "pub fn answer() -> u8 { 42 }\n",
    )
    .unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let workdir_parent = tempfile::tempdir().unwrap();
    let sparse_workdir = workdir_parent.path().join("sparse-missing-manifest");
    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane_with_workdir_paths(
        "sparse-missing-manifest",
        Some("main"),
        true,
        None,
        None,
        Some(sparse_workdir.clone()),
        &["README.md".to_string()],
    )
    .unwrap();
    assert!(sparse_workdir.join("README.md").is_file());
    assert!(!sparse_workdir.join("src/lib.rs").exists());

    let sparse_manifest_path = sparse_workdir.join(".trail/sparse-selection.json");
    fs::remove_file(&sparse_manifest_path).unwrap();
    let report = db
        .sync_lane_workdir_with_paths(
            "sparse-missing-manifest",
            false,
            &["src/lib.rs".to_string()],
        )
        .unwrap();
    assert!(report.changed_paths.is_empty());
    assert_eq!(
        fs::read_to_string(sparse_workdir.join("src/lib.rs")).unwrap(),
        "pub fn answer() -> u8 { 42 }\n"
    );

    let sparse_manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&sparse_manifest_path).unwrap()).unwrap();
    let materialized_paths = sparse_manifest["materialized_paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| path.as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        materialized_paths,
        BTreeSet::from(["README.md", "src/lib.rs"])
    );
    assert_eq!(
        db.lane_status("sparse-missing-manifest")
            .unwrap()
            .workdir_state,
        Some(WorktreeState::Clean)
    );
}

#[test]
fn large_roots_default_lanes_to_no_materialize() {
    let temp = tempfile::tempdir().unwrap();
    let files_dir = temp.path().join("files");
    fs::create_dir_all(&files_dir).unwrap();
    for idx in 0..=10_000 {
        fs::write(files_dir.join(format!("file-{idx:05}.txt")), "tiny\n").unwrap();
    }
    let init = Trail::init_with_text_policy(
        temp.path(),
        "main",
        InitImportMode::WorkingTree,
        false,
        Some("minimal"),
    )
    .unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    match db.show(&init.operation.0).unwrap() {
        ShowResult::Operation { value } => {
            assert!(value.operation.changes.is_empty());
        }
        other => panic!("expected operation show result, got {other:?}"),
    }
    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
    let file_history_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM file_history", [], |row| row.get(0))
        .unwrap();
    assert_eq!(file_history_rows, 0);
    let file_history = db
        .history_for_path("files/file-00000.txt")
        .unwrap()
        .file_history;
    assert_eq!(file_history.len(), 1);
    assert_eq!(file_history[0].kind, trail::FileChangeKind::Added);
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    fs::write(temp.path().join("README.md"), "hello\ndirty\n").unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("doc-bot", Some("main"), true, None, None)
        .unwrap();
    assert_eq!(spawned.requested_workdir_mode, LaneWorkdirMode::Auto);
    assert_eq!(spawned.workdir_mode, LaneWorkdirMode::PortableCopy);
    assert_eq!(spawned.workdir_backend, Some(WorkdirBackend::Copy));
    assert_eq!(
        spawned
            .materialization
            .as_ref()
            .and_then(|report| report.fallback_reason),
        Some(MaterializationFallbackReason::NativeSourceUnavailable)
    );
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
fn auto_reports_mixed_when_portable_restart_can_clone_only_clean_files() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("clean.txt"), "clean\n").unwrap();
    fs::write(temp.path().join("dirty.txt"), "recorded\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    fs::write(temp.path().join("dirty.txt"), "changed\n").unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("mixed-bot", Some("main"), true, None, None)
        .unwrap();
    let report = spawned.materialization.unwrap();
    assert_eq!(spawned.workdir_mode, LaneWorkdirMode::PortableCopy);
    assert_eq!(spawned.workdir_backend, Some(WorkdirBackend::Mixed));
    assert_eq!(report.cloned_files, 1);
    assert_eq!(report.copied_files, 1);
    assert_eq!(
        report.fallback_reason,
        Some(MaterializationFallbackReason::NativeSourceUnavailable)
    );
    let workdir = PathBuf::from(spawned.workdir.unwrap());
    assert_eq!(
        fs::read_to_string(workdir.join("clean.txt")).unwrap(),
        "clean\n"
    );
    assert_eq!(
        fs::read_to_string(workdir.join("dirty.txt")).unwrap(),
        "recorded\n"
    );
}

#[test]
fn strict_native_cow_refuses_an_unvalidated_source_without_copying() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    fs::write(temp.path().join("README.md"), "hello\ndirty\n").unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let workdir_parent = tempfile::tempdir().unwrap();
    let destination = workdir_parent.path().join("strict-workdir");
    let error = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "strict-bot",
            Some("main"),
            LaneWorkdirMode::NativeCow,
            None,
            None,
            Some(destination.clone()),
            &[],
            false,
        )
        .unwrap_err();

    assert!(matches!(error, Error::NativeCowSourceUnavailable));
    assert!(!destination.exists());
    assert!(db.lane_details("strict-bot").is_err());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn strict_native_cow_accepts_a_git_tracked_file_inside_an_ignored_directory() {
    let temp = tempfile::tempdir().unwrap();
    run_git(temp.path(), &["init", "--quiet"]);
    run_git(temp.path(), &["config", "user.name", "Trail COW"]);
    run_git(
        temp.path(),
        &["config", "user.email", "trail-cow@example.invalid"],
    );
    fs::create_dir(temp.path().join("generated")).unwrap();
    fs::write(temp.path().join("generated/tracked.txt"), "tracked\n").unwrap();
    fs::write(temp.path().join(".gitignore"), ".trail/\ngenerated/\n").unwrap();
    run_git(temp.path(), &["add", ".gitignore"]);
    run_git(temp.path(), &["add", "--force", "generated/tracked.txt"]);
    run_git(temp.path(), &["commit", "--quiet", "-m", "base"]);
    Trail::init(temp.path(), "main", InitImportMode::GitTracked, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "tracked-ignore-cow",
            Some("main"),
            LaneWorkdirMode::NativeCow,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
    assert_eq!(spawned.workdir_mode, LaneWorkdirMode::NativeCow);
    assert_eq!(spawned.workdir_backend, Some(WorkdirBackend::Clone));
    let workdir = PathBuf::from(spawned.workdir.unwrap());
    assert_eq!(
        fs::read_to_string(workdir.join("generated/tracked.txt")).unwrap(),
        "tracked\n"
    );

    fs::write(workdir.join("agent-output.txt"), "agent output\n").unwrap();
    fs::remove_file(workdir.join(".trail/workdir-manifest.json")).unwrap();
    let preview = db
        .preview_lane_workdir_record("tracked-ignore-cow")
        .unwrap();
    assert_eq!(
        preview
            .changed_paths
            .iter()
            .map(|path| (path.path.as_str(), path.kind.clone()))
            .collect::<Vec<_>>(),
        vec![("agent-output.txt", trail::FileChangeKind::Added)],
        "the authoritative lane comparison must retain ignored baseline files"
    );

    let recorded = db
        .record_lane_workdir(
            "tracked-ignore-cow",
            Some("record agent output after marker loss".into()),
        )
        .unwrap();
    assert_eq!(
        recorded
            .changed_paths
            .iter()
            .map(|path| (path.path.as_str(), path.kind.clone()))
            .collect::<Vec<_>>(),
        vec![("agent-output.txt", trail::FileChangeKind::Added)],
        "the mutating lane record path must retain ignored baseline files"
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn strict_native_cow_accepts_a_non_git_baseline_file_inside_an_ignored_directory() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("generated")).unwrap();
    fs::write(temp.path().join("generated/baseline.txt"), "baseline\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    fs::write(temp.path().join(".trailignore"), "generated/\n").unwrap();
    fs::write(temp.path().join("generated/untracked.txt"), "untracked\n").unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "non-git-ignore-cow",
            Some("main"),
            LaneWorkdirMode::NativeCow,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
    assert_eq!(spawned.workdir_mode, LaneWorkdirMode::NativeCow);
    assert_eq!(spawned.workdir_backend, Some(WorkdirBackend::Clone));
    let workdir = PathBuf::from(spawned.workdir.unwrap());
    assert_eq!(
        fs::read_to_string(workdir.join("generated/baseline.txt")).unwrap(),
        "baseline\n"
    );
    assert!(
        !workdir.join("generated/untracked.txt").exists(),
        "ignored untracked files must remain outside the immutable materialization root"
    );
}

#[test]
fn non_git_ignore_cannot_hide_a_trail_baseline_file() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("generated")).unwrap();
    fs::write(temp.path().join("generated/tracked.txt"), "baseline\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    fs::write(temp.path().join(".trailignore"), "generated/\n").unwrap();
    let mut db = Trail::open(temp.path()).unwrap();

    let status = db.status(None).unwrap();
    assert!(
        status.changed_paths.is_empty(),
        "a present immutable Trail baseline file must not become a deletion when ignored: {:?}",
        status.changed_paths
    );

    let recorded = db
        .record(
            None,
            Some("ignore existing baseline".into()),
            Actor::human(),
            false,
        )
        .unwrap();
    assert!(recorded.operation.is_none());
    assert!(recorded.changed_paths.is_empty());
    assert_eq!(
        fs::read_to_string(temp.path().join("generated/tracked.txt")).unwrap(),
        "baseline\n"
    );
}

#[test]
fn strict_native_cow_reuses_a_complete_clean_lane_source() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("source-bot", Some("main"), true, None, None)
        .unwrap();
    fs::write(temp.path().join("README.md"), "hello\ndirty\n").unwrap();
    assert_eq!(
        db.lane_status("source-bot").unwrap().workdir_state,
        Some(WorktreeState::Clean)
    );

    let spawned = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "strict-from-lane",
            Some("main"),
            LaneWorkdirMode::NativeCow,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
    assert_eq!(spawned.workdir_mode, LaneWorkdirMode::NativeCow);
    assert_eq!(spawned.workdir_backend, Some(WorkdirBackend::Clone));
    assert_eq!(
        fs::read_to_string(PathBuf::from(spawned.workdir.unwrap()).join("README.md")).unwrap(),
        "hello\n"
    );
}

#[cfg(unix)]
#[test]
fn strict_native_cow_does_not_preserve_source_hardlink_aliasing() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), "shared\n").unwrap();
    fs::hard_link(temp.path().join("a.txt"), temp.path().join("b.txt")).unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "hardlink-bot",
            Some("main"),
            LaneWorkdirMode::NativeCow,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
    let workdir = PathBuf::from(spawned.workdir.unwrap());
    let a = fs::metadata(workdir.join("a.txt")).unwrap();
    let b = fs::metadata(workdir.join("b.txt")).unwrap();
    assert_ne!(a.ino(), b.ino());
    fs::write(workdir.join("a.txt"), "changed\n").unwrap();
    assert_eq!(
        fs::read_to_string(workdir.join("b.txt")).unwrap(),
        "shared\n"
    );
}

#[test]
fn strict_native_cow_probes_an_empty_root() {
    let temp = tempfile::tempdir().unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "empty-bot",
            Some("main"),
            LaneWorkdirMode::NativeCow,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
    assert_eq!(spawned.workdir_mode, LaneWorkdirMode::NativeCow);
    assert_eq!(spawned.workdir_backend, Some(WorkdirBackend::Clone));
    assert_eq!(spawned.materialization.unwrap().cloned_files, 0);
}

#[test]
fn legacy_cow_backend_is_not_treated_as_strict_clone_evidence() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("legacy-bot", Some("main"), true, None, None)
        .unwrap();
    drop(db);

    let connection = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
    connection
        .execute(
            "UPDATE lanes SET metadata_json = ?1 WHERE name = 'legacy-bot'",
            [r#"{"workdir_mode":"native-cow","cow_backend":"clone"}"#],
        )
        .unwrap();
    drop(connection);

    let db = Trail::open(temp.path()).unwrap();
    let report = db.lane_workdir("legacy-bot").unwrap();
    assert_eq!(report.workdir_mode, LaneWorkdirMode::NativeCow);
    assert_eq!(report.requested_workdir_mode, LaneWorkdirMode::NativeCow);
    assert_eq!(report.workdir_backend, None);
    assert_eq!(report.materialization, None);
}

#[test]
fn lane_workdir_sync_refuses_dirty_and_force_refreshes() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("doc-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = spawned.workdir.unwrap();
    let workdir_path = PathBuf::from(&workdir);
    let workdir_parent = workdir_path.parent().unwrap().to_path_buf();
    let workdir_stage_prefix = format!(
        ".{}.trail-sync-",
        workdir_path.file_name().unwrap().to_string_lossy()
    );
    let assert_no_sync_stage_dirs = || {
        let staged = fs::read_dir(&workdir_parent)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.starts_with(&workdir_stage_prefix))
            .collect::<Vec<_>>();
        assert!(staged.is_empty(), "leftover sync staging dirs: {staged:?}");
    };
    let readme = std::path::Path::new(&workdir).join("README.md");
    fs::write(&readme, "hello\ndirty\n").unwrap();

    let err = db.sync_lane_workdir("doc-bot", false).unwrap_err();
    assert!(matches!(err, Error::DirtyWorktreeWithMessage(_)));
    drop(db);

    let synced = run_trail_json(temp.path(), &["lane", "sync-workdir", "doc-bot", "--force"]);
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
    assert_no_sync_stage_dirs();

    let db = Trail::open(temp.path()).unwrap();
    let status = db.lane_status("doc-bot").unwrap();
    assert_eq!(status.workdir_state, Some(WorktreeState::Clean));
    drop(db);

    #[cfg(unix)]
    {
        let inode_before = fs::metadata(&readme).unwrap().ino();
        let clean_sync = run_trail_json(temp.path(), &["lane", "sync-workdir", "doc-bot"]);
        assert_eq!(clean_sync["forced"], false);
        assert!(clean_sync.get("rescue_workdir").is_none());
        assert!(clean_sync["changed_paths"].as_array().unwrap().is_empty());
        assert_eq!(fs::metadata(&readme).unwrap().ino(), inode_before);
    }

    fs::remove_dir_all(&workdir).unwrap();
    let recreated = run_trail_json(temp.path(), &["lane", "sync-workdir", "doc-bot"]);
    assert_eq!(recreated["forced"], false);
    assert_eq!(fs::read_to_string(&readme).unwrap(), "hello\n");
    assert_no_sync_stage_dirs();

    fs::remove_dir_all(&workdir).unwrap();
    fs::write(&workdir, "not a directory\n").unwrap();
    let replaced = run_trail_json(temp.path(), &["lane", "sync-workdir", "doc-bot", "--force"]);
    assert_eq!(replaced["forced"], true);
    let replaced_rescue = PathBuf::from(replaced["rescue_workdir"].as_str().unwrap());
    assert!(replaced_rescue.is_dir());
    assert_eq!(
        fs::read_to_string(replaced_rescue.join("files").join("doc-bot")).unwrap(),
        "not a directory\n"
    );
    let replaced_rescue_manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(replaced_rescue.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(replaced_rescue_manifest["lane"], "doc-bot");
    assert_eq!(replaced_rescue_manifest["replaced_workdir_path"], true);
    assert!(replaced_rescue_manifest["copied_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path == "doc-bot"));
    assert_eq!(fs::read_to_string(&readme).unwrap(), "hello\n");
    assert_no_sync_stage_dirs();

    #[cfg(unix)]
    {
        fs::remove_dir_all(&workdir).unwrap();
        let symlink_target = workdir_parent.join("external-symlink-target");
        fs::create_dir_all(&symlink_target).unwrap();
        std::os::unix::fs::symlink(&symlink_target, &workdir).unwrap();
        let replaced_symlink =
            run_trail_json(temp.path(), &["lane", "sync-workdir", "doc-bot", "--force"]);
        assert_eq!(replaced_symlink["forced"], true);
        let symlink_rescue = PathBuf::from(replaced_symlink["rescue_workdir"].as_str().unwrap());
        assert!(symlink_rescue.is_dir());
        let symlink_rescue_manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(symlink_rescue.join("manifest.json")).unwrap())
                .unwrap();
        assert_eq!(symlink_rescue_manifest["lane"], "doc-bot");
        assert_eq!(symlink_rescue_manifest["replaced_workdir_path"], true);
        assert_eq!(
            symlink_rescue_manifest["symlink_target"].as_str().unwrap(),
            symlink_target.to_string_lossy().as_ref()
        );
        assert!(symlink_rescue_manifest["skipped_paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "doc-bot: symlink"));
        assert_eq!(fs::read_to_string(&readme).unwrap(), "hello\n");
        assert_no_sync_stage_dirs();
    }
}

#[test]
#[cfg(unix)]
fn lane_workdir_sync_refuses_executable_bit_dirty_and_force_rescues() {
    let temp = tempfile::tempdir().unwrap();
    let script = temp.path().join("script.sh");
    fs::write(&script, "#!/bin/sh\necho hello\n").unwrap();
    let mut permissions = fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o644);
    fs::set_permissions(&script, permissions).unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("mode-sync-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = PathBuf::from(spawned.workdir.unwrap());
    let workdir_script = workdir.join("script.sh");
    assert_eq!(
        fs::metadata(&workdir_script).unwrap().permissions().mode() & 0o111,
        0
    );

    let mut permissions = fs::metadata(&workdir_script).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&workdir_script, permissions).unwrap();

    let err = db.sync_lane_workdir("mode-sync-bot", false).unwrap_err();
    assert!(matches!(err, Error::DirtyWorktreeWithMessage(_)));
    let status = db.lane_status("mode-sync-bot").unwrap();
    assert_eq!(status.workdir_state, Some(WorktreeState::DirtyTracked));
    assert_eq!(status.workdir_changed_paths.len(), 1);
    let changed = &status.workdir_changed_paths[0];
    assert_eq!(changed.path, "script.sh");
    assert_eq!(changed.kind, trail::FileChangeKind::Modified);
    assert_eq!(changed.before_hash, changed.after_hash);

    let synced = db.sync_lane_workdir("mode-sync-bot", true).unwrap();
    assert!(synced.forced);
    assert_eq!(synced.changed_paths.len(), 1);
    assert_eq!(synced.changed_paths[0].path, "script.sh");
    assert_eq!(
        synced.changed_paths[0].before_hash,
        synced.changed_paths[0].after_hash
    );
    let rescue = PathBuf::from(synced.rescue_workdir.unwrap());
    let rescued_script = rescue.join("files").join("script.sh");
    assert_eq!(
        fs::read_to_string(&rescued_script).unwrap(),
        "#!/bin/sh\necho hello\n"
    );
    assert_ne!(
        fs::metadata(rescued_script).unwrap().permissions().mode() & 0o111,
        0
    );
    assert_eq!(
        fs::metadata(&workdir_script).unwrap().permissions().mode() & 0o111,
        0
    );
    assert_eq!(
        db.lane_status("mode-sync-bot").unwrap().workdir_state,
        Some(WorktreeState::Clean)
    );
}

#[test]
fn lane_workdir_watch_records_only_lane_branch() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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

    db.enqueue_lane_merge("doc-bot", "main", 0).unwrap();
    let run = db.run_lane_merge_queue(None).unwrap();
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
#[cfg(unix)]
fn dirty_lane_workdir_records_executable_bit_changes() {
    let temp = tempfile::tempdir().unwrap();
    let script = temp.path().join("script.sh");
    fs::write(&script, "#!/bin/sh\necho hello\n").unwrap();
    let mut permissions = fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o644);
    fs::set_permissions(&script, permissions).unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("mode-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = PathBuf::from(spawned.workdir.unwrap());
    let workdir_script = workdir.join("script.sh");
    assert_eq!(
        fs::metadata(&workdir_script).unwrap().permissions().mode() & 0o111,
        0
    );

    let mut permissions = fs::metadata(&workdir_script).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&workdir_script, permissions).unwrap();

    let status = db.lane_status("mode-bot").unwrap();
    assert_eq!(status.workdir_state, Some(WorktreeState::DirtyTracked));
    assert_eq!(status.workdir_changed_paths.len(), 1);
    let changed = &status.workdir_changed_paths[0];
    assert_eq!(changed.path, "script.sh");
    assert_eq!(changed.kind, trail::FileChangeKind::Modified);
    assert_eq!(changed.before_hash, changed.after_hash);

    let preview = db.preview_lane_workdir_record("mode-bot").unwrap();
    assert!(!preview.clean);
    assert_eq!(preview.changed_paths.len(), 1);
    assert_eq!(preview.changed_paths[0].path, "script.sh");
    assert_eq!(
        preview.changed_paths[0].before_hash,
        preview.changed_paths[0].after_hash
    );

    let recorded = db
        .record_lane_workdir("mode-bot", Some("record executable bit".to_string()))
        .unwrap();
    assert!(recorded.operation.is_some());
    assert_eq!(recorded.changed_paths.len(), 1);
    assert_eq!(recorded.changed_paths[0].path, "script.sh");
    assert_eq!(
        db.lane_status("mode-bot").unwrap().workdir_state,
        Some(WorktreeState::Clean)
    );

    db.merge_lane("mode-bot", "main").unwrap();
    db.checkout("main", true).unwrap();
    assert_ne!(
        fs::metadata(temp.path().join("script.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
}

#[cfg(unix)]
#[test]
fn materialized_lane_status_detects_manifest_candidate_paths() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    fs::write(temp.path().join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("doc-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = PathBuf::from(spawned.workdir.unwrap());
    let readme_metadata = fs::symlink_metadata(workdir.join("README.md")).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(workdir.join(".trail/workdir-manifest.json")).unwrap(),
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
        Some(&trail::FileChangeKind::Modified)
    );
    assert_eq!(
        changed.get("src/lib.rs"),
        Some(&trail::FileChangeKind::Deleted)
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
        Some(&trail::FileChangeKind::Modified)
    );
    assert_eq!(
        recorded_paths.get("src/lib.rs"),
        Some(&trail::FileChangeKind::Deleted)
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let spawned = db
        .spawn_lane("doc-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = PathBuf::from(spawned.workdir.unwrap());
    let manifest = workdir.join(".trail/workdir-manifest.json");
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
        Some(&trail::FileChangeKind::Modified)
    );
    assert_eq!(
        changed.get("src/lib.rs"),
        Some(&trail::FileChangeKind::Deleted)
    );
    assert_eq!(
        changed.get("src/new.rs"),
        Some(&trail::FileChangeKind::Added)
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
        Some(&trail::FileChangeKind::Modified)
    );
    assert_eq!(
        recorded_paths.get("src/lib.rs"),
        Some(&trail::FileChangeKind::Deleted)
    );
    assert_eq!(
        recorded_paths.get("src/new.rs"),
        Some(&trail::FileChangeKind::Added)
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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

    let blocked_rename: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "rename", "from": "README.md", "to": "src/renamed.md"}
        ]
    }))
    .unwrap();
    let err = apply_lane_patch_at_head(&mut db, "sparse-bot", blocked_rename).unwrap_err();
    assert!(matches!(err, Error::PatchRejected(message) if message.contains("src/renamed.md")));

    let sparse_manifest = workdir.join(".trail/sparse-selection.json");
    fs::remove_file(&sparse_manifest).unwrap();
    let blocked_without_manifest: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write", "path": "src/lib.rs", "content": "pub fn bypass() {}\n"}
        ]
    }))
    .unwrap();
    let err =
        apply_lane_patch_at_head(&mut db, "sparse-bot", blocked_without_manifest).unwrap_err();
    assert!(
        matches!(err, Error::PatchRejected(message) if message.contains("sparse path boundary"))
    );

    let allowed_without_manifest: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nallowed\nagain\n"}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "sparse-bot", allowed_without_manifest).unwrap();
    assert!(sparse_manifest.exists());
    assert!(!workdir.join("src/lib.rs").exists());

    fs::remove_file(&sparse_manifest).unwrap();
    fs::write(workdir.join("EXTRA.md"), "outside sparse selection\n").unwrap();
    let preview = db.preview_lane_workdir_record("sparse-bot").unwrap();
    assert!(!preview.policy.allowed);
    assert!(preview
        .policy
        .error
        .as_deref()
        .is_some_and(|message| message.contains("sparse path boundary")));
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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

    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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

    let spawned = db
        .spawn_lane("record-claim-bot", Some("main"), true, None, None)
        .unwrap();
    let workdir = PathBuf::from(spawned.workdir.unwrap());
    db.claim_lane_path("record-claim-bot", "README.md", 600)
        .unwrap();
    db.config_set("lane.claim_enforcement", "reject").unwrap();
    fs::write(
        workdir.join("src/lib.rs"),
        "pub fn outside_record_claim() {}\n",
    )
    .unwrap();

    let preview = db.preview_lane_workdir_record("record-claim-bot").unwrap();
    assert!(!preview.policy.allowed);
    assert!(preview
        .policy
        .error
        .as_deref()
        .unwrap()
        .contains("outside active write claims"));
    let err = db
        .record_lane_workdir("record-claim-bot", Some("record outside claim".to_string()))
        .unwrap_err();
    assert!(
        matches!(err, Error::PatchRejected(message) if message.contains("outside active write claims"))
    );

    db.config_set("lane.claim_enforcement", "warn").unwrap();
    let recorded = db
        .record_lane_workdir("record-claim-bot", Some("warn outside claim".to_string()))
        .unwrap();
    assert!(recorded.operation.is_some());
    assert!(recorded
        .changed_paths
        .iter()
        .any(|path| path.path == "src/lib.rs"));
    let record_warnings = db
        .list_lane_events(
            Some("record-claim-bot"),
            None,
            None,
            Some("lane_policy_warning"),
            10,
        )
        .unwrap();
    assert_eq!(record_warnings.len(), 1);
    assert_eq!(
        record_warnings[0].payload.as_ref().unwrap()["paths"][0],
        "src/lib.rs"
    );
}

#[test]
fn claim_enforcement_counts_write_leases_as_claim_boundaries() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("docs")).unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    fs::write(temp.path().join("docs/notes.md"), "notes\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("lease-policy-bot", Some("main"), false, None, None)
        .unwrap();
    db.acquire_lease("lease-policy-bot", Some("README.md"), "write", 600)
        .unwrap();
    db.acquire_lease("lease-policy-bot", Some("docs/notes.md"), "read", 600)
        .unwrap();
    db.config_set("lane.claim_enforcement", "reject").unwrap();

    let leased_patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nleased\n"}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "lease-policy-bot", leased_patch).unwrap();

    let read_leased_patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "write", "path": "docs/notes.md", "content": "notes\nchanged\n"}
        ]
    }))
    .unwrap();
    let err = apply_lane_patch_at_head(&mut db, "lease-policy-bot", read_leased_patch.clone())
        .unwrap_err();
    assert!(
        matches!(err, Error::PatchRejected(message) if message.contains("outside active write claims"))
    );

    db.acquire_lease("lease-policy-bot", None, "write", 600)
        .unwrap();
    apply_lane_patch_at_head(&mut db, "lease-policy-bot", read_leased_patch).unwrap();
}

#[test]
fn lane_event_and_trace_payload_limits_are_enforced() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("payload-bot", Some("main"), false, None, None)
        .unwrap();
    let session = db
        .start_lane_session(
            "payload-bot",
            Some("Payload limits".to_string()),
            Some("session-payload".to_string()),
        )
        .unwrap();
    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
    let count_events = |event_type: &str| -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM lane_events WHERE event_type = ?1",
            [event_type],
            |row| row.get(0),
        )
        .unwrap()
    };
    let count_trace_index_rows = || -> i64 {
        conn.query_row("SELECT COUNT(*) FROM lane_trace_span_events", [], |row| {
            row.get(0)
        })
        .unwrap()
    };

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
    assert_eq!(count_events("large_payload"), 0);

    let oversized_secret = "s".repeat(128);
    let err = db
        .add_lane_session_event(
            "payload-bot",
            &session.session.session_id,
            "large_secret_payload",
            Some(serde_json::json!({
                "api_key": oversized_secret
            })),
        )
        .unwrap_err();
    assert!(
        matches!(err, Error::InvalidInput(message) if message.contains("max_event_payload_bytes"))
    );
    assert_eq!(count_events("large_secret_payload"), 0);

    db.config_set("lane.max_event_payload_bytes", "0").unwrap();
    let turn = db
        .begin_lane_session_turn("payload-bot", &session.session.session_id, None)
        .unwrap();
    db.config_set("lane.max_trace_payload_bytes", "512")
        .unwrap();
    let started_before = count_events("span_started");
    let trace_index_before = count_trace_index_rows();
    let oversized_trace_secret = "s".repeat(2048);
    let err = db
        .start_lane_trace_span(
            &turn.turn.turn_id,
            "tool",
            "large secret trace payload",
            None,
            None,
            Some(serde_json::json!({
                "api_key": oversized_trace_secret
            })),
        )
        .unwrap_err();
    assert!(
        matches!(err, Error::InvalidInput(message) if message.contains("max_trace_payload_bytes"))
    );
    assert_eq!(count_events("span_started"), started_before);
    assert_eq!(count_trace_index_rows(), trace_index_before);

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
    assert_eq!(count_events("span_started"), started_before);
    assert_eq!(count_trace_index_rows(), trace_index_before);

    db.config_set("lane.max_trace_payload_bytes", "0").unwrap();
    let span = db
        .start_lane_trace_span(
            &turn.turn.turn_id,
            "tool",
            "small trace payload",
            None,
            None,
            None,
        )
        .unwrap();
    assert_eq!(count_events("span_started"), started_before + 1);
    assert_eq!(count_trace_index_rows(), trace_index_before + 1);

    db.config_set("lane.max_trace_payload_bytes", "16").unwrap();
    let ended_before = count_events("span_ended");
    let err = db
        .end_lane_trace_span(
            &span.span.span_id,
            "completed",
            Some(serde_json::json!({
                "long": "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz"
            })),
        )
        .unwrap_err();
    assert!(
        matches!(err, Error::InvalidInput(message) if message.contains("max_trace_payload_bytes"))
    );
    assert_eq!(count_events("span_ended"), ended_before);
    assert_eq!(count_trace_index_rows(), trace_index_before + 1);
    assert_eq!(
        db.show_lane_trace_span(&span.span.span_id).unwrap().status,
        "running"
    );
}

#[test]
fn lane_claims_are_soft_leases_across_cli_api_and_mcp() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("doc-bot", Some("main"), false, None, None)
        .unwrap();
    db.spawn_lane("test-bot", Some("main"), false, None, None)
        .unwrap();
    drop(db);

    let cli_claim = run_trail_json(
        temp.path(),
        &["lane", "claim", "doc-bot", "README.md", "--ttl-secs", "120"],
    );
    assert_eq!(cli_claim["claimed"], true);
    assert_eq!(cli_claim["path"], "README.md");
    assert_eq!(cli_claim["lease"]["mode"], "write");
    let cli_lease_id = cli_claim["lease"]["lease_id"].as_str().unwrap().to_string();

    let mut db = Trail::open(temp.path()).unwrap();
    let api_conflict = trail::server::handle_http_request(
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

    let tools = trail::mcp::handle_json_rpc(
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
        .any(|tool| tool["name"] == "trail.lane_claim"));

    let mcp_conflict = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "trail.lane_claim",
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

    let same_claim = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "trail.lane_claim",
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("doc-bot", Some("main"), false, None, None)
        .unwrap();
    db.spawn_lane("test-bot", Some("main"), false, None, None)
        .unwrap();

    let acquired = trail::server::handle_http_request(
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

    let listed = trail::server::handle_http_request(
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

    let tools = trail::mcp::handle_json_rpc(
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
        "trail.lease_acquire",
        "trail.lease_list",
        "trail.lease_release",
    ] {
        assert!(tool_list.iter().any(|tool| tool["name"] == name), "{name}");
    }

    let conflicting = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "trail.lease_acquire",
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

    let released = trail::server::handle_http_request(
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

    let read_lease = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "trail.lease_acquire",
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

    let all_leases = trail::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/leases?all=true", serde_json::Value::Null),
    );
    assert_eq!(all_leases.status, 200);
    let all_leases: serde_json::Value = all_leases.body_json().unwrap();
    assert_eq!(all_leases.as_array().unwrap().len(), 1);

    let mcp_list = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "trail.lease_list",
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
    let mcp_release = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "trail.lease_release",
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
fn local_http_bodyless_mutations_reject_request_bodies() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let lease = db
        .acquire_lease("doc-bot", Some("README.md"), "write", 120)
        .unwrap()
        .lease;
    let anchor = db
        .create_anchor("README.md:1", "stable readme", Some("main"))
        .unwrap()
        .anchor;
    let queued = db.enqueue_lane_merge("doc-bot", "main", 0).unwrap().entry;

    let assert_rejected = |response: &trail::server::HttpResponse, endpoint: &str| {
        assert_eq!(response.status, 400);
        let body: serde_json::Value = response.body_json().unwrap();
        let message = body["error"]["message"].as_str().unwrap();
        assert!(message.contains(endpoint), "{message}");
        assert!(
            message.contains("does not accept a request body"),
            "{message}"
        );
    };

    let bad_lease_release = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "DELETE",
            &format!("/v1/leases/{}", lease.lease_id),
            serde_json::json!({ "unexpected": true }),
        ),
    );
    assert_rejected(&bad_lease_release, "DELETE /v1/leases/{lease_id}");
    assert!(db
        .list_leases(false)
        .unwrap()
        .iter()
        .any(|item| item.lease_id == lease.lease_id));

    let bad_anchor_delete = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "DELETE",
            &format!("/v1/anchors/{}", anchor.id.0),
            serde_json::json!({ "unexpected": true }),
        ),
    );
    assert_rejected(&bad_anchor_delete, "DELETE /v1/anchors/{anchor_id}");
    assert!(db
        .list_anchors()
        .unwrap()
        .iter()
        .any(|item| item.id == anchor.id));

    let bad_queue_remove = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "DELETE",
            &format!("/v1/lanes/merges/queue/{}", queued.queue_id),
            serde_json::json!({ "unexpected": true }),
        ),
    );
    assert_rejected(
        &bad_queue_remove,
        "DELETE /v1/lanes/merges/queue/{selector}",
    );
    assert!(db
        .list_lane_merge_queue()
        .unwrap()
        .iter()
        .any(|item| item.queue_id == queued.queue_id && item.status == "queued"));

    let bad_lane_remove = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "DELETE",
            "/v1/lanes/doc-bot",
            serde_json::json!({ "unexpected": true }),
        ),
    );
    assert_rejected(&bad_lane_remove, "DELETE /v1/lanes/{lane_or_id}");
    assert_eq!(db.lane_details("doc-bot").unwrap().branch.status, "active");
}

#[test]
fn merge_lane_and_queue_enforce_readiness_blockers() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
    let approval = db
        .request_lane_approval(
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

    db.enqueue_lane_merge("approval-bot", "main", 0).unwrap();
    let explain = db.explain_lane_merge_queue("approval-bot").unwrap();
    assert!(explain
        .blockers
        .iter()
        .any(|issue| issue.code == "pending_approvals"));
    assert!(explain.next_steps.iter().any(|step| {
        step.contains("trail approvals list --lane approval-bot")
            && step.contains(&format!(
                "trail approvals decide {} --decision approved",
                approval.approval.approval_id
            ))
    }));

    let run = db.run_lane_merge_queue(None).unwrap();
    assert_eq!(run.processed.len(), 1);
    assert_eq!(run.processed[0].status, "failed");
    assert!(run.stopped_on_failure);
    assert!(run.processed[0]
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("pending_approvals"));
    assert!(!temp.path().join("docs/approval.md").exists());

    db.spawn_lane("late-approval-bot", Some("main"), false, None, None)
        .unwrap();
    let late_patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "queued before approval was requested",
        "edits": [
            {"op": "write", "path": "docs/late-approval.md", "content": "became blocked\n"}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "late-approval-bot", late_patch).unwrap();
    let late_queue = db
        .enqueue_lane_merge("late-approval-bot", "main", 0)
        .unwrap();
    assert!(db.lane_readiness("late-approval-bot").unwrap().ready);
    db.request_lane_approval(
        "late-approval-bot",
        "deploy.preview",
        "Approval added after queueing",
        None,
        None,
        None,
    )
    .unwrap();
    let late_run = db.run_lane_merge_queue(None).unwrap();
    assert_eq!(late_run.processed.len(), 1);
    assert_eq!(late_run.processed[0].queue_id, late_queue.entry.queue_id);
    assert_eq!(late_run.processed[0].status, "failed");
    assert!(late_run.stopped_on_failure);
    assert!(late_run.processed[0]
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("pending_approvals"));
    assert!(!temp.path().join("docs/late-approval.md").exists());

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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
fn lane_merge_queue_runs_lane_branch_into_main() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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

    let queued = db.enqueue_lane_merge("doc-bot", "main", 10).unwrap();
    assert_eq!(queued.entry.status, "queued");
    assert_eq!(queued.entry.lane, "doc-bot");
    assert_eq!(db.list_lane_merge_queue().unwrap().len(), 1);

    let run = db.run_lane_merge_queue(None).unwrap();
    assert_eq!(run.processed.len(), 1);
    assert_eq!(run.processed[0].status, "merged");
    assert!(!run.stopped_on_conflict);
    assert!(!run.stopped_on_failure);

    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("docs/guide.md")).unwrap(),
        "# Guide\n"
    );

    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
    let queue_status: String = conn
        .query_row(
            "SELECT status FROM lane_merge_queue WHERE queue_id = ?1",
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
fn lane_merge_queue_rejects_branch_sources() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let error = db.enqueue_lane_merge("main", "main", 0).unwrap_err();
    assert!(matches!(error, Error::InvalidInput(_)));
    assert!(error.to_string().contains("lane"));
}

#[test]
fn local_api_drives_lane_merge_queue() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("api-bot", Some("main"), false, None, None)
        .unwrap();
    let add = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes/merges/queue",
            serde_json::json!({
                "lane": "api-bot",
                "into": "main",
                "priority": 10
            }),
        ),
    );
    assert_eq!(add.status, 201);
    let add: serde_json::Value = add.body_json().unwrap();
    assert_eq!(add["entry"]["lane"], "api-bot");
    assert!(add["entry"]["queue_id"]
        .as_str()
        .unwrap()
        .starts_with("lmq_"));

    let list = trail::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/lanes/merges/queue", serde_json::Value::Null),
    );
    assert_eq!(list.status, 200);
    assert_eq!(
        list.body_json::<serde_json::Value>().unwrap()[0]["lane"],
        "api-bot"
    );

    let aliased_body = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes/merges/queue",
            serde_json::json!({"source": "api-bot", "target": "main"}),
        ),
    );
    assert_eq!(aliased_body.status, 400);

    let legacy = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/merge-queue",
            serde_json::json!({"source": "api-bot", "target": "main"}),
        ),
    );
    assert_eq!(legacy.status, 400);

    let openapi = trail::server::openapi_spec();
    assert!(openapi["paths"].get("/v1/lanes/merges/queue").is_some());
    assert!(openapi["paths"].get("/v1/merge-queue").is_none());
    assert_eq!(
        openapi["components"]["schemas"]["LaneMergeQueueAddRequest"]["required"],
        serde_json::json!(["lane", "into"])
    );
}

#[test]
fn lane_merge_queue_explain_reports_dry_run_conflicts_without_recording_conflict_state() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
    let queued = db.enqueue_lane_merge("explain-bot", "main", 0).unwrap();
    let queue_id = queued.entry.queue_id.clone();

    let explain = db.explain_lane_merge_queue("explain-bot").unwrap();
    assert!(explain
        .blockers
        .iter()
        .any(|issue| issue.code == "merge_conflicts"));
    assert_eq!(
        explain.dry_run.as_ref().unwrap().conflicts,
        vec!["both changed `README.md` differently"]
    );
    assert!(db.list_conflicts().unwrap().is_empty());

    let api_explain = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            &format!("/v1/lanes/merges/queue/{queue_id}/explain"),
            serde_json::Value::Null,
        ),
    );
    assert_eq!(api_explain.status, 200);
    let api_explain: serde_json::Value = api_explain.body_json().unwrap();
    assert_eq!(api_explain["entry"]["queue_id"], queue_id);
    assert!(api_explain["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue["code"] == "merge_conflicts"));

    let api_ref_explain = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "GET",
            "/v1/lanes/merges/queue/explain-bot/explain",
            serde_json::Value::Null,
        ),
    );
    assert_eq!(api_ref_explain.status, 200);
    let api_ref_explain: serde_json::Value = api_ref_explain.body_json().unwrap();
    assert_eq!(api_ref_explain["entry"]["lane"], "explain-bot");

    let tools = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }),
    )
    .unwrap();
    assert!(tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == "trail.lane_merge_queue_explain"));

    let mcp_explain = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "trail.lane_merge_queue_explain",
                "arguments": {
                    "selector": "explain-bot"
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp_explain["result"]["isError"], false);
    assert_eq!(
        mcp_explain["result"]["structuredContent"]["dry_run"]["conflicts"][0],
        "both changed `README.md` differently"
    );
    assert!(db.list_conflicts().unwrap().is_empty());
    drop(db);

    let cli = run_trail_json(
        temp.path(),
        &["lane", "merge-queue", "explain", "explain-bot"],
    );
    assert_eq!(cli["entry"]["lane"], "explain-bot");
    assert!(cli["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue["code"] == "merge_conflicts"));
    assert_eq!(
        cli["dry_run"]["conflicts"][0],
        "both changed `README.md` differently"
    );

    let port = free_loopback_port();
    let mut daemon = DaemonGuard {
        child: Command::new(trail_bin())
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
    let daemon_cli = run_trail_json_daemon(
        temp.path(),
        &daemon_url,
        &["lane", "merge-queue", "explain", "explain-bot"],
    );
    assert_eq!(daemon_cli["entry"]["queue_id"], queue_id);
    assert!(daemon_cli["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|issue| issue["code"] == "merge_conflicts"));

    drop(daemon);
    let db = Trail::open(temp.path()).unwrap();
    assert!(db.list_conflicts().unwrap().is_empty());
}

#[test]
fn lane_merge_queue_pauses_on_conflict() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
    let queued = db.enqueue_lane_merge("doc-bot", "main", 0).unwrap();

    let run = db.run_lane_merge_queue(None).unwrap();
    assert_eq!(run.processed.len(), 1);
    assert_eq!(run.processed[0].status, "conflicted");
    assert!(run.stopped_on_conflict);
    assert!(!run.stopped_on_failure);

    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
    let queue_status: String = conn
        .query_row(
            "SELECT status FROM lane_merge_queue WHERE queue_id = ?1",
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
            "SELECT status FROM lane_merge_queue WHERE queue_id = ?1",
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
fn conflict_resolution_uses_source_snapshot_after_source_ref_moves() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("doc-bot", Some("main"), false, None, None)
        .unwrap();
    let lane_patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "lane readme",
        "edits": [
            {"op": "write", "path": "README.md", "content": "hello\nlane\n"}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "doc-bot", lane_patch).unwrap();

    fs::write(temp.path().join("README.md"), "hello\nhuman\n").unwrap();
    db.record(
        Some("main"),
        Some("human readme".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();
    assert!(matches!(
        db.merge_lane("doc-bot", "main").unwrap_err(),
        Error::Conflict(_)
    ));
    let conflict = db.list_conflicts().unwrap().remove(0);
    let shown = db.show_conflict(&conflict.conflict_set_id).unwrap();
    let explanation = shown.explanation.as_ref().unwrap();
    let snapshot_target_change = explanation.merge.target_change.clone();
    let snapshot_source_change = explanation.merge.source_change.clone();
    let snapshot_source_root = explanation.merge.source_root.clone().unwrap();
    assert_eq!(
        explanation.paths[0].lines[0].source.as_deref(),
        Some("lane\n")
    );

    let later_patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "message": "later lane work",
        "edits": [
            {"op": "write", "path": "docs/source-extra.md", "content": "newer lane work\n"}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "doc-bot", later_patch).unwrap();
    let moved_lane = db.lane_details("doc-bot").unwrap();
    assert_ne!(moved_lane.branch.head_change, snapshot_source_change);
    assert_eq!(moved_lane.branch.status, "conflicted");

    let shown_after_move = db.show_conflict(&conflict.conflict_set_id).unwrap();
    let moved_explanation = shown_after_move.explanation.as_ref().unwrap();
    assert_eq!(
        moved_explanation.merge.source_root.as_ref(),
        Some(&snapshot_source_root)
    );
    assert_eq!(
        moved_explanation.paths[0].lines[0].source.as_deref(),
        Some("lane\n")
    );

    let resolved = db
        .resolve_conflict(&conflict.conflict_set_id, "source")
        .unwrap();
    assert_eq!(resolved.resolution, "source");
    assert_eq!(db.lane_details("doc-bot").unwrap().branch.status, "active");
    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "hello\nlane\n"
    );
    assert!(!temp.path().join("docs/source-extra.md").exists());

    let resolved_op = db.show(&resolved.operation.0).unwrap();
    match resolved_op {
        ShowResult::Operation { value } => {
            assert_eq!(value.operation.parents[0], snapshot_target_change);
            assert_eq!(value.operation.parents[1], snapshot_source_change);
            assert_eq!(
                value.operation.before_root,
                moved_explanation.merge.target_root
            );
            assert_eq!(value.operation.after_root, resolved.root_id);
        }
        other => panic!("expected operation, got {other:?}"),
    }
}

#[test]
fn repeated_conflicts_surface_known_resolution_suggestions() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    db.create_branch("target-a", Some("main")).unwrap();
    db.create_branch("target-b", Some("main")).unwrap();
    db.spawn_lane("source-a", Some("main"), false, None, None)
        .unwrap();
    db.spawn_lane("source-b", Some("main"), false, None, None)
        .unwrap();

    for lane in ["source-a", "source-b"] {
        let patch: PatchDocument = serde_json::from_value(serde_json::json!({
            "edits": [
                {"op": "write", "path": "README.md", "content": "hello\nsource\n"}
            ]
        }))
        .unwrap();
        apply_lane_patch_at_head(&mut db, lane, patch).unwrap();
    }

    for branch in ["target-a", "target-b"] {
        db.checkout(branch, true).unwrap();
        fs::write(temp.path().join("README.md"), "hello\ntarget\n").unwrap();
        db.record(
            Some(branch),
            Some(format!("{branch} target edit")),
            Actor::human(),
            false,
        )
        .unwrap();
    }

    assert!(matches!(
        db.merge_lane("source-a", "target-a").unwrap_err(),
        Error::Conflict(_)
    ));
    let first_conflict = db
        .list_conflicts()
        .unwrap()
        .into_iter()
        .find(|conflict| {
            conflict.status == "open"
                && conflict.source_ref.as_deref() == Some("refs/lanes/source-a")
        })
        .unwrap();
    let resolved = db
        .resolve_conflict(&first_conflict.conflict_set_id, "source")
        .unwrap();
    assert_eq!(resolved.resolution, "source");

    assert!(matches!(
        db.merge_lane("source-b", "target-b").unwrap_err(),
        Error::Conflict(_)
    ));
    let second_conflict = db
        .list_conflicts()
        .unwrap()
        .into_iter()
        .find(|conflict| {
            conflict.status == "open"
                && conflict.source_ref.as_deref() == Some("refs/lanes/source-b")
        })
        .unwrap();
    let shown = db.show_conflict(&second_conflict.conflict_set_id).unwrap();
    let path = &shown.explanation.as_ref().unwrap().paths[0];
    assert_eq!(path.path, "README.md");
    assert_eq!(path.conflict_class, "modify/modify");
    assert!(path.known_resolutions.iter().any(|known| {
        known.resolution == "source"
            && known.conflict_set_id == first_conflict.conflict_set_id
            && known.operation == resolved.operation
            && known.confidence == "known"
    }));
}

#[test]
fn conflict_explanations_classify_common_non_line_conflicts() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "base\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
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
    fs::create_dir_all(temp.path().join("docs")).unwrap();
    fs::write(temp.path().join("README.md"), "base\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("rename-bot", Some("main"), false, None, None)
        .unwrap();
    let rename_patch: PatchDocument = serde_json::from_value(serde_json::json!({
        "edits": [
            {"op": "rename", "from": "README.md", "to": "docs/README.md"}
        ]
    }))
    .unwrap();
    apply_lane_patch_at_head(&mut db, "rename-bot", rename_patch).unwrap();
    fs::write(temp.path().join("README.md"), "target changed\n").unwrap();
    db.record(
        Some("main"),
        Some("target modifies renamed file".to_string()),
        Actor::human(),
        false,
    )
    .unwrap();
    assert!(matches!(
        db.merge_lane("rename-bot", "main").unwrap_err(),
        Error::Conflict(_)
    ));
    assert_eq!(
        only_conflict_path_class(&db),
        ("README.md".to_string(), "rename/modify".to_string())
    );

    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("asset.bin"), [0_u8, 1, 2]).unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
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
    let shown = run_trail_json(
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
    let resolved = run_trail_json(
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
    let mut db = Trail::open(temp.path()).unwrap();
    db.checkout("main", true).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "hello\nmanual-cli\n"
    );

    let (temp, mut db, conflict_id) =
        conflicted_readme_workspace("hello\nlane-api\n", "hello\nhuman-api\n");
    let bad_manual = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            &format!("/v1/conflicts/{conflict_id}/resolve"),
            serde_json::json!({
                "manual": {
                    "files": {
                        "README.md": {
                            "content": "hello\nignored typo\n",
                            "surprise": true
                        }
                    }
                }
            }),
        ),
    );
    assert_eq!(bad_manual.status, 400);
    let bad_manual_body: serde_json::Value = bad_manual.body_json().unwrap();
    let bad_manual_message = bad_manual_body["error"]["message"].as_str().unwrap();
    assert!(
        bad_manual_message.contains("unknown field")
            || bad_manual_message.contains("did not match any variant"),
        "{bad_manual_message}"
    );

    let resolved = trail::server::handle_http_request(
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
    let bad_mcp = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "tools/call",
            "params": {
                "name": "trail.conflict_resolve",
                "arguments": {
                    "conflict_set_id": conflict_id,
                    "manual": {
                        "files": {
                            "README.md": {
                                "content": "hello\nignored typo\n",
                                "surprise": true
                            }
                        }
                    }
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(bad_mcp["result"]["isError"], true);
    let bad_mcp_message = bad_mcp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        bad_mcp_message.contains("unknown field")
            || bad_mcp_message.contains("did not match any variant"),
        "{bad_mcp_message}"
    );

    let resolved = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "trail.conflict_resolve",
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
fn local_api_and_mcp_drive_lane_merge_queue_and_conflicts() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\nworld\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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

    let queued = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/lanes/merges/queue",
            serde_json::json!({
                "lane": "api-queue-bot",
                "into": "main",
                "priority": 5
            }),
        ),
    );
    assert_eq!(queued.status, 201);
    let queued: serde_json::Value = queued.body_json().unwrap();
    assert_eq!(queued["entry"]["status"], "queued");
    let queue_id = queued["entry"]["queue_id"].as_str().unwrap().to_string();

    let listed = trail::server::handle_http_request(
        &mut db,
        &api_request("GET", "/v1/lanes/merges/queue", serde_json::Value::Null),
    );
    assert_eq!(listed.status, 200);
    let listed: serde_json::Value = listed.body_json().unwrap();
    assert!(listed
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["queue_id"] == queue_id));

    let tools = trail::mcp::handle_json_rpc(
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
        "trail.lane_merge_queue_add",
        "trail.lane_merge_queue_list",
        "trail.lane_merge_queue_run",
        "trail.lane_merge_queue_explain",
        "trail.lane_merge_queue_remove",
        "trail.conflict_list",
        "trail.conflict_show",
        "trail.conflict_resolve",
    ] {
        assert!(tool_list.iter().any(|tool| tool["name"] == name), "{name}");
    }
    let conflict_resolve_schema = &tool_list
        .iter()
        .find(|tool| tool["name"] == "trail.conflict_resolve")
        .unwrap()["inputSchema"];
    let conflict_resolve_modes = conflict_resolve_schema["oneOf"].as_array().unwrap();
    assert_eq!(conflict_resolve_modes.len(), 2);
    assert_eq!(
        conflict_resolve_modes[0]["required"],
        serde_json::json!(["take"])
    );
    assert_eq!(
        conflict_resolve_modes[0]["not"]["required"],
        serde_json::json!(["manual"])
    );
    assert_eq!(
        conflict_resolve_modes[1]["required"],
        serde_json::json!(["manual"])
    );
    assert_eq!(
        conflict_resolve_modes[1]["not"]["required"],
        serde_json::json!(["take"])
    );
    assert_eq!(
        conflict_resolve_schema["properties"]["manual"]["additionalProperties"],
        false
    );
    assert_eq!(
        conflict_resolve_schema["properties"]["manual"]["properties"]["files"]
            ["additionalProperties"]["oneOf"][1]["additionalProperties"],
        false
    );

    let run = trail::server::handle_http_request(
        &mut db,
        &api_request("POST", "/v1/lanes/merges/queue/run", serde_json::json!({})),
    );
    assert_eq!(run.status, 200);
    let run: serde_json::Value = run.body_json().unwrap();
    assert_eq!(run["processed"][0]["status"], "conflicted");
    assert_eq!(run["stopped_on_conflict"], true);

    let conflicts = trail::server::handle_http_request(
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

    let shown = trail::server::handle_http_request(
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

    let mcp_show = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "trail.conflict_show",
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

    let mcp_resolve = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "trail.conflict_resolve",
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

    let queue = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "trail.lane_merge_queue_list",
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
    let mcp_add = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "trail.lane_merge_queue_add",
                "arguments": {
                    "lane": "cancel-queue-bot",
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

    let removed = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "DELETE",
            &format!("/v1/lanes/merges/queue/{cancel_queue_id}"),
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    fs::write(temp.path().join("b.txt"), "same\n").unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
    let before: i64 = conn
        .query_row("SELECT COUNT(*) FROM objects", [], |row| row.get(0))
        .unwrap();

    fs::write(temp.path().join("README.md"), "hello\nstatus\n").unwrap();
    let db = Trail::open(temp.path()).unwrap();
    let status = db.status(Some("main")).unwrap();
    assert_ne!(status.worktree_state, WorktreeState::Clean);

    let after: i64 = conn
        .query_row("SELECT COUNT(*) FROM objects", [], |row| row.get(0))
        .unwrap();
    assert_eq!(before, after);
}

#[cfg(unix)]
#[test]
fn status_maintains_persisted_worktree_file_index() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a.txt"), "a1\n").unwrap();
    fs::write(temp.path().join("b.txt"), "b1\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
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

    let clean_db = Trail::open(temp.path()).unwrap();
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
    let mut db = Trail::open(temp.path()).unwrap();
    let status = db.status(Some("main")).unwrap();
    assert_eq!(baseline_root(&conn), None);
    let changes = status
        .changed_paths
        .iter()
        .map(|path| (path.path.as_str(), path.kind.clone()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(changes.get("a.txt"), Some(&trail::FileChangeKind::Modified));
    assert_eq!(changes.get("b.txt"), Some(&trail::FileChangeKind::Deleted));

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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let db = Trail::open(temp.path()).unwrap();
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
    Trail::init_with_text_policy(
        full.path(),
        "main",
        InitImportMode::WorkingTree,
        false,
        Some("full"),
    )
    .unwrap();
    let db = Trail::open(full.path()).unwrap();
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

    Trail::init_with_text_policy(
        temp.path(),
        "main",
        InitImportMode::WorkingTree,
        false,
        Some("minimal"),
    )
    .unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    fs::write(temp.path().join("b.txt"), "b1\n").unwrap();
    let report = run_trail_json(
        temp.path(),
        &["index", "watch", "--once", "--interval-ms", "1"],
    );
    assert_eq!(report["files"].as_u64(), Some(2));
    assert_eq!(report["indexed_entries"].as_u64(), Some(2));
    assert!(report["duration_ms"].as_u64().is_some());

    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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

fn wait_for_status<F>(db: &Trail, mut ready: F) -> trail::StatusReport
where
    F: FnMut(&trail::StatusReport) -> bool,
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    fs::write(temp.path().join("README.md"), "changed\n").unwrap();
    fs::write(temp.path().join(".trail/lock"), "test writer").unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let err = db
        .record(Some("main"), None, Actor::human(), false)
        .unwrap_err();
    assert!(matches!(err, Error::WorkspaceLocked(_)));
}

#[test]
fn lane_patch_records_message_and_event_indexes() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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

    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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
    match db.show(&applied.operation.checkpoint_alias()).unwrap() {
        ShowResult::Operation { value } => assert_eq!(value.operation.change_id, applied.operation),
        other => panic!("expected checkpoint alias to show operation, got {other:?}"),
    }

    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
    let message_id: String = conn
        .query_row("SELECT message_id FROM messages LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(message_id.starts_with("message_"));
    match db.show(&message_id).unwrap() {
        ShowResult::Message { value } => assert_eq!(value.body, "lane adds line"),
        other => panic!("expected message show result, got {other:?}"),
    }

    let file_history = db.history_for_path("README.md").unwrap();
    assert!(file_history.file_history.len() >= 2);

    let why = db.why("README.md:2", Some("refs/lanes/doc-bot")).unwrap();
    let line_id = why.line_id.alias();
    assert!(line_id.starts_with("line_"));
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
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

    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
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
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();

    let conn = Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
    conn.execute(
        "INSERT INTO objects \
         (object_id, kind, version, codec, hash_alg, size_bytes, bytes, created_at) \
         VALUES ('object_unreachable_test', 'Blob', 1, 'cbor', 'sha256', 0, x'', 0)",
        [],
    )
    .unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let dry_run = db.gc(true).unwrap();
    assert!(dry_run.prunable_objects >= 1);
    assert_eq!(dry_run.pruned_objects, 0);

    let report = db.gc(false).unwrap();
    assert!(report.pruned_objects >= 1);
    let still_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM objects WHERE object_id = 'object_unreachable_test'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(still_exists, 0);
    assert!(db.fsck().unwrap().errors.is_empty());
}
