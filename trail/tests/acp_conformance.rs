#![cfg(any(unix, windows))]

#[path = "support/acp_harness.rs"]
mod acp_harness;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Map, Value};

const META: &str = include_str!("fixtures/acp/v1/meta.json");
const SCHEMA: &str = include_str!("fixtures/acp/v1/schema.json");
const METHOD_CASES: &str = include_str!("fixtures/acp/v1/method_cases.json");
const VARIANT_CASES: &str = include_str!("fixtures/acp/v1/variant_cases.json");

#[derive(Debug, Deserialize)]
struct MethodCase {
    method: String,
    side: String,
    envelope: String,
}

#[derive(Debug, Deserialize)]
struct VariantCase {
    union: String,
    name: String,
    value: Value,
}

fn strings(values: impl IntoIterator<Item = &'static str>) -> BTreeSet<String> {
    values.into_iter().map(str::to_string).collect()
}

#[test]
fn method_manifest_exactly_matches_the_pinned_v1_metadata() {
    let meta: Value = serde_json::from_str(META).unwrap();
    let cases: Vec<MethodCase> = serde_json::from_str(METHOD_CASES).unwrap();
    assert_eq!(cases.len(), 23, "ACP v1 has exactly 23 protocol methods");

    let expected_groups = [
        ("agentMethods", "agent"),
        ("clientMethods", "client"),
        ("protocolMethods", "protocol"),
    ];
    let mut pinned = BTreeMap::new();
    for (group, side) in expected_groups {
        for method in meta[group].as_object().unwrap().values() {
            let method = method.as_str().unwrap().to_string();
            assert!(
                pinned.insert(method, side).is_none(),
                "duplicate pinned method"
            );
        }
    }

    let notification_methods = strings(["session/cancel", "session/update", "$/cancel_request"]);
    let mut covered = BTreeMap::new();
    for case in cases {
        let expected_side = pinned
            .get(&case.method)
            .unwrap_or_else(|| panic!("fixture has unpinned method {}", case.method));
        assert_eq!(&case.side, expected_side, "wrong side for {}", case.method);
        let expected_envelope = if notification_methods.contains(&case.method) {
            "notification"
        } else {
            "request"
        };
        assert_eq!(
            case.envelope, expected_envelope,
            "wrong envelope for {}",
            case.method
        );
        assert!(
            covered.insert(case.method.clone(), case.side).is_none(),
            "duplicate method fixture {}",
            case.method
        );
        eprintln!("ACP v1 method: {}", case.method);
    }
    assert_eq!(
        covered.keys().collect::<Vec<_>>(),
        pinned.keys().collect::<Vec<_>>()
    );
}

