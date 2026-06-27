#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneRunState {
    pub run_id: String,
    pub lane_id: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub approval_id: Option<String>,
    pub status: String,
    pub reason: String,
    pub summary: String,
    pub state: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interruption: Option<serde_json::Value>,
    pub created_at: i64,
    pub updated_at: i64,
    pub resumed_at: Option<i64>,
    pub reviewer: Option<String>,
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneRunPauseReport {
    pub run_state: LaneRunState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneRunResumeReport {
    pub run_state: LaneRunState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneApproval {
    pub approval_id: String,
    pub lane_id: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub action: String,
    pub summary: String,
    pub payload: Option<serde_json::Value>,
    pub status: String,
    pub requested_at: i64,
    pub decided_at: Option<i64>,
    pub reviewer: Option<String>,
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneApprovalRequestReport {
    pub approval: LaneApproval,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_state: Option<LaneRunState>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneApprovalDecisionReport {
    pub approval: LaneApproval,
    pub decision: String,
    #[serde(default)]
    pub run_states: Vec<LaneRunState>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneSessionStartReport {
    pub session: LaneSession,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneSessionCurrentReport {
    pub lane_id: String,
    pub lane_name: String,
    pub ref_name: String,
    pub session: Option<LaneSession>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneSessionEndReport {
    pub session: LaneSession,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneSessionDetails {
    pub session: LaneSession,
    pub turns: Vec<LaneTurn>,
    pub messages: Vec<Message>,
    pub events: Vec<LaneEventRecord>,
    pub operations: Vec<TimelineEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneRemoveReport {
    pub lane_id: String,
    pub ref_name: String,
    pub removed_workdir: Option<String>,
    pub forced: bool,
}
