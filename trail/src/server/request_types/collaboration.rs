use serde::Deserialize;

use crate::model::ConflictManualResolution;

use super::default_completed_status;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MergeLaneRequest {
    #[serde(default, alias = "lane", alias = "name")]
    pub(crate) lane_id: Option<String>,
    #[serde(default)]
    pub(crate) strategy: Option<String>,
    #[serde(default, alias = "dry-run")]
    pub(crate) dry_run: bool,
    #[serde(default)]
    pub(crate) direct: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionStartRequest {
    pub(crate) lane: String,
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionEndRequest {
    #[serde(default = "default_completed_status")]
    pub(crate) status: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApprovalRequest {
    pub(crate) lane: String,
    pub(crate) action: String,
    pub(crate) summary: String,
    #[serde(default)]
    pub(crate) payload: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default, alias = "turn")]
    pub(crate) turn_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApprovalDecisionRequest {
    pub(crate) decision: String,
    #[serde(default)]
    pub(crate) reviewer: Option<String>,
    #[serde(default)]
    pub(crate) note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LaneRunPauseRequest {
    pub(crate) lane: String,
    pub(crate) reason: String,
    pub(crate) summary: String,
    #[serde(default)]
    pub(crate) state: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) interruption: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default, alias = "turn")]
    pub(crate) turn_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LaneRunResumeRequest {
    #[serde(default)]
    pub(crate) reviewer: Option<String>,
    #[serde(default)]
    pub(crate) note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LeaseAcquireRequest {
    pub(crate) lane: String,
    #[serde(default)]
    pub(crate) path: Option<String>,
    #[serde(default)]
    pub(crate) mode: Option<String>,
    #[serde(default, alias = "ttl")]
    pub(crate) ttl_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnchorCreateRequest {
    pub(crate) path_line: String,
    pub(crate) label: String,
    #[serde(default)]
    pub(crate) branch: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MergeQueueAddRequest {
    pub(crate) source: String,
    #[serde(alias = "into", alias = "target_branch")]
    pub(crate) target: String,
    #[serde(default)]
    pub(crate) priority: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MergeQueueRunRequest {
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConflictResolveRequest {
    #[serde(default)]
    pub(crate) take: Option<String>,
    #[serde(default)]
    pub(crate) manual: Option<ConflictManualResolution>,
}

pub(crate) fn default_lease_mode() -> String {
    "write".to_string()
}
