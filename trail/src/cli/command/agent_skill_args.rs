use clap::Args;

use trail::agent_skills::AgentSkillProvider;

#[derive(Clone, Debug, clap::ValueEnum)]
pub(super) enum AgentSkillProviderArg {
    Codex,
    #[value(alias = "claude-code")]
    Claude,
}

impl AgentSkillProviderArg {
    pub(super) fn as_domain(&self) -> AgentSkillProvider {
        match self {
            Self::Codex => AgentSkillProvider::Codex,
            Self::Claude => AgentSkillProvider::Claude,
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
