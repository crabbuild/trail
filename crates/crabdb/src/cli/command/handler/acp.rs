use super::*;
use crabdb::model::{AcpDoctorCheck, AcpDoctorReport};
use std::io::{BufRead, Write};
use std::process::{Command, Stdio};

pub(super) fn handle_acp_command(ctx: &RuntimeContext, acp: AcpCommand) -> Result<()> {
    match acp.command {
        AcpSubcommand::Install(args) => handle_acp_install(ctx, args),
        AcpSubcommand::Doctor(args) => handle_acp_doctor(ctx, args),
        AcpSubcommand::List => handle_acp_list(ctx),
        AcpSubcommand::Sessions(args) => handle_acp_sessions(ctx, args),
        AcpSubcommand::Relay(args) => handle_acp_relay(ctx, args),
        AcpSubcommand::TestAgent(args) => handle_acp_test_agent(args),
    }
}

pub(super) fn handle_transcript_command(ctx: &RuntimeContext, args: TranscriptArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.transcript(&args.selector)?;
    render_transcript(&report, ctx.json, ctx.quiet)
}

pub(super) fn handle_top_turn_command(ctx: &RuntimeContext, turn: TopTurnCommand) -> Result<()> {
    match turn.command {
        TopTurnSubcommand::Show(args) => {
            let db = open_db(ctx)?;
            let details = db.show_lane_turn(&args.turn_id)?;
            render_lane_turn_details(&details, ctx.json, ctx.quiet)
        }
    }
}

pub(super) fn handle_demo_command(ctx: &RuntimeContext, demo: DemoCommand) -> Result<()> {
    match demo.command {
        DemoSubcommand::Acp(args) => handle_demo_acp(ctx, args),
    }
}

fn handle_demo_acp(ctx: &RuntimeContext, args: DemoAcpArgs) -> Result<()> {
    if args.agent == "fake" {
        let workspace = create_demo_workspace()?;
        CrabDb::init(&workspace, "main", InitImportMode::WorkingTree, false)?;
        let report = run_acp_fake_smoke_in_workspace(&workspace, "acp-demo", false)?;
        if ctx.json {
            return render_json(&serde_json::json!({
                "agent": "fake",
                "workspace": workspace,
                "lane": report.lane,
                "session_id": report.session_id,
                "status": report.status,
                "next_steps": [
                    format!("crabdb --workspace {} transcript acp-demo", workspace.display()),
                    format!("crabdb --workspace {} lane review acp-demo", workspace.display())
                ]
            }));
        }
        if !ctx.quiet {
            println!("ACP fake demo: {}", report.status);
            println!("Workspace: {}", workspace.display());
            if let Some(lane) = &report.lane {
                println!("Lane: {lane}");
                println!(
                    "Transcript: crabdb --workspace {} transcript {lane}",
                    workspace.display()
                );
                println!(
                    "Review: crabdb --workspace {} lane review {lane}",
                    workspace.display()
                );
            }
        }
        return Ok(());
    }

    let install = crabdb::acp::acp_install_report(&args.agent, "generic", true)?;
    if ctx.json {
        return render_json(&serde_json::json!({
            "agent": args.agent,
            "steps": [
                "crabdb init --working-tree",
                "crabdb acp doctor --agent fake",
                "run an ACP editor with the generated relay command",
                "crabdb transcript <lane>",
                "crabdb lane review <lane>",
                "crabdb lane rewind <lane> --to <checkpoint>"
            ],
            "relay_command": install.relay_command
        }));
    }
    if !ctx.quiet {
        println!("ACP demo workflow ({})", args.agent);
        println!("1. Initialize a workspace: crabdb init --working-tree");
        println!("2. Check setup: crabdb acp doctor --agent {}", args.agent);
        println!("3. Configure your ACP editor with:");
        println!("{}", install.snippet);
        println!("4. After one prompt: crabdb transcript <lane>");
        println!("5. Review or recover: crabdb lane review <lane>");
    }
    Ok(())
}

fn handle_acp_install(ctx: &RuntimeContext, args: AcpInstallArgs) -> Result<()> {
    let report = crabdb::acp::acp_install_report(&args.agent, &args.editor, args.dry_run)?;
    render_acp_install(&report, ctx.json, ctx.quiet, args.print_only)
}

