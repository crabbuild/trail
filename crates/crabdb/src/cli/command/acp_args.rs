use std::path::PathBuf;

use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(super) enum AcpSubcommand {
    /// Run a local ACP stdio relay in front of a real ACP coding agent.
    Relay(AcpRelayArgs),
}

#[derive(Args)]
pub(super) struct AcpCommand {
    #[command(subcommand)]
    pub(super) command: AcpSubcommand,
}

#[derive(Args)]
pub(super) struct AcpRelayArgs {
    #[arg(long)]
    pub(super) lane: Option<String>,
    #[arg(long)]
    pub(super) from: Option<String>,
    #[arg(
        long,
        default_missing_value = "true",
        num_args = 0..=1,
        require_equals = true,
        conflicts_with = "no_materialize"
    )]
    pub(super) materialize: Option<bool>,
    #[arg(long = "no-materialize")]
    pub(super) no_materialize: bool,
    #[arg(long)]
    pub(super) workdir: Option<PathBuf>,
    #[arg(long)]
    pub(super) provider: Option<String>,
    #[arg(long)]
    pub(super) model: Option<String>,
    #[arg(long = "no-mcp")]
    pub(super) no_mcp: bool,
    #[arg(last = true, num_args = 1.., required = true)]
    pub(super) command: Vec<String>,
}
