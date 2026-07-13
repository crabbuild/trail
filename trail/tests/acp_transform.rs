#![cfg(unix)]

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use serde_json::Value;
use trail::{InitImportMode, Trail};

fn trail_bin() -> PathBuf {
    std::env::var_os("TRAIL_TEST_BIN")
        .map(PathBuf::from)
        .or_else(|| option_env!("CARGO_BIN_EXE_trail").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/debug/trail"))
}

fn workspace() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "transform fixture\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    temp
}

fn write_agent(workspace: &Path, name: &str, body: &str) -> PathBuf {
    let agent = workspace.join(name);
    fs::write(&agent, format!("#!/bin/sh\nset -eu\n{body}")).unwrap();
    let mut permissions = fs::metadata(&agent).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&agent, permissions).unwrap();
    agent
}

fn spawn_relay(workspace: &Path, agent: &Path) -> Child {
    Command::new(trail_bin())
        .arg("--workspace")
        .arg(workspace)
        .args(["acp", "relay", "--"])
        .arg(agent)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn read_json(reader: &mut impl BufRead) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(!line.is_empty(), "relay closed before returning a frame");
    serde_json::from_str(line.trim_end()).unwrap()
}

#[test]
fn selects_v1_without_rewriting_the_offer_and_then_enables_transformations() {
    let temp = workspace();
    let received = temp.path().join("received.jsonl");
    let agent = write_agent(
        temp.path(),
        "v1-agent.sh",
        &format!(
            r#"IFS= read -r initialize
printf '%s\n' "$initialize" > '{}'
printf '%s\n' '{{"jsonrpc":"2.0","id":"init","result":{{"protocolVersion":1,"agentCapabilities":{{}},"_meta":{{"agent":"keep"}}}}}}'
IFS= read -r session
printf '%s\n' "$session" >> '{}'
printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"sessionId":"s"}}}}'
"#,
            received.display(),
            received.display()
        ),
    );
    let mut child = spawn_relay(temp.path(), &agent);
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let initialize = r#"{"jsonrpc":"2.0","id":"init","method":"initialize","params":{"protocolVersion":2,"clientCapabilities":{},"_meta":{"offeredVersions":[1,2]}}}"#;
    writeln!(stdin, "{initialize}").unwrap();
    let response = read_json(&mut stdout);
    assert_eq!(response["result"]["protocolVersion"], 1);
    assert_eq!(response["result"]["_meta"]["agent"], "keep");
    assert_eq!(response["result"]["_meta"]["trail"]["relay"], true);

    let session = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"session/new","params":{{"cwd":"{}","mcpServers":[]}}}}"#,
        temp.path().display()
    );
    writeln!(stdin, "{session}").unwrap();
    let _ = read_json(&mut stdout);
    drop(stdin);
    assert!(child.wait().unwrap().success());

    let lines = fs::read_to_string(received).unwrap();
    let mut lines = lines.lines();
    assert_eq!(lines.next(), Some(initialize));
    let forwarded_session: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    let servers = forwarded_session["params"]["mcpServers"]
        .as_array()
        .unwrap();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0]["name"], "trail");
}

#[test]
fn later_version_selection_is_forwarded_without_v1_transformations() {
    let temp = workspace();
    let received = temp.path().join("received.jsonl");
    let response = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":2,"agentCapabilities":{},"_meta":{"agent":"keep"}}}"#;
    let agent = write_agent(
        temp.path(),
        "v2-agent.sh",
        &format!(
            r#"IFS= read -r initialize
printf '%s\n' '{}'
IFS= read -r session
printf '%s\n' "$session" > '{}'
printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"sessionId":"s"}}}}'
"#,
            response,
            received.display()
        ),
    );
    let mut child = spawn_relay(temp.path(), &agent);
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":2,"clientCapabilities":{{}}}}}}"#).unwrap();
    let mut raw_response = String::new();
    stdout.read_line(&mut raw_response).unwrap();
    assert_eq!(raw_response, format!("{response}\n"));

    let session = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"session/new","params":{{"cwd":"{}","mcpServers":[]}}}}"#,
        temp.path().display()
    );
    writeln!(stdin, "{session}").unwrap();
    let _ = read_json(&mut stdout);
    drop(stdin);
    assert!(child.wait().unwrap().success());
    assert_eq!(
        fs::read_to_string(received).unwrap(),
        format!("{session}\n")
    );
}

#[test]
fn initialize_error_disables_session_transformations() {
    let temp = workspace();
    let received = temp.path().join("received.jsonl");
    let agent = write_agent(
        temp.path(),
        "error-agent.sh",
        &format!(
            r#"IFS= read -r initialize
printf '%s\n' '{{"jsonrpc":"2.0","id":1,"error":{{"code":-32000,"message":"unsupported"}}}}'
IFS= read -r session
printf '%s\n' "$session" > '{}'
printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"sessionId":"s"}}}}'
"#,
            received.display()
        ),
    );
    let mut child = spawn_relay(temp.path(), &agent);
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":1,"clientCapabilities":{{}}}}}}"#).unwrap();
    let _ = read_json(&mut stdout);
    let session = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"session/new","params":{{"cwd":"{}","mcpServers":[]}}}}"#,
        temp.path().display()
    );
    writeln!(stdin, "{session}").unwrap();
    let _ = read_json(&mut stdout);
    drop(stdin);
    assert!(child.wait().unwrap().success());
    assert_eq!(
        fs::read_to_string(received).unwrap(),
        format!("{session}\n")
    );
}

