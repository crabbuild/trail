#![cfg(unix)]

use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;
use trail::{InitImportMode, Trail};

const FIXTURES: &str = include_str!("fixtures/acp/v1/session_updates.jsonl");
const SCHEMA: &str = include_str!("fixtures/acp/v1/schema.json");

fn trail_bin() -> PathBuf {
    std::env::var_os("TRAIL_TEST_BIN")
        .map(PathBuf::from)
        .or_else(|| option_env!("CARGO_BIN_EXE_trail").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/debug/trail"))
}

fn fixture_messages() -> Vec<Value> {
    FIXTURES
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn update_kind(message: &Value) -> &str {
    message["params"]["update"]["sessionUpdate"]
        .as_str()
        .unwrap()
}

fn values_at(messages: &[Value], field: &str) -> BTreeSet<String> {
    messages
        .iter()
        .filter_map(|message| message["params"]["update"].get(field))
        .flat_map(|value| {
            value
                .as_array()
                .cloned()
                .unwrap_or_else(|| vec![value.clone()])
        })
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect()
}

#[test]
fn fixture_inventory_exactly_covers_the_pinned_session_update_union() {
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let messages = fixture_messages();
    for message in &messages {
        if let Err(error) = validator.validate(message) {
            panic!(
                "{} fixture is not schema-valid: {error}",
                update_kind(message)
            );
        }
    }

    let pinned = schema["$defs"]["SessionUpdate"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|branch| {
            branch["properties"]["sessionUpdate"]["const"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect::<BTreeSet<_>>();
    let covered = messages
        .iter()
        .map(update_kind)
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    assert_eq!(covered, pinned);

    assert_eq!(
        values_at(&messages, "kind"),
        [
            "delete",
            "edit",
            "execute",
            "fetch",
            "move",
            "other",
            "read",
            "search",
            "switch_mode",
            "think",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    );
    assert_eq!(
        values_at(&messages, "status"),
        ["completed", "failed", "in_progress", "pending"]
            .into_iter()
            .map(str::to_string)
            .collect()
    );

    let tool_content = messages
        .iter()
        .flat_map(|message| {
            message["params"]["update"]["content"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|content| content["type"].as_str().map(str::to_string))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        tool_content,
        ["content", "diff", "terminal"]
            .into_iter()
            .map(str::to_string)
            .collect()
    );

    let plan_entries = messages
        .iter()
        .find(|message| update_kind(message) == "plan")
        .unwrap()["params"]["update"]["entries"]
        .as_array()
        .unwrap();
    assert_eq!(
        plan_entries
            .iter()
            .filter_map(|entry| entry["priority"].as_str())
            .collect::<BTreeSet<_>>(),
        ["high", "low", "medium"].into_iter().collect()
    );
    assert_eq!(
        plan_entries
            .iter()
            .filter_map(|entry| entry["status"].as_str())
            .collect::<BTreeSet<_>>(),
        ["completed", "in_progress", "pending"]
            .into_iter()
            .collect()
    );
}

fn write_update_agent(workspace: &Path) -> PathBuf {
    let fixture = workspace.join("session_updates.jsonl");
    fs::write(&fixture, FIXTURES).unwrap();
    let agent = workspace.join("update-agent.py");
    let source = format!(
        r#"#!/usr/bin/env python3
import json
import sys

fixture = {fixture:?}
for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    request_id = message.get("id")
    if method == "initialize":
        print(json.dumps({{"jsonrpc":"2.0","id":request_id,"result":{{"protocolVersion":1,"agentCapabilities":{{}}}}}}, separators=(",", ":")), flush=True)
    elif method == "session/new":
        result = {{"sessionId":"update-session","modes":{{"currentModeId":"ask","availableModes":[{{"id":"ask","name":"Ask"}},{{"id":"code","name":"Code"}}]}},"configOptions":[{{"id":"stale","name":"Stale","type":"boolean","currentValue":True}}]}}
        print(json.dumps({{"jsonrpc":"2.0","id":request_id,"result":result}}, separators=(",", ":")), flush=True)
    elif method == "session/prompt":
        with open(fixture, "r", encoding="utf-8") as updates:
            for update in updates:
                print(update.rstrip("\\n"), flush=True)
        print(json.dumps({{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"update-session","update":{{"sessionUpdate":"vendor_progress","phase":"indexing","token":"must-redact"}}}}}}, separators=(",", ":")), flush=True)
        print(json.dumps({{"jsonrpc":"2.0","id":request_id,"result":{{"stopReason":"end_turn"}}}}, separators=(",", ":")), flush=True)
"#,
        fixture = fixture.to_string_lossy()
    );
    fs::write(&agent, source).unwrap();
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

#[test]
fn every_session_update_has_ordered_redacted_semantic_projection() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "update fixture\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let agent = write_update_agent(temp.path());
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
    send(
        &mut stdin,
        &serde_json::json!({"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":"update-session","prompt":[{"type":"text","text":"Emit update fixtures"}]}}),
    );
    loop {
        let message = receive(&mut stdout);
        if message.get("id") == Some(&Value::from(3)) {
            assert_eq!(message["result"]["stopReason"], "end_turn");
            break;
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
    let mapping = db.lane_acp_session("update-session").unwrap();
    assert_eq!(mapping.current_mode_id.as_deref(), Some("code"));
    assert_eq!(mapping.config_options["model"], "fast");
    assert_eq!(mapping.config_options["autoFix"], true);
    assert_eq!(mapping.config_options.as_object().unwrap().len(), 2);
    let session = db.show_lane_session(&mapping.trail_session_id).unwrap();
    assert_eq!(
        session.session.title.as_deref(),
        Some("Schema fixture session")
    );

    let raw_updates = session
        .events
        .iter()
        .filter(|event| event.event_type == "acp_session_update")
        .filter_map(|event| event.payload.as_ref())
        .collect::<Vec<_>>();
    assert!(session
        .events
        .iter()
        .any(|event| event.event_type == "acp_session_configuration"));
    let stable_order = raw_updates
        .iter()
        .filter(|payload| payload["stable"] == true)
        .filter_map(|payload| payload["acpVariant"].as_str())
        .collect::<Vec<_>>();
    let expected_order = fixture_messages()
        .iter()
        .map(update_kind)
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert_eq!(stable_order, expected_order);
    let thought = raw_updates
        .iter()
        .find(|payload| payload["acpVariant"] == "agent_thought_chunk")
        .unwrap();
    assert_eq!(thought["thoughtContentExcluded"], true);
    assert!(thought["update"].get("content").is_none());
    assert!(!thought.to_string().contains("private chain of thought"));
    let extension = raw_updates
        .iter()
        .find(|payload| payload["acpVariant"] == "vendor_progress")
        .unwrap();
    assert_eq!(extension["stable"], false);
    assert!(!extension.to_string().contains("must-redact"));

    assert!(session
        .messages
        .iter()
        .any(|message| message.role == "assistant" && message.body.contains("streamed agent")));
    for event_type in [
        "tool_call",
        "tool_call_update",
        "plan_update",
        "acp_available_commands_update",
        "acp_usage_update",
    ] {
        assert!(
            session
                .events
                .iter()
                .any(|event| event.event_type == event_type),
            "missing {event_type} projection"
        );
    }
}
