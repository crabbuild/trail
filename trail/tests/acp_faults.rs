#![cfg(any(unix, windows))]

#[path = "support/acp_harness.rs"]
mod acp_harness;

use std::io::{Read, Write};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

const ACP_MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

fn echo_agent(workspace: &std::path::Path, scenario: &str) -> Vec<String> {
    acp_harness::fixture_agent_command(
        workspace,
        scenario,
        r#"#!/usr/bin/env python3
import sys
for line in sys.stdin.buffer:
    sys.stdout.buffer.write(line)
    sys.stdout.buffer.flush()
"#,
    )
}

fn run_with_input(agent_source: &str, input: &[u8]) -> std::process::Output {
    let temp = acp_harness::workspace();
    let agent = acp_harness::fixture_agent_command(temp.path(), "fault", agent_source);
    let mut child = acp_harness::spawn_relay(temp.path(), &agent);
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn malformed_utf8_and_invalid_json_rpc_envelopes_fail_without_invented_output() {
    let malformed_utf8 = run_with_input(
        r#"#!/usr/bin/env python3
import sys
sys.stdout.buffer.write(b"\xff\n")
sys.stdout.buffer.flush()
"#,
        b"",
    );
    assert!(!malformed_utf8.status.success());
    assert!(malformed_utf8.stdout.is_empty());
    assert!(String::from_utf8_lossy(&malformed_utf8.stderr).contains("I/O error"));

    let invalid = [
        br#"null
"#
        .as_slice(),
        br#"{"jsonrpc":"1.0","method":"ext/invalid"}
"#,
        br#"{"jsonrpc":"2.0","id":{},"method":"ext/invalid"}
"#,
        br#"{"jsonrpc":"2.0","id":1}
"#,
        br#"{"jsonrpc":"2.0","id":1,"result":{},"error":{"code":1,"message":"both"}}
"#,
        br#"{"jsonrpc":"2.0","id":1,"error":{"code":"bad","message":7}}
"#,
    ];
    for (index, frame) in invalid.into_iter().enumerate() {
        let temp = acp_harness::workspace();
        let agent = echo_agent(temp.path(), &format!("invalid-{index}"));
        let mut child = acp_harness::spawn_relay(temp.path(), &agent);
        child.stdin.take().unwrap().write_all(frame).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            !output.status.success(),
            "invalid envelope {index} was accepted"
        );
        assert!(
            output.stdout.is_empty(),
            "relay invented output for case {index}"
        );
    }
}

fn frame_of_size(size: usize) -> Vec<u8> {
    let prefix = br#"{"jsonrpc":"2.0","method":"ext/limit","params":{"data":""#;
    let suffix = b"\"}}\n";
    assert!(size >= prefix.len() + suffix.len());
    let mut frame = Vec::with_capacity(size);
    frame.extend_from_slice(prefix);
    frame.resize(size - suffix.len(), b'x');
    frame.extend_from_slice(suffix);
    assert_eq!(frame.len(), size);
    frame
}

#[test]
fn transport_accepts_the_frame_limit_and_rejects_one_byte_above_it() {
    let temp = acp_harness::workspace();
    let agent = echo_agent(temp.path(), "frame-limit");
    let mut child = acp_harness::spawn_relay(temp.path(), &agent);
    let mut stdin = child.stdin.take().unwrap();
    let writer = thread::spawn(move || {
        stdin
            .write_all(&frame_of_size(ACP_MAX_FRAME_BYTES))
            .unwrap();
    });
    let mut stdout = child.stdout.take().unwrap();
    let mut received = Vec::new();
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        let bytes = stdout.read(&mut chunk).unwrap();
        if bytes == 0 {
            break;
        }
        received.extend_from_slice(&chunk[..bytes]);
        thread::sleep(Duration::from_millis(2));
    }
    writer.join().unwrap();
    let status = child.wait().unwrap();
    assert!(status.success());
    assert_eq!(received.len(), ACP_MAX_FRAME_BYTES);
    assert_eq!(received, frame_of_size(ACP_MAX_FRAME_BYTES));

    let temp = acp_harness::workspace();
    let agent = echo_agent(temp.path(), "frame-too-large");
    let mut child = acp_harness::spawn_relay(temp.path(), &agent);
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&frame_of_size(ACP_MAX_FRAME_BYTES + 1))
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("transport limit"));
}

