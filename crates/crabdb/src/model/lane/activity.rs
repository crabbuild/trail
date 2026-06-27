#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneSession {
    pub session_id: String,
    pub lane_id: String,
    pub title: Option<String>,
    pub status: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub metadata_json: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneSessionContextReport {
    pub session: LaneSession,
    pub message_count: u64,
    pub event_count: u64,
    pub turn_count: u64,
    pub operation_count: u64,
    pub recent_messages: Vec<Message>,
    pub recent_events: Vec<LaneEventRecord>,
    pub recent_turns: Vec<LaneTurn>,
    pub recent_operations: Vec<TimelineEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneAcpSession {
    pub acp_session_id: String,
    pub upstream_session_id: Option<String>,
    pub lane_id: String,
    pub crabdb_session_id: String,
    pub cwd: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub upstream_command_json: Option<String>,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AcpProviderProfile {
    pub agent: String,
    pub display_name: String,
    pub available: bool,
    pub relay_command: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AcpInstallReport {
    pub agent: String,
    pub editor: String,
    pub dry_run: bool,
    pub relay_command: Vec<String>,
    pub snippet: String,
    pub detected: bool,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AcpDoctorCheck {
    pub name: String,
    pub status: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AcpDoctorReport {
    pub status: String,
    pub provider: String,
    pub relay_command: Vec<String>,
    pub lane: Option<String>,
    pub session_id: Option<String>,
    pub checks: Vec<AcpDoctorCheck>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AcpSessionListReport {
    pub sessions: Vec<LaneAcpSession>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TranscriptReport {
    pub selector: String,
    pub resolved_kind: String,
    pub lane_id: String,
    pub lane_name: String,
    pub session: LaneSession,
    pub acp_session: Option<LaneAcpSession>,
    pub turns: Vec<TranscriptTurn>,
    pub operations: Vec<TimelineEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TranscriptTurn {
    pub turn: LaneTurn,
    pub messages: Vec<TranscriptMessage>,
    pub events: Vec<LaneEventRecord>,
    pub checkpoint: Option<ChangeId>,
    pub tool_summaries: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TranscriptMessage {
    pub role: String,
    pub body: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StatusSuggestion {
    pub command: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTaskReport {
    pub task_id: String,
    pub name: String,
    pub title: String,
    pub provider: Option<String>,
    pub editor: Option<String>,
    pub lane: String,
    pub workdir: Option<String>,
    pub session_id: Option<String>,
    pub acp_session_id: Option<String>,
    pub status: AgentTaskStatus,
    pub changed_paths: Vec<FileDiffSummary>,
    pub latest_checkpoint: Option<ChangeId>,
    pub turns: usize,
    pub tool_events: usize,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskStatus {
    Empty,
    Active,
    Dirty,
    Ready,
    Blocked,
    Conflicted,
    Applied,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTaskListReport {
    pub tasks: Vec<AgentTaskReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentInboxReport {
    pub total: usize,
    pub attention_count: usize,
    pub groups: Vec<AgentInboxGroup>,
    pub items: Vec<AgentInboxItem>,
    pub tasks: Vec<AgentTaskReport>,
    pub next: StatusSuggestion,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentInboxGroup {
    pub key: String,
    pub label: String,
    pub status: AgentTaskStatus,
    pub tasks: Vec<AgentTaskReport>,
    pub items: Vec<AgentInboxItem>,
    pub next: Option<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentInboxItem {
    pub task: AgentTaskReport,
    pub attention: String,
    pub detail: String,
    pub new_changed_paths: usize,
    pub new_changed_lines: u64,
    pub review_first: Option<AgentInboxReviewTarget>,
    pub next: StatusSuggestion,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentInboxReviewTarget {
    pub path: String,
    pub reason: String,
    pub command: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTaskViewReport {
    pub task: AgentTaskReport,
    pub review: LaneReviewPacketReport,
    pub transcript: Option<TranscriptReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentWorkdirReport {
    pub task: AgentTaskReport,
    pub workdir: Option<String>,
    pub cd_command: Option<String>,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentStoryReport {
    pub task: AgentTaskReport,
    pub summary: String,
    pub turn_summaries: Vec<AgentStoryTurn>,
    pub changed_files: Vec<FileDiffSummary>,
    pub tool_summaries: Vec<String>,
    pub risk_notes: Vec<String>,
    pub next: StatusSuggestion,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentStoryTurn {
    pub index: usize,
    pub id: String,
    pub turn_id: Option<String>,
    pub prompt_preview: Option<String>,
    pub outcome_preview: Option<String>,
    pub checkpoint: Option<ChangeId>,
    pub changed_paths: Vec<FileDiffSummary>,
    pub tool_summaries: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentRiskLevel {
    Low,
    Medium,
    High,
    Blocking,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRiskReason {
    pub code: String,
    pub severity: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRiskReport {
    pub task: AgentTaskReport,
    pub level: AgentRiskLevel,
    pub score: u8,
    pub summary: String,
    pub reasons: Vec<AgentRiskReason>,
    pub recommendations: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentReadyReport {
    pub task: AgentTaskReport,
    pub ready: bool,
    pub status: String,
    pub summary: String,
    pub readiness_status: String,
    pub risk: AgentRiskReport,
    pub blockers: Vec<LaneReadinessIssue>,
    pub warnings: Vec<LaneReadinessIssue>,
    pub apply_preview: Option<AgentApplyReport>,
    pub apply_error: Option<String>,
    pub next: StatusSuggestion,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentReviewBundleReport {
    pub task: AgentTaskReport,
    pub summary: String,
    pub readiness_status: String,
    pub ready_to_apply: bool,
    pub story: AgentStoryReport,
    pub risk: AgentRiskReport,
    pub changes: AgentChangesReport,
    pub review: LaneReviewPacketReport,
    pub transcript: Option<TranscriptReport>,
    pub markdown: String,
    pub next: StatusSuggestion,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentReceiptReport {
    pub task: AgentTaskReport,
    pub summary: String,
    pub status: AgentTaskStatus,
    pub readiness_status: String,
    pub ready_to_apply: bool,
    pub risk: AgentRiskReport,
    pub changed_paths: Vec<FileDiffSummary>,
    pub turns: Vec<AgentStoryTurn>,
    pub tool_summaries: Vec<String>,
    pub validation: Vec<LaneTestSummary>,
    pub latest_checkpoint: Option<ChangeId>,
    pub next: StatusSuggestion,
    pub suggestions: Vec<StatusSuggestion>,
    pub markdown: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentPrDraftReport {
    pub task: AgentTaskReport,
    pub title: String,
    pub body: String,
    pub ready_to_apply: bool,
    pub readiness_status: String,
    pub risk: AgentRiskReport,
    pub changed_paths: Vec<FileDiffSummary>,
    pub validation: Vec<LaneTestSummary>,
    pub latest_checkpoint: Option<ChangeId>,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentSummaryReport {
    pub task: AgentTaskReport,
    pub summary: String,
    pub ready: bool,
    pub ready_status: String,
    pub readiness_status: String,
    pub risk: AgentRiskReport,
    pub blockers: Vec<LaneReadinessIssue>,
    pub warnings: Vec<LaneReadinessIssue>,
    pub changed_paths: Vec<FileDiffSummary>,
    pub validation: Vec<LaneTestSummary>,
    pub latest_checkpoint: Option<ChangeId>,
    pub apply_preview: Option<AgentApplyReport>,
    pub apply_error: Option<String>,
    pub receipt_markdown: String,
    pub pr_title: String,
    pub pr_body: String,
    pub next: StatusSuggestion,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentValidationReport {
    pub task: AgentTaskReport,
    pub status: String,
    pub summary: String,
    pub needs_test: bool,
    pub needs_eval: bool,
    pub latest_test: Option<LaneTestSummary>,
    pub latest_eval: Option<LaneTestSummary>,
    pub recent_gates: Vec<LaneTestSummary>,
    pub changed_paths: Vec<FileDiffSummary>,
    pub next: StatusSuggestion,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentDiagnosisReport {
    pub task: AgentTaskReport,
    pub status: String,
    pub severity: String,
    pub summary: String,
    pub likely_issue: String,
    pub evidence: Vec<String>,
    pub ready: bool,
    pub ready_status: String,
    pub readiness_status: String,
    pub risk: AgentRiskReport,
    pub blockers: Vec<LaneReadinessIssue>,
    pub warnings: Vec<LaneReadinessIssue>,
    pub checkpoints: Vec<AgentCheckpointEntry>,
    pub recovery_options: Vec<StatusSuggestion>,
    pub next: StatusSuggestion,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentReviewReport {
    pub task: AgentTaskReport,
    pub summary: String,
    pub readiness_status: String,
    pub ready_to_apply: bool,
    pub risk: AgentRiskReport,
    pub transcript_turns: usize,
    pub tool_events: usize,
    pub latest_checkpoint: Option<ChangeId>,
    pub priorities: Vec<AgentReviewPriority>,
    pub blockers: Vec<LaneReadinessIssue>,
    pub warnings: Vec<LaneReadinessIssue>,
    pub next: StatusSuggestion,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentReviewPriority {
    pub rank: usize,
    pub change: FileDiffSummary,
    pub score: u8,
    pub reasons: Vec<String>,
    pub touched_by: Vec<AgentFileTouch>,
    pub why_command: String,
    pub diff_command: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentFocusReport {
    pub task: AgentTaskReport,
    pub path: String,
    pub source: String,
    pub summary: String,
    pub priority: Option<AgentReviewPriority>,
    pub why: AgentWhyReport,
    pub diff: AgentDiffReport,
    pub next: StatusSuggestion,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentCheckpointReport {
    pub task: AgentTaskReport,
    pub base_change: ChangeId,
    pub head_change: ChangeId,
    pub entries: Vec<AgentCheckpointEntry>,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentCheckpointEntry {
    pub kind: String,
    pub index: usize,
    pub id: String,
    pub label: String,
    pub turn_id: Option<String>,
    pub prompt_preview: Option<String>,
    pub before_change: Option<ChangeId>,
    pub checkpoint: Option<ChangeId>,
    pub before_target: Option<String>,
    pub checkpoint_target: Option<String>,
    pub changed_paths: Vec<FileDiffSummary>,
    pub rewind_before_command: Option<String>,
    pub rewind_checkpoint_command: Option<String>,
    pub diff_command: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentFilesReport {
    pub task: AgentTaskReport,
    pub lane: String,
    pub grouping: String,
    pub files: Vec<AgentFileEntry>,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentFileReport {
    pub task: AgentTaskReport,
    pub lane: String,
    pub path: String,
    pub matched: bool,
    pub summary: String,
    pub change: Option<FileDiffSummary>,
    pub file: Option<AgentFileEntry>,
    pub change_cards: Vec<AgentChangeCard>,
    pub groups: Vec<AgentChangeGroup>,
    pub why: AgentWhyReport,
    pub diff: Option<AgentDiffReport>,
    pub next: StatusSuggestion,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentDeltaReport {
    pub task: AgentTaskReport,
    pub lane: String,
    pub mode: String,
    pub summary: String,
    pub group: Option<AgentChangeGroup>,
    pub file_filter: Option<String>,
    pub matched: bool,
    pub changed_paths: Vec<FileDiffSummary>,
    pub diff: Option<AgentDiffReport>,
    pub next: StatusSuggestion,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentReviewMarker {
    pub event_id: String,
    pub checkpoint: ChangeId,
    pub reviewed_at: i64,
    pub changed_paths: usize,
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentNewReport {
    pub task: AgentTaskReport,
    pub lane: String,
    pub status: String,
    pub summary: String,
    pub reviewed: Option<AgentReviewMarker>,
    pub base_change: ChangeId,
    pub head_change: ChangeId,
    pub new_groups: Vec<AgentChangeGroup>,
    pub file_filter: Option<String>,
    pub matched: bool,
    pub changed_paths: Vec<FileDiffSummary>,
    pub diff: Option<AgentDiffReport>,
    pub next: StatusSuggestion,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentMarkReviewedReport {
    pub task: AgentTaskReport,
    pub lane: String,
    pub marker: AgentReviewMarker,
    pub previous: Option<AgentReviewMarker>,
    pub summary: String,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentFileEntry {
    pub change: FileDiffSummary,
    pub touched_by: Vec<AgentFileTouch>,
    pub why_command: String,
    pub diff_command: Option<String>,
    pub report_command: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentFileTouch {
    pub kind: String,
    pub index: usize,
    pub id: String,
    pub turn_id: Option<String>,
    pub operation_id: Option<ChangeId>,
    pub checkpoint: Option<ChangeId>,
    pub prompt_preview: Option<String>,
    pub diff_command: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentCompareReport {
    pub left: AgentTaskReport,
    pub right: AgentTaskReport,
    pub left_risk: AgentRiskReport,
    pub right_risk: AgentRiskReport,
    pub summary: String,
    pub shared_paths: Vec<AgentComparePath>,
    pub left_only_paths: Vec<FileDiffSummary>,
    pub right_only_paths: Vec<FileDiffSummary>,
    pub recommendation: StatusSuggestion,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentComparePath {
    pub path: String,
    pub left: FileDiffSummary,
    pub right: FileDiffSummary,
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentChangesReport {
    pub task: AgentTaskReport,
    pub lane: String,
    pub grouping: String,
    pub summary: String,
    pub next: StatusSuggestion,
    pub base_change: ChangeId,
    pub head_change: ChangeId,
    pub total_changed_paths: Vec<FileDiffSummary>,
    pub cards: Vec<AgentChangeCard>,
    pub groups: Vec<AgentChangeGroup>,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentChangeSetReport {
    pub task: AgentTaskReport,
    pub lane: String,
    pub selector: String,
    pub summary: String,
    pub card: AgentChangeCard,
    pub groups: Vec<AgentChangeGroup>,
    pub files: Vec<AgentFileEntry>,
    pub diffs: Vec<AgentDiffReport>,
    pub next: StatusSuggestion,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTimelineReport {
    pub task: AgentTaskReport,
    pub lane: String,
    pub mode: String,
    pub summary: String,
    pub base_change: ChangeId,
    pub head_change: ChangeId,
    pub items: Vec<AgentTimelineItem>,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTimelineItem {
    pub kind: String,
    pub index: usize,
    pub id: String,
    pub title: String,
    pub status: Option<String>,
    pub prompt_preview: Option<String>,
    pub assistant_preview: Option<String>,
    pub before_change: Option<ChangeId>,
    pub after_change: Option<ChangeId>,
    pub checkpoint: Option<ChangeId>,
    pub operations: Vec<ChangeId>,
    pub changed_paths: Vec<FileDiffSummary>,
    pub tool_summaries: Vec<String>,
    pub message_count: usize,
    pub event_count: usize,
    pub view_command: Option<String>,
    pub diff_command: Option<String>,
    pub rewind_before_command: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentChangeCard {
    pub rank: usize,
    pub key: String,
    pub title: String,
    pub summary: String,
    pub risk: AgentRiskLevel,
    pub reasons: Vec<String>,
    pub changed_paths: Vec<FileDiffSummary>,
    pub touched_by: Vec<AgentFileTouch>,
    pub operations: Vec<ChangeId>,
    pub tool_summaries: Vec<String>,
    pub review_command: String,
    pub focus_command: Option<String>,
    pub why_command: Option<String>,
    pub diff_command: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentChangeGroup {
    pub kind: String,
    pub index: usize,
    pub id: String,
    pub turn_id: Option<String>,
    pub operation_id: Option<ChangeId>,
    pub operations: Vec<ChangeId>,
    pub before_change: Option<ChangeId>,
    pub after_change: Option<ChangeId>,
    pub checkpoint: Option<ChangeId>,
    pub status: Option<String>,
    pub prompt_preview: Option<String>,
    pub assistant_preview: Option<String>,
    pub tool_summaries: Vec<String>,
    pub changed_paths: Vec<FileDiffSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentWhyReport {
    pub task: AgentTaskReport,
    pub path: String,
    pub matched: bool,
    pub summary: String,
    pub task_change: Option<FileDiffSummary>,
    pub groups: Vec<AgentChangeGroup>,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentTurnReport {
    pub task: AgentTaskReport,
    pub index: usize,
    pub id: String,
    pub turn_id: String,
    pub status: String,
    pub prompt_preview: Option<String>,
    pub assistant_preview: Option<String>,
    pub checkpoint: Option<ChangeId>,
    pub before_change: ChangeId,
    pub after_change: Option<ChangeId>,
    pub changed_paths: Vec<FileDiffSummary>,
    pub tool_summaries: Vec<String>,
    pub messages: Vec<TranscriptMessage>,
    pub event_count: usize,
    pub diff: Option<AgentDiffReport>,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentDiffReport {
    pub task: AgentTaskReport,
    pub target_kind: String,
    pub target: String,
    pub turn_id: Option<String>,
    pub operation_id: Option<ChangeId>,
    pub before_change: ChangeId,
    pub after_change: ChangeId,
    pub file_filter: Option<String>,
    pub diff: DiffSummary,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentStatusReport {
    pub status: AgentTaskStatus,
    pub latest: Option<AgentTaskReport>,
    pub risk: Option<AgentRiskReport>,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentNextReport {
    pub status: AgentTaskStatus,
    pub task: Option<AgentTaskReport>,
    pub focus: String,
    pub summary: String,
    pub primary: StatusSuggestion,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentAskReport {
    pub selector: String,
    pub question: String,
    pub intent: String,
    pub tool: String,
    pub read_only: bool,
    pub routed_command: String,
    pub report: serde_json::Value,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentBriefReport {
    pub task: AgentTaskReport,
    pub next: AgentNextReport,
    pub risk: AgentRiskReport,
    pub ready_to_apply: bool,
    pub readiness_status: String,
    pub blockers: Vec<LaneReadinessIssue>,
    pub warnings: Vec<LaneReadinessIssue>,
    pub changed_paths: Vec<FileDiffSummary>,
    pub groups: Vec<AgentChangeGroup>,
    pub latest_change_diff: Option<DiffSummary>,
    pub tool_summaries: Vec<String>,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentGitApplyPlan {
    pub crab_branch: String,
    pub git_branch: Option<String>,
    pub base_change: ChangeId,
    pub result_change: Option<ChangeId>,
    pub range: Option<String>,
    pub would_record: bool,
    pub would_create_git_commit: bool,
    pub would_fast_forward: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentApplyReport {
    pub task: AgentTaskReport,
    pub status: String,
    pub dry_run: bool,
    pub git_apply_plan: AgentGitApplyPlan,
    pub recorded: Option<LaneRecordReport>,
    pub merge: Option<MergeReport>,
    pub git_export: Option<GitExportReport>,
    pub fast_forwarded: bool,
    pub warnings: Vec<String>,
    pub suggestions: Vec<StatusSuggestion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentSetupReport {
    pub provider: String,
    pub editor: String,
    pub command: Vec<String>,
    pub snippet: String,
    pub detected: bool,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRunReport {
    pub task: AgentTaskReport,
    pub provider: String,
    pub command: Vec<String>,
    pub workdir: Option<String>,
    pub exit_code: Option<i32>,
    pub recorded: Option<LaneRecordReport>,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneTurn {
    pub turn_id: String,
    pub lane_id: String,
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
pub struct LaneTurnStartReport {
    pub turn: LaneTurn,
    pub session: LaneSession,
    pub base_root: ObjectId,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneTurnDetails {
    pub turn: LaneTurn,
    pub session: Option<LaneSession>,
    pub messages: Vec<Message>,
    pub events: Vec<LaneEventRecord>,
    pub operations: Vec<TimelineEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneTurnEventReport {
    pub event: LaneEventRecord,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneTurnEndReport {
    pub turn: LaneTurn,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneEventRecord {
    pub event_id: String,
    pub lane_id: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub event_type: String,
    pub change_id: Option<ChangeId>,
    pub message_id: Option<MessageId>,
    pub payload: Option<serde_json::Value>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneTraceSpan {
    pub span_id: String,
    pub trace_id: String,
    pub lane_id: String,
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
pub struct LaneTraceSummaryReport {
    pub lane_id: Option<String>,
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
    pub slowest_spans: Vec<LaneTraceSpan>,
    pub open_spans: Vec<LaneTraceSpan>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NamedCount {
    pub name: String,
    pub count: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneTraceSpanStartReport {
    pub span: LaneTraceSpan,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneTraceSpanEndReport {
    pub span: LaneTraceSpan,
}
