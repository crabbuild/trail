#![cfg(unix)]

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

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

fn connection_receipts(db: &Trail, connection_id: &str) -> Vec<trail::AgentHookReceipt> {
    let mut receipts = Vec::new();
    for offset in (0..3_000).step_by(1_000) {
        let page = db
            .list_agent_hook_receipts_for_connection_page(connection_id, None, offset, 1_000)
            .unwrap();
        let page_len = page.len();
        receipts.extend(page);
        if page_len < 1_000 {
            break;
        }
    }
    receipts
}

fn live_capture_connection_id(workspace: &Path) -> String {
    fs::read_dir(workspace.join(".trail/acp-ingress"))
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .find_map(|name| name.strip_suffix(".owner").map(str::to_string))
        .expect("relay did not publish its capture connection owner")
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
    let connection_id = live_capture_connection_id(temp.path());
    let receipt_reader = Trail::open(temp.path()).unwrap();

    let mut lock_holder = Command::new(trail_bin())
        .arg("__test-workspace-lock-holder")
        .arg(temp.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let lock_stdin = lock_holder.stdin.take().unwrap();
    let mut lock_stdout = BufReader::new(lock_holder.stdout.take().unwrap());
    let mut lock_ready = String::new();
    lock_stdout.read_line(&mut lock_ready).unwrap();
    assert_eq!(lock_ready, "READY\n", "workspace lock holder was not ready");
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
    drop(lock_stdin);
    let lock_status = lock_holder.wait().unwrap();
    let mut lock_stderr = String::new();
    lock_holder
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut lock_stderr)
        .unwrap();
    assert!(
        lock_status.success(),
        "workspace lock holder failed\nstderr:\n{lock_stderr}"
    );
    let stdin = writer.join().unwrap();
    reader.join().unwrap();
    let expected_receipts = FRAME_COUNT * 2 + 6;
    let processed_deadline = Instant::now() + Duration::from_secs(45);
    let mut last_processed = 0;
    loop {
        let processed = receipt_reader
            .count_agent_hook_receipts_for_connection(&connection_id, Some("processed"))
            .unwrap();
        assert!(
            processed >= last_processed,
            "processed capture receipt count regressed: {processed} after {last_processed}"
        );
        if processed == expected_receipts {
            break;
        }
        assert!(
            Instant::now() < processed_deadline,
            "capture did not process every forwarded receipt: {processed} of {}",
            expected_receipts
        );
        last_processed = processed;
        thread::sleep(Duration::from_millis(50));
    }
    let processed_receipts = connection_receipts(&receipt_reader, &connection_id);
    assert_eq!(processed_receipts.len(), expected_receipts);
    assert!(processed_receipts
        .iter()
        .all(|receipt| receipt.status == "processed"));
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

    let receipt_reader = Trail::open(temp.path()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut last_processed = 0;
    loop {
        let processed = receipt_reader
            .count_agent_hook_receipts_for_connection(CONNECTION_ID, Some("processed"))
            .unwrap();
        assert!(
            processed >= last_processed,
            "processed replay receipt count regressed: {processed} after {last_processed}"
        );
        if processed == TOTAL {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timed out replaying capture journal: {processed} of {TOTAL} receipts"
        );
        last_processed = processed;
        thread::sleep(Duration::from_millis(50));
    }
    let receipts = connection_receipts(&receipt_reader, CONNECTION_ID);
    assert_eq!(receipts.len(), TOTAL);
    assert!(receipts.iter().all(|receipt| receipt.status == "processed"));
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
