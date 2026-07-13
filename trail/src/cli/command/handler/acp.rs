use std::collections::BTreeMap;

use super::*;
use trail::model::{AcpDoctorCheck, AcpDoctorReport, AcpProviderProfile};

pub(super) fn handle_acp_command(ctx: &RuntimeContext, acp: AcpCommand) -> Result<()> {
    match acp.command {
        AcpSubcommand::Relay(args) => handle_acp_relay(ctx, args),
    }
}

pub(super) fn handle_transcript_command(ctx: &RuntimeContext, args: TranscriptArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.transcript(&args.selector)?;
    render_transcript(&report, ctx.json, &ctx.render)
}

pub(super) fn handle_top_turn_command(ctx: &RuntimeContext, turn: TopTurnCommand) -> Result<()> {
    match turn.command {
        TopTurnSubcommand::Show(args) => {
            let db = open_db(ctx)?;
            let details = db.show_lane_turn(&args.turn_id)?;
            render_lane_turn_details(&details, ctx.json, &ctx.render)
        }
    }
}

pub(super) fn handle_acp_status(ctx: &RuntimeContext, args: AgentAcpStatusArgs) -> Result<()> {
    let db = open_db(ctx).ok();
    let mut profiles =
        trail::acp::acp_provider_profiles_with_registry(db.as_ref().map(|db| db.db_dir()))?;
    if args.provider.is_some() || args.provider_flag.is_some() {
        let provider = resolve_agent_provider_argument(args.provider, args.provider_flag, None)?;
        let canonical = trail::acp::acp_provider_profile_with_registry(
            &provider,
            db.as_ref().map(|db| db.db_dir()),
        )?
        .agent;
        profiles.retain(|profile| profile.agent == canonical);
    }
    render_acp_profiles(&profiles, ctx.json, &ctx.render)
}

pub(super) fn handle_acp_sessions(ctx: &RuntimeContext, args: AgentAcpSessionsArgs) -> Result<()> {
    let db = open_db(ctx)?;
    let report = db.list_lane_acp_sessions(args.lane.as_deref())?;
    render_acp_sessions(&report, ctx.json, &ctx.render)
}

