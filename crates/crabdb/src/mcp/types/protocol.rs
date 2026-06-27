use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolCall {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) arguments: Value,
    #[serde(default, rename = "_meta")]
    pub(crate) _meta: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResourceReadArgs {
    pub(crate) uri: String,
    #[serde(default, rename = "_meta")]
    pub(crate) _meta: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PromptGetArgs {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) arguments: Value,
    #[serde(default, rename = "_meta")]
    pub(crate) _meta: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompletionArgs {
    #[serde(rename = "ref")]
    pub(crate) reference: CompletionReference,
    pub(crate) argument: CompletionArgument,
    #[serde(default, rename = "_meta")]
    pub(crate) _meta: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompletionReference {
    #[serde(rename = "type")]
    pub(crate) reference_type: String,
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) uri: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompletionArgument {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) value: String,
}
