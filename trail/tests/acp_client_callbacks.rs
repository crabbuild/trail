#![cfg(unix)]

use std::collections::{BTreeMap, BTreeSet};
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

fn write_callback_agent(workspace: &Path) -> PathBuf {
    let agent = workspace.join("callback-agent.py");
    fs::write(
        &agent,
        r#"#!/usr/bin/env python3
import json
import sys

session_id = "callback-session"
effective_cwd = None
prompt_id = None
phase_one = {"perm-selected", "perm-cancelled", "perm-error", "read-ok", "read-error", "write-ok", "write-error", "create-ok", "create-error", "create-abandoned"}
phase_two = {"output-running", "output-exited", "output-signal", "output-error", "wait-ok", "wait-error", "kill-ok", "kill-error", "release-ok", "release-error"}
seen_one = set()
seen_two = set()

def emit(request_id, method, params):
    print(json.dumps({"jsonrpc":"2.0","id":request_id,"method":method,"params":params}, separators=(",", ":")), flush=True)

for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    request_id = message.get("id")
    if method == "initialize":
        print(json.dumps({"jsonrpc":"2.0","id":request_id,"result":{"protocolVersion":1,"agentCapabilities":{}}}, separators=(",", ":")), flush=True)
    elif method == "session/new":
        effective_cwd = message["params"]["cwd"]
        print(json.dumps({"jsonrpc":"2.0","id":request_id,"result":{"sessionId":session_id}}, separators=(",", ":")), flush=True)
    elif method == "session/prompt":
        prompt_id = request_id
        options = [
            {"optionId":"allow-once","name":"Allow once","kind":"allow_once"},
            {"optionId":"allow-always","name":"Allow always","kind":"allow_always"},
            {"optionId":"reject-once","name":"Reject once","kind":"reject_once"},
            {"optionId":"reject-always","name":"Reject always","kind":"reject_always"}
        ]
        tool = {"toolCallId":"permission-tool","title":"Permission matrix"}
        emit("perm-selected", "session/request_permission", {"sessionId":session_id,"toolCall":tool,"options":options})
        emit(101, "session/request_permission", {"sessionId":session_id,"toolCall":tool,"options":options})
        emit("perm-error", "session/request_permission", {"sessionId":session_id,"toolCall":tool,"options":options})
        emit("read-ok", "fs/read_text_file", {"sessionId":session_id,"path":effective_cwd + "/read.txt","line":2,"limit":5})
        emit(202, "fs/read_text_file", {"sessionId":session_id,"path":effective_cwd + "/missing.txt"})
        emit("write-ok", "fs/write_text_file", {"sessionId":session_id,"path":effective_cwd + "/write.txt","content":"safe\napi_key=write-secret\n"})
        emit(303, "fs/write_text_file", {"sessionId":session_id,"path":effective_cwd + "/denied.txt","content":"password=denied-secret"})
        terminal = {"sessionId":session_id,"command":"printf","args":["--","hello"],"env":[{"name":"API_TOKEN","value":"terminal-secret"}],"cwd":effective_cwd,"outputByteLimit":16}
        emit("create-ok", "terminal/create", terminal)
        emit(404, "terminal/create", terminal)
        emit("create-abandoned", "terminal/create", terminal)
        print(json.dumps({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":session_id,"update":{"sessionUpdate":"usage_update","used":1,"size":100}}}, separators=(",", ":")), flush=True)
    elif method is None:
        key = str(request_id)
        aliases = {"101":"perm-cancelled", "202":"read-error", "303":"write-error", "404":"create-error"}
        key = aliases.get(key, key)
        if key in phase_one:
            seen_one.add(key)
            if seen_one == phase_one:
                emit("output-running", "terminal/output", {"sessionId":session_id,"terminalId":"terminal-1"})
                emit(505, "terminal/output", {"sessionId":session_id,"terminalId":"terminal-1"})
                emit("output-signal", "terminal/output", {"sessionId":session_id,"terminalId":"terminal-1"})
                emit("output-error", "terminal/output", {"sessionId":session_id,"terminalId":"terminal-1"})
                emit("kill-ok", "terminal/kill", {"sessionId":session_id,"terminalId":"terminal-1"})
                emit(707, "terminal/kill", {"sessionId":session_id,"terminalId":"terminal-1"})
                emit("wait-ok", "terminal/wait_for_exit", {"sessionId":session_id,"terminalId":"terminal-1"})
                emit(606, "terminal/wait_for_exit", {"sessionId":session_id,"terminalId":"terminal-1"})
                emit("release-ok", "terminal/release", {"sessionId":session_id,"terminalId":"terminal-1"})
                emit(808, "terminal/release", {"sessionId":session_id,"terminalId":"terminal-1"})
        else:
            aliases = {"505":"output-exited", "606":"wait-error", "707":"kill-error", "808":"release-error"}
            key = aliases.get(key, key)
            if key in phase_two:
                seen_two.add(key)
                if seen_two == phase_two:
                    emit("shutdown-pending", "fs/read_text_file", {"sessionId":session_id,"path":effective_cwd + "/pending.txt"})
                    print(json.dumps({"jsonrpc":"2.0","id":prompt_id,"result":{"stopReason":"end_turn"}}, separators=(",", ":")), flush=True)
                    sys.exit(0)
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&agent).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&agent, permissions).unwrap();
    agent
}

fn send(stdin: &mut impl Write, message: &Value) {
    serde_json::to_writer(&mut *stdin, message).unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();
}

fn receive(stdout: &mut impl BufRead) -> Value {
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert!(!line.is_empty());
    serde_json::from_str(line.trim()).unwrap()
}

fn exchange(stdin: &mut impl Write, stdout: &mut impl BufRead, message: Value) -> Value {
    send(stdin, &message);
    receive(stdout)
}

fn error_response(id: Value) -> Value {
    serde_json::json!({"jsonrpc":"2.0","id":id,"error":{"code":-32001,"message":"fixture rejection"}})
}

#[test]
fn all_client_callbacks_preserve_correlation_paths_artifacts_and_terminal_lifecycle() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "callback fixture\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let agent = write_callback_agent(temp.path());
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
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{"fs":{"readTextFile":true,"writeTextFile":true},"terminal":true}}}),
    );
    exchange(
        &mut stdin,
        &mut stdout,
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":temp.path(),"mcpServers":[]}}),
    );
    send(
        &mut stdin,
        &serde_json::json!({"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":"callback-session","prompt":[{"type":"text","text":"Run callback matrix"}]}}),
    );

    let mut seen_methods = BTreeSet::new();
    loop {
        let message = receive(&mut stdout);
        if message.get("id") == Some(&Value::from(3)) {
            break;
        }
        let method = message["method"].as_str().unwrap();
        if method == "session/update" {
            continue;
        }
        seen_methods.insert(method.to_string());
        let id = message["id"].clone();
        if matches!(method, "fs/read_text_file" | "fs/write_text_file") {
            assert!(message["params"]["path"]
                .as_str()
                .unwrap()
                .starts_with(temp.path().to_string_lossy().as_ref()));
        }
        if method == "terminal/create" {
            assert_eq!(
                message["params"]["cwd"],
                temp.path().to_string_lossy().as_ref()
            );
            assert_eq!(message["params"]["env"][0]["value"], "terminal-secret");
        }
        let response = match (method, id.as_str(), id.as_i64()) {
            ("session/request_permission", Some("perm-selected"), _) => {
                serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"outcome":{"outcome":"selected","optionId":"allow-once"}}})
            }
            ("session/request_permission", _, Some(101)) => {
                serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"outcome":{"outcome":"cancelled"}}})
            }
            ("session/request_permission", _, _) => error_response(id),
            ("fs/read_text_file", Some("read-ok"), _) => {
                serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"content":"line 2\ntoken=read-secret\n"}})
            }
            ("fs/read_text_file", Some("shutdown-pending"), _) => continue,
            ("fs/read_text_file", _, _) => error_response(id),
            ("fs/write_text_file", Some("write-ok"), _) => {
                serde_json::json!({"jsonrpc":"2.0","id":id,"result":{}})
            }
            ("fs/write_text_file", _, _) => error_response(id),
            ("terminal/create", Some("create-ok"), _) => {
                serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"terminalId":"terminal-1"}})
            }
            ("terminal/create", Some("create-abandoned"), _) => {
                serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"terminalId":"terminal-2"}})
            }
            ("terminal/create", _, _) => error_response(id),
            ("terminal/output", Some("output-running"), _) => {
                serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"output":"0123456789abcdef","truncated":true,"exitStatus":null}})
            }
            ("terminal/output", _, Some(505)) => {
                serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"output":"done","truncated":false,"exitStatus":{"exitCode":0,"signal":null}}})
            }
            ("terminal/output", Some("output-signal"), _) => {
                serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"output":"killed","truncated":false,"exitStatus":{"exitCode":null,"signal":"SIGTERM"}}})
            }
            ("terminal/output", _, _) => error_response(id),
            ("terminal/wait_for_exit", Some("wait-ok"), _) => {
                serde_json::json!({"jsonrpc":"2.0","id":id,"result":{"exitCode":null,"signal":"SIGKILL"}})
            }
            ("terminal/wait_for_exit", _, _) => error_response(id),
            ("terminal/kill", Some("kill-ok"), _) => {
                serde_json::json!({"jsonrpc":"2.0","id":id,"result":{}})
            }
            ("terminal/kill", _, _) => error_response(id),
            ("terminal/release", Some("release-ok"), _) => {
                serde_json::json!({"jsonrpc":"2.0","id":id,"result":{}})
            }
            ("terminal/release", _, _) => error_response(id),
            _ => panic!("unexpected callback: {message}"),
        };
        send(&mut stdin, &response);
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "relay failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        seen_methods,
        [
            "fs/read_text_file",
            "fs/write_text_file",
            "session/request_permission",
            "terminal/create",
            "terminal/kill",
            "terminal/output",
            "terminal/release",
            "terminal/wait_for_exit",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    );

    let db = Trail::open(temp.path()).unwrap();
    let mapping = db.lane_acp_session("callback-session").unwrap();
    let session = db.show_lane_session(&mapping.trail_session_id).unwrap();
    let requested = session
        .events
        .iter()
        .filter(|event| event.event_type == "acp_client_callback_requested")
        .filter_map(|event| event.payload.as_ref())
        .collect::<Vec<_>>();
    let finished = session
        .events
        .iter()
        .filter(|event| event.event_type == "acp_client_callback_finished")
        .filter_map(|event| event.payload.as_ref())
        .collect::<Vec<_>>();
    let requested_methods = requested
        .iter()
        .filter_map(|payload| payload["method"].as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(requested_methods.len(), 8);
    assert!(requested
        .iter()
        .any(|payload| payload["request_id"].is_string()));
    assert!(requested
        .iter()
        .any(|payload| payload["request_id"].is_number()));
    let permission_request = requested
        .iter()
        .find(|payload| payload["method"] == "session/request_permission")
        .unwrap();
    assert_eq!(
        permission_request["details"]["option_kinds"]
            .as_object()
            .unwrap()
            .values()
            .filter_map(Value::as_str)
            .collect::<BTreeSet<_>>(),
        ["allow_always", "allow_once", "reject_always", "reject_once"]
            .into_iter()
            .collect()
    );
    let terminal_request = requested
        .iter()
        .find(|payload| payload["method"] == "terminal/create")
        .unwrap();
    assert_eq!(
        terminal_request["details"]["env_names"],
        serde_json::json!(["API_TOKEN"])
    );
    assert!(finished.iter().any(|payload| payload["success"] == true));
    assert!(finished.iter().any(|payload| payload["success"] == false));
    assert!(finished.iter().any(|payload| {
        payload["error"]["message"] == "relay shutdown with callback in flight"
    }));
    assert!(session
        .events
        .iter()
        .any(|event| event.event_type == "acp_terminal_abandoned"));
    assert!(session
        .events
        .iter()
        .any(|event| event.event_type == "acp_usage_update"));

    let by_method = finished.iter().fold(
        BTreeMap::<&str, (usize, usize)>::new(),
        |mut counts, payload| {
            let method = payload["method"].as_str().unwrap();
            let entry = counts.entry(method).or_default();
            if payload["success"] == true {
                entry.0 += 1;
            } else {
                entry.1 += 1;
            }
            counts
        },
    );
    for method in [
        "session/request_permission",
        "fs/read_text_file",
        "fs/write_text_file",
        "terminal/create",
        "terminal/output",
        "terminal/wait_for_exit",
        "terminal/kill",
        "terminal/release",
    ] {
        let (success, error) = by_method[method];
        assert!(success > 0, "missing {method} success: {by_method:?}");
        assert!(error > 0, "missing {method} error: {by_method:?}");
    }

    let path_event = requested
        .iter()
        .find(|payload| payload["method"] == "fs/read_text_file")
        .unwrap();
    assert_ne!(
        path_event["details"]["effective_path"],
        path_event["details"]["forwarded_path"]
    );
    let output_event = finished
        .iter()
        .find(|payload| {
            payload["method"] == "terminal/output" && payload["result"]["truncated"] == true
        })
        .unwrap();
    assert_eq!(output_event["result"]["output_byte_len"], 16);
    assert!(!output_event.to_string().contains("0123456789abcdef"));
    let kill_position = finished
        .iter()
        .position(|payload| {
            payload["method"] == "terminal/kill" && payload["request_id"] == "kill-ok"
        })
        .unwrap();
    let wait_position = finished
        .iter()
        .position(|payload| {
            payload["method"] == "terminal/wait_for_exit" && payload["request_id"] == "wait-ok"
        })
        .unwrap();
    let release_position = finished
        .iter()
        .position(|payload| {
            payload["method"] == "terminal/release" && payload["request_id"] == "release-ok"
        })
        .unwrap();
    assert!(kill_position < wait_position);
    assert!(wait_position < release_position);

    let artifacts = db
        .list_lane_artifacts(&mapping.trail_session_id, None, 20)
        .unwrap();
    assert_eq!(artifacts.len(), 2);
    for artifact in artifacts {
        assert_eq!(artifact.artifact_kind, "acp_fs_write");
        let content =
            String::from_utf8(db.lane_artifact_content(&artifact.artifact_id).unwrap()).unwrap();
        assert!(!content.contains("write-secret"));
        assert!(!content.contains("denied-secret"));
    }
    assert!(!session
        .events
        .iter()
        .filter_map(|event| event.payload.as_ref())
        .any(|payload| payload.to_string().contains("terminal-secret")));
}
