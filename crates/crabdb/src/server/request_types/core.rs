use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct IgnorePatternRequest {
    pub(crate) pattern: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct IgnoreCheckRequest {
    pub(crate) path: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GuardrailCheckRequest {
    pub(crate) agent: Option<String>,
    pub(crate) action: String,
    pub(crate) summary: Option<String>,
    pub(crate) payload: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConfigSetRequest {
    pub(crate) key: String,
    pub(crate) value: String,
}

pub(crate) fn default_completed_status() -> String {
    "completed".to_string()
}
