use super::*;
use crabdb::model::{AcpDoctorCheck, AcpDoctorReport, AcpProviderProfile};

pub(super) fn handle_acp_command(ctx: &RuntimeContext, acp: AcpCommand) -> Result<()> {
    match acp.command {
        AcpSubcommand::Install(args) => handle_acp_install(ctx, args),
        AcpSubcommand::Doctor(args) => handle_acp_doctor(ctx, args),
        AcpSubcommand::List => handle_acp_list(ctx),
        AcpSubcommand::Sessions(args) => handle_acp_sessions(ctx, args),
        AcpSubcommand::Relay(args) => handle_acp_relay(ctx, args),
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
    let install = crabdb::acp::acp_install_report(&args.agent, "generic", true)?;
    if ctx.json {
        return render_json(&serde_json::json!({
            "agent": args.agent,
            "steps": [
                "crabdb init --working-tree",
                format!("crabdb acp doctor --agent {}", install.agent),
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

    let db_result = open_db(ctx);
    match &db_result {
        Ok(_) => checks.push(acp_check("workspace", "ok", "CrabDB workspace opened")),
        Err(err) => {
            status = "failed".to_string();
            checks.push(acp_check("workspace", "failed", &format!("{err}")));
        }
    }

    let profile = match crabdb::acp::acp_provider_profile(&args.agent) {
        Ok(profile) => profile,
        Err(_) if !args.relay_command.is_empty() => AcpProviderProfile {
            agent: args.agent.clone(),
            display_name: args.agent.clone(),
            available: true,
            relay_command: args.relay_command.clone(),
            notes: vec!["using caller-supplied ACP relay command".to_string()],
            supports_acp: true,
            supports_mcp: false,
            supports_terminal: false,
            default_terminal_command: None,
        },
        Err(err) => return Err(err),
    };
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
        profile.relay_command.clone()
    } else {
        args.relay_command.clone()
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
        checks.push(acp_check(
            "capture",
            "skipped",
            "external provider launch is validated by command availability; run a real ACP prompt through an editor to verify capture",
        ));
    } else {
        warnings.push("capture smoke skipped because workspace could not be opened".to_string());
    }

    let report = AcpDoctorReport {
        status,
        provider: profile.agent,
        relay_command,
        lane: None,
        session_id: None,
        checks,
        warnings,
    };
    render_acp_doctor(&report, ctx.json, ctx.quiet)
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
