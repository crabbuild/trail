#![cfg(unix)]

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use trail::{AgentCaptureTransport, AgentHookReceiptInput, InitImportMode, Trail};

fn trail_bin() -> PathBuf {
    std::env::var_os("TRAIL_TEST_BIN")
        .map(PathBuf::from)
        .or_else(|| option_env!("CARGO_BIN_EXE_trail").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/debug/trail"))
}

fn write_echo_agent(workspace: &Path) -> PathBuf {
    let agent = workspace.join("capture-echo-agent.sh");
    fs::write(
        &agent,
        r#"#!/bin/sh
set -eu
IFS= read -r initialize
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
IFS= read -r session
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"sessionId":"capture-session"}}'
IFS= read -r prompt
printf '%s\n' "$prompt"
exec /bin/cat
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&agent).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&agent, permissions).unwrap();
    agent
}

fn write_initialize_agent(workspace: &Path) -> PathBuf {
    let agent = workspace.join("capture-initialize-agent.sh");
    fs::write(
        &agent,
        r#"#!/bin/sh
set -eu
IFS= read -r initialize
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
exec /bin/cat
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&agent).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&agent, permissions).unwrap();
    agent
}

fn receipt_payload(connection_id: &str, sequence: u64) -> serde_json::Value {
    serde_json::json!({
        "connection_id": connection_id,
        "direction": "agent_to_client",
        "sequence": sequence,
        "received_at": i64::try_from(sequence).unwrap() + 1,
        "message": {
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {"sequence": sequence}
        },
        "project": false
    })
}

fn connection_receipts(workspace: &Path, connection_id: &str) -> Vec<trail::AgentHookReceipt> {
    let db = Trail::open(workspace).unwrap();
    let mut receipts = Vec::new();
    for offset in (0..3_000).step_by(1_000) {
        let page = db
            .list_agent_hook_receipts_page(Some("trail-acp"), None, offset, 1_000)
            .unwrap();
        let page_len = page.len();
        receipts.extend(
            page.into_iter()
                .filter(|receipt| receipt.connection_id.as_deref() == Some(connection_id)),
        );
        if page_len < 1_000 {
            break;
        }
    }
    receipts
}

#[test]
fn forwarding_never_waits_for_database_writer() {
    const FRAME_COUNT: usize = 1_000;

    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "capture fixture\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let agent = write_echo_agent(temp.path());
    let mut child = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .args(["acp", "relay", "--"])
        .arg(agent)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":1,"clientCapabilities":{{}}}}}}"#).unwrap();
    let mut initialize_response = String::new();
    stdout.read_line(&mut initialize_response).unwrap();
    assert!(initialize_response.contains(r#""protocolVersion":1"#));
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"session/new","params":{{"cwd":"{}","mcpServers":[]}}}}"#,
        temp.path().display()
    )
    .unwrap();
    let mut session_response = String::new();
    stdout.read_line(&mut session_response).unwrap();
    assert!(session_response.contains("capture-session"));

    let lock = temp.path().join(".trail/lock");
    let lock_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
        {
            Ok(mut file) => {
                writeln!(
                    file,
                    "pid={} created_at={}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                )
                .unwrap();
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                assert!(
                    Instant::now() < lock_deadline,
                    "writer lock never became idle"
                );
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("failed to acquire test writer lock: {error}"),
        }
    }
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{{"sessionId":"capture-session","prompt":[{{"type":"text","text":"stress capture"}}]}}}}"#).unwrap();
    let mut prompt_echo = String::new();
    stdout.read_line(&mut prompt_echo).unwrap();
    assert!(prompt_echo.contains(r#""id":3"#));

    let started = Instant::now();
    let writer = thread::spawn(move || {
        for sequence in 0..FRAME_COUNT {
            writeln!(
                stdin,
                r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"capture-session","update":{{"sessionUpdate":"tool_call_update","toolCallId":"tool-{sequence}","status":"completed"}}}}}}"#
            )
            .unwrap();
        }
        stdin
    });
    let (done_tx, done_rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut line = String::new();
        for _ in 0..FRAME_COUNT {
            line.clear();
            stdout.read_line(&mut line).unwrap();
            assert!(!line.is_empty());
        }
        done_tx.send(started.elapsed()).unwrap();
    });

    let within_budget = done_rx.recv_timeout(Duration::from_millis(250)).ok();
    fs::remove_file(&lock).unwrap();
    let stdin = writer.join().unwrap();
    reader.join().unwrap();
    let shutdown_started = Instant::now();
    drop(stdin);
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "relay failed\nstderr:\n{stderr}");
    assert!(
        shutdown_started.elapsed() < Duration::from_millis(2_500),
        "capture shutdown exceeded its bounded drain budget: {:?}",
        shutdown_started.elapsed()
    );
    let elapsed = within_budget.unwrap_or_else(|| started.elapsed());
    assert!(
        elapsed < Duration::from_millis(250),
        "forwarding waited for Trail's writer lock: {elapsed:?}"
    );
}