fn handle_acp_list(ctx: &RuntimeContext) -> Result<()> {
    let profiles = crabdb::acp::acp_provider_profiles();
    render_acp_profiles(&profiles, ctx.json, ctx.quiet)
}

fn handle_acp_sessions(ctx: &RuntimeContext, args: AcpSessionsArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.list_lane_acp_sessions(args.lane.as_deref())?;
    render_acp_sessions(&report, ctx.json, ctx.quiet)
}

fn handle_acp_doctor(ctx: &RuntimeContext, args: AcpDoctorArgs) -> Result<()> {
    let mut checks = Vec::new();
    let mut warnings = Vec::new();
    let mut status = "ok".to_string();
    let mut smoke_lane = None;
    let mut smoke_session_id = None;

    let db_result = open_db(ctx);
    match &db_result {
        Ok(_) => checks.push(acp_check("workspace", "ok", "CrabDB workspace opened")),
        Err(err) => {
            status = "failed".to_string();
            checks.push(acp_check("workspace", "failed", &format!("{err}")));
        }
    }

    let profile = crabdb::acp::acp_provider_profile(&args.agent)?;
    if profile.available {
        checks.push(acp_check(
            "provider",
            "ok",
            &format!("{} profile available", profile.agent),
        ));
    } else {
        status = "failed".to_string();
        checks.push(acp_check("provider", "failed", &profile.notes.join("; ")));
    }

    let relay_command = if args.relay_command.is_empty() {
        profile.relay_command
    } else {
        args.relay_command
    };
    if relay_command.iter().any(|part| part == "relay")
        && relay_command.iter().any(|part| part == "--")
    {
        checks.push(acp_check("relay", "ok", "relay command shape is valid"));
    } else {
        status = "failed".to_string();
        checks.push(acp_check(
            "relay",
            "failed",
            "relay command should include `acp relay` and `-- <upstream>`",
        ));
    }

    if let Some(upstream) = upstream_command_name(&relay_command) {
        if command_available(upstream) || upstream.starts_with('<') {
            checks.push(acp_check("launch", "ok", "upstream command is available"));
        } else {
            status = "failed".to_string();
            checks.push(acp_check(
                "launch",
                "failed",
                &format!("upstream command `{upstream}` was not found on PATH"),
            ));
        }
    } else {
        status = "failed".to_string();
        checks.push(acp_check(
            "launch",
            "failed",
            "relay command does not include an upstream command",
        ));
    }

    if db_result.is_ok() {
        if args.agent == "fake" {
            match run_acp_fake_smoke(ctx, "acp-doctor", false) {
                Ok(smoke) if smoke.status == "ok" => {
                    smoke_lane = smoke.lane;
                    smoke_session_id = smoke.session_id;
                    checks.extend(smoke.checks);
                }
                Ok(smoke) => {
                    status = "failed".to_string();
                    smoke_lane = smoke.lane;
                    smoke_session_id = smoke.session_id;
                    checks.extend(smoke.checks);
                    warnings.extend(smoke.warnings);
                }
                Err(err) => {
                    status = "failed".to_string();
                    checks.push(acp_check("capture", "failed", &format!("{err}")));
                }
            }
        } else {
            checks.push(acp_check(
                "capture",
                "ok",
                "use `crabdb acp doctor --agent fake` for deterministic end-to-end capture smoke; external provider launch is validated by command availability",
            ));
        }
    } else {
        warnings.push("capture smoke skipped because workspace could not be opened".to_string());
    }

    let report = AcpDoctorReport {
        status,
        provider: profile.agent,
        relay_command,
        lane: smoke_lane,
        session_id: smoke_session_id,
        checks,
        warnings,
    };
    render_acp_doctor(&report, ctx.json, ctx.quiet)
}

