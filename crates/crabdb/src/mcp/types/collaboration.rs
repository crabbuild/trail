use serde::Deserialize;
use serde_json::Value;

use super::turns::default_completed_status;

#[derive(Debug, Deserialize)]
pub(crate) struct SessionStartArgs {
    pub(crate) lane: String,
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SessionListArgs {
    #[serde(default)]
    pub(crate) lane: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SessionCurrentArgs {
    #[serde(default)]
    pub(crate) lane: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SessionIdArgs {
    pub(crate) session_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SessionContextArgs {
    pub(crate) session_id: String,
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SessionEndArgs {
    pub(crate) session_id: String,
    #[serde(default = "default_completed_status")]
    pub(crate) status: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApprovalRequestArgs {
    pub(crate) lane: String,
    pub(crate) action: String,
    pub(crate) summary: String,
    #[serde(default)]
    pub(crate) payload: Option<Value>,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default, alias = "turn")]
    pub(crate) turn_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApprovalListArgs {
    #[serde(default)]
    pub(crate) lane: Option<String>,
    #[serde(default)]
    pub(crate) status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApprovalShowArgs {
    pub(crate) approval_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApprovalDecideArgs {
    pub(crate) approval_id: String,
    pub(crate) decision: String,
    #[serde(default)]
    pub(crate) reviewer: Option<String>,
    #[serde(default)]
    pub(crate) note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LaneRunPauseArgs {
    pub(crate) lane: String,
    pub(crate) reason: String,
    pub(crate) summary: String,
    #[serde(default)]
    pub(crate) state: Option<Value>,
    #[serde(default)]
    pub(crate) interruption: Option<Value>,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default, alias = "turn")]
    pub(crate) turn_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LaneRunListArgs {
    #[serde(default)]
    pub(crate) lane: Option<String>,
    #[serde(default)]
    pub(crate) status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LaneRunShowArgs {
    pub(crate) run_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LaneRunResumeArgs {
    pub(crate) run_id: String,
    #[serde(default)]
    pub(crate) reviewer: Option<String>,
    #[serde(default)]
    pub(crate) note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnchorCreateArgs {
    pub(crate) path_line: String,
    pub(crate) label: String,
    #[serde(default)]
    pub(crate) branch: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnchorIdArgs {
    pub(crate) anchor_id: String,
    #[serde(default)]
    pub(crate) branch: Option<String>,
}
