use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub(crate) struct ToolCall {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) arguments: Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResourceReadArgs {
    pub(crate) uri: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PromptGetArgs {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) arguments: Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CompletionArgs {
    #[serde(rename = "ref")]
    pub(crate) reference: CompletionReference,
    pub(crate) argument: CompletionArgument,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CompletionReference {
    #[serde(rename = "type")]
    pub(crate) reference_type: String,
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) uri: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CompletionArgument {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) value: String,
}