pub(super) fn handle_acp_doctor(ctx: &RuntimeContext, args: AgentAcpDoctorArgs) -> Result<()> {
    let mut checks = Vec::new();
    let warnings = Vec::new();
    let mut status = "ok".to_string();

    let db_result = open_db(ctx);
    match &db_result {
        Ok(db) => {
            checks.push(acp_check("workspace", "ok", "Trail workspace opened"));
            let capture_journal = capture_journal_check(db);
            if capture_journal.status == "failed" {
                status = "failed".to_string();
            }
            checks.push(capture_journal);
            match trail::acp::validate_acp_path_mapping(db.workspace_root()) {
                Ok(()) => checks.push(acp_check(
                    "path_mapping",
                    "ok",
                    "workspace paths isolate correctly and external roots are preserved",
                )),
                Err(err) => {
                    status = "failed".to_string();
                    checks.push(acp_check("path_mapping", "failed", &err.to_string()));
                }
            }
        }
        Err(err) => {
            status = "failed".to_string();
            checks.push(acp_check("workspace", "failed", &format!("{err}")));
            checks.push(acp_check(
                "capture_journal",
                "skipped",
                "workspace unavailable",
            ));
            checks.push(acp_check(
                "path_mapping",
                "skipped",
                "workspace unavailable",
            ));
        }
    }

    let provider = resolve_agent_provider_argument(args.provider, args.provider_flag, None)?;
    let profile = match trail::acp::acp_provider_profile_with_registry(
        &provider,
        db_result.as_ref().ok().map(|db| db.db_dir()),
    ) {
        Ok(profile) => profile,
        Err(_) if !args.relay_command.is_empty() => AcpProviderProfile {
            agent: provider.clone(),
            display_name: provider,
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
    if relay_command.iter().any(|part| part == "relay") {
        checks.push(acp_check(
            "relay",
            "ok",
            "relay command is valid; built-in providers use the short form",
        ));
    } else {
        status = "failed".to_string();
        checks.push(acp_check(
            "relay",
            "failed",
            "relay command should include `trail acp relay`",
        ));
    }

    if args.relay_command.is_empty() {
        if profile.available {
            checks.push(acp_check(
                "launch",
                "ok",
                "built-in ACP adapter command is available",
            ));
        } else {
            status = "failed".to_string();
            checks.push(acp_check(
                "launch",
                "failed",
                "built-in ACP adapter command is unavailable",
            ));
        }
    } else if let Some(upstream) = upstream_command_name(&relay_command) {
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

    checks.push(acp_check(
        "capture_smoke",
        "skipped",
        "run a real ACP prompt through an editor to add provider-specific capture evidence",
    ));

    let report = AcpDoctorReport {
        status,
        provider: profile.agent,
        relay_command,
        lane: None,
        session_id: None,
        conformance: trail::acp::acp_v1_conformance_evidence(),
        checks,
        warnings,
    };
    render_acp_doctor(&report, ctx.json, &ctx.render)
}

fn capture_journal_check(db: &trail::Trail) -> AcpDoctorCheck {
    let journal = db.db_dir().join("acp-ingress");
    if let Err(err) = std::fs::create_dir_all(&journal) {
        return acp_check(
            "capture_journal",
            "failed",
            &format!("cannot prepare durable ACP capture journal: {err}"),
        );
    }
    match std::fs::read_dir(&journal) {
        Ok(entries) => {
            let pending = entries.filter_map(|entry| entry.ok()).count();
            acp_check(
                "capture_journal",
                "ok",
                &format!("durable capture journal is accessible ({pending} pending spill files)"),
            )
        }
        Err(err) => acp_check(
            "capture_journal",
            "failed",
            &format!("cannot read durable ACP capture journal: {err}"),
        ),
    }
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

    let (provider, upstream_command, upstream_env) =
        resolve_acp_relay_command(&args, Some(db.db_dir()))?;

    trail::acp::run_stdio_relay(AcpRelayOptions {
        workspace_root: db.workspace_root().to_path_buf(),
        db_dir: db.db_dir().to_path_buf(),
        lane: args.lane.clone(),
        from_ref: args.from.clone(),
        provider,
        model: args.model.clone(),
        materialize,
        workdir: args.workdir.clone(),
        inject_mcp: !args.no_mcp,
        upstream_command,
        upstream_env,
    })
}

fn resolve_acp_relay_command(
    args: &AcpRelayArgs,
    cache_dir: Option<&std::path::Path>,
) -> Result<(Option<String>, Vec<String>, BTreeMap<String, String>)> {
    if args.command.is_empty() {
        let agent = args.agent.as_deref().or(args.provider.as_deref()).ok_or_else(|| {
            Error::InvalidInput(
                "choose a built-in ACP agent, for example `trail acp relay codex`, or pass a custom ACP command after `--`".to_string(),
            )
        })?;
        let launch = trail::acp::resolve_acp_provider(agent, cache_dir)?;
        return Ok((
            Some(launch.profile.agent),
            launch.upstream_command,
            launch.upstream_env,
        ));
    }

    if let Some(agent) = args.agent.as_deref() {
        let profile = trail::acp::acp_provider_profile_with_registry(agent, cache_dir)?;
        if let Some(provider) = args.provider.as_deref() {
            let explicit_profile =
                trail::acp::acp_provider_profile_with_registry(provider, cache_dir)?;
            if profile.agent != explicit_profile.agent {
                return Err(Error::InvalidInput(format!(
                    "built-in agent `{}` does not match --provider `{}`",
                    profile.agent, explicit_profile.agent
                )));
            }
        }
        return Ok((Some(profile.agent), args.command.clone(), BTreeMap::new()));
    }

    Ok((args.provider.clone(), args.command.clone(), BTreeMap::new()))
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
