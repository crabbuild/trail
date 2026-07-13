#![cfg(unix)]

use std::fs;
use std::io::{BufRead, BufReader, Write};
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

fn write_turn_agent(workspace: &Path) -> PathBuf {
    let agent = workspace.join("turn-agent.py");
    fs::write(
        &agent,
        r#"#!/usr/bin/env python3
import json
import sys

pending_prompt = None
config_values = {}
for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    request_id = message.get("id")
    if method == "initialize":
        result = {"protocolVersion":1,"agentCapabilities":{"promptCapabilities":{"image":True,"audio":True,"embeddedContext":True},"sessionCapabilities":{}}}
        print(json.dumps({"jsonrpc":"2.0","id":request_id,"result":result}, separators=(",", ":")), flush=True)
    elif method == "session/new":
        print(json.dumps({"jsonrpc":"2.0","id":request_id,"result":{"sessionId":"turn-session"}}, separators=(",", ":")), flush=True)
        print(json.dumps({"jsonrpc":"2.0","id":"agent-callback","method":"session/request_permission","params":{"sessionId":"turn-session","toolCall":{"toolCallId":"permission-tool","title":"Permission fixture"},"options":[{"optionId":"allow","name":"Allow","kind":"allow_once"}]}}, separators=(",", ":")), flush=True)
        print(json.dumps({"jsonrpc":"2.0","method":"$/cancel_request","params":{"requestId":"agent-callback"}}, separators=(",", ":")), flush=True)
    elif method in ("session/set_mode", "session/set_config_option"):
        if str(request_id).endswith("-error"):
            response = {"jsonrpc":"2.0","id":request_id,"error":{"code":-32002,"message":"configuration rejected"}}
        else:
            result = {}
            if method == "session/set_config_option":
                config_id = message["params"]["configId"]
                value = message["params"]["value"]
                config_values[config_id] = "deep" if config_id == "model" else value
                result["configOptions"] = [
                    {"id":key,"name":key,"type":"boolean","currentValue":current}
                    if isinstance(current, bool)
                    else {"id":key,"name":key,"type":"select","currentValue":current,"options":[{"value":current,"name":current}]}
                    for key, current in config_values.items()
                ]
            response = {"jsonrpc":"2.0","id":request_id,"result":result}
        print(json.dumps(response, separators=(",", ":")), flush=True)
    elif method == "session/prompt" and request_id == "prompt-cancel":
        pending_prompt = request_id
        print(json.dumps({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"turn-session","update":{"sessionUpdate":"agent_message_chunk","messageId":"cancelled-answer","content":{"type":"text","text":"partial answer"}}}}, separators=(",", ":")), flush=True)
    elif method == "session/cancel":
        pass
    elif method == "$/cancel_request":
        if pending_prompt is not None:
            print(json.dumps({"jsonrpc":"2.0","id":pending_prompt,"error":{"code":-32800,"message":"request cancelled"}}, separators=(",", ":")), flush=True)
            pending_prompt = None
    elif method == "session/prompt":
        print(json.dumps({"jsonrpc":"2.0","id":request_id,"result":{"stopReason":"end_turn"}}, separators=(",", ":")), flush=True)
    elif method is None:
        pass
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&agent).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&agent, permissions).unwrap();
    agent
}

fn send(stdin: &mut impl Write, message: &serde_json::Value) {
    serde_json::to_writer(&mut *stdin, message).unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();
}

fn receive(stdout: &mut impl BufRead) -> serde_json::Value {
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert!(!line.is_empty());
    serde_json::from_str(line.trim()).unwrap()
}

fn exchange(
    stdin: &mut impl Write,
    stdout: &mut impl BufRead,
    message: serde_json::Value,
) -> serde_json::Value {
    send(stdin, &message);
    receive(stdout)
}

