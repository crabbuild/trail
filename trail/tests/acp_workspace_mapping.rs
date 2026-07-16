#![cfg(unix)]

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;
use trail::{InitImportMode, Trail};

fn trail_bin() -> PathBuf {
    std::env::var_os("TRAIL_TEST_BIN")
        .map(PathBuf::from)
        .or_else(|| option_env!("CARGO_BIN_EXE_trail").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/debug/trail"))
}

fn write_agent(workspace: &Path, received: &Path) -> PathBuf {
    let agent = workspace.join("mapping-agent.sh");
    fs::write(
        &agent,
        format!(
            r#"#!/bin/sh
set -eu
IFS= read -r initialize
printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":1,"agentCapabilities":{{}}}}}}'
IFS= read -r session
printf '%s\n' "$session" > '{}'
printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"sessionId":"mapped"}}}}'
"#,
            received.display()
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&agent).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&agent, permissions).unwrap();
    agent
}

#[test]
fn maps_nested_cwd_without_collapsing_to_lane_root() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("packages/app")).unwrap();
    fs::write(temp.path().join("README.md"), "mapping fixture\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let received = temp.path().join(".trail/received.json");
    let agent = write_agent(temp.path(), &received);
    let mut child = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .args([
            "acp",
            "relay",
            "--lane",
            "mapping-test",
            "--materialize",
            "--",
        ])
        .arg(agent)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":1,"clientCapabilities":{{}}}}}}"#).unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert!(!line.is_empty());
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"session/new","params":{{"cwd":"{}","mcpServers":[]}}}}"#,
        temp.path().join("packages/app").display()
    )
    .unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    assert!(!line.is_empty());
    drop(stdin);
    assert!(child.wait().unwrap().success());

    let db = Trail::open(temp.path()).unwrap();
    let lane = db.lane_details("mapping-test").unwrap();
    let lane_root = PathBuf::from(lane.branch.workdir.unwrap());
    let forwarded: Value = serde_json::from_slice(&fs::read(received).unwrap()).unwrap();
    assert_eq!(
        PathBuf::from(forwarded["params"]["cwd"].as_str().unwrap()),
        lane_root.join("packages/app")
    );
}

#[test]
fn maps_and_persists_normalized_additional_roots_without_reordering() {
    let temp = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("packages/app")).unwrap();
    fs::create_dir_all(temp.path().join("shared")).unwrap();
    fs::write(temp.path().join("README.md"), "mapping fixture\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let received = temp.path().join(".trail/received.json");
    let agent = write_agent(temp.path(), &received);
    let mut child = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .args([
            "acp",
            "relay",
            "--lane",
            "mapping-matrix",
            "--materialize",
            "--",
        ])
        .arg(agent)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":1,"clientCapabilities":{{}}}}}}"#).unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert!(!line.is_empty());
    let cwd = temp.path().join("packages/./app/../app");
    let shared = temp.path().join("shared");
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "session/new",
        "params": {
            "cwd": cwd,
            "mcpServers": [],
            "additionalDirectories": [
                shared,
                temp.path().join("shared/."),
                temp.path().join("packages"),
                external.path(),
                external.path(),
                "C:\\external\\repo",
                "\\\\server\\share"
            ]
        }
    });
    serde_json::to_writer(&mut stdin, &request).unwrap();
    stdin.write_all(b"\n").unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    assert!(!line.is_empty());
    drop(stdin);
    assert!(child.wait().unwrap().success());

    let db = Trail::open(temp.path()).unwrap();
    let lane = db.lane_details("mapping-matrix").unwrap();
    let lane_root = PathBuf::from(lane.branch.workdir.unwrap());
    let forwarded: Value = serde_json::from_slice(&fs::read(received).unwrap()).unwrap();
    assert_eq!(
        forwarded["params"]["cwd"],
        lane_root.join("packages/app").to_string_lossy().as_ref()
    );
    assert_eq!(
        forwarded["params"]["additionalDirectories"],
        serde_json::json!([
            lane_root.join("shared").to_string_lossy(),
            lane_root.join("packages").to_string_lossy(),
            external.path().to_string_lossy(),
            "C:\\external\\repo",
            "\\\\server\\share"
        ])
    );

    let mapping = db.lane_acp_session("mapped").unwrap();
    assert_eq!(mapping.path_mappings.len(), 6);
    assert_eq!(mapping.path_mappings[0].original, cwd.to_string_lossy());
    assert!(mapping.path_mappings[0].isolated);
    assert!(mapping.path_mappings[1].isolated);
    assert!(mapping.path_mappings[2].isolated);
    assert!(!mapping.path_mappings[3].isolated);
    assert!(!mapping.path_mappings[4].isolated);
    assert!(!mapping.path_mappings[5].isolated);
}

#[test]
fn symlink_escape_is_preserved_as_an_external_root() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    fs::create_dir_all(external.path().join("nested")).unwrap();
    symlink(external.path(), temp.path().join("escape")).unwrap();
    fs::write(temp.path().join("README.md"), "mapping fixture\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let received = temp.path().join(".trail/received.json");
    let agent = write_agent(temp.path(), &received);
    let mut child = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .args([
            "acp",
            "relay",
            "--lane",
            "mapping-symlink",
            "--materialize",
            "--",
        ])
        .arg(agent)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":1,"clientCapabilities":{{}}}}}}"#).unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    let escaped = temp.path().join("escape/nested");
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "session/new",
        "params": {
            "cwd": temp.path(),
            "mcpServers": [],
            "additionalDirectories": [escaped]
        }
    });
    serde_json::to_writer(&mut stdin, &request).unwrap();
    stdin.write_all(b"\n").unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    drop(stdin);
    assert!(child.wait().unwrap().success());

    let forwarded: Value = serde_json::from_slice(&fs::read(received).unwrap()).unwrap();
    assert_eq!(
        forwarded["params"]["additionalDirectories"][0],
        escaped.to_string_lossy().as_ref()
    );
    let db = Trail::open(temp.path()).unwrap();
    let mapping = db.lane_acp_session("mapped").unwrap();
    let escaped_mapping = mapping
        .path_mappings
        .iter()
        .find(|mapping| mapping.original == escaped.to_string_lossy())
        .unwrap();
    assert!(!escaped_mapping.isolated);
    assert_eq!(escaped_mapping.effective, escaped.to_string_lossy());
}
