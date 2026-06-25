#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRunState {
    pub run_id: String,
    pub agent_id: String,
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
pub struct AgentRunPauseReport {
    pub run_state: AgentRunState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRunResumeReport {
    pub run_state: AgentRunState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentApproval {
    pub approval_id: String,
    pub agent_id: String,
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
pub struct AgentApprovalRequestReport {
    pub approval: AgentApproval,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_state: Option<AgentRunState>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentApprovalDecisionReport {
    pub approval: AgentApproval,
    pub decision: String,
    #[serde(default)]
    pub run_states: Vec<AgentRunState>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentSessionStartReport {
    pub session: AgentSession,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentSessionCurrentReport {
    pub agent_id: String,
    pub agent_name: String,
    pub ref_name: String,
    pub session: Option<AgentSession>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentSessionEndReport {
    pub session: AgentSession,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentSessionDetails {
    pub session: AgentSession,
    pub turns: Vec<AgentTurn>,
    pub messages: Vec<Message>,
    pub events: Vec<AgentEventRecord>,
    pub operations: Vec<TimelineEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRemoveReport {
    pub agent_id: String,
    pub ref_name: String,
    pub removed_workdir: Option<String>,
    pub forced: bool,
}
