use clap::{Args, Subcommand};

#[derive(Args)]
pub(super) struct TimelineArgs {
    #[arg(long, default_value_t = 30)]
    pub(super) limit: usize,
    #[arg(long)]
    pub(super) branch: Option<String>,
    #[arg(long)]
    pub(super) session: Option<String>,
    #[arg(long)]
    pub(super) lane: Option<String>,
}

#[derive(Args)]
pub(super) struct ShowArgs {
    pub(super) selector: String,
}

#[derive(Subcommand)]
pub(super) enum ObjectSubcommand {
    /// Show a structured object summary for a specific object id.
    Show(ObjectShowArgs),
}

#[derive(Args)]
pub(super) struct ObjectCommand {
    #[command(subcommand)]
    pub(super) command: ObjectSubcommand,
}

#[derive(Args)]
pub(super) struct ObjectShowArgs {
    pub(super) object_id: String,
}

#[derive(Subcommand)]
pub(super) enum RootSubcommand {
    /// Show a `WorktreeRoot` object with stable file metadata.
    Show(RootShowArgs),
}

#[derive(Args)]
pub(super) struct RootCommand {
    #[command(subcommand)]
    pub(super) command: RootSubcommand,
}

#[derive(Args)]
pub(super) struct RootShowArgs {
    pub(super) root_id: String,
}

#[derive(Subcommand)]
pub(super) enum TextSubcommand {
    /// Show a `TextContent` object and stable line identifiers.
    Show(TextShowArgs),
}

#[derive(Args)]
pub(super) struct TextCommand {
    #[command(subcommand)]
    pub(super) command: TextSubcommand,
}

#[derive(Args)]
pub(super) struct TextShowArgs {
    pub(super) text_id: String,
    #[arg(long, default_value_t = 50)]
    pub(super) limit: usize,
}

#[derive(Subcommand)]
pub(super) enum MapSubcommand {
    /// Show a mapped range from a prolly map root.
    Range(MapRangeArgs),
    /// Diff two prolly map roots with optional range filtering.
    Diff(MapDiffArgs),
}

#[derive(Args)]
pub(super) struct MapCommand {
    #[command(subcommand)]
    pub(super) command: MapSubcommand,
}

#[derive(Args)]
pub(super) struct MapRangeArgs {
    pub(super) map_id: String,
    #[arg(long = "map-type", value_enum, default_value = "raw")]
    pub(super) map_type: MapTypeArg,
    #[arg(long)]
    pub(super) start: Option<String>,
    #[arg(long)]
    pub(super) end: Option<String>,
    #[arg(long, default_value_t = 50)]
    pub(super) limit: usize,
}

#[derive(Args)]
pub(super) struct MapDiffArgs {
    pub(super) left_map_id: String,
    pub(super) right_map_id: String,
    #[arg(long = "map-type", value_enum, default_value = "raw")]
    pub(super) map_type: MapTypeArg,
    #[arg(long)]
    pub(super) start: Option<String>,
    #[arg(long)]
    pub(super) end: Option<String>,
    #[arg(long, default_value_t = 50)]
    pub(super) limit: usize,
}

#[derive(Clone, Debug, clap::ValueEnum)]
pub(super) enum MapTypeArg {
    Raw,
    Path,
    FileIndex,
    TextOrder,
    LineIndex,
}

impl MapTypeArg {
    pub(super) fn as_str(&self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Path => "path",
            Self::FileIndex => "file-index",
            Self::TextOrder => "text-order",
            Self::LineIndex => "line-index",
        }
    }
}

#[derive(Args)]
pub(super) struct WhyArgs {
    pub(super) path_line: Option<String>,
    #[arg(long)]
    pub(super) at: Option<String>,
    #[arg(long = "line-id")]
    pub(super) line_id: Option<String>,
}

#[derive(Args)]
pub(super) struct HistoryArgs {
    pub(super) selector: Option<String>,
    #[arg(long)]
    pub(super) file_id: Option<String>,
    #[arg(long)]
    pub(super) line_id: Option<String>,
}

#[derive(Args)]
pub(super) struct CodeFromArgs {
    pub(super) selector: String,
}
