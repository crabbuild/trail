
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub old_path: Option<String>,
    pub file_id: Option<FileId>,
    pub kind: FileChangeKind,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub line_changes: Vec<LineChange>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    TypeChanged,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LineChange {
    pub line_id: LineId,
    pub kind: LineChangeKind,
    pub old_line_number: Option<u64>,
    pub new_line_number: Option<u64>,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum LineChangeKind {
    Added,
    Modified,
    Deleted,
    Moved,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RefRecord {
    pub name: String,
    pub change_id: ChangeId,
    pub root_id: ObjectId,
    pub operation_id: ObjectId,
    pub generation: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub version: u16,
    pub id: MessageId,
    pub role: String,
    pub body: String,
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub change_id: Option<ChangeId>,
    pub created_at: i64,
}

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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentSession {
    pub session_id: String,
    pub agent_id: String,
    pub title: Option<String>,
    pub status: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub metadata_json: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentSessionContextReport {
    pub session: AgentSession,
    pub message_count: u64,
    pub event_count: u64,
    pub turn_count: u64,
    pub operation_count: u64,
    pub recent_messages: Vec<Message>,
    pub recent_events: Vec<AgentEventRecord>,
    pub recent_turns: Vec<AgentTurn>,
    pub recent_operations: Vec<TimelineEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTurn {
    pub turn_id: String,
    pub agent_id: String,
    pub session_id: Option<String>,
    pub base_change: ChangeId,
    pub before_change: ChangeId,
    pub after_change: Option<ChangeId>,
    pub status: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub metadata_json: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTurnStartReport {
    pub turn: AgentTurn,
    pub session: AgentSession,
    pub base_root: ObjectId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTurnDetails {
    pub turn: AgentTurn,
    pub session: Option<AgentSession>,
    pub messages: Vec<Message>,
    pub events: Vec<AgentEventRecord>,
    pub operations: Vec<TimelineEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTurnEventReport {
    pub event: AgentEventRecord,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTurnEndReport {
    pub turn: AgentTurn,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentEventRecord {
    pub event_id: String,
    pub agent_id: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub event_type: String,
    pub change_id: Option<ChangeId>,
    pub message_id: Option<MessageId>,
    pub payload: Option<serde_json::Value>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTraceSpan {
    pub span_id: String,
    pub trace_id: String,
    pub agent_id: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub parent_span_id: Option<String>,
    pub span_type: String,
    pub name: String,
    pub status: String,
    pub started_event_id: String,
    pub ended_event_id: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub duration_ms: Option<u64>,
    pub attributes: Option<serde_json::Value>,
    pub result: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTraceSummaryReport {
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub trace_id: Option<String>,
    pub span_count: u64,
    pub open_span_count: u64,
    pub ended_span_count: u64,
    pub failed_span_count: u64,
    pub total_duration_ms: u64,
    pub max_duration_ms: u64,
    pub average_duration_ms: Option<f64>,
    pub status_counts: Vec<NamedCount>,
    pub span_type_counts: Vec<NamedCount>,
    pub trace_counts: Vec<NamedCount>,
    pub slowest_spans: Vec<AgentTraceSpan>,
    pub open_spans: Vec<AgentTraceSpan>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NamedCount {
    pub name: String,
    pub count: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTraceSpanStartReport {
    pub span: AgentTraceSpan,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTraceSpanEndReport {
    pub span: AgentTraceSpan,
}

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