fn discriminator_names(definition: &Value, property: &str, alternatives: &str) -> BTreeSet<String> {
    definition[alternatives]
        .as_array()
        .unwrap()
        .iter()
        .map(|branch| {
            branch["properties"][property]["const"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect()
}

fn const_names(definition: &Value) -> BTreeSet<String> {
    definition["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|branch| branch["const"].as_str().unwrap().to_string())
        .collect()
}

fn pinned_variant_names(schema: &Value, union: &str) -> BTreeSet<String> {
    let definition = &schema["$defs"][union];
    match union {
        "ContentBlock" | "ToolCallContent" | "SessionConfigOption" => {
            discriminator_names(definition, "type", "oneOf")
        }
        "RequestPermissionOutcome" => discriminator_names(definition, "outcome", "oneOf"),
        "SessionUpdate" => discriminator_names(definition, "sessionUpdate", "oneOf"),
        "EmbeddedResourceResource" => definition["anyOf"]
            .as_array()
            .unwrap()
            .iter()
            .map(|branch| branch["title"].as_str().unwrap().to_string())
            .collect(),
        "McpServer" => definition["anyOf"]
            .as_array()
            .unwrap()
            .iter()
            .map(|branch| {
                branch["properties"]["type"]["const"]
                    .as_str()
                    .or_else(|| branch["title"].as_str())
                    .unwrap()
                    .to_string()
            })
            .collect(),
        "TerminalExitStatus" => strings(["exit_code", "signal"]),
        "PermissionOptionKind"
        | "ToolCallStatus"
        | "ToolKind"
        | "PlanEntryPriority"
        | "PlanEntryStatus"
        | "Role"
        | "StopReason" => const_names(definition),
        other => panic!("variant inventory extractor missing {other}"),
    }
}

fn validator_for_definition(schema: &Value, definition: &str) -> jsonschema::Validator {
    let subschema = json!({
        "$schema": schema["$schema"].clone(),
        "$ref": format!("#/$defs/{definition}"),
        "$defs": schema["$defs"].clone(),
    });
    jsonschema::validator_for(&subschema).unwrap()
}

fn validate_definition(schema: &Value, definition: &str, value: &Value) {
    if let Err(error) = validator_for_definition(schema, definition).validate(value) {
        panic!("{definition} fixture is not schema-valid: {error}; value={value}");
    }
}

#[test]
fn variant_manifest_exactly_matches_and_validates_against_every_stable_union() {
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    let cases: Vec<VariantCase> = serde_json::from_str(VARIANT_CASES).unwrap();
    let required_unions = strings([
        "ContentBlock",
        "EmbeddedResourceResource",
        "McpServer",
        "PermissionOptionKind",
        "RequestPermissionOutcome",
        "SessionConfigOption",
        "SessionUpdate",
        "TerminalExitStatus",
        "ToolCallContent",
        "ToolCallStatus",
        "ToolKind",
        "PlanEntryPriority",
        "PlanEntryStatus",
        "Role",
        "StopReason",
    ]);
    let fixture_unions = cases
        .iter()
        .map(|case| case.union.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(fixture_unions, required_unions);

    for union in required_unions {
        let union_cases = cases
            .iter()
            .filter(|case| case.union == union)
            .collect::<Vec<_>>();
        let names = union_cases
            .iter()
            .map(|case| case.name.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            names,
            pinned_variant_names(&schema, &union),
            "variant drift in {union}"
        );
        assert_eq!(
            names.len(),
            union_cases.len(),
            "duplicate fixture in {union}"
        );

        let validator = validator_for_definition(&schema, &union);
        for case in union_cases {
            if let Err(error) = validator.validate(&case.value) {
                panic!("{}.{} is not schema-valid: {error}", union, case.name);
            }
            eprintln!("ACP v1 variant: {}.{}", union, case.name);
        }
    }
}

fn boolean_combinations(fields: &[&str]) -> Vec<Value> {
    (0..(1usize << fields.len()))
        .map(|bits| {
            Value::Object(
                fields
                    .iter()
                    .enumerate()
                    .map(|(index, field)| ((*field).to_string(), json!(bits & (1 << index) != 0)))
                    .collect(),
            )
        })
        .collect()
}

fn session_capability_combinations() -> Vec<Value> {
    let fields = ["list", "delete", "additionalDirectories", "resume", "close"];
    (0..3usize.pow(fields.len() as u32))
        .map(|mut state| {
            let mut object = Map::new();
            for field in fields {
                match state % 3 {
                    0 => {}
                    1 => {
                        object.insert(field.to_string(), Value::Null);
                    }
                    2 => {
                        object.insert(field.to_string(), json!({}));
                    }
                    _ => unreachable!(),
                }
                state /= 3;
            }
            Value::Object(object)
        })
        .collect()
}

#[test]
fn every_capability_shape_round_trips_through_the_real_relay_unchanged() {
    let temp = acp_harness::workspace();
    let agent = acp_harness::fixture_agent_command(
        temp.path(),
        "capability-matrix",
        r#"#!/usr/bin/env python3
import json
import sys

for line in sys.stdin:
    request = json.loads(line)
    meta = request["params"]["_meta"]
    response = {
        "jsonrpc": "2.0",
        "id": request["id"],
        "result": {
            "protocolVersion": 1,
            "agentCapabilities": meta["agentCapabilities"],
            "_meta": {"clientCapabilities": request["params"]["clientCapabilities"]},
        },
    }
    print(json.dumps(response, separators=(",", ":")), flush=True)
"#,
    );
    let mut child = acp_harness::spawn_relay(temp.path(), &agent);
    let (mut stdin, mut stdout) = acp_harness::relay_stdio(&mut child);

    let mut cases = Vec::new();
    for prompt in boolean_combinations(&["image", "audio", "embeddedContext"]) {
        cases.push((json!({}), json!({"promptCapabilities": prompt})));
    }
    for mcp in boolean_combinations(&["http", "sse"]) {
        cases.push((json!({}), json!({"mcpCapabilities": mcp})));
    }
    for client in boolean_combinations(&["readTextFile", "writeTextFile", "terminal"]) {
        cases.push((
            json!({
                "fs": {
                    "readTextFile": client["readTextFile"],
                    "writeTextFile": client["writeTextFile"],
                },
                "terminal": client["terminal"],
            }),
            json!({}),
        ));
    }
    cases.extend([
        (json!({}), json!({})),
        (json!({"session": null}), json!({})),
        (json!({"session": {}}), json!({})),
    ]);
    for session in session_capability_combinations() {
        cases.push((json!({}), json!({"sessionCapabilities": session})));
    }
    assert_eq!(cases.len(), 8 + 4 + 8 + 3 + 243);

    let wire_validator =
        jsonschema::validator_for(&serde_json::from_str::<Value>(SCHEMA).unwrap()).unwrap();
    for (index, (client_capabilities, agent_capabilities)) in cases.iter().enumerate() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": index as u64,
            "method": "initialize",
            "params": {
                "protocolVersion": 1,
                "clientCapabilities": client_capabilities,
                "_meta": {"agentCapabilities": agent_capabilities, "case": index},
            },
        });
        assert!(
            wire_validator.is_valid(&request),
            "invalid generated case {index}: {request}"
        );
        acp_harness::write_json(&mut stdin, &request);
        let response = acp_harness::read_json(&mut stdout);
        assert!(
            wire_validator.is_valid(&response),
            "invalid response case {index}: {response}"
        );
        assert_eq!(response["result"]["agentCapabilities"], *agent_capabilities);
        assert_eq!(
            response["result"]["_meta"]["clientCapabilities"],
            *client_capabilities
        );
        eprintln!("ACP v1 capability combination: {index}");
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "relay failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[derive(Clone)]
struct RequestCase {
    method: &'static str,
    request_definition: &'static str,
    response_definition: Option<&'static str>,
    params: Value,
    result: Option<Value>,
}

fn request_cases(cwd: &str) -> Vec<RequestCase> {
    vec![
        RequestCase {
            method: "initialize",
            request_definition: "InitializeRequest",
            response_definition: Some("InitializeResponse"),
            params: json!({"protocolVersion":1,"clientCapabilities":{"fs":{"readTextFile":true,"writeTextFile":true},"terminal":true},"_meta":{"fixture":"keep"}}),
            result: Some(
                json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"list":{},"delete":{},"resume":{},"close":{}}}}),
            ),
        },
        RequestCase {
            method: "authenticate",
            request_definition: "AuthenticateRequest",
            response_definition: Some("AuthenticateResponse"),
            params: json!({"methodId":"fixture-token"}),
            result: Some(json!({})),
        },
        RequestCase {
            method: "session/new",
            request_definition: "NewSessionRequest",
            response_definition: Some("NewSessionResponse"),
            params: json!({"cwd":cwd,"mcpServers":[]}),
            result: Some(json!({"sessionId":"matrix-session"})),
        },
        RequestCase {
            method: "session/load",
            request_definition: "LoadSessionRequest",
            response_definition: Some("LoadSessionResponse"),
            params: json!({"cwd":cwd,"mcpServers":[],"sessionId":"matrix-session"}),
            result: Some(json!({})),
        },
        RequestCase {
            method: "session/resume",
            request_definition: "ResumeSessionRequest",
            response_definition: Some("ResumeSessionResponse"),
            params: json!({"cwd":cwd,"mcpServers":[],"sessionId":"matrix-session"}),
            result: Some(json!({})),
        },
        RequestCase {
            method: "session/list",
            request_definition: "ListSessionsRequest",
            response_definition: Some("ListSessionsResponse"),
            params: json!({}),
            result: Some(json!({"sessions":[]})),
        },
        RequestCase {
            method: "session/prompt",
            request_definition: "PromptRequest",
            response_definition: Some("PromptResponse"),
            params: json!({"sessionId":"matrix-session","prompt":[{"type":"text","text":"method matrix"}]}),
            result: Some(json!({"stopReason":"end_turn"})),
        },
        RequestCase {
            method: "session/set_mode",
            request_definition: "SetSessionModeRequest",
            response_definition: Some("SetSessionModeResponse"),
            params: json!({"sessionId":"matrix-session","modeId":"code"}),
            result: Some(json!({})),
        },
        RequestCase {
            method: "session/set_config_option",
            request_definition: "SetSessionConfigOptionRequest",
            response_definition: Some("SetSessionConfigOptionResponse"),
            params: json!({"sessionId":"matrix-session","configId":"model","value":"fast"}),
            result: Some(
                json!({"configOptions":[{"id":"model","name":"Model","type":"select","currentValue":"fast","options":[{"value":"fast","name":"Fast"}]}]}),
            ),
        },
        RequestCase {
            method: "session/cancel",
            request_definition: "CancelNotification",
            response_definition: None,
            params: json!({"sessionId":"matrix-session"}),
            result: None,
        },
        RequestCase {
            method: "session/close",
            request_definition: "CloseSessionRequest",
            response_definition: Some("CloseSessionResponse"),
            params: json!({"sessionId":"matrix-session"}),
            result: Some(json!({})),
        },
        RequestCase {
            method: "session/delete",
            request_definition: "DeleteSessionRequest",
            response_definition: Some("DeleteSessionResponse"),
            params: json!({"sessionId":"matrix-session"}),
            result: Some(json!({})),
        },
        RequestCase {
            method: "logout",
            request_definition: "LogoutRequest",
            response_definition: Some("LogoutResponse"),
            params: json!({}),
            result: Some(json!({})),
        },
        RequestCase {
            method: "session/update",
            request_definition: "SessionNotification",
            response_definition: None,
            params: json!({"sessionId":"matrix-session","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"update"}}}),
            result: None,
        },
        RequestCase {
            method: "session/request_permission",
            request_definition: "RequestPermissionRequest",
            response_definition: Some("RequestPermissionResponse"),
            params: json!({"sessionId":"matrix-session","toolCall":{"toolCallId":"tool-1","title":"Permission"},"options":[{"optionId":"allow","name":"Allow","kind":"allow_once"}]}),
            result: Some(json!({"outcome":{"outcome":"selected","optionId":"allow"}})),
        },
        RequestCase {
            method: "fs/read_text_file",
            request_definition: "ReadTextFileRequest",
            response_definition: Some("ReadTextFileResponse"),
            params: json!({"sessionId":"matrix-session","path":format!("{cwd}/README.md"),"line":1,"limit":10}),
            result: Some(json!({"content":"ACP conformance fixture\n"})),
        },
        RequestCase {
            method: "fs/write_text_file",
            request_definition: "WriteTextFileRequest",
            response_definition: Some("WriteTextFileResponse"),
            params: json!({"sessionId":"matrix-session","path":format!("{cwd}/matrix.txt"),"content":"matrix"}),
            result: Some(json!({})),
        },
        RequestCase {
            method: "terminal/create",
            request_definition: "CreateTerminalRequest",
            response_definition: Some("CreateTerminalResponse"),
            params: json!({"sessionId":"matrix-session","command":"printf","args":["matrix"],"cwd":cwd}),
            result: Some(json!({"terminalId":"terminal-matrix"})),
        },
        RequestCase {
            method: "terminal/output",
            request_definition: "TerminalOutputRequest",
            response_definition: Some("TerminalOutputResponse"),
            params: json!({"sessionId":"matrix-session","terminalId":"terminal-matrix"}),
            result: Some(
                json!({"output":"matrix","truncated":false,"exitStatus":{"exitCode":0,"signal":null}}),
            ),
        },
        RequestCase {
            method: "terminal/wait_for_exit",
            request_definition: "WaitForTerminalExitRequest",
            response_definition: Some("WaitForTerminalExitResponse"),
            params: json!({"sessionId":"matrix-session","terminalId":"terminal-matrix"}),
            result: Some(json!({"exitCode":0,"signal":null})),
        },
        RequestCase {
            method: "terminal/kill",
            request_definition: "KillTerminalRequest",
            response_definition: Some("KillTerminalResponse"),
            params: json!({"sessionId":"matrix-session","terminalId":"terminal-matrix"}),
            result: Some(json!({})),
        },
        RequestCase {
            method: "terminal/release",
            request_definition: "ReleaseTerminalRequest",
            response_definition: Some("ReleaseTerminalResponse"),
            params: json!({"sessionId":"matrix-session","terminalId":"terminal-matrix"}),
            result: Some(json!({})),
        },
        RequestCase {
            method: "$/cancel_request",
            request_definition: "CancelRequestNotification",
            response_definition: None,
            params: json!({"requestId":"request-in-flight"}),
            result: None,
        },
    ]
}

fn add_extension_evidence(params: &Value, force_error: bool) -> Value {
    let mut params = params.as_object().unwrap().clone();
    let mut metadata = params
        .remove("_meta")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    metadata.insert("extensionToken".to_string(), json!("preserve-me"));
    if force_error {
        metadata.insert("forceError".to_string(), json!(true));
    }
    params.insert("_meta".to_string(), Value::Object(metadata));
    Value::Object(params)
}

fn response_error(id: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32001,"message":"fixture rejection","data":{"_meta":{"extensionToken":"preserve-me"}}}})
}

