use std::fs;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::io::{Read, Write};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::net::TcpListener;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::path::PathBuf;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::process::Command;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::thread;

use trail::{Error, InitImportMode, StructuredErrorEnvelope, Trail};

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct AutoDaemonGuard(PathBuf);

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Drop for AutoDaemonGuard {
    fn drop(&mut self) {
        let endpoint = self.0.join(".trail/index/change-ledger/daemon.json");
        if let Ok(bytes) = fs::read(endpoint) {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if let Some(pid) = value["pid"].as_i64() {
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTERM);
                    }
                }
            }
        }
    }
}

fn api_request(method: &str, path: &str, body: serde_json::Value) -> Vec<u8> {
    api_request_with_headers(method, path, &[], body)
}

fn api_request_with_headers(
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: serde_json::Value,
) -> Vec<u8> {
    let body = if body.is_null() {
        Vec::new()
    } else {
        serde_json::to_vec(&body).unwrap()
    };
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    request.into_bytes().into_iter().chain(body).collect()
}

#[test]
fn reconciliation_report_is_shared_by_rest_and_mcp() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();

    let rest = trail::server::handle_http_request(
        &mut db,
        &api_request("POST", "/v1/index/reconcile", serde_json::json!({})),
    );
    assert_eq!(rest.status, 200);
    let rest: serde_json::Value = rest.body_json().unwrap();
    assert_eq!(rest["scope_kind"], "workspace");
    assert_eq!(rest["resulting_state"], "trusted");
    assert!(rest["observed_paths"].as_u64().unwrap() >= 1);

    let mcp_temp = tempfile::tempdir().unwrap();
    fs::write(mcp_temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(mcp_temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut mcp_db = Trail::open(mcp_temp.path()).unwrap();
    let mcp = trail::mcp::handle_json_rpc(
        &mut mcp_db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "trail.index_reconcile",
                "arguments": {}
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp["result"]["isError"], false, "MCP response: {mcp}");
    let mcp = &mcp["result"]["structuredContent"];
    assert!(mcp["scope_id"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
    assert_eq!(mcp["scope_kind"], "workspace");
    assert_eq!(mcp["resulting_state"], "trusted");
}

#[test]
fn reconciliation_supports_materialized_lane_scope() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("reconcile-bot", Some("main"), true, None, None)
        .unwrap();

    let lane = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/index/reconcile",
            serde_json::json!({ "lane": "reconcile-bot" }),
        ),
    );
    let lane_status = lane.status;
    let lane: serde_json::Value = lane.body_json().unwrap();
    assert_eq!(lane_status, 200, "lane reconcile response: {lane}");
    assert_eq!(lane["scope_kind"], "materialized_lane");
    assert_eq!(lane["resulting_state"], "trusted");
}

#[test]
fn reconcile_required_has_stable_structured_recovery_fields() {
    let error = Error::ChangeLedgerReconcileRequired {
        scope: "workspace-scope".into(),
        state: "untrusted_gap".into(),
        reason: "observer overflow".into(),
        command: "trail index reconcile".into(),
    };
    let value = serde_json::to_value(StructuredErrorEnvelope::from_error(&error)).unwrap();
    assert_eq!(
        value.pointer("/error/code").unwrap(),
        "CHANGE_LEDGER_RECONCILE_REQUIRED"
    );
    assert_eq!(value.pointer("/error/status").unwrap(), 409);
    assert_eq!(value.pointer("/error/exit").unwrap(), 16);
    assert_eq!(value.pointer("/error/scope").unwrap(), "workspace-scope");
    assert_eq!(value.pointer("/error/state").unwrap(), "untrusted_gap");
    assert_eq!(value.pointer("/error/reason").unwrap(), "observer overflow");
    assert_eq!(
        value.pointer("/error/recovery/command").unwrap(),
        "trail index reconcile"
    );
}

#[test]
fn reconcile_required_preserves_lane_recovery_command() {
    let error = Error::ChangeLedgerReconcileRequired {
        scope: "lane-scope".into(),
        state: "untrusted_gap".into(),
        reason: "observer overflow".into(),
        command: "trail index reconcile --lane reconcile-bot".into(),
    };
    let value = serde_json::to_value(StructuredErrorEnvelope::from_error(&error)).unwrap();
    assert_eq!(
        value.pointer("/error/recovery/command").unwrap(),
        "trail index reconcile --lane reconcile-bot"
    );
}

