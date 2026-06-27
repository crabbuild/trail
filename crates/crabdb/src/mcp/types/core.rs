use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub(crate) struct StatusArgs {
    #[serde(default)]
    pub(crate) branch: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DiffArgs {
    #[serde(default)]
    pub(crate) range: Option<String>,
    #[serde(default)]
    pub(crate) root: Option<String>,
    #[serde(default)]
    pub(crate) dirty: bool,
    #[serde(default)]
    pub(crate) patch: bool,
    #[serde(default, alias = "show-line-ids")]
    pub(crate) show_line_ids: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TimelineArgs {
    #[serde(default)]
    pub(crate) branch: Option<String>,
    #[serde(default)]
    pub(crate) session: Option<String>,
    #[serde(default)]
    pub(crate) lane: Option<String>,
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConfigKeyArgs {
    pub(crate) key: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConfigSetArgs {
    pub(crate) key: String,
    pub(crate) value: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WhyArgs {
    #[serde(default)]
    pub(crate) path_line: Option<String>,
    #[serde(default)]
    pub(crate) line_id: Option<String>,
    #[serde(default)]
    pub(crate) branch: Option<String>,
    #[serde(default)]
    pub(crate) at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HistoryArgs {
    #[serde(default)]
    pub(crate) selector: Option<String>,
    #[serde(default)]
    pub(crate) path: Option<String>,
    #[serde(default)]
    pub(crate) file_id: Option<String>,
    #[serde(default)]
    pub(crate) line_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CodeFromArgs {
    pub(crate) selector: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct IgnorePatternArgs {
    pub(crate) pattern: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct IgnoreCheckArgs {
    pub(crate) path: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GuardrailCheckArgs {
    pub(crate) lane: Option<String>,
    pub(crate) action: String,
    pub(crate) summary: Option<String>,
    pub(crate) payload: Option<Value>,
    #[serde(default)]
    pub(crate) paths: Vec<String>,
}
