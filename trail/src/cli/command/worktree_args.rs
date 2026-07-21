use std::path::PathBuf;

use clap::Args;

#[derive(Args)]
pub(super) struct InitArgs {
    #[arg(long)]
    pub(super) from_git: bool,
    #[arg(long)]
    pub(super) working_tree: bool,
    #[arg(long, default_value = "main")]
    pub(super) branch: String,
    #[arg(long = "text-policy", value_enum)]
    pub(super) text_policy: Option<TextPolicyArg>,
    #[arg(long)]
    pub(super) force: bool,
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub(super) enum TextPolicyArg {
    Minimal,
    Balanced,
    Full,
}

impl TextPolicyArg {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Balanced => "balanced",
            Self::Full => "full",
        }
    }
}

#[derive(Args)]
pub(super) struct StatusArgs {
    #[arg(long)]
    pub(super) branch: Option<String>,
}

#[derive(Args)]
pub(super) struct RecordArgs {
    #[arg(short, long)]
    pub(super) message: Option<String>,
    #[arg(long, num_args = 1..)]
    pub(super) paths: Vec<String>,
    #[arg(long)]
    pub(super) kind: Option<String>,
    #[arg(long)]
    pub(super) session: Option<String>,
    #[arg(long)]
    pub(super) allow_ignored: bool,
}

#[derive(Args)]
pub(super) struct WatchArgs {
    #[arg(short, long)]
    pub(super) message: Option<String>,
    #[arg(long)]
    pub(super) session: Option<String>,
    #[arg(long, default_value_t = 2)]
    pub(super) interval_secs: u64,
    #[arg(long = "debounce-ms", alias = "debounce")]
    pub(super) debounce_ms: Option<u64>,
    #[arg(long = "include-untracked")]
    pub(super) include_untracked: bool,
    #[arg(long)]
    pub(super) once: bool,
}

#[derive(Args)]
pub(super) struct DiffArgs {
    pub(super) range: Option<String>,
    #[arg(long)]
    pub(super) patch: bool,
    #[arg(long)]
    pub(super) stat: bool,
    #[arg(long = "name-only")]
    pub(super) name_only: bool,
    #[arg(long = "name-status")]
    pub(super) name_status: bool,
    #[arg(long)]
    pub(super) dirty: bool,
    #[arg(long)]
    pub(super) root: Option<String>,
    #[arg(long = "show-line-ids")]
    pub(super) show_line_ids: bool,
}

#[derive(Args)]
pub(super) struct CheckoutArgs {
    pub(super) target: String,
    #[arg(long)]
    pub(super) force: bool,
    #[arg(long)]
    pub(super) record_dirty: bool,
    #[arg(long)]
    pub(super) dry_run: bool,
    #[arg(long)]
    pub(super) workdir: Option<PathBuf>,
}

#[derive(Args)]
pub(super) struct BranchArgs {
    pub(super) name: Option<String>,
    #[arg(long)]
    pub(super) from: Option<String>,
    #[arg(long)]
    pub(super) delete: Option<String>,
    #[arg(long)]
    pub(super) rename: Option<String>,
    #[arg(long)]
    pub(super) to: Option<String>,
}

#[derive(Args)]
pub(super) struct MergeArgs {
    pub(super) source: String,
    #[arg(long)]
    pub(super) into: String,
    #[arg(long)]
    pub(super) strategy: Option<String>,
    #[arg(long)]
    pub(super) dry_run: bool,
}
