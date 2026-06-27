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