#[test]
fn all_23_methods_route_success_error_ids_extensions_and_semantic_capture() {
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    let wire_validator = jsonschema::validator_for(&schema).unwrap();
    let temp = acp_harness::workspace();
    let cwd = temp.path().to_string_lossy().into_owned();
    let cases = request_cases(&cwd);
    assert_eq!(cases.len(), 23);

    for case in &cases {
        validate_definition(&schema, case.request_definition, &case.params);
        if let (Some(definition), Some(result)) = (case.response_definition, &case.result) {
            validate_definition(&schema, definition, result);
        }
    }

    let agent_results = cases
        .iter()
        .filter(|case| case.method != "initialize")
        .filter_map(|case| case.result.as_ref().map(|result| (case.method, result)))
        .collect::<BTreeMap<_, _>>();
    let received_path = temp.path().join("matrix-received.jsonl");
    let source = format!(
        r#"#!/usr/bin/env python3
import json
import sys

results = json.loads(r'''{results}''')
received_path = {received_path:?}
pending = {{}}

def send(value):
    print(json.dumps(value, separators=(",", ":")), flush=True)

for line in sys.stdin:
    message = json.loads(line)
    with open(received_path, "a", encoding="utf-8") as received:
        received.write(json.dumps(message, separators=(",", ":")) + "\n")
    method = message.get("method")
    request_id = message.get("id")
    if method == "ext/emit":
        frame = message["params"]["frame"]
        send(frame)
        if "id" in frame:
            pending[str(frame["id"])] = request_id
        else:
            send({{"jsonrpc":"2.0","id":request_id,"result":{{"emitted":True}}}})
    elif method is None:
        control_id = pending.pop(str(request_id))
        send({{"jsonrpc":"2.0","id":control_id,"result":{{"observed":message}}}})
    elif "id" not in message:
        continue
    elif message.get("params", {{}}).get("_meta", {{}}).get("forceError"):
        send({{"jsonrpc":"2.0","id":request_id,"error":{{"code":-32001,"message":"fixture rejection","data":{{"_meta":{{"extensionToken":"preserve-me"}}}}}}}})
    else:
        if method == "initialize":
            result = {{"protocolVersion":1,"agentCapabilities":{{"loadSession":True,"sessionCapabilities":{{"list":{{}},"delete":{{}},"resume":{{}},"close":{{}}}}}}}}
        else:
            result = json.loads(json.dumps(results[method]))
        result["_meta"] = {{"observed":message,"extensionToken":"preserve-me"}}
        send({{"jsonrpc":"2.0","id":request_id,"result":result}})
"#,
        results = serde_json::to_string(&agent_results).unwrap(),
        received_path = received_path.to_string_lossy(),
    );
    let agent = acp_harness::fixture_agent_command(temp.path(), "method-matrix", &source);
    let mut child = acp_harness::spawn_relay(temp.path(), &agent);
    let (mut stdin, mut stdout) = acp_harness::relay_stdio(&mut child);

    let agent_side_order = [
        "initialize",
        "authenticate",
        "session/new",
        "session/load",
        "session/resume",
        "session/list",
        "session/prompt",
        "session/set_mode",
        "session/set_config_option",
    ];
    let mut numeric_id = 1_000u64;
    for method in agent_side_order {
        let case = cases.iter().find(|case| case.method == method).unwrap();
        let success_id = json!(format!("success:{method}"));
        let request = json!({
            "jsonrpc":"2.0", "id":success_id, "method":method,
            "params":add_extension_evidence(&case.params, false),
            "unknownTopLevel":{"extension":true}
        });
        assert!(
            wire_validator.is_valid(&request),
            "invalid {method} request: {request}"
        );
        acp_harness::write_json(&mut stdin, &request);
        let response = acp_harness::read_json(&mut stdout);
        assert!(
            wire_validator.is_valid(&response),
            "invalid {method} response: {response}"
        );
        assert_eq!(response["id"], success_id);
        validate_definition(
            &schema,
            case.response_definition.unwrap(),
            &response["result"],
        );
        assert_eq!(
            response["result"]["_meta"]["observed"]["unknownTopLevel"]["extension"],
            true
        );
        assert_eq!(
            response["result"]["_meta"]["observed"]["params"]["_meta"]["extensionToken"],
            "preserve-me"
        );

        numeric_id += 1;
        let error_request = json!({
            "jsonrpc":"2.0", "id":numeric_id, "method":method,
            "params":add_extension_evidence(&case.params, true)
        });
        assert!(wire_validator.is_valid(&error_request));
        acp_harness::write_json(&mut stdin, &error_request);
        let error = acp_harness::read_json(&mut stdout);
        assert!(
            wire_validator.is_valid(&error),
            "invalid {method} error: {error}"
        );
        assert_eq!(error["id"], numeric_id);
        assert_eq!(error["error"]["code"], -32001);
        assert_eq!(
            error["error"]["data"]["_meta"]["extensionToken"],
            "preserve-me"
        );
    }

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
        let case = cases.iter().find(|case| case.method == method).unwrap();
        for force_error in [false, true] {
            numeric_id += 1;
            let callback_id = if force_error {
                json!(numeric_id)
            } else {
                json!(format!("callback:{method}"))
            };
            let callback = json!({
                "jsonrpc":"2.0", "id":callback_id, "method":method,
                "params":add_extension_evidence(&case.params, false),
                "unknownTopLevel":{"extension":true}
            });
            assert!(
                wire_validator.is_valid(&callback),
                "invalid callback {method}: {callback}"
            );
            let control_id = format!("emit:{method}:{force_error}");
            acp_harness::write_json(
                &mut stdin,
                &json!({"jsonrpc":"2.0","id":control_id,"method":"ext/emit","params":{"frame":callback}}),
            );
            let forwarded = acp_harness::read_json(&mut stdout);
            assert_eq!(forwarded["method"], method);
            assert_eq!(forwarded["id"], callback_id);
            assert_eq!(forwarded["unknownTopLevel"]["extension"], true);
            assert_eq!(
                forwarded["params"]["_meta"]["extensionToken"],
                "preserve-me"
            );

            let response = if force_error {
                response_error(callback_id.clone())
            } else {
                json!({"jsonrpc":"2.0","id":callback_id,"result":case.result.clone().unwrap()})
            };
            assert!(
                wire_validator.is_valid(&response),
                "invalid callback response {method}: {response}"
            );
            acp_harness::write_json(&mut stdin, &response);
            let control_response = acp_harness::read_json(&mut stdout);
            assert_eq!(control_response["id"], control_id);
            assert_eq!(control_response["result"]["observed"], response);
        }
    }

    for method in ["session/update", "$/cancel_request"] {
        let case = cases.iter().find(|case| case.method == method).unwrap();
        let notification = json!({"jsonrpc":"2.0","method":method,"params":add_extension_evidence(&case.params, false),"unknownTopLevel":{"extension":true}});
        assert!(wire_validator.is_valid(&notification));
        let control_id = format!("emit:{method}");
        acp_harness::write_json(
            &mut stdin,
            &json!({"jsonrpc":"2.0","id":control_id,"method":"ext/emit","params":{"frame":notification}}),
        );
        let forwarded = acp_harness::read_json(&mut stdout);
        assert_eq!(forwarded["method"], method);
        assert_eq!(forwarded["unknownTopLevel"]["extension"], true);
        assert_eq!(acp_harness::read_json(&mut stdout)["id"], control_id);
    }

    let read_case = cases
        .iter()
        .find(|case| case.method == "fs/read_text_file")
        .unwrap();
    for (control_id, callback_id) in [("interleave-a", 77), ("interleave-b", 78)] {
        let callback = json!({"jsonrpc":"2.0","id":callback_id,"method":"fs/read_text_file","params":read_case.params});
        acp_harness::write_json(
            &mut stdin,
            &json!({"jsonrpc":"2.0","id":control_id,"method":"ext/emit","params":{"frame":callback}}),
        );
    }
    let first_callback = acp_harness::read_json(&mut stdout);
    let second_callback = acp_harness::read_json(&mut stdout);
    assert_eq!(
        (
            first_callback["id"].as_u64(),
            second_callback["id"].as_u64()
        ),
        (Some(77), Some(78))
    );
    acp_harness::write_json(
        &mut stdin,
        &json!({"jsonrpc":"2.0","id":78,"result":{"content":"second"}}),
    );
    assert_eq!(acp_harness::read_json(&mut stdout)["id"], "interleave-b");
    acp_harness::write_json(
        &mut stdin,
        &json!({"jsonrpc":"2.0","id":77,"result":{"content":"first"}}),
    );
    assert_eq!(acp_harness::read_json(&mut stdout)["id"], "interleave-a");

    for method in ["session/cancel", "$/cancel_request"] {
        let case = cases.iter().find(|case| case.method == method).unwrap();
        let notification = json!({"jsonrpc":"2.0","method":method,"params":add_extension_evidence(&case.params, false),"unknownTopLevel":{"extension":true}});
        assert!(wire_validator.is_valid(&notification));
        acp_harness::write_json(&mut stdin, &notification);
    }

    for method in ["session/close", "session/delete", "logout"] {
        let case = cases.iter().find(|case| case.method == method).unwrap();
        for force_error in [false, true] {
            numeric_id += 1;
            let request = json!({"jsonrpc":"2.0","id":numeric_id,"method":method,"params":add_extension_evidence(&case.params, force_error)});
            acp_harness::write_json(&mut stdin, &request);
            let response = acp_harness::read_json(&mut stdout);
            assert_eq!(response["id"], numeric_id);
            assert!(wire_validator.is_valid(&response));
            if force_error {
                assert_eq!(response["error"]["code"], -32001);
            } else {
                validate_definition(
                    &schema,
                    case.response_definition.unwrap(),
                    &response["result"],
                );
            }
        }
    }

    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "relay failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let received = fs::read_to_string(received_path).unwrap();
    let received = received
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    for method in ["session/cancel", "$/cancel_request"] {
        let notification = received
            .iter()
            .find(|message| message["method"] == method)
            .unwrap_or_else(|| panic!("agent did not receive {method}"));
        assert_eq!(notification["unknownTopLevel"]["extension"], true);
        assert_eq!(
            notification["params"]["_meta"]["extensionToken"],
            "preserve-me"
        );
    }

    let db = trail::Trail::open(temp.path()).unwrap();
    let receipts = db
        .list_agent_hook_receipts(Some("trail-acp"), None, 1_000)
        .unwrap();
    assert!(
        receipts.len() >= 23,
        "expected durable evidence for method matrix, got {}",
        receipts.len()
    );
    assert!(receipts
        .iter()
        .all(|receipt| receipt.connection_sequence.is_some()));
    assert!(receipts
        .iter()
        .any(|receipt| receipt.direction.as_deref() == Some("client_to_agent")));
    assert!(receipts
        .iter()
        .any(|receipt| receipt.direction.as_deref() == Some("agent_to_client")));
}

#[test]
fn conformance_harness_shutdown_is_bounded_with_a_callback_in_flight() {
    let temp = acp_harness::workspace();
    let agent = acp_harness::fixture_agent_command(
        temp.path(),
        "in-flight-shutdown",
        r#"#!/usr/bin/env python3
import json
import sys
import time

for line in sys.stdin:
    message = json.loads(line)
    if message.get("method") == "ext/hang":
        print(json.dumps({"jsonrpc":"2.0","id":"pending","method":"fs/read_text_file","params":{"sessionId":"session","path":"/tmp/pending"}}, separators=(",", ":")), flush=True)
        time.sleep(30)
"#,
    );
    let mut child = acp_harness::spawn_relay(temp.path(), &agent);
    let (mut stdin, mut stdout) = acp_harness::relay_stdio(&mut child);
    acp_harness::write_json(
        &mut stdin,
        &json!({"jsonrpc":"2.0","id":1,"method":"ext/hang","params":{}}),
    );
    assert_eq!(acp_harness::read_json(&mut stdout)["id"], "pending");
    drop(stdin);
    let started = Instant::now();
    let status = child.wait().unwrap();
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(status.success());
}
