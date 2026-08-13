use clap::Args;

use trail::agent_skills::AgentSkillProvider;

#[derive(Clone, Debug, clap::ValueEnum)]
pub(super) enum AgentSkillProviderArg {
    Codex,
    #[value(alias = "claude-code")]
    Claude,
    #[value(alias = "github-copilot")]
    Copilot,
    #[value(alias = "gemini-cli")]
    Gemini,
    Cursor,
    Windsurf,
    Cline,
    #[value(alias = "roo-code", alias = "roocode")]
    Roo,
    #[value(alias = "kilo-code", alias = "kilocode")]
    Kilo,
    #[value(name = "opencode", alias = "open-code")]
    OpenCode,
    Amp,
    #[value(alias = "kiro-cli")]
    Kiro,
    #[value(alias = "qwen-code")]
    Qwen,
}

impl AgentSkillProviderArg {
    pub(super) fn as_domain(&self) -> AgentSkillProvider {
        match self {
            Self::Codex => AgentSkillProvider::Codex,
            Self::Claude => AgentSkillProvider::Claude,
            Self::Copilot => AgentSkillProvider::Copilot,
            Self::Gemini => AgentSkillProvider::Gemini,
            Self::Cursor => AgentSkillProvider::Cursor,
            Self::Windsurf => AgentSkillProvider::Windsurf,
            Self::Cline => AgentSkillProvider::Cline,
            Self::Roo => AgentSkillProvider::Roo,
            Self::Kilo => AgentSkillProvider::Kilo,
            Self::OpenCode => AgentSkillProvider::OpenCode,
            Self::Amp => AgentSkillProvider::Amp,
            Self::Kiro => AgentSkillProvider::Kiro,
            Self::Qwen => AgentSkillProvider::Qwen,
        }
    }
}

#[derive(Args)]
pub(super) struct InstallArgs {
    /// Agent whose user-level skills directory should receive Trail's skill suite.
    #[arg(value_enum)]
    pub(super) provider: AgentSkillProviderArg,
    /// Replace an unmanaged or locally edited Trail skill installation.
    #[arg(long)]
    pub(super) force: bool,
    /// Report the installation action without changing files.
    #[arg(long)]
    pub(super) dry_run: bool,
}
