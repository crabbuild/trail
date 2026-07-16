#![cfg(unix)]

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use trail::{InitImportMode, Trail};

fn trail_bin() -> PathBuf {
    std::env::var_os("TRAIL_TEST_BIN")
        .map(PathBuf::from)
        .or_else(|| option_env!("CARGO_BIN_EXE_trail").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/debug/trail"))
}

fn write_lifecycle_agent(workspace: &Path) -> PathBuf {
    let agent = workspace.join("lifecycle-agent.py");
    fs::write(
        &agent,
        r#"#!/usr/bin/env python3
import json
import sys

for line in sys.stdin:
    request = json.loads(line)
    request_id = request.get("id")
    method = request.get("method")
    if method == "initialize":
        result = {
            "protocolVersion": 1,
            "agentCapabilities": {
                "loadSession": True,
                "sessionCapabilities": {
                    "list": {}, "delete": {}, "resume": {}, "close": {}
                }
            },
            "authMethods": [{"id": "fixture", "name": "Fixture"}]
        }
        response = {"jsonrpc": "2.0", "id": request_id, "result": result}
    elif str(request_id).endswith("-error"):
        response = {
            "jsonrpc": "2.0",
            "id": request_id,
            "error": {"code": -32001, "message": "fixture failure"}
        }
    elif method == "session/new":
        response = {"jsonrpc": "2.0", "id": request_id, "result": {"sessionId": "life-session"}}
    elif method == "session/list":
        response = {
            "jsonrpc": "2.0",
            "id": request_id,
            "result": {"sessions": [], "nextCursor": "next-page"}
        }
    else:
        response = {"jsonrpc": "2.0", "id": request_id, "result": {}}
    print(json.dumps(response, separators=(",", ":")), flush=True)
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&agent).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&agent, permissions).unwrap();
    agent
}

fn write_load_replay_agent(workspace: &Path) -> PathBuf {
    let agent = workspace.join("load-replay-agent.py");
    fs::write(
        &agent,
        r#"#!/usr/bin/env python3
import json
import sys

initialize = json.loads(sys.stdin.readline())
print(json.dumps({"jsonrpc":"2.0","id":initialize["id"],"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":True}}}, separators=(",", ":")), flush=True)
load = json.loads(sys.stdin.readline())
session_id = load["params"]["sessionId"]
print(json.dumps({"jsonrpc":"2.0","id":load["id"],"result":{"modes":{"currentModeId":"code","availableModes":[]},"configOptions":[]}}, separators=(",", ":")), flush=True)
updates = [
    {"sessionUpdate":"user_message_chunk","messageId":"user-1","content":{"type":"text","text":"Question one"}},
    {"sessionUpdate":"agent_message_chunk","messageId":"agent-1","content":{"type":"text","text":"Answer one"}},
    {"sessionUpdate":"tool_call","toolCallId":"tool-1","title":"Inspect","kind":"read","status":"pending","content":[],"locations":[]},
    {"sessionUpdate":"tool_call_update","toolCallId":"tool-1","status":"completed"},
    {"sessionUpdate":"plan","entries":[{"content":"Inspect files","priority":"high","status":"completed"}]},
    {"sessionUpdate":"usage_update","used":10,"size":100},
    {"sessionUpdate":"user_message_chunk","messageId":"user-2","content":{"type":"text","text":"Question two"}},
    {"sessionUpdate":"agent_message_chunk","messageId":"agent-2","content":{"type":"text","text":"Answer two"}}
]
for update in updates:
    print(json.dumps({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":session_id,"update":update}}, separators=(",", ":")), flush=True)
sys.stdin.read()
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&agent).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&agent, permissions).unwrap();
    agent
}

fn exchange(
    stdin: &mut impl Write,
    stdout: &mut impl BufRead,
    request: serde_json::Value,
) -> serde_json::Value {
    serde_json::to_writer(&mut *stdin, &request).unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();
    let mut response = String::new();
    stdout.read_line(&mut response).unwrap();
    assert!(
        !response.is_empty(),
        "agent closed before responding to {request}"
    );
    serde_json::from_str(response.trim()).unwrap()
}