#[test]
fn pending_receipts_and_spill_replay_once_in_connection_order() {
    const DURABLE_COUNT: u64 = 500;
    const SPILL_COUNT: u64 = 500;
    const TOTAL: usize = (DURABLE_COUNT + SPILL_COUNT) as usize;
    const CONNECTION_ID: &str = "recovery-connection";

    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "capture recovery fixture\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    for sequence in 0..DURABLE_COUNT {
        db.persist_agent_hook_receipt(AgentHookReceiptInput {
            installation_id: None,
            provider: "trail-acp".to_string(),
            native_event: "acp/frame".to_string(),
            native_session_id: None,
            native_turn_id: None,
            transport: AgentCaptureTransport::Acp,
            connection_id: Some(CONNECTION_ID.to_string()),
            direction: Some("agent_to_client".to_string()),
            connection_sequence: Some(sequence),
            dedupe_key: format!("acp:{CONNECTION_ID}:agent_to_client:{sequence}"),
            payload: receipt_payload(CONNECTION_ID, sequence),
            occurred_at: Some(i64::try_from(sequence).unwrap() + 1),
        })
        .unwrap();
    }
    drop(db);

    let spill_dir = temp.path().join(".trail/acp-ingress");
    fs::create_dir_all(&spill_dir).unwrap();
    let mut spill = String::new();
    for sequence in DURABLE_COUNT..(DURABLE_COUNT + SPILL_COUNT) {
        let frame = serde_json::json!({
            "connection_id": CONNECTION_ID,
            "direction": "AgentToClient",
            "sequence": sequence,
            "received_at": i64::try_from(sequence).unwrap() + 1,
            "redacted_message": {
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {"sequence": sequence}
            },
            "project": false
        });
        let line = serde_json::to_string(&frame).unwrap();
        spill.push_str(&line);
        spill.push('\n');
        spill.push_str(&line);
        spill.push('\n');
    }
    fs::write(spill_dir.join(format!("{CONNECTION_ID}.jsonl")), spill).unwrap();

    let agent = write_initialize_agent(temp.path());
    let mut child = Command::new(trail_bin())
        .arg("--workspace")
        .arg(temp.path())
        .args(["acp", "relay", "--"])
        .arg(agent)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":1,"clientCapabilities":{{}}}}}}"#).unwrap();
    let mut initialize_response = String::new();
    stdout.read_line(&mut initialize_response).unwrap();
    assert!(initialize_response.contains(r#""protocolVersion":1"#));

    let deadline = Instant::now() + Duration::from_secs(45);
    let receipts = loop {
        let receipts = connection_receipts(temp.path(), CONNECTION_ID);
        if receipts.len() == TOTAL && receipts.iter().all(|receipt| receipt.status == "processed") {
            break receipts;
        }
        assert!(
            Instant::now() < deadline,
            "timed out replaying capture journal: {} of {TOTAL} receipts",
            receipts.len()
        );
        thread::sleep(Duration::from_millis(50));
    };
    let mut sequences = receipts
        .iter()
        .map(|receipt| receipt.connection_sequence.unwrap())
        .collect::<Vec<_>>();
    sequences.sort_unstable();
    assert_eq!(
        sequences,
        (0..u64::try_from(TOTAL).unwrap()).collect::<Vec<_>>()
    );
    assert_eq!(
        receipts
            .iter()
            .map(|receipt| receipt.dedupe_key.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        TOTAL
    );

    drop(stdin);
    assert!(child.wait().unwrap().success());
    let remaining_spill = fs::read_dir(spill_dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.metadata().is_ok_and(|metadata| metadata.len() > 0))
        .count();
    assert_eq!(
        remaining_spill, 0,
        "acknowledged spill files remain non-empty"
    );
}
