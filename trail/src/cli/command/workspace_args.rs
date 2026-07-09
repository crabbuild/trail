use clap::{Args, Subcommand};

#[derive(Subcommand)]
pub(super) enum ConfigSubcommand {
    /// List all currently configured workspace keys.
    List,
    /// Print one typed workspace config value.
    Get(ConfigGetArgs),
    /// Set one typed workspace config value after validation.
    Set(ConfigSetArgs),
}

#[derive(Args)]
pub(super) struct ConfigCommand {
    #[command(subcommand)]
    pub(super) command: ConfigSubcommand,
}

#[derive(Args)]
pub(super) struct ConfigGetArgs {
    pub(super) key: String,
}

#[derive(Args)]
pub(super) struct ConfigSetArgs {
    pub(super) key: String,
    pub(super) value: String,
}

#[derive(Subcommand)]
pub(super) enum IgnoreSubcommand {
    /// Print the active ignore patterns.
    List,
    /// Add a path pattern to `.trailignore`.
    Add(IgnorePatternArgs),
    /// Remove a path pattern from `.trailignore`.
    Remove(IgnorePatternArgs),
    /// Check whether a path is currently ignored.
    Check(IgnoreCheckArgs),
}

#[derive(Args)]
pub(super) struct IgnoreCommand {
    #[command(subcommand)]
    pub(super) command: IgnoreSubcommand,
}

#[derive(Args)]
pub(super) struct IgnorePatternArgs {
    pub(super) pattern: String,
}

#[derive(Args)]
pub(super) struct IgnoreCheckArgs {
    pub(super) path: String,
}

#[derive(Subcommand)]
pub(super) enum GuardrailsSubcommand {
    /// Preflight a lane action against policy and ignore checks.
    Check(GuardrailCheckArgs),
}

#[derive(Args)]
pub(super) struct GuardrailsCommand {
    #[command(subcommand)]
    pub(super) command: GuardrailsSubcommand,
}

#[derive(Args)]
pub(super) struct GuardrailCheckArgs {
    #[arg(long)]
    pub(super) lane: Option<String>,
    #[arg(long)]
    pub(super) action: String,
    #[arg(long)]
    pub(super) summary: Option<String>,
    #[arg(long = "payload-json")]
    pub(super) payload_json: Option<String>,
    #[arg(long = "path")]
    pub(super) paths: Vec<String>,
}