#[test]
fn rest_and_mcp_return_identical_lane_reconcile_failure_fields() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane("reconcile-bot", Some("main"), true, None, None)
        .unwrap();

    let initial = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/index/reconcile",
            serde_json::json!({ "lane": "reconcile-bot" }),
        ),
    );
    assert_eq!(initial.status, 200);

    let conn = rusqlite::Connection::open(temp.path().join(".trail/index/trail.sqlite")).unwrap();
    assert_eq!(
        conn.execute(
            "UPDATE changed_path_scopes SET epoch=epoch+1 WHERE scope_kind='materialized_lane'",
            [],
        )
        .unwrap(),
        1
    );

    let rest = trail::server::handle_http_request(
        &mut db,
        &api_request(
            "POST",
            "/v1/index/reconcile",
            serde_json::json!({ "lane": "reconcile-bot" }),
        ),
    );
    assert_eq!(rest.status, 409);
    let rest: serde_json::Value = rest.body_json().unwrap();

    let mcp = trail::mcp::handle_json_rpc(
        &mut db,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "trail.index_reconcile",
                "arguments": { "lane": "reconcile-bot" }
            }
        }),
    )
    .unwrap();
    assert_eq!(mcp["result"]["isError"], true);
    let mcp = &mcp["result"]["structuredContent"];
    for pointer in [
        "/error/code",
        "/error/status",
        "/error/exit",
        "/error/scope",
        "/error/state",
        "/error/reason",
        "/error/recovery/command",
    ] {
        assert_eq!(
            rest.pointer(pointer),
            mcp.pointer(pointer),
            "field {pointer}"
        );
    }
    assert_eq!(
        rest.pointer("/error/recovery/command").unwrap(),
        "trail index reconcile --lane reconcile-bot"
    );
}