fn handle_acp_test_agent(args: AcpTestAgentArgs) -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut cwd = std::env::current_dir().map_err(Error::from)?;
    let session_id = args.session_id.as_str();
    for line in stdin.lock().lines() {
        let line = line.map_err(Error::from)?;
        if line.trim().is_empty() {
            continue;
        }
        let message: serde_json::Value = serde_json::from_str(&line)?;
        let id = message
            .get("id")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        match message.get("method").and_then(serde_json::Value::as_str) {
            Some("initialize") => {
                write_json_line_to_stdout(
                    &mut stdout,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "protocolVersion": 1, "agentCapabilities": {} }
                    }),
                )?;
            }
            Some("session/new") | Some("session/load") | Some("session/resume") => {
                if let Some(next_cwd) = message
                    .get("params")
                    .and_then(|params| params.get("cwd"))
                    .and_then(serde_json::Value::as_str)
                {
                    cwd = std::path::PathBuf::from(next_cwd);
                }
                write_json_line_to_stdout(
                    &mut stdout,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "sessionId": session_id }
                    }),
                )?;
            }
            Some("session/prompt") => {
                write_json_line_to_stdout(
                    &mut stdout,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "session/update",
                        "params": {
                            "sessionId": session_id,
                            "update": {
                                "sessionUpdate": "available_commands_update",
                                "commands": [{ "name": "write_file", "description": "diagnostic command" }]
                            }
                        }
                    }),
                )?;
                write_json_line_to_stdout(
                    &mut stdout,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "session/update",
                        "params": {
                            "sessionId": session_id,
                            "update": {
                                "sessionUpdate": "tool_call",
                                "toolCallId": "tool_doctor",
                                "title": "write README",
                                "status": "pending"
                            }
                        }
                    }),
                )?;
                if args.request_permission {
                    write_json_line_to_stdout(
                        &mut stdout,
                        &serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": 50,
                            "method": "session/request_permission",
                            "params": {
                                "sessionId": session_id,
                                "toolCall": { "title": "approve diagnostic write" },
                                "options": [{ "optionId": "allow", "kind": "allow_once", "name": "Allow" }]
                            }
                        }),
                    )?;
                }
                let assistant_text = if let Some(bytes) = args.huge_message_bytes {
                    "x".repeat(bytes)
                } else {
                    "diagnostic complete".to_string()
                };
                write_json_line_to_stdout(
                    &mut stdout,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "session/update",
                        "params": {
                            "sessionId": session_id,
                            "update": {
                                "sessionUpdate": "tool_call_update",
                                "toolCallId": "tool_doctor",
                                "status": "completed"
                            }
                        }
                    }),
                )?;
                write_json_line_to_stdout(
                    &mut stdout,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "session/update",
                        "params": {
                            "sessionId": session_id,
                            "update": {
                                "sessionUpdate": "agent_message_chunk",
                                "messageId": "msg_doctor",
                                "content": { "type": "text", "text": assistant_text }
                            }
                        }
                    }),
                )?;
                if args.malformed_after_update {
                    stdout.write_all(b"{not-json\n").map_err(Error::from)?;
                    stdout.flush().map_err(Error::from)?;
                    return Ok(());
                }
                if args.crash_after_update {
                    std::process::exit(42);
                }
                if let Some(ms) = args.sleep_before_result_ms {
                    std::thread::sleep(std::time::Duration::from_millis(ms));
                }
                std::fs::write(cwd.join("README.md"), "diagnostic complete\n")
                    .map_err(Error::from)?;
                write_json_line_to_stdout(
                    &mut stdout,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "stopReason": "end_turn" }
                    }),
                )?;
            }
            Some("session/close") => {
                write_json_line_to_stdout(
                    &mut stdout,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {}
                    }),
                )?;
            }
            _ => {
                write_json_line_to_stdout(
                    &mut stdout,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {}
                    }),
                )?;
            }
        }
    }
    Ok(())
}

fn handle_acp_relay(ctx: &RuntimeContext, args: AcpRelayArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let materialize = if args.no_materialize {
        false
    } else if let Some(materialize) = args.materialize {
        materialize
    } else {
        true
    };

    crabdb::acp::run_stdio_relay(AcpRelayOptions {
        workspace_root: db.workspace_root().to_path_buf(),
        db_dir: db.db_dir().to_path_buf(),
        lane: args.lane,
        from_ref: args.from,
        provider: args.provider,
        model: args.model,
        materialize,
        workdir: args.workdir,
        inject_mcp: !args.no_mcp,
        upstream_command: args.command,
    })
}

