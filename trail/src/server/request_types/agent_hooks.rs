use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentCaptureRunRequest {
    #[serde(default)]
    pub(crate) lane: Option<String>,
    pub(crate) workdir: String,
    pub(crate) owner_agent: String,
    pub(crate) owner_session_id: String,
    #[serde(default)]
    pub(crate) executor_agent: Option<String>,
    #[serde(default)]
    pub(crate) work_item_id: Option<String>,
    pub(crate) lease_ms: u64,
    #[serde(default)]
    pub(crate) metadata: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentCaptureRunLeaseRequest {
    pub(crate) owner_agent: String,
    pub(crate) owner_session_id: String,
    #[serde(default)]
    pub(crate) lease_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentAttestationCreateRequest {
    #[serde(default = "default_capture_policy")]
    pub(crate) capture_policy: String,
    #[serde(default)]
    pub(crate) metadata: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentLearningReviewRequest {
    pub(crate) reviewer: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentArtifactRedactRequest {
    pub(crate) reason: String,
}

fn default_capture_policy() -> String {
    "native-agent-hooks/v1".to_string()
}
