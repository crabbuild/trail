#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneRecord {
    pub lane_id: String,
    pub name: String,
    pub kind: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub created_at: i64,
    pub metadata_json: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneBranch {
    pub lane_id: String,
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
pub struct LaneDetails {
    pub record: LaneRecord,
    pub branch: LaneBranch,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneStatusReport {
    pub lane: LaneDetails,
    pub changed_paths: Vec<FileDiffSummary>,
    pub queued_merges: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_status: Option<LaneBaseStatus>,
    pub workdir_state: Option<WorktreeState>,
    pub workdir_changed_paths: Vec<FileDiffSummary>,
    pub latest_test: Option<LaneTestSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_eval: Option<LaneTestSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneBaseStatus {
    pub target_branch: String,
    pub target_ref: String,
    pub target_change: ChangeId,
    pub lane_base_change: ChangeId,
    pub operations_behind: Option<u64>,
    pub stale: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneContributionReport {
    pub status: LaneStatusReport,
    pub operations: Vec<TimelineEntry>,
    pub sessions: Vec<LaneSession>,
    pub recent_events: Vec<LaneEventRecord>,
    pub approvals: Vec<LaneApproval>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneReviewEvidenceSummary {
    pub operations: usize,
    pub sessions: usize,
    pub events: usize,
    pub spans: usize,
    pub approvals: usize,
    pub pending_approvals: usize,
    pub conflicts: usize,
    pub queued_merges: u64,
    pub gates: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneReviewPacketReport {
    pub lane: LaneDetails,
    pub readiness: LaneReadinessReport,
    pub changed_paths: Vec<FileDiffSummary>,
    pub workdir_state: Option<WorktreeState>,
    pub evidence_summary: LaneReviewEvidenceSummary,
    pub latest_test: Option<LaneTestSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_eval: Option<LaneTestSummary>,
    pub recent_gates: Vec<LaneTestSummary>,
    pub recent_operations: Vec<TimelineEntry>,
    pub recent_sessions: Vec<LaneSession>,
    pub recent_events: Vec<LaneEventRecord>,
    pub recent_spans: Vec<LaneTraceSpan>,
    pub recent_approvals: Vec<LaneApproval>,
    pub conflicts: Vec<ConflictSetSummary>,
    pub next_steps: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneReadinessReport {
    pub lane: LaneDetails,
    pub ready: bool,
    pub status: String,
    pub blockers: Vec<LaneReadinessIssue>,
    pub warnings: Vec<LaneReadinessIssue>,
    pub changed_paths: Vec<FileDiffSummary>,
    pub workdir_state: Option<WorktreeState>,
    pub workdir_changed_paths: Vec<FileDiffSummary>,
    pub queued_merges: u64,
    pub pending_approvals: Vec<LaneApproval>,
    pub conflicts: Vec<ConflictSetSummary>,
    pub latest_test: Option<LaneTestSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_eval: Option<LaneTestSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneReadinessIssue {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneHandoffReport {
    pub lane: LaneDetails,
    pub readiness: LaneReadinessReport,
    pub current_session: Option<LaneSessionDetails>,
    pub recent_sessions: Vec<LaneSession>,
    pub recent_events: Vec<LaneEventRecord>,
    pub recent_spans: Vec<LaneTraceSpan>,
    pub recent_operations: Vec<TimelineEntry>,
    pub next_steps: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneMessageReport {
    pub lane_id: String,
    pub message_id: MessageId,
    pub role: String,
    pub session_id: Option<String>,
}
