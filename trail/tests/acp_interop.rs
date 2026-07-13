#![cfg(any(unix, windows))]

#[path = "support/acp_harness.rs"]
mod acp_harness;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use trail::Trail;

fn reference_peer() -> PathBuf {
    std::env::var_os("TRAIL_ACP_REFERENCE_PEER")
        .map(PathBuf::from)
        .expect("TRAIL_ACP_REFERENCE_PEER must point to the official-type peer")
}

fn run_client(mode: &str, workspace: &Path, agent: &Path) -> std::process::Output {
    Command::new(reference_peer())
        .arg(mode)
        .arg(workspace)
        .arg(acp_harness::trail_bin())
        .arg(agent)
        .output()
        .unwrap()
}

#[test]
fn official_v1_client_through_trail_to_official_v1_agent() {
    let temp = acp_harness::workspace();
    let output = run_client("client", temp.path(), &reference_peer());
    assert!(
        output.status.success(),
        "official peer failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let db = Trail::open(temp.path()).unwrap();
    let mapping = db.lane_acp_session("official-session").unwrap();
    let session = db.show_lane_session(&mapping.trail_session_id).unwrap();
    assert!(session
        .events
        .iter()
        .any(|event| event.event_type == "acp_prompt_started"));
    assert!(session
        .events
        .iter()
        .any(|event| event.event_type == "acp_prompt_finished"));
    let callback_methods = session
        .events
        .iter()
        .filter(|event| event.event_type == "acp_client_callback_requested")
        .filter_map(|event| event.payload.as_ref())
        .filter_map(|payload| payload["method"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(callback_methods.len(), 8);
}

#[test]
#[cfg(unix)]
fn official_v1_client_through_trail_to_independent_fixture_agent() {
    let temp = acp_harness::workspace();
    let agent = acp_harness::fixture_agent_command(
        temp.path(),
        "interop-fixture",
        r#"#!/usr/bin/env python3
import json
import sys

for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    if method == "initialize":
        result = {"protocolVersion":1,"agentCapabilities":{}}
    elif method == "session/new":
        result = {"sessionId":"fixture-session"}
    elif method == "session/prompt":
        print(json.dumps({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fixture-session","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"fixture update"}}}}, separators=(",", ":")), flush=True)
        result = {"stopReason":"end_turn"}
    elif method == "session/close":
        result = {}
    else:
        raise RuntimeError("unexpected method " + str(method))
    print(json.dumps({"jsonrpc":"2.0","id":message["id"],"result":result}, separators=(",", ":")), flush=True)
"#,
    );
    assert_eq!(agent.len(), 1, "interop fixture expects a direct launcher");
    let output = run_client("client-basic", temp.path(), Path::new(&agent[0]));
    assert!(
        output.status.success(),
        "official client / fixture agent failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(fs::metadata(temp.path().join(".trail")).is_ok());
}