#[test]
fn lifecycle_outcomes_preserve_mapping_state_and_redact_auth_metadata() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "lifecycle fixture\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let agent = write_lifecycle_agent(temp.path());
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

    let initialize = exchange(
        &mut stdin,
        &mut stdout,
        serde_json::json!({
            "jsonrpc":"2.0", "id":"initialize", "method":"initialize",
            "params":{"protocolVersion":1,"clientCapabilities":{}}
        }),
    );
    assert_eq!(initialize["result"]["protocolVersion"], 1);

    for (id, method, params) in [
        (
            "authenticate-error",
            "authenticate",
            serde_json::json!({"methodId":"fixture","_meta":{"authorization":"Bearer never-store-this"}}),
        ),
        (
            "authenticate-success",
            "authenticate",
            serde_json::json!({"methodId":"fixture"}),
        ),
        ("logout-error", "logout", serde_json::json!({})),
        ("logout-success", "logout", serde_json::json!({})),
        (
            "new-error",
            "session/new",
            serde_json::json!({"cwd":temp.path(),"mcpServers":[]}),
        ),
        (
            "new-success",
            "session/new",
            serde_json::json!({"cwd":temp.path(),"mcpServers":[]}),
        ),
        (
            "load-error",
            "session/load",
            serde_json::json!({"sessionId":"life-session","cwd":temp.path(),"mcpServers":[]}),
        ),
        (
            "load-success",
            "session/load",
            serde_json::json!({"sessionId":"life-session","cwd":temp.path(),"mcpServers":[]}),
        ),
        (
            "resume-error",
            "session/resume",
            serde_json::json!({"sessionId":"life-session","cwd":temp.path(),"mcpServers":[]}),
        ),
        (
            "resume-success",
            "session/resume",
            serde_json::json!({"sessionId":"life-session","cwd":temp.path(),"mcpServers":[]}),
        ),
        (
            "list-error",
            "session/list",
            serde_json::json!({"cursor":"first-page"}),
        ),
        (
            "list-success",
            "session/list",
            serde_json::json!({"cursor":"first-page"}),
        ),
        (
            "close-error",
            "session/close",
            serde_json::json!({"sessionId":"life-session"}),
        ),
        (
            "close-success",
            "session/close",
            serde_json::json!({"sessionId":"life-session"}),
        ),
        (
            "delete-error",
            "session/delete",
            serde_json::json!({"sessionId":"life-session"}),
        ),
        (
            "delete-success",
            "session/delete",
            serde_json::json!({"sessionId":"life-session"}),
        ),
    ] {
        let response = exchange(
            &mut stdin,
            &mut stdout,
            serde_json::json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
        );
        if id.ends_with("-error") {
            assert_eq!(response["error"]["code"], -32001, "{method} error response");
        } else {
            assert!(
                response.get("result").is_some(),
                "{method} success response"
            );
        }
    }

    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "relay failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db = Trail::open(temp.path()).unwrap();
    let mapping = db
        .try_lane_acp_session("life-session")
        .unwrap()
        .expect("successful session/new must create a durable mapping");
    assert_eq!(
        db.list_lane_acp_sessions(None).unwrap().sessions.len(),
        1,
        "failed lifecycle requests must not create mappings"
    );
    assert_eq!(mapping.status, "deleted");
    let session = db.show_lane_session(&mapping.trail_session_id).unwrap();
    let failed_methods = session
        .events
        .iter()
        .filter(|event| event.event_type == "acp_session_request_failed")
        .filter_map(|event| event.payload.as_ref())
        .filter_map(|payload| payload.get("method"))
        .filter_map(serde_json::Value::as_str)
        .collect::<std::collections::HashSet<_>>();
    assert!(failed_methods.contains("session/load"));
    assert!(failed_methods.contains("session/resume"));
    assert!(failed_methods.contains("session/delete"));
    assert!(session
        .events
        .iter()
        .any(|event| event.event_type == "acp_session_close_failed"));
    assert!(session
        .events
        .iter()
        .any(|event| event.event_type == "acp_session_deleted"));
    let connection_events = session
        .events
        .iter()
        .filter(|event| event.event_type == "acp_connection_lifecycle")
        .filter_map(|event| event.payload.as_ref())
        .collect::<Vec<_>>();
    let connection_methods = connection_events
        .iter()
        .filter_map(|payload| payload.get("method"))
        .filter_map(serde_json::Value::as_str)
        .collect::<std::collections::HashSet<_>>();
    for method in ["initialize", "authenticate", "logout", "session/list"] {
        assert!(
            connection_methods.contains(method),
            "missing normalized {method} lifecycle evidence: {connection_events:?}"
        );
    }
    assert!(connection_events.iter().any(|payload| {
        payload["method"] == "session/list"
            && payload["outcome"] == "success"
            && payload["context"]["request_cursor"] == "first-page"
            && payload["result"]["nextCursor"] == "next-page"
    }));

    let mut persisted = Vec::new();
    for entry in walkdir::WalkDir::new(temp.path().join(".trail")) {
        let entry = entry.unwrap();
        if entry.file_type().is_file() {
            let mut file = fs::File::open(entry.path()).unwrap();
            file.read_to_end(&mut persisted).unwrap();
        }
    }
    assert!(
        !persisted
            .windows(b"never-store-this".len())
            .any(|window| window == b"never-store-this"),
        "authentication metadata leaked into Trail storage"
    );
}

#[test]
fn load_replay_reconstructs_ordered_transcript_turns() {
    const UPDATE_COUNT: usize = 8;

    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "load replay fixture\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let agent = write_load_replay_agent(temp.path());
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
    exchange(
        &mut stdin,
        &mut stdout,
        serde_json::json!({
            "jsonrpc":"2.0", "id":1, "method":"initialize",
            "params":{"protocolVersion":1,"clientCapabilities":{}}
        }),
    );
    let response = exchange(
        &mut stdin,
        &mut stdout,
        serde_json::json!({
            "jsonrpc":"2.0", "id":2, "method":"session/load",
            "params":{"sessionId":"replay-session","cwd":temp.path(),"mcpServers":[]}
        }),
    );
    assert_eq!(response["result"]["modes"]["currentModeId"], "code");
    for _ in 0..UPDATE_COUNT {
        let mut update = String::new();
        stdout.read_line(&mut update).unwrap();
        assert!(update.contains(r#""method":"session/update"#));
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "relay failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db = Trail::open(temp.path()).unwrap();
    let mapping = db.try_lane_acp_session("replay-session").unwrap().unwrap();
    let session = db.show_lane_session(&mapping.trail_session_id).unwrap();
    assert_eq!(
        session.turns.len(),
        2,
        "load replay must create two history turns"
    );
    let messages = session
        .messages
        .iter()
        .map(|message| (message.role.as_str(), message.body.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        messages,
        vec![
            ("user", "Question one"),
            ("assistant", "Answer one"),
            ("user", "Question two"),
            ("assistant", "Answer two")
        ]
    );
    let event_types = session
        .events
        .iter()
        .map(|event| event.event_type.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert!(event_types.contains("tool_call"));
    assert!(event_types.contains("tool_call_update"));
    assert!(event_types.contains("plan_update"));
    assert!(event_types.contains("acp_usage_update"));
}
