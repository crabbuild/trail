use serde::Deserialize;

use super::default_completed_status;

#[derive(Debug, Deserialize)]
pub(crate) struct SpawnAgentRequest {
    pub(crate) name: String,
    #[serde(default, alias = "from_ref", alias = "branch")]
    pub(crate) from: Option<String>,
    #[serde(default)]
    pub(crate) materialize: Option<bool>,
    #[serde(default, alias = "workdir_path")]
    pub(crate) workdir: Option<String>,
    #[serde(default)]
    pub(crate) paths: Vec<String>,
    #[serde(default, alias = "include_neighborhood")]
    pub(crate) include_neighbors: bool,
    #[serde(default)]
    pub(crate) provider: Option<String>,
    #[serde(default)]
    pub(crate) model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BeginTurnRequest {
    pub(crate) agent: String,
    #[serde(default)]
    pub(crate) branch: Option<String>,
    #[serde(default)]
    pub(crate) session_title: Option<String>,
    #[serde(default)]
    pub(crate) base_change: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AddMessageRequest {
    pub(crate) role: String,
    #[serde(default)]
    pub(crate) content: Option<String>,
    #[serde(default)]
    pub(crate) text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AddEventRequest {
    #[serde(alias = "type")]
    pub(crate) event_type: String,
    #[serde(default)]
    pub(crate) payload: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) change_id: Option<String>,
    #[serde(default)]
    pub(crate) message_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StartSpanRequest {
    #[serde(alias = "type")]
    pub(crate) span_type: String,
    pub(crate) name: String,
    #[serde(default, alias = "parent_span_id")]
    pub(crate) parent: Option<String>,
    #[serde(default, alias = "trace_id")]
    pub(crate) trace: Option<String>,
    #[serde(default, alias = "attributes_json")]
    pub(crate) attributes: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EndSpanRequest {
    #[serde(default = "default_completed_status")]
    pub(crate) status: String,
    #[serde(default, alias = "result_json")]
    pub(crate) result: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EndTurnRequest {
    #[serde(default = "default_completed_status")]
    pub(crate) status: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentTestRequest {
    pub(crate) command: Vec<String>,
    #[serde(default, alias = "turn")]
    pub(crate) turn_id: Option<String>,
    #[serde(default, alias = "timeout_seconds")]
    pub(crate) timeout_secs: Option<u64>,
    #[serde(default)]
    pub(crate) suite: Option<String>,
    #[serde(default)]
    pub(crate) score: Option<f64>,
    #[serde(default)]
    pub(crate) threshold: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentReadFileRequest {
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) hydrate: Option<bool>,
    #[serde(default)]
    pub(crate) force: bool,
    #[serde(default, alias = "include_neighborhood")]
    pub(crate) include_neighbors: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SyncWorkdirRequest {
    #[serde(default)]
    pub(crate) force: bool,
    #[serde(default)]
    pub(crate) paths: Vec<String>,
    #[serde(default, alias = "include_neighborhood")]
    pub(crate) include_neighbors: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentClaimRequest {
    pub(crate) path: String,
    #[serde(default, alias = "ttl")]
    pub(crate) ttl_secs: Option<u64>,
}