#[test]
fn prompt_configuration_and_both_cancellation_directions_are_projected() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "turn fixture\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let agent = write_turn_agent(temp.path());
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
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{}}}),
    );
    exchange(
        &mut stdin,
        &mut stdout,
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":temp.path(),"mcpServers":[]}}),
    );
    let permission = receive(&mut stdout);
    assert_eq!(permission["method"], "session/request_permission");
    let agent_cancel = receive(&mut stdout);
    assert_eq!(agent_cancel["method"], "$/cancel_request");
    send(
        &mut stdin,
        &serde_json::json!({"jsonrpc":"2.0","id":"agent-callback","result":{"outcome":{"outcome":"cancelled"}}}),
    );

    for (id, method, params, succeeds) in [
        (
            "mode-error",
            "session/set_mode",
            serde_json::json!({"sessionId":"turn-session","modeId":"ask"}),
            false,
        ),
        (
            "mode-success",
            "session/set_mode",
            serde_json::json!({"sessionId":"turn-session","modeId":"code"}),
            true,
        ),
        (
            "config-bool-success",
            "session/set_config_option",
            serde_json::json!({"sessionId":"turn-session","configId":"autoFix","type":"boolean","value":true}),
            true,
        ),
        (
            "config-select-success",
            "session/set_config_option",
            serde_json::json!({"sessionId":"turn-session","configId":"model","value":"fast"}),
            true,
        ),
    ] {
        let response = exchange(
            &mut stdin,
            &mut stdout,
            serde_json::json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
        );
        assert_eq!(response.get("result").is_some(), succeeds);
    }

    let prompt = serde_json::json!([
        {"type":"text","text":"Explain the attached context","_meta":{"extension":"text-meta"}},
        {"type":"image","data":"aW1hZ2U=","mimeType":"image/png","_meta":{"extension":"image-meta"}},
        {"type":"audio","data":"YXVkaW8=","mimeType":"audio/wav"},
        {"type":"resource_link","name":"README","uri":"file:///README.md","_meta":{"extension":"link-meta"}},
        {"type":"resource","resource":{"uri":"file:///context.txt","mimeType":"text/plain","text":"embedded context"},"_meta":{"extension":"resource-meta"}}
    ]);
    send(
        &mut stdin,
        &serde_json::json!({"jsonrpc":"2.0","id":"prompt-cancel","method":"session/prompt","params":{"sessionId":"turn-session","prompt":prompt}}),
    );
    let update = receive(&mut stdout);
    assert_eq!(update["method"], "session/update");
    send(
        &mut stdin,
        &serde_json::json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"turn-session"}}),
    );
    send(
        &mut stdin,
        &serde_json::json!({"jsonrpc":"2.0","method":"$/cancel_request","params":{"requestId":"prompt-cancel"}}),
    );
    let cancelled = receive(&mut stdout);
    assert_eq!(cancelled["error"]["code"], -32800);

    let completed = exchange(
        &mut stdin,
        &mut stdout,
        serde_json::json!({"jsonrpc":"2.0","id":"prompt-complete","method":"session/prompt","params":{"sessionId":"turn-session","prompt":[{"type":"text","text":"Second prompt"}]}}),
    );
    assert_eq!(completed["result"]["stopReason"], "end_turn");
    send(
        &mut stdin,
        &serde_json::json!({"jsonrpc":"2.0","method":"$/cancel_request","params":{"requestId":"prompt-complete"}}),
    );
    let reused_id = exchange(
        &mut stdin,
        &mut stdout,
        serde_json::json!({"jsonrpc":"2.0","id":"prompt-complete","method":"session/set_config_option","params":{"sessionId":"turn-session","configId":"afterCancel","value":true}}),
    );
    assert!(reused_id.get("result").is_some());

    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "relay failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let db = Trail::open(temp.path()).unwrap();
    let mapping = db.lane_acp_session("turn-session").unwrap();
    assert_eq!(mapping.current_mode_id.as_deref(), Some("code"));
    assert_eq!(mapping.config_options["autoFix"], true);
    assert_eq!(mapping.config_options["model"], "deep");
    assert_eq!(mapping.config_options["afterCancel"], true);
    let session = db.show_lane_session(&mapping.trail_session_id).unwrap();
    assert_eq!(session.turns.len(), 2);
    let statuses = session
        .turns
        .iter()
        .map(|turn| turn.status.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert!(statuses.contains("cancelled"));
    assert!(statuses.contains("completed"));
    let content_event = session
        .events
        .iter()
        .find(|event| event.event_type == "acp_prompt_content")
        .and_then(|event| event.payload.as_ref())
        .unwrap();
    assert_eq!(content_event["blocks"].as_array().unwrap().len(), 5);
    assert_eq!(
        content_event["blocks"][0]["_meta"]["extension"],
        "text-meta"
    );
    assert_eq!(
        content_event["blocks"][3]["_meta"]["extension"],
        "link-meta"
    );
    assert_eq!(
        content_event["blocks"][4]["resource"]["text"],
        "embedded context"
    );
    assert_eq!(content_event["blocks"][1]["data"]["encoding"], "base64");
    assert_eq!(content_event["blocks"][2]["data"]["encoding"], "base64");
    assert!(!serde_json::to_string(content_event)
        .unwrap()
        .contains("aW1hZ2U="));
    let cancel_events = session
        .events
        .iter()
        .filter(|event| event.event_type == "acp_request_cancelled")
        .count();
    assert!(
        cancel_events >= 3,
        "missing cancellation direction/race evidence"
    );
    let reused_id_event = session
        .events
        .iter()
        .filter(|event| event.event_type == "acp_session_config_changed")
        .filter_map(|event| event.payload.as_ref())
        .find(|payload| payload["config_id"] == "afterCancel")
        .unwrap();
    assert_eq!(reused_id_event["cancel_requested"], false);
}