#[test]
fn session_before_initialize_is_forwarded_without_creating_a_transformation() {
    let temp = workspace();
    let received = temp.path().join("received.jsonl");
    let agent = write_agent(
        temp.path(),
        "early-session-agent.sh",
        &format!(
            r#"IFS= read -r session
printf '%s\n' "$session" > '{}'
printf '%s\n' '{{"jsonrpc":"2.0","id":9,"result":{{"sessionId":"early"}}}}'
"#,
            received.display()
        ),
    );
    let mut child = spawn_relay(temp.path(), &agent);
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let session = format!(
        r#"{{"jsonrpc":"2.0","id":9,"method":"session/new","params":{{"cwd":"{}","mcpServers":[]}}}}"#,
        temp.path().display()
    );
    writeln!(stdin, "{session}").unwrap();
    let _ = read_json(&mut stdout);
    drop(stdin);
    assert!(child.wait().unwrap().success());
    assert_eq!(
        fs::read_to_string(received).unwrap(),
        format!("{session}\n")
    );
}

#[test]
fn duplicate_initialize_is_not_treated_as_a_second_negotiation() {
    let temp = workspace();
    let second_response = r#"{"jsonrpc":"2.0","id":2,"result":{"protocolVersion":1,"agentCapabilities":{},"_meta":{"agent":"second"}}}"#;
    let agent = write_agent(
        temp.path(),
        "duplicate-agent.sh",
        &format!(
            r#"IFS= read -r first
printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":1,"agentCapabilities":{{}}}}}}'
IFS= read -r second
printf '%s\n' '{}'
"#,
            second_response
        ),
    );
    let mut child = spawn_relay(temp.path(), &agent);
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":1,"clientCapabilities":{{}}}}}}"#).unwrap();
    assert_eq!(
        read_json(&mut stdout)["result"]["_meta"]["trail"]["relay"],
        true
    );
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"initialize","params":{{"protocolVersion":1,"clientCapabilities":{{}}}}}}"#).unwrap();
    let mut raw = String::new();
    stdout.read_line(&mut raw).unwrap();
    assert_eq!(raw, format!("{second_response}\n"));
    drop(stdin);
    assert!(child.wait().unwrap().success());
}

#[test]
fn invalid_initialize_metadata_candidate_rolls_back_to_raw_bytes() {
    let temp = workspace();
    let response = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{},"_meta":"opaque"}}"#;
    let agent = write_agent(
        temp.path(),
        "invalid-meta-agent.sh",
        &format!(
            r#"IFS= read -r initialize
printf '%s\n' '{}'
"#,
            response
        ),
    );
    let mut child = spawn_relay(temp.path(), &agent);
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":1,"clientCapabilities":{{}}}}}}"#).unwrap();
    let mut raw = String::new();
    stdout.read_line(&mut raw).unwrap();
    assert_eq!(raw, format!("{response}\n"));
    drop(stdin);
    assert!(child.wait().unwrap().success());
}

#[test]
fn mcp_injection_preserves_same_name_servers_and_appends_trail_identity() {
    let temp = workspace();
    let received = temp.path().join("received.jsonl");
    let agent = write_agent(
        temp.path(),
        "mcp-dedup-agent.sh",
        &format!(
            r#"IFS= read -r initialize
printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":1,"agentCapabilities":{{}}}}}}'
IFS= read -r session
printf '%s\n' "$session" > '{}'
printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"sessionId":"s"}}}}'
"#,
            received.display()
        ),
    );
    let mut child = spawn_relay(temp.path(), &agent);
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":1,"clientCapabilities":{{}}}}}}"#).unwrap();
    let _ = read_json(&mut stdout);
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"session/new","params":{{"cwd":"{}","mcpServers":[{{"name":"first","command":"/bin/first","args":[],"env":[]}},{{"name":"trail","command":"/custom/trail","args":["mcp"],"env":[]}}]}}}}"#,
        temp.path().display()
    )
    .unwrap();
    let _ = read_json(&mut stdout);
    drop(stdin);
    assert!(child.wait().unwrap().success());

    let forwarded: Value =
        serde_json::from_str(fs::read_to_string(received).unwrap().trim()).unwrap();
    let servers = forwarded["params"]["mcpServers"].as_array().unwrap();
    assert_eq!(servers.len(), 3);
    assert_eq!(servers[0]["name"], "first");
    assert_eq!(servers[1]["name"], "trail");
    assert_eq!(servers[1]["command"], "/custom/trail");
    assert_eq!(servers[2]["name"], "trail");
    assert_eq!(servers[2]["args"], serde_json::json!(["mcp"]));
}

#[test]
fn invalid_mcp_candidate_rolls_back_the_entire_session_transform() {
    let temp = workspace();
    let received = temp.path().join("received.jsonl");
    let agent = write_agent(
        temp.path(),
        "invalid-mcp-agent.sh",
        &format!(
            r#"IFS= read -r initialize
printf '%s\n' '{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":1,"agentCapabilities":{{}}}}}}'
IFS= read -r session
printf '%s\n' "$session" > '{}'
printf '%s\n' '{{"jsonrpc":"2.0","id":2,"result":{{"sessionId":"s"}}}}'
"#,
            received.display()
        ),
    );
    let mut child = spawn_relay(temp.path(), &agent);
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":1,"clientCapabilities":{{}}}}}}"#).unwrap();
    let _ = read_json(&mut stdout);
    let session = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"session/new","params":{{"cwd":"{}","mcpServers":{{"invalid":true}}}}}}"#,
        temp.path().display()
    );
    writeln!(stdin, "{session}").unwrap();
    let _ = read_json(&mut stdout);
    drop(stdin);
    assert!(child.wait().unwrap().success());
    assert_eq!(
        fs::read_to_string(received).unwrap(),
        format!("{session}\n")
    );
}
