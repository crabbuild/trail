use std::path::PathBuf;

use trail::agent_skills::{install_agent_skills, AgentSkillInstallRequest, AgentSkillProvider};
use trail::{Error, Result};

use super::*;

pub(super) fn handle_install_command(ctx: &RuntimeContext, args: &InstallArgs) -> Result<()> {
    let provider = args.provider.as_domain();
    let config_root = provider_config_root(provider)?;
    let report = install_agent_skills(AgentSkillInstallRequest {
        provider,
        config_root: &config_root,
        force: args.force,
        dry_run: args.dry_run,
    })?;
    render_semantic_report(
        "Trail agent skills installation",
        &report,
        ctx.json,
        &ctx.render,
    )
}

fn provider_config_root(provider: AgentSkillProvider) -> Result<PathBuf> {
    if provider == AgentSkillProvider::Codex
        && let Some(root) = std::env::var_os("CODEX_HOME").filter(|root| !root.is_empty())
    {
        return Ok(PathBuf::from(root));
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| {
            Error::InvalidInput(
                "cannot locate the user home directory for agent skill installation".to_string(),
            )
        })?;
    let xdg_config_home = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|root| !root.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    Ok(match provider {
        AgentSkillProvider::Codex => home.join(".codex"),
        AgentSkillProvider::Claude => home.join(".claude"),
        AgentSkillProvider::Copilot => home.join(".copilot"),
        AgentSkillProvider::Gemini => home.join(".gemini"),
        AgentSkillProvider::Cursor => home.join(".cursor"),
        AgentSkillProvider::Windsurf => home.join(".codeium/windsurf"),
        AgentSkillProvider::Cline => home.join(".cline"),
        AgentSkillProvider::Roo => home.join(".roo"),
        AgentSkillProvider::Kilo => home.join(".kilo"),
        AgentSkillProvider::OpenCode => xdg_config_home.join("opencode"),
        AgentSkillProvider::Amp => xdg_config_home.join("agents"),
        AgentSkillProvider::Kiro => home.join(".kiro"),
        AgentSkillProvider::Qwen => home.join(".qwen"),
    })
}
