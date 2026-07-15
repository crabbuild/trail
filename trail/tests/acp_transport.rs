#![cfg(unix)]

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use trail::{InitImportMode, Trail};

fn trail_bin() -> PathBuf {
    option_env!("CARGO_BIN_EXE_trail")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/debug/trail"))
}

fn workspace() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "transport fixture\n").unwrap();
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
    let mut command = Command::new(trail_bin());
    command
        .arg("--workspace")
        .arg(workspace)
        .args(["acp", "relay", "--"])
        .arg(agent)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.spawn().unwrap()
}

#[test]
fn forwards_unmodified_frames_byte_exactly() {
    let temp = workspace();
    let agent = write_agent(
        temp.path(),
        "echo-acp-agent.sh",
        "IFS= read -r frame\nprintf '%s\\n' \"$frame\"\n",
    );
    let mut child = spawn_relay(temp.path(), &agent);
    let raw = b" { \"method\" : \"ext/byte_exact\", \"params\" : { \"z\" : 1, \"a\" : true }, \"id\" : \"same\", \"jsonrpc\" : \"2.0\" } \n";
    child.stdin.take().unwrap().write_all(raw).unwrap();

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "relay failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, raw);
}

#[test]
fn preserves_crlf_and_keeps_upstream_stderr_off_protocol_stdout() {
    let temp = workspace();
    let agent = write_agent(
        temp.path(),
        "stderr-cat-acp-agent.sh",
        "printf '%s\\n' 'agent diagnostic' >&2\nexec /bin/cat\n",
    );
    let mut child = spawn_relay(temp.path(), &agent);
    let raw = b"{\"jsonrpc\":\"2.0\",\"method\":\"ext/crlf\"}\r\n";
    child.stdin.take().unwrap().write_all(raw).unwrap();

    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, raw);
    assert!(String::from_utf8_lossy(&output.stderr).contains("agent diagnostic"));
}

#[test]
fn preserves_bidirectional_same_id_traffic_and_out_of_order_responses() {
    let temp = workspace();
    let agent = write_agent(
        temp.path(),
        "concurrent-acp-agent.sh",
        r#"IFS= read -r first
IFS= read -r second
printf '%s\n' '{"jsonrpc":"2.0","id":7,"method":"fs/read_text_file","params":{"sessionId":"s","path":"README.md"}}'
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"second"}}'
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"first"}}'
IFS= read -r callback_response
test "$callback_response" = '{"jsonrpc":"2.0","id":7,"result":{"content":"fixture"}}'
"#,
    );
    let mut child = spawn_relay(temp.path(), &agent);
    let mut stdin = child.stdin.take().unwrap();
    stdin
        .write_all(
            b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ext/first\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ext/second\"}\n",
        )
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut stdout = BufReader::new(stdout);
    let mut lines = Vec::new();
    for _ in 0..3 {
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        lines.push(serde_json::from_str::<serde_json::Value>(line.trim()).unwrap());
    }
    assert_eq!(lines[0]["method"], "fs/read_text_file");
    assert_eq!(lines[0]["id"], 7);
    assert_eq!(lines[1]["id"], 2);
    assert_eq!(lines[2]["id"], 1);
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"content\":\"fixture\"}}\n")
        .unwrap();
    drop(stdin);

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "relay failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn malformed_agent_frame_fails_without_polluting_stdout() {
    let temp = workspace();
    let agent = write_agent(
        temp.path(),
        "malformed-acp-agent.sh",
        "printf '%s\\n' '{not-json}'\n",
    );
    let child = spawn_relay(temp.path(), &agent);

    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("I/O error") && stderr.contains("line 1 column"),
        "unexpected malformed-frame diagnostic: {stderr}"
    );
}

#[test]
fn editor_eof_bounds_an_unresponsive_agent_shutdown() {
    let temp = workspace();
    let agent = write_agent(
        temp.path(),
        "sleeping-acp-agent.sh",
        "printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"method\":\"ext/ready\"}'\nIFS= read -r frame || true\nsleep 10\n",
    );
    let mut child = spawn_relay(temp.path(), &agent);
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut ready = String::new();
    stdout.read_line(&mut ready).unwrap();
    assert_eq!(ready, "{\"jsonrpc\":\"2.0\",\"method\":\"ext/ready\"}\n");
    drop(child.stdin.take().unwrap());
    let started = Instant::now();
    let status = child.wait().unwrap();

    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(3),
        "relay process shutdown took {elapsed:?}"
    );
    assert!(status.success());
}
