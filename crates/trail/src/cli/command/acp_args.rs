use std::path::PathBuf;

use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(super) enum AcpSubcommand {
    /// Print guided setup for an ACP provider without mutating editor config.
    Install(AcpInstallArgs),
    /// Run ACP integration diagnostics.
    Doctor(AcpDoctorArgs),
    /// List supported ACP provider profiles.
    List,
    /// List captured ACP sessions.
    Sessions(AcpSessionsArgs),
    /// Run a local ACP stdio relay in front of a real ACP coding agent.
    Relay(AcpRelayArgs),
}

#[derive(Args)]
pub(super) struct AcpCommand {
    #[command(subcommand)]
    pub(super) command: AcpSubcommand,
}

#[derive(Args)]
pub(super) struct AcpInstallArgs {
    #[arg(
        long,
        default_value = "claude-code",
        value_name = "AGENT",
        help = "ACP provider profile: claude-code, codex, cursor"
    )]
    pub(super) agent: String,
    #[arg(long, default_value = "generic")]
    pub(super) editor: String,
    #[arg(long)]
    pub(super) dry_run: bool,
    #[arg(long = "print")]
    pub(super) print_only: bool,
}

#[derive(Args)]
pub(super) struct AcpDoctorArgs {
    #[arg(
        long,
        default_value = "claude-code",
        value_name = "AGENT",
        help = "ACP provider profile: claude-code, codex, cursor"
    )]
    pub(super) agent: String,
    #[arg(long = "relay-command", num_args = 1.., allow_hyphen_values = true)]
    pub(super) relay_command: Vec<String>,
}

#[derive(Args)]
pub(super) struct AcpSessionsArgs {
    #[arg(long)]
    pub(super) lane: Option<String>,
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

#[derive(Args)]
pub(super) struct TranscriptArgs {
    pub(super) selector: String,
}

#[derive(Subcommand)]
pub(super) enum TopTurnSubcommand {
    /// Show one durable turn by id.
    Show(TopTurnShowArgs),
}

#[derive(Args)]
pub(super) struct TopTurnCommand {
    #[command(subcommand)]
    pub(super) command: TopTurnSubcommand,
}

#[derive(Args)]
pub(super) struct TopTurnShowArgs {
    pub(super) turn_id: String,
}