#[test]
fn one_hundred_requests_per_direction_keep_same_ids_and_reverse_order_correlated() {
    const COUNT: usize = 100;
    let temp = acp_harness::workspace();
    let agent = acp_harness::fixture_agent_command(
        temp.path(),
        "concurrency",
        &format!(
            r#"#!/usr/bin/env python3
import json
import sys

requests = [json.loads(sys.stdin.readline()) for _ in range({COUNT})]
for request in requests:
    callback = {{"jsonrpc":"2.0","id":request["id"],"method":"fs/read_text_file","params":{{"sessionId":"s","path":"/tmp/" + str(request["id"])}}}}
    print(json.dumps(callback, separators=(",", ":")), flush=True)
for request in reversed(requests):
    print(json.dumps({{"jsonrpc":"2.0","id":request["id"],"result":{{"sequence":request["id"]}}}}, separators=(",", ":")), flush=True)
responses = [json.loads(sys.stdin.readline()) for _ in range({COUNT})]
assert [response["id"] for response in responses] == list(reversed(range({COUNT})))
"#
        ),
    );
    let mut child = acp_harness::spawn_relay(temp.path(), &agent);
    let (mut stdin, mut stdout) = acp_harness::relay_stdio(&mut child);
    for id in 0..COUNT {
        acp_harness::write_json(
            &mut stdin,
            &json!({"jsonrpc":"2.0","id":id,"method":"ext/concurrent","params":{"sequence":id}}),
        );
    }

    let callbacks = (0..COUNT)
        .map(|_| acp_harness::read_json(&mut stdout))
        .collect::<Vec<_>>();
    assert_eq!(
        callbacks
            .iter()
            .map(|frame| frame["id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        (0..COUNT as u64).collect::<Vec<_>>()
    );
    let responses = (0..COUNT)
        .map(|_| acp_harness::read_json(&mut stdout))
        .collect::<Vec<_>>();
    assert_eq!(
        responses
            .iter()
            .map(|frame| frame["id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        (0..COUNT as u64).rev().collect::<Vec<_>>()
    );

    for callback in callbacks.into_iter().rev() {
        acp_harness::write_json(
            &mut stdin,
            &json!({"jsonrpc":"2.0","id":callback["id"],"result":{"content":"ok"}}),
        );
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn fragmented_writer_and_slow_reader_apply_backpressure_without_loss() {
    let temp = acp_harness::workspace();
    let agent = echo_agent(temp.path(), "backpressure");
    let mut child = acp_harness::spawn_relay(temp.path(), &agent);
    let mut stdin = child.stdin.take().unwrap();
    let frame = json!({"jsonrpc":"2.0","id":"fragmented","method":"ext/backpressure","params":{"data":"x".repeat(1024 * 1024)}});
    let mut raw = serde_json::to_vec(&frame).unwrap();
    raw.push(b'\n');
    let expected = raw.clone();
    let writer = thread::spawn(move || {
        for chunk in raw.chunks(4_093) {
            stdin.write_all(chunk).unwrap();
            thread::yield_now();
        }
    });
    thread::sleep(Duration::from_millis(100));
    let mut received = Vec::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut received)
        .unwrap();
    writer.join().unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(received, expected);
}

#[test]
fn child_crash_and_editor_eof_have_deterministic_distinct_outcomes() {
    let crashed = run_with_input(
        r#"#!/usr/bin/env python3
import sys
sys.stdin.readline()
sys.exit(17)
"#,
        br#"{"jsonrpc":"2.0","id":1,"method":"ext/crash"}
"#,
    );
    assert!(!crashed.status.success());
    assert!(crashed.stdout.is_empty());
    assert!(String::from_utf8_lossy(&crashed.stderr).contains("status"));

    let temp = acp_harness::workspace();
    let agent = acp_harness::fixture_agent_command(
        temp.path(),
        "editor-eof",
        r#"#!/usr/bin/env python3
import time
time.sleep(30)
"#,
    );
    let mut child = acp_harness::spawn_relay(temp.path(), &agent);
    drop(child.stdin.take().unwrap());
    let started = Instant::now();
    let status = child.wait().unwrap();
    assert!(status.success());
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn multiple_sessions_and_opposite_direction_cancellation_races_do_not_cross_correlate() {
    const SESSION_COUNT: usize = 12;
    let temp = acp_harness::workspace();
    let agent = acp_harness::fixture_agent_command(
        temp.path(),
        "sessions-cancellation",
        r#"#!/usr/bin/env python3
import json
import sys

for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    if method == "initialize":
        print(json.dumps({"jsonrpc":"2.0","id":message["id"],"result":{"protocolVersion":1,"agentCapabilities":{}}}, separators=(",", ":")), flush=True)
    elif method == "session/new":
        print(json.dumps({"jsonrpc":"2.0","id":message["id"],"result":{"sessionId":"session-" + str(message["id"])}}, separators=(",", ":")), flush=True)
    elif method == "session/prompt":
        request_id = message["id"]
        print(json.dumps({"jsonrpc":"2.0","method":"$/cancel_request","params":{"requestId":request_id}}, separators=(",", ":")), flush=True)
        print(json.dumps({"jsonrpc":"2.0","id":request_id,"result":{"stopReason":"cancelled"}}, separators=(",", ":")), flush=True)
"#,
    );
    let mut child = acp_harness::spawn_relay(temp.path(), &agent);
    let (mut stdin, mut stdout) = acp_harness::relay_stdio(&mut child);
    acp_harness::write_json(
        &mut stdin,
        &json!({"jsonrpc":"2.0","id":"init","method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{}}}),
    );
    assert_eq!(acp_harness::read_json(&mut stdout)["id"], "init");
    for id in 0..SESSION_COUNT {
        acp_harness::write_json(
            &mut stdin,
            &json!({"jsonrpc":"2.0","id":id,"method":"session/new","params":{"cwd":temp.path(),"mcpServers":[]}}),
        );
        assert_eq!(
            acp_harness::read_json(&mut stdout)["result"]["sessionId"],
            format!("session-{id}")
        );
    }
    for id in 0..SESSION_COUNT {
        let request_id = format!("turn-{id}");
        acp_harness::write_json(
            &mut stdin,
            &json!({"jsonrpc":"2.0","id":request_id,"method":"session/prompt","params":{"sessionId":format!("session-{id}"),"prompt":[{"type":"text","text":"cancel"}]}}),
        );
        acp_harness::write_json(
            &mut stdin,
            &json!({"jsonrpc":"2.0","method":"$/cancel_request","params":{"requestId":request_id}}),
        );
        let cancellation = acp_harness::read_json(&mut stdout);
        let response = acp_harness::read_json(&mut stdout);
        assert_eq!(cancellation["params"]["requestId"], request_id);
        assert_eq!(response["id"], request_id);
        assert_eq!(response["result"]["stopReason"], "cancelled");
    }
    drop(stdin);
    assert!(child.wait().unwrap().success());
}
