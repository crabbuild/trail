#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRecord {
    pub agent_id: String,
    pub name: String,
    pub kind: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub created_at: i64,
    pub metadata_json: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentBranch {
    pub agent_id: String,
    pub ref_name: String,
    pub base_change: ChangeId,
    pub head_change: ChangeId,
    pub base_root: ObjectId,
    pub head_root: ObjectId,
    pub session_id: Option<String>,
    pub workdir: Option<String>,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentDetails {
    pub record: AgentRecord,
    pub branch: AgentBranch,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentStatusReport {
    pub agent: AgentDetails,
    pub changed_paths: Vec<FileDiffSummary>,
    pub queued_merges: u64,
    pub workdir_state: Option<WorktreeState>,
    pub workdir_changed_paths: Vec<FileDiffSummary>,
    pub latest_test: Option<AgentTestSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_eval: Option<AgentTestSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentContributionReport {
    pub status: AgentStatusReport,
    pub operations: Vec<TimelineEntry>,
    pub sessions: Vec<AgentSession>,
    pub recent_events: Vec<AgentEventRecord>,
    pub approvals: Vec<AgentApproval>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentReadinessReport {
    pub agent: AgentDetails,
    pub ready: bool,
    pub status: String,
    pub blockers: Vec<AgentReadinessIssue>,
    pub warnings: Vec<AgentReadinessIssue>,
    pub changed_paths: Vec<FileDiffSummary>,
    pub workdir_state: Option<WorktreeState>,
    pub workdir_changed_paths: Vec<FileDiffSummary>,
    pub queued_merges: u64,
    pub pending_approvals: Vec<AgentApproval>,
    pub conflicts: Vec<ConflictSetSummary>,
    pub latest_test: Option<AgentTestSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_eval: Option<AgentTestSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentReadinessIssue {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentHandoffReport {
    pub agent: AgentDetails,
    pub readiness: AgentReadinessReport,
    pub current_session: Option<AgentSessionDetails>,
    pub recent_sessions: Vec<AgentSession>,
    pub recent_events: Vec<AgentEventRecord>,
    pub recent_spans: Vec<AgentTraceSpan>,
    pub recent_operations: Vec<TimelineEntry>,
    pub next_steps: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentMessageReport {
    pub agent_id: String,
    pub message_id: MessageId,
    pub role: String,
    pub session_id: Option<String>,
}