#[test]
fn reconcile_route_requires_auth_and_accepts_an_empty_body() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let mut db = Trail::open(temp.path()).unwrap();
    let auth = trail::server::ServerAuth::bearer("secret-token").unwrap();

    let request = api_request("POST", "/v1/index/reconcile", serde_json::Value::Null);
    let missing = trail::server::handle_http_request_with_auth(&mut db, &request, &auth);
    assert_eq!(missing.status, 401);

    let authorized = api_request_with_headers(
        "POST",
        "/v1/index/reconcile",
        &[("Authorization", "Bearer secret-token")],
        serde_json::Value::Null,
    );
    let response = trail::server::handle_http_request_with_auth(&mut db, &authorized, &auth);
    assert_eq!(response.status, 200);
    let report: serde_json::Value = response.body_json().unwrap();
    assert_eq!(report["scope_kind"], "workspace");
    assert_eq!(report["resulting_state"], "trusted");
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn reconcile_cli_renders_human_json_and_ndjson() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("README.md"), "hello\n").unwrap();
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
    let _daemon = AutoDaemonGuard(temp.path().to_path_buf());
    let canonical = temp.path().canonicalize().unwrap();
    let run = |format: &str| {
        Command::new(env!("CARGO_BIN_EXE_trail"))
            .arg("--workspace")
            .arg(temp.path())
            .args(["--format", format, "index", "reconcile"])
            .env("HOME", &canonical)
            .env("XDG_CONFIG_HOME", canonical.join(".config"))
            .env("GIT_CONFIG_GLOBAL", "")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap()
    };

    let human = run("human");
    assert!(
        human.status.success(),
        "human reconcile failed: {}",
        String::from_utf8_lossy(&human.stderr)
    );
    assert!(String::from_utf8_lossy(&human.stdout).contains("Reconciled changed-path ledger"));

    let json = run("json");
    assert!(
        json.status.success(),
        "JSON reconcile failed: {}",
        String::from_utf8_lossy(&json.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(json["scope_kind"], "workspace");
    assert_eq!(json["resulting_state"], "trusted");

    let ndjson = run("ndjson");
    assert!(
        ndjson.status.success(),
        "NDJSON reconcile failed: {}",
        String::from_utf8_lossy(&ndjson.stderr)
    );
    let lines = ndjson
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    let ndjson: serde_json::Value = serde_json::from_slice(lines[0]).unwrap();
    assert_eq!(ndjson["scope_id"], json["scope_id"]);
    assert_eq!(ndjson["resulting_state"], "trusted");
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn reconcile_cli_failure_preserves_lane_recovery_in_every_format() {
    let temp = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let body = serde_json::json!({
        "error": {
            "code": "CHANGE_LEDGER_RECONCILE_REQUIRED",
            "status": 409,
            "exit": 16,
            "message": "changed-path ledger reconciliation required",
            "scope": "lane-scope",
            "state": "untrusted_gap",
            "reason": "observer overflow",
            "recovery": {
                "command": "trail index reconcile --lane reconcile-bot"
            }
        }
    })
    .to_string();
    let server = thread::spawn(move || {
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            loop {
                let mut chunk = [0_u8; 4096];
                let read = stream.read(&mut chunk).unwrap();
                assert!(read > 0, "client closed before completing request");
                request.extend_from_slice(&chunk[..read]);
                let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = std::str::from_utf8(&request[..header_end]).unwrap();
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
            write!(
                stream,
                "HTTP/1.1 409 Conflict\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
            stream.flush().unwrap();
        }
    });
    let daemon_url = format!("http://{address}");
    let run = |format: &str| {
        Command::new(env!("CARGO_BIN_EXE_trail"))
            .arg("--workspace")
            .arg(temp.path())
            .arg("--daemon-url")
            .arg(&daemon_url)
            .args([
                "--format",
                format,
                "index",
                "reconcile",
                "--lane",
                "reconcile-bot",
            ])
            .output()
            .unwrap()
    };

    let human = run("human");
    assert!(!human.status.success());
    assert!(
        String::from_utf8_lossy(&human.stderr)
            .contains("trail index reconcile --lane reconcile-bot"),
        "human stderr: {}",
        String::from_utf8_lossy(&human.stderr)
    );

    for format in ["json", "ndjson"] {
        let output = run(format);
        assert!(!output.status.success(), "{format} unexpectedly succeeded");
        let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(error["error"]["code"], "CHANGE_LEDGER_RECONCILE_REQUIRED");
        assert_eq!(error["error"]["status"], 409);
        assert_eq!(error["error"]["exit"], 16);
        assert_eq!(
            error["error"]["recovery"]["command"],
            "trail index reconcile --lane reconcile-bot"
        );
    }
    server.join().unwrap();
}

#[test]
fn schema_reinitialize_guidance_requires_backup_and_force_init() {
    let error = Error::SchemaReinitializeRequired {
        found: "schema 17".into(),
        guidance: "back up this workspace, then run `trail init --force` to create schema v19"
            .into(),
    };
    let value = serde_json::to_value(StructuredErrorEnvelope::from_error(&error)).unwrap();
    assert_eq!(
        value.pointer("/error/code").unwrap(),
        "SCHEMA_REINITIALIZE_REQUIRED"
    );
    assert_eq!(
        value.pointer("/error/recovery/command").unwrap(),
        "trail init --force"
    );
    let rendered = serde_json::to_string(&value).unwrap().to_ascii_lowercase();
    assert!(rendered.contains("back up"));
    assert!(!rendered.contains("migration"));
    assert!(!rendered.contains("migrate"));
}

#[test]
fn openapi_documents_reconcile_request_report_and_structured_error() {
    let spec = trail::server::openapi_spec();
    assert!(spec["paths"].get("/v1/index/reconcile").is_some());
    assert_eq!(
        spec["components"]["schemas"]["ErrorBody"]["properties"]["error"]["properties"]["recovery"]
            ["oneOf"][0]["$ref"],
        "#/components/schemas/StructuredRecovery"
    );
    let operation = &spec["paths"]["/v1/index/reconcile"]["post"];
    assert_eq!(operation["requestBody"]["required"], false);
    assert_eq!(
        operation["responses"]["409"]["$ref"],
        "#/components/responses/Error"
    );
}
