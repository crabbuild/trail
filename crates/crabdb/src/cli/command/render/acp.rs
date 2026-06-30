use super::render_json;

use crabdb::model::*;
use crabdb::Result;

pub(crate) fn render_acp_profiles(
    profiles: &[AcpProviderProfile],
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(profiles);
    }
    if !quiet {
        for profile in profiles {
            let status = if profile.available {
                "available"
            } else {
                "missing"
            };
            println!("{} ({status})", profile.agent);
            println!(
                "  capabilities: acp={} mcp={} terminal={}",
                profile.supports_acp, profile.supports_mcp, profile.supports_terminal
            );
            println!("  {}", shell_join(&profile.relay_command));
            for note in &profile.notes {
                println!("  note: {note}");
            }
        }
    }
    Ok(())
}

pub(crate) fn render_acp_install(
    report: &AcpInstallReport,
    json: bool,
    quiet: bool,
    print_only: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if print_only {
            println!("{}", report.snippet);
            return Ok(());
        }
        let detected = if report.detected { "yes" } else { "no" };
        println!("ACP install plan: {}", report.agent);
        println!("Editor: {}", report.editor);
        println!("Detected: {detected}");
        println!("Relay command:");
        println!("  {}", shell_join(&report.relay_command));
        if !report.warnings.is_empty() {
            println!("Warnings:");
            for warning in &report.warnings {
                println!("  {warning}");
            }
        }
        println!("Snippet:");
        println!("{}", report.snippet);
    }
    Ok(())
}

pub(crate) fn render_acp_doctor(report: &AcpDoctorReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("ACP doctor: {}", report.status);
        for check in &report.checks {
            println!("[{}] {}: {}", check.status, check.name, check.message);
        }
        for warning in &report.warnings {
            println!("warning: {warning}");
        }
    }
    Ok(())
}

pub(crate) fn render_acp_sessions(
    report: &AcpSessionListReport,
    json: bool,
    quiet: bool,
) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        if report.sessions.is_empty() {
            println!("No ACP sessions");
        }
        for session in &report.sessions {
            let provider = session.provider.as_deref().unwrap_or("-");
            println!(
                "{} {} {} {}",
                session.acp_session_id, session.status, provider, session.crabdb_session_id
            );
        }
    }
    Ok(())
}

pub(crate) fn render_transcript(report: &TranscriptReport, json: bool, quiet: bool) -> Result<()> {
    if json {
        return render_json(report);
    }
    if !quiet {
        println!("Transcript: {}", report.selector);
        println!("Lane: {}", report.lane_name);
        println!(
            "Session: {} ({})",
            report.session.session_id, report.session.status
        );
        if let Some(acp) = &report.acp_session {
            println!("ACP session: {} ({})", acp.acp_session_id, acp.status);
        }
        for turn in &report.turns {
            let checkpoint = turn
                .checkpoint
                .as_ref()
                .map(|change| change.0.as_str())
                .unwrap_or("-");
            println!();
            println!(
                "Turn: {} {} checkpoint {}",
                turn.turn.turn_id, turn.turn.status, checkpoint
            );
            for message in &turn.messages {
                println!(
                    "{}: {}",
                    message.role,
                    single_line_preview(&message.body, 160)
                );
            }
            for tool in &turn.tool_summaries {
                println!("tool: {tool}");
            }
        }
    }
    Ok(())
}

fn shell_join(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| {
            if part.chars().all(|ch| {
                ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '/' | '.' | '@' | ':')
            }) {
                part.clone()
            } else {
                format!("'{}'", part.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn single_line_preview(value: &str, limit: usize) -> String {
    let mut preview = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if preview.len() > limit {
        preview.truncate(limit.saturating_sub(3));
        preview.push_str("...");
    }
    preview
}