fn acp_check(name: &str, status: &str, message: &str) -> AcpDoctorCheck {
    AcpDoctorCheck {
        name: name.to_string(),
        status: status.to_string(),
        message: message.to_string(),
    }
}

fn upstream_command_name(relay_command: &[String]) -> Option<&str> {
    let index = relay_command.iter().position(|part| part == "--")?;
    relay_command.get(index + 1).map(String::as_str)
}

fn command_available(command: &str) -> bool {
    if command.contains(std::path::MAIN_SEPARATOR) {
        return std::path::Path::new(command).is_file();
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(command).is_file())
}

fn run_acp_fake_smoke(
    ctx: &RuntimeContext,
    lane: &str,
    crash_after_update: bool,
) -> Result<AcpDoctorReport> {
    let db = open_db(ctx)?;
    let workspace = db.workspace_root().to_path_buf();
    drop(db);
    run_acp_fake_smoke_in_workspace(&workspace, lane, crash_after_update)
}

fn run_acp_fake_smoke_in_workspace(
    workspace: &std::path::Path,
    lane: &str,
    crash_after_update: bool,
) -> Result<AcpDoctorReport> {
    let current_exe = std::env::current_exe().map_err(Error::from)?;
    let acp_session_id = unique_fake_acp_session_id(lane)?;
    let mut relay = Command::new(&current_exe)
        .arg("--workspace")
        .arg(&workspace)
        .arg("acp")
        .arg("relay")
        .arg("--lane")
        .arg(lane)
        .arg("--materialize")
        .arg("--provider")
        .arg("fake")
        .arg("--")
        .arg(&current_exe)
        .arg("acp")
        .arg("test-agent")
        .arg("--session-id")
        .arg(&acp_session_id)
        .args(if crash_after_update {
            vec!["--crash-after-update"]
        } else {
            Vec::new()
        })
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(Error::from)?;

    let mut stdin = relay
        .stdin
        .take()
        .ok_or_else(|| Error::InvalidInput("failed to open relay stdin".to_string()))?;
    let stdout = relay
        .stdout
        .take()
        .ok_or_else(|| Error::InvalidInput("failed to open relay stdout".to_string()))?;
    let mut stdout = std::io::BufReader::new(stdout);

    write_json_line_to_stdout(
        &mut stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": { "protocolVersion": 1 }
        }),
    )?;
    let initialize = read_json_line_from_reader(&mut stdout)?;

    write_json_line_to_stdout(
        &mut stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "session/new",
            "params": { "cwd": workspace, "mcpServers": [] }
        }),
    )?;
    let session_new = read_json_line_from_reader(&mut stdout)?;

    write_json_line_to_stdout(
        &mut stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "session/prompt",
            "params": {
                "sessionId": acp_session_id,
                "prompt": [{ "type": "text", "text": "run diagnostic" }]
            }
        }),
    )?;

    let mut prompt_result = None;
    while let Ok(message) = read_json_line_from_reader(&mut stdout) {
        if message.get("id").and_then(serde_json::Value::as_i64) == Some(2) {
            prompt_result = Some(message);
            break;
        }
    }
    drop(stdin);
    let output = relay.wait_with_output().map_err(Error::from)?;

    let mut checks = Vec::new();
    let mut warnings = Vec::new();
    let mut status = "ok".to_string();
    if initialize["result"]["_meta"]["crabdb"]["relay"] == true {
        checks.push(acp_check(
            "initialize",
            "ok",
            "ACP initialize roundtrip completed",
        ));
    } else {
        status = "failed".to_string();
        checks.push(acp_check(
            "initialize",
            "failed",
            "initialize response did not include CrabDB relay metadata",
        ));
    }

    if session_new["result"]["sessionId"] == acp_session_id {
        checks.push(acp_check("session", "ok", "ACP session/new completed"));
    } else {
        status = "failed".to_string();
        checks.push(acp_check("session", "failed", "ACP session/new failed"));
    }

    if crash_after_update {
        if !output.status.success() {
            checks.push(acp_check("crash", "ok", "upstream crash was observed"));
        } else {
            status = "failed".to_string();
            checks.push(acp_check(
                "crash",
                "failed",
                "expected upstream crash did not occur",
            ));
        }
    } else if output.status.success() && prompt_result.is_some() {
        checks.push(acp_check("prompt", "ok", "ACP session/prompt completed"));
    } else {
        status = "failed".to_string();
        checks.push(acp_check(
            "prompt",
            "failed",
            &format!(
                "relay prompt failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        ));
    }

    let db = CrabDb::open(workspace)?;
    let mapping = db.try_lane_acp_session(&acp_session_id)?;
    let Some(mapping) = mapping else {
        status = "failed".to_string();
        checks.push(acp_check(
            "mapping",
            "failed",
            "ACP session mapping was not recorded",
        ));
        return Ok(AcpDoctorReport {
            status,
            provider: "fake".to_string(),
            relay_command: Vec::new(),
            lane: Some(lane.to_string()),
            session_id: None,
            checks,
            warnings,
        });
    };
    checks.push(acp_check(
        "mapping",
        "ok",
        "CrabDB ACP session mapping recorded",
    ));

    let details = db.show_lane_session(&mapping.crabdb_session_id)?;
    let has_user = details
        .messages
        .iter()
        .any(|message| message.role == "user" && message.body.contains("diagnostic"));
    let has_assistant = details
        .messages
        .iter()
        .any(|message| message.role == "assistant" && message.body.contains("diagnostic complete"));
    let has_tool = details.events.iter().any(|event| {
        matches!(
            event.event_type.as_str(),
            "tool_call" | "tool_call_update" | "span_started" | "span_ended"
        )
    });
    let has_private = details.events.iter().any(|event| {
        event
            .payload
            .as_ref()
            .is_some_and(|payload| payload.to_string().contains("agent_thought"))
    });
    let has_checkpoint = details.turns.iter().any(|turn| turn.after_change.is_some())
        || details
            .operations
            .iter()
            .any(|operation| operation.kind == OperationKind::LaneRecord);
    if has_user
        && (has_assistant || crash_after_update)
        && has_tool
        && has_checkpoint
        && !has_private
    {
        checks.push(acp_check(
            "capture",
            "ok",
            "prompt, messages, tool events, spans, and checkpoint recorded",
        ));
    } else {
        status = "failed".to_string();
        checks.push(acp_check(
            "capture",
            "failed",
            "captured session is missing prompt, messages, tool events, checkpoint, or privacy filtering",
        ));
        warnings.push(format!(
            "capture flags: user={has_user} assistant={has_assistant} tool={has_tool} checkpoint={has_checkpoint} private={has_private}"
        ));
    }

    Ok(AcpDoctorReport {
        status,
        provider: "fake".to_string(),
        relay_command: Vec::new(),
        lane: Some(lane.to_string()),
        session_id: Some(mapping.crabdb_session_id),
        checks,
        warnings,
    })
}

fn unique_fake_acp_session_id(lane: &str) -> Result<String> {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| Error::InvalidInput(format!("system clock before UNIX_EPOCH: {err}")))?
        .as_nanos();
    let lane_part = lane
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>();
    Ok(format!(
        "sess_fake_{}_{}_{}",
        lane_part,
        std::process::id(),
        suffix
    ))
}

fn create_demo_workspace() -> Result<std::path::PathBuf> {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| Error::InvalidInput(format!("system clock before UNIX_EPOCH: {err}")))?
        .as_nanos();
    let workspace =
        std::env::temp_dir().join(format!("crabdb-acp-demo-{}-{suffix}", std::process::id()));
    std::fs::create_dir_all(&workspace).map_err(Error::from)?;
    std::fs::write(workspace.join("README.md"), "hello\n").map_err(Error::from)?;
    Ok(workspace)
}

fn write_json_line_to_stdout<W: Write>(writer: &mut W, value: &serde_json::Value) -> Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n").map_err(Error::from)?;
    writer.flush().map_err(Error::from)
}

fn read_json_line_from_reader<R: BufRead>(reader: &mut R) -> Result<serde_json::Value> {
    let mut line = String::new();
    let bytes = reader.read_line(&mut line).map_err(Error::from)?;
    if bytes == 0 {
        return Err(Error::InvalidInput(
            "relay stdout closed before JSON response".to_string(),
        ));
    }
    serde_json::from_str(line.trim_end()).map_err(Error::from)
}
