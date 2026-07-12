use std::fmt;

/// Stable schema identifier for adapter-neutral agent lifecycle events.
pub const AGENT_LIFECYCLE_EVENT_SCHEMA: &str = "trail.agent_lifecycle_event";
/// Current normalized lifecycle event version.
pub const AGENT_LIFECYCLE_EVENT_VERSION: u16 = 1;
/// Maximum serialized provider payload accepted at a mutation boundary.
pub const AGENT_LIFECYCLE_MAX_PAYLOAD_BYTES: usize = 1024 * 1024;

/// Transport that supplied evidence to Trail's shared capture coordinator.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentCaptureTransport {
    Acp,
    NativeHooks,
    Terminal,
    Hybrid,
}

/// How directly an evidence field was observed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentEvidenceConfidence {
    ProtocolStructured,
    NativeStructured,
    NativeTranscript,
    CanonicalExport,
    WorktreeObserved,
    Heuristic,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentEvidenceSource {
    Acp,
    NativeHook,
    NativeTranscript,
    CanonicalExport,
    WorkdirObserved,
    Reconstructed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentEvidenceFactKind {
    SessionLifecycle,
    TurnBoundary,
    UserMessage,
    AssistantMessage,
    ToolCall,
    Approval,
    Usage,
    WorkspaceChange,
    TranscriptLocator,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AgentEvidenceFact {
    pub kind: AgentEvidenceFactKind,
    pub key: String,
    pub value: serde_json::Value,
    pub source: AgentEvidenceSource,
    pub confidence: AgentEvidenceConfidence,
    pub observed_at: Option<i64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentEvidenceMergeDecision {
    Insert,
    Enrich,
    IgnoreDuplicate,
    PreserveHigherPrecedence,
    PreserveConflict,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AgentEvidenceMergeResult {
    pub fact: AgentEvidenceFact,
    pub decision: AgentEvidenceMergeDecision,
    pub conflict: Option<AgentEvidenceFact>,
}

/// Merge the same factual identity monotonically; lower-precedence evidence can never
/// overwrite stronger evidence, and equal-precedence conflicts remain explicit.
pub fn merge_agent_evidence_fact(
    existing: Option<&AgentEvidenceFact>,
    incoming: AgentEvidenceFact,
) -> crate::Result<AgentEvidenceMergeResult> {
    validate_agent_capture_token("evidence fact key", &incoming.key, 512)?;
    if incoming.observed_at.is_some_and(|value| value < 0) {
        return Err(crate::Error::InvalidInput(
            "agent evidence observed_at must be non-negative".to_string(),
        ));
    }
    let Some(existing) = existing else {
        return Ok(AgentEvidenceMergeResult {
            fact: incoming,
            decision: AgentEvidenceMergeDecision::Insert,
            conflict: None,
        });
    };
    if existing.kind != incoming.kind || existing.key != incoming.key {
        return Err(crate::Error::InvalidInput(
            "agent evidence merge requires the same fact kind and key".to_string(),
        ));
    }
    if existing.value == incoming.value {
        let incoming_rank = agent_evidence_precedence(incoming.kind, incoming.source);
        let existing_rank = agent_evidence_precedence(existing.kind, existing.source);
        return Ok(if incoming_rank > existing_rank {
            AgentEvidenceMergeResult {
                fact: incoming,
                decision: AgentEvidenceMergeDecision::Enrich,
                conflict: None,
            }
        } else {
            AgentEvidenceMergeResult {
                fact: existing.clone(),
                decision: AgentEvidenceMergeDecision::IgnoreDuplicate,
                conflict: None,
            }
        });
    }
    let incoming_rank = agent_evidence_precedence(incoming.kind, incoming.source);
    let existing_rank = agent_evidence_precedence(existing.kind, existing.source);
    if incoming_rank > existing_rank {
        Ok(AgentEvidenceMergeResult {
            fact: incoming,
            decision: AgentEvidenceMergeDecision::Enrich,
            conflict: Some(existing.clone()),
        })
    } else if incoming_rank < existing_rank {
        Ok(AgentEvidenceMergeResult {
            fact: existing.clone(),
            decision: AgentEvidenceMergeDecision::PreserveHigherPrecedence,
            conflict: Some(incoming),
        })
    } else {
        Ok(AgentEvidenceMergeResult {
            fact: existing.clone(),
            decision: AgentEvidenceMergeDecision::PreserveConflict,
            conflict: Some(incoming),
        })
    }
}

pub fn agent_evidence_precedence(kind: AgentEvidenceFactKind, source: AgentEvidenceSource) -> u8 {
    use AgentEvidenceFactKind as Kind;
    use AgentEvidenceSource as Source;
    match (kind, source) {
        (Kind::WorkspaceChange, Source::WorkdirObserved) => 100,
        (Kind::WorkspaceChange, Source::CanonicalExport) => 80,
        (Kind::UserMessage | Kind::AssistantMessage, Source::NativeTranscript) => 100,
        (Kind::UserMessage | Kind::AssistantMessage, Source::CanonicalExport) => 95,
        (Kind::UserMessage | Kind::AssistantMessage, Source::Acp) => 90,
        (Kind::ToolCall | Kind::Approval, Source::Acp) => 100,
        (Kind::ToolCall | Kind::Approval, Source::NativeHook) => 90,
        (Kind::Usage, Source::CanonicalExport) => 100,
        (Kind::Usage, Source::Acp | Source::NativeHook) => 90,
        (Kind::SessionLifecycle | Kind::TurnBoundary, Source::NativeHook) => 100,
        (Kind::SessionLifecycle | Kind::TurnBoundary, Source::Acp) => 90,
        (Kind::TranscriptLocator, Source::NativeHook) => 100,
        (_, Source::CanonicalExport) => 85,
        (_, Source::NativeTranscript) => 80,
        (_, Source::Acp) => 75,
        (_, Source::NativeHook) => 70,
        (_, Source::WorkdirObserved) => 65,
        (_, Source::Reconstructed) => 10,
    }
}

/// Provider-native identifiers carried without assigning them Trail semantics.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentNativeEventIdentity {
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub message_id: Option<String>,
    pub tool_id: Option<String>,
    pub subagent_id: Option<String>,
    pub event_name: String,
    pub sequence: Option<u64>,
}

/// Trace and causal identifiers supplied by a provider or allocated by Trail.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentEventCorrelation {
    pub parent_event_id: Option<String>,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub parent_span_id: Option<String>,
}

/// Durable receipt and artifact provenance for one normalized event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentEventEvidence {
    pub receipt_id: String,
    pub raw_digest: Option<String>,
    pub transcript_offset: Option<u64>,
    pub confidence: AgentEvidenceConfidence,
}

/// A forward-compatible event type encoded exactly as the public wire string.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct AgentLifecycleEventType(pub String);

impl AgentLifecycleEventType {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Classify a wire event for the pure lifecycle state machine.
    ///
    /// Unrecognized values deliberately remain inert evidence. Adapters may retain
    /// `provider.<provider>.<event>` values without granting them lifecycle authority.
    pub fn kind(&self) -> AgentLifecycleEventKind {
        match self.0.as_str() {
            "session.started" => AgentLifecycleEventKind::SessionStarted,
            "session.resumed" => AgentLifecycleEventKind::SessionResumed,
            "session.updated" => AgentLifecycleEventKind::SessionUpdated,
            "session.ended" => AgentLifecycleEventKind::SessionEnded,
            "turn.started" => AgentLifecycleEventKind::TurnStarted,
            "turn.completed" => AgentLifecycleEventKind::TurnCompleted,
            "turn.failed" => AgentLifecycleEventKind::TurnFailed,
            "turn.cancelled" => AgentLifecycleEventKind::TurnCancelled,
            "message.user" => AgentLifecycleEventKind::MessageUser,
            "message.assistant.delta" => AgentLifecycleEventKind::MessageAssistantDelta,
            "message.assistant.completed" => AgentLifecycleEventKind::MessageAssistantCompleted,
            "plan.updated" => AgentLifecycleEventKind::PlanUpdated,
            "tool.started" => AgentLifecycleEventKind::ToolStarted,
            "tool.completed" => AgentLifecycleEventKind::ToolCompleted,
            "tool.failed" => AgentLifecycleEventKind::ToolFailed,
            "approval.requested" => AgentLifecycleEventKind::ApprovalRequested,
            "approval.decided" => AgentLifecycleEventKind::ApprovalDecided,
            "subagent.started" => AgentLifecycleEventKind::SubagentStarted,
            "subagent.completed" => AgentLifecycleEventKind::SubagentCompleted,
            "subagent.failed" => AgentLifecycleEventKind::SubagentFailed,
            "compaction.started" => AgentLifecycleEventKind::CompactionStarted,
            "compaction.completed" => AgentLifecycleEventKind::CompactionCompleted,
            "usage.updated" => AgentLifecycleEventKind::UsageUpdated,
            "model.updated" => AgentLifecycleEventKind::ModelUpdated,
            "workspace.diff" => AgentLifecycleEventKind::WorkspaceDiff,
            "workspace.file_changed" => AgentLifecycleEventKind::WorkspaceFileChanged,
            "workspace.checkpoint" => AgentLifecycleEventKind::WorkspaceCheckpoint,
            "context.injected" => AgentLifecycleEventKind::ContextInjected,
            "diagnostic" => AgentLifecycleEventKind::Diagnostic,
            _ => AgentLifecycleEventKind::Unknown,
        }
    }
}

impl From<AgentLifecycleEventKind> for AgentLifecycleEventType {
    fn from(value: AgentLifecycleEventKind) -> Self {
        Self(value.wire_name().to_string())
    }
}

/// Adapter-neutral lifecycle evidence accepted by ACP and native hook transports.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AgentLifecycleEvent {
    pub schema: String,
    pub version: u16,
    pub event_id: String,
    pub event_type: AgentLifecycleEventType,
    pub occurred_at: Option<i64>,
    pub received_at: i64,
    pub provider: String,
    pub provider_version: Option<String>,
    pub transport: AgentCaptureTransport,
    pub workspace_id: String,
    pub lane_id: Option<String>,
    pub capture_run_id: Option<String>,
    pub native: AgentNativeEventIdentity,
    pub correlation: AgentEventCorrelation,
    pub payload: serde_json::Value,
    pub evidence: AgentEventEvidence,
}

/// Untrusted, provider-native receipt accepted by durable capture ingress.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AgentHookReceiptInput {
    pub installation_id: Option<String>,
    pub provider: String,
    pub native_event: String,
    pub native_session_id: Option<String>,
    pub native_turn_id: Option<String>,
    pub transport: AgentCaptureTransport,
    pub dedupe_key: String,
    pub payload: serde_json::Value,
    pub occurred_at: Option<i64>,
}

/// Durable receipt journal row. Large raw data is held by `raw_object_id`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentHookReceipt {
    pub receipt_id: String,
    pub workspace_id: String,
    pub installation_id: Option<String>,
    pub mapping_id: Option<String>,
    pub provider: String,
    pub native_event: String,
    pub native_session_id: Option<String>,
    pub native_turn_id: Option<String>,
    pub transport: AgentCaptureTransport,
    pub dedupe_key: String,
    pub payload_digest: String,
    pub raw_object_id: ObjectId,
    pub raw_artifact_id: Option<String>,
    pub receive_sequence: Option<u64>,
    pub status: String,
    pub attempt_count: u32,
    pub next_attempt_at: Option<i64>,
    pub diagnostic: Option<String>,
    pub occurred_at: Option<i64>,
    pub received_at: i64,
    pub processed_at: Option<i64>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentHookReceiptIngestReport {
    pub receipt: AgentHookReceipt,
    pub duplicate: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AgentHookReplayReport {
    pub receipt: AgentHookReceipt,
    pub mapping: Option<LaneAgentSession>,
    pub normalized_events: Vec<AgentLifecycleEvent>,
    pub actions: Vec<AgentCaptureAction>,
    pub diagnostics: Vec<AgentTransitionDiagnostic>,
    pub replayed: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentHookReplayFailure {
    pub receipt_id: String,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AgentHookReplayBatchReport {
    pub recovered_stale_receipts: usize,
    pub replayed: Vec<AgentHookReplayReport>,
    pub failures: Vec<AgentHookReplayFailure>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentCaptureRecoveryReport {
    pub expired_run_ids: Vec<String>,
    pub interrupted_mapping_ids: Vec<String>,
    pub interrupted_turn_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct LaneArtifactInput {
    pub lane: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub provider: String,
    pub artifact_kind: String,
    pub format: String,
    pub source: AgentEvidenceSource,
    pub source_locator_redacted: Option<String>,
    pub content: Vec<u8>,
    pub start_offset: Option<u64>,
    pub end_offset: Option<u64>,
    pub redaction_profile: Option<String>,
    pub trust: String,
    pub supersedes_artifact_id: Option<String>,
    pub metadata_json: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LaneArtifact {
    pub artifact_id: String,
    pub workspace_id: String,
    pub lane_id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub provider: String,
    pub artifact_kind: String,
    pub format: String,
    pub source: AgentEvidenceSource,
    pub source_locator_redacted: Option<String>,
    pub content_object_id: Option<ObjectId>,
    pub content_digest: String,
    pub size_bytes: u64,
    pub start_offset: Option<u64>,
    pub end_offset: Option<u64>,
    pub redaction_profile: Option<String>,
    pub retention_status: String,
    pub trust: String,
    pub supersedes_artifact_id: Option<String>,
    pub created_at: i64,
    pub metadata_json: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LaneAgentSessionInput {
    pub provider: String,
    pub native_session_id: String,
    pub parent_native_session_id: Option<String>,
    pub lane: String,
    pub trail_session_id: String,
    pub capture_run_id: Option<String>,
    pub primary_transport: AgentCaptureTransport,
    pub transcript_identity: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LaneAgentSession {
    pub mapping_id: String,
    pub workspace_id: String,
    pub provider: String,
    pub native_session_id: String,
    pub parent_native_session_id: Option<String>,
    pub trail_session_id: String,
    pub lane_id: String,
    pub capture_run_id: Option<String>,
    pub primary_transport: AgentCaptureTransport,
    pub transcript_identity: Option<String>,
    pub transcript_offset: Option<u64>,
    pub resume_json: Option<String>,
    pub last_attestation_id: Option<String>,
    pub status: AgentCapturePhase,
    pub pending_turn_outcome: Option<AgentTurnOutcome>,
    pub session_close_requested: bool,
    pub capture_epoch: u64,
    pub finalization_owner: Option<String>,
    pub finalization_lease_expires_at: Option<i64>,
    pub next_receive_sequence: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentFinalizationLeaseReport {
    pub mapping: LaneAgentSession,
    pub acquired: bool,
    pub owner: String,
    pub expires_at: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentCaptureRunInput {
    pub lane: Option<String>,
    pub workdir: String,
    pub owner_agent: String,
    pub owner_session_id: String,
    pub executor_agent: Option<String>,
    pub work_item_id: Option<String>,
    pub lease_ms: u64,
    pub metadata_json: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentCaptureRun {
    pub capture_run_id: String,
    pub workspace_id: String,
    pub lane_id: Option<String>,
    pub workdir: String,
    pub canonical_workdir: String,
    pub owner_agent: String,
    pub owner_session_id: String,
    pub executor_agent: Option<String>,
    pub work_item_id: Option<String>,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub expires_at: i64,
    pub ended_at: Option<i64>,
    pub metadata_json: Option<String>,
}

impl AgentLifecycleEvent {
    /// Validate untrusted adapter output before it enters durable semantic storage.
    pub fn validate(&self) -> crate::Result<()> {
        if self.schema != AGENT_LIFECYCLE_EVENT_SCHEMA {
            return Err(crate::Error::InvalidInput(format!(
                "unsupported agent lifecycle schema `{}`",
                self.schema
            )));
        }
        if self.version != AGENT_LIFECYCLE_EVENT_VERSION {
            return Err(crate::Error::InvalidInput(format!(
                "unsupported agent lifecycle event version {}; supported version is {}",
                self.version, AGENT_LIFECYCLE_EVENT_VERSION
            )));
        }
        validate_agent_capture_token("event_id", &self.event_id, 256)?;
        validate_agent_capture_token("event_type", self.event_type.as_str(), 128)?;
        validate_agent_provider_name(&self.provider)?;
        validate_agent_capture_token("workspace_id", &self.workspace_id, 256)?;
        validate_agent_capture_token("receipt_id", &self.evidence.receipt_id, 256)?;
        validate_optional_agent_capture_token("lane_id", self.lane_id.as_deref(), 256)?;
        validate_optional_agent_capture_token(
            "capture_run_id",
            self.capture_run_id.as_deref(),
            256,
        )?;
        validate_optional_agent_capture_token(
            "native.session_id",
            self.native.session_id.as_deref(),
            1024,
        )?;
        validate_optional_agent_capture_token(
            "native.turn_id",
            self.native.turn_id.as_deref(),
            1024,
        )?;
        validate_optional_agent_capture_token(
            "native.message_id",
            self.native.message_id.as_deref(),
            1024,
        )?;
        validate_optional_agent_capture_token(
            "native.tool_id",
            self.native.tool_id.as_deref(),
            1024,
        )?;
        validate_optional_agent_capture_token(
            "native.subagent_id",
            self.native.subagent_id.as_deref(),
            1024,
        )?;
        validate_agent_capture_token("native.event_name", &self.native.event_name, 256)?;
        validate_optional_agent_capture_token(
            "correlation.parent_event_id",
            self.correlation.parent_event_id.as_deref(),
            256,
        )?;
        validate_optional_agent_capture_token(
            "correlation.trace_id",
            self.correlation.trace_id.as_deref(),
            256,
        )?;
        validate_optional_agent_capture_token(
            "correlation.span_id",
            self.correlation.span_id.as_deref(),
            256,
        )?;
        validate_optional_agent_capture_token(
            "correlation.parent_span_id",
            self.correlation.parent_span_id.as_deref(),
            256,
        )?;
        if self.received_at < 0 || self.occurred_at.is_some_and(|value| value < 0) {
            return Err(crate::Error::InvalidInput(
                "agent lifecycle timestamps must be non-negative milliseconds".to_string(),
            ));
        }
        if self.event_type.kind() == AgentLifecycleEventKind::Unknown
            && !self
                .event_type
                .as_str()
                .starts_with(&format!("provider.{}.", self.provider))
        {
            return Err(crate::Error::InvalidInput(format!(
                "unknown lifecycle event `{}` must use the `provider.{}.` namespace",
                self.event_type, self.provider
            )));
        }
        let payload_bytes = serde_json::to_vec(&self.payload)?;
        if payload_bytes.len() > AGENT_LIFECYCLE_MAX_PAYLOAD_BYTES {
            return Err(crate::Error::InvalidInput(format!(
                "agent lifecycle payload is {} bytes; maximum is {}",
                payload_bytes.len(),
                AGENT_LIFECYCLE_MAX_PAYLOAD_BYTES
            )));
        }
        Ok(())
    }
}

fn validate_agent_provider_name(value: &str) -> crate::Result<()> {
    validate_agent_capture_token("provider", value, 64)?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(crate::Error::InvalidInput(
            "agent provider must use lowercase ASCII letters, digits, and hyphens".to_string(),
        ));
    }
    Ok(())
}

fn validate_optional_agent_capture_token(
    field: &str,
    value: Option<&str>,
    max_len: usize,
) -> crate::Result<()> {
    if let Some(value) = value {
        validate_agent_capture_token(field, value, max_len)?;
    }
    Ok(())
}

fn validate_agent_capture_token(field: &str, value: &str, max_len: usize) -> crate::Result<()> {
    if value.is_empty() {
        return Err(crate::Error::InvalidInput(format!(
            "agent lifecycle field `{field}` must not be empty"
        )));
    }
    if value.len() > max_len {
        return Err(crate::Error::InvalidInput(format!(
            "agent lifecycle field `{field}` exceeds {max_len} bytes"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(crate::Error::InvalidInput(format!(
            "agent lifecycle field `{field}` contains control characters"
        )));
    }
    Ok(())
}

/// Persisted lifecycle phase for one provider-native session mapping.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCapturePhase {
    Idle,
    Active,
    Finalizing,
    Ended,
    Interrupted,
}

/// Closed-turn outcome chosen by the pure transition function.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTurnOutcome {
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

/// Exhaustive semantic vocabulary consumed by the lifecycle transition function.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLifecycleEventKind {
    SessionStarted,
    SessionResumed,
    SessionUpdated,
    SessionEnded,
    TurnStarted,
    TurnCompleted,
    TurnFailed,
    TurnCancelled,
    MessageUser,
    MessageAssistantDelta,
    MessageAssistantCompleted,
    PlanUpdated,
    ToolStarted,
    ToolCompleted,
    ToolFailed,
    ApprovalRequested,
    ApprovalDecided,
    SubagentStarted,
    SubagentCompleted,
    SubagentFailed,
    CompactionStarted,
    CompactionCompleted,
    UsageUpdated,
    ModelUpdated,
    WorkspaceDiff,
    WorkspaceFileChanged,
    WorkspaceCheckpoint,
    ContextInjected,
    Diagnostic,
    Unknown,
    /// Internal durable signal emitted after all finalization actions commit.
    FinalizationCompleted,
    /// Internal recovery signal emitted when finalization cannot be completed.
    FinalizationInterrupted,
}

impl AgentLifecycleEventKind {
    pub const ALL: [Self; 32] = [
        Self::SessionStarted,
        Self::SessionResumed,
        Self::SessionUpdated,
        Self::SessionEnded,
        Self::TurnStarted,
        Self::TurnCompleted,
        Self::TurnFailed,
        Self::TurnCancelled,
        Self::MessageUser,
        Self::MessageAssistantDelta,
        Self::MessageAssistantCompleted,
        Self::PlanUpdated,
        Self::ToolStarted,
        Self::ToolCompleted,
        Self::ToolFailed,
        Self::ApprovalRequested,
        Self::ApprovalDecided,
        Self::SubagentStarted,
        Self::SubagentCompleted,
        Self::SubagentFailed,
        Self::CompactionStarted,
        Self::CompactionCompleted,
        Self::UsageUpdated,
        Self::ModelUpdated,
        Self::WorkspaceDiff,
        Self::WorkspaceFileChanged,
        Self::WorkspaceCheckpoint,
        Self::ContextInjected,
        Self::Diagnostic,
        Self::Unknown,
        Self::FinalizationCompleted,
        Self::FinalizationInterrupted,
    ];

    pub fn wire_name(self) -> &'static str {
        match self {
            Self::SessionStarted => "session.started",
            Self::SessionResumed => "session.resumed",
            Self::SessionUpdated => "session.updated",
            Self::SessionEnded => "session.ended",
            Self::TurnStarted => "turn.started",
            Self::TurnCompleted => "turn.completed",
            Self::TurnFailed => "turn.failed",
            Self::TurnCancelled => "turn.cancelled",
            Self::MessageUser => "message.user",
            Self::MessageAssistantDelta => "message.assistant.delta",
            Self::MessageAssistantCompleted => "message.assistant.completed",
            Self::PlanUpdated => "plan.updated",
            Self::ToolStarted => "tool.started",
            Self::ToolCompleted => "tool.completed",
            Self::ToolFailed => "tool.failed",
            Self::ApprovalRequested => "approval.requested",
            Self::ApprovalDecided => "approval.decided",
            Self::SubagentStarted => "subagent.started",
            Self::SubagentCompleted => "subagent.completed",
            Self::SubagentFailed => "subagent.failed",
            Self::CompactionStarted => "compaction.started",
            Self::CompactionCompleted => "compaction.completed",
            Self::UsageUpdated => "usage.updated",
            Self::ModelUpdated => "model.updated",
            Self::WorkspaceDiff => "workspace.diff",
            Self::WorkspaceFileChanged => "workspace.file_changed",
            Self::WorkspaceCheckpoint => "workspace.checkpoint",
            Self::ContextInjected => "context.injected",
            Self::Diagnostic => "diagnostic",
            Self::Unknown => "provider.unknown.unknown",
            Self::FinalizationCompleted => "trail.finalization.completed",
            Self::FinalizationInterrupted => "trail.finalization.interrupted",
        }
    }

    pub(crate) fn terminal_outcome(self) -> Option<AgentTurnOutcome> {
        match self {
            Self::TurnCompleted => Some(AgentTurnOutcome::Completed),
            Self::TurnFailed => Some(AgentTurnOutcome::Failed),
            Self::TurnCancelled => Some(AgentTurnOutcome::Cancelled),
            _ => None,
        }
    }

    fn starts_or_implies_turn(self) -> bool {
        matches!(
            self,
            Self::TurnStarted
                | Self::MessageUser
                | Self::MessageAssistantDelta
                | Self::PlanUpdated
                | Self::ToolStarted
                | Self::ApprovalRequested
                | Self::SubagentStarted
                | Self::CompactionStarted
        )
    }

    fn completes_existing_turn_activity(self) -> bool {
        matches!(
            self,
            Self::MessageAssistantCompleted
                | Self::ToolCompleted
                | Self::ToolFailed
                | Self::ApprovalDecided
                | Self::SubagentCompleted
                | Self::SubagentFailed
                | Self::CompactionCompleted
                | Self::WorkspaceDiff
                | Self::WorkspaceFileChanged
                | Self::WorkspaceCheckpoint
        )
    }
}

/// Bounded persisted facts used by the pure state transition.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentTransitionContext {
    pub has_session: bool,
    pub has_open_turn: bool,
    pub duplicate: bool,
    pub has_new_evidence: bool,
    pub session_close_requested: bool,
    /// Outcome stored when the session entered `finalizing`.
    pub pending_turn_outcome: Option<AgentTurnOutcome>,
}

/// Ordered, idempotently keyed side effects selected by the pure transition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "action")]
pub enum AgentCaptureAction {
    EnsureSession,
    CreateCaptureEpoch,
    BeginTurn { synthetic: bool },
    CaptureBaseline,
    AppendEvidence,
    StartRootSpan,
    StartToolSpan { synthetic: bool },
    EndToolSpan,
    StartSubagentSpan { synthetic: bool },
    EndSubagentSpan,
    StartCompactionSpan,
    EndCompactionSpan,
    RequestTurnFinalization { outcome: AgentTurnOutcome },
    ReconcileWorkdir,
    ImportTranscript,
    CloseTurn { outcome: AgentTurnOutcome },
    RequestSessionClose,
    FinalizeAttestation,
    CloseSession { interrupted: bool },
    WarnDuplicate,
    RecoverInterruptedTurn,
    DeferUntilFinalized,
}

/// Stable diagnostic produced by lifecycle decisions, not by side effects.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTransitionDiagnostic {
    DuplicateEvent,
    SyntheticTurnStart,
    PriorTurnInterrupted,
    EventDeferredDuringFinalization,
    LateEventAfterSessionClose,
    FinalizationInterrupted,
}

/// Complete pure decision for one lifecycle event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentTransitionResult {
    pub new_phase: AgentCapturePhase,
    pub actions: Vec<AgentCaptureAction>,
    pub diagnostics: Vec<AgentTransitionDiagnostic>,
}

/// Side-effect boundary used by every lifecycle transport after the pure decision step.
pub trait AgentCaptureActionExecutor {
    fn execute_agent_capture_actions(
        &mut self,
        event: &AgentLifecycleEvent,
        actions: &[AgentCaptureAction],
    ) -> crate::Result<()>;
}

/// Adapter-neutral coordinator entrypoint. Adapters emit events; executors own I/O.
#[derive(Clone, Copy, Debug, Default)]
pub struct AgentCaptureCoordinator;

impl AgentCaptureCoordinator {
    pub fn decide(
        phase: AgentCapturePhase,
        event: AgentLifecycleEventKind,
        context: AgentTransitionContext,
    ) -> AgentTransitionResult {
        transition_agent_capture(phase, event, context)
    }

    pub fn dispatch<E: AgentCaptureActionExecutor>(
        executor: &mut E,
        phase: AgentCapturePhase,
        event: &AgentLifecycleEvent,
        context: AgentTransitionContext,
    ) -> crate::Result<AgentTransitionResult> {
        let result = Self::decide(phase, event.event_type.kind(), context);
        executor.execute_agent_capture_actions(event, &result.actions)?;
        Ok(result)
    }
}

impl AgentTransitionResult {
    fn unchanged(phase: AgentCapturePhase) -> Self {
        Self {
            new_phase: phase,
            actions: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn append(mut self, action: AgentCaptureAction) -> Self {
        self.actions.push(action);
        self
    }

    fn diagnose(mut self, diagnostic: AgentTransitionDiagnostic) -> Self {
        self.diagnostics.push(diagnostic);
        self
    }
}

/// Choose lifecycle mutations without performing database or filesystem I/O.
pub fn transition_agent_capture(
    phase: AgentCapturePhase,
    event: AgentLifecycleEventKind,
    context: AgentTransitionContext,
) -> AgentTransitionResult {
    if context.duplicate {
        let mut result = AgentTransitionResult::unchanged(phase);
        if context.has_new_evidence {
            result.actions.push(AgentCaptureAction::AppendEvidence);
        }
        result.actions.push(AgentCaptureAction::WarnDuplicate);
        result
            .diagnostics
            .push(AgentTransitionDiagnostic::DuplicateEvent);
        return result;
    }

    if event == AgentLifecycleEventKind::Unknown {
        return AgentTransitionResult::unchanged(phase).append(AgentCaptureAction::AppendEvidence);
    }

    match phase {
        AgentCapturePhase::Idle => transition_idle(event, context),
        AgentCapturePhase::Active => transition_active(event),
        AgentCapturePhase::Finalizing => transition_finalizing(event, context),
        AgentCapturePhase::Ended | AgentCapturePhase::Interrupted => {
            transition_closed(phase, event)
        }
    }
}

fn transition_idle(
    event: AgentLifecycleEventKind,
    context: AgentTransitionContext,
) -> AgentTransitionResult {
    if matches!(
        event,
        AgentLifecycleEventKind::SessionStarted
            | AgentLifecycleEventKind::SessionResumed
            | AgentLifecycleEventKind::SessionUpdated
    ) {
        let mut result = AgentTransitionResult::unchanged(AgentCapturePhase::Idle);
        if !context.has_session {
            result.actions.push(AgentCaptureAction::EnsureSession);
        }
        result.actions.push(AgentCaptureAction::AppendEvidence);
        return result;
    }

    if event == AgentLifecycleEventKind::SessionEnded {
        return AgentTransitionResult {
            new_phase: AgentCapturePhase::Ended,
            actions: vec![
                AgentCaptureAction::EnsureSession,
                AgentCaptureAction::AppendEvidence,
                AgentCaptureAction::ImportTranscript,
                AgentCaptureAction::FinalizeAttestation,
                AgentCaptureAction::CloseSession { interrupted: false },
            ],
            diagnostics: Vec::new(),
        };
    }

    if let Some(outcome) = event.terminal_outcome() {
        return begin_synthetic_finalization(outcome);
    }

    // Providers may run hooks concurrently, so a tool/subagent/completion callback can
    // arrive after Stop finalized the turn. Keep the evidence, but never manufacture a
    // new active turn from a completion-only event. A corresponding start event still
    // recovers a missing turn and later completion callbacks close its spans normally.
    if event.completes_existing_turn_activity() {
        return AgentTransitionResult::unchanged(AgentCapturePhase::Idle)
            .append(AgentCaptureAction::EnsureSession)
            .append(AgentCaptureAction::AppendEvidence);
    }

    if event.starts_or_implies_turn() {
        let synthetic = event != AgentLifecycleEventKind::TurnStarted;
        let mut result = AgentTransitionResult {
            new_phase: AgentCapturePhase::Active,
            actions: vec![
                AgentCaptureAction::EnsureSession,
                AgentCaptureAction::CaptureBaseline,
                AgentCaptureAction::BeginTurn { synthetic },
                AgentCaptureAction::StartRootSpan,
                AgentCaptureAction::AppendEvidence,
            ],
            diagnostics: Vec::new(),
        };
        if synthetic {
            result
                .diagnostics
                .push(AgentTransitionDiagnostic::SyntheticTurnStart);
        }
        append_event_span_actions(&mut result.actions, event, true);
        return result;
    }

    match event {
        AgentLifecycleEventKind::FinalizationInterrupted => AgentTransitionResult {
            new_phase: AgentCapturePhase::Interrupted,
            actions: vec![AgentCaptureAction::CloseSession { interrupted: true }],
            diagnostics: vec![AgentTransitionDiagnostic::FinalizationInterrupted],
        },
        AgentLifecycleEventKind::FinalizationCompleted => {
            AgentTransitionResult::unchanged(AgentCapturePhase::Idle)
        }
        AgentLifecycleEventKind::UsageUpdated
        | AgentLifecycleEventKind::ModelUpdated
        | AgentLifecycleEventKind::ContextInjected
        | AgentLifecycleEventKind::Diagnostic => {
            AgentTransitionResult::unchanged(AgentCapturePhase::Idle)
                .append(AgentCaptureAction::EnsureSession)
                .append(AgentCaptureAction::AppendEvidence)
        }
        AgentLifecycleEventKind::Unknown => unreachable!("unknown events return above"),
        _ => AgentTransitionResult::unchanged(AgentCapturePhase::Idle)
            .append(AgentCaptureAction::AppendEvidence),
    }
}

fn begin_synthetic_finalization(outcome: AgentTurnOutcome) -> AgentTransitionResult {
    AgentTransitionResult {
        new_phase: AgentCapturePhase::Finalizing,
        actions: vec![
            AgentCaptureAction::EnsureSession,
            AgentCaptureAction::CaptureBaseline,
            AgentCaptureAction::BeginTurn { synthetic: true },
            AgentCaptureAction::StartRootSpan,
            AgentCaptureAction::AppendEvidence,
            AgentCaptureAction::RequestTurnFinalization { outcome },
            AgentCaptureAction::ReconcileWorkdir,
            AgentCaptureAction::ImportTranscript,
        ],
        diagnostics: vec![AgentTransitionDiagnostic::SyntheticTurnStart],
    }
}

fn transition_active(event: AgentLifecycleEventKind) -> AgentTransitionResult {
    if event == AgentLifecycleEventKind::TurnStarted {
        return AgentTransitionResult {
            new_phase: AgentCapturePhase::Active,
            actions: vec![
                AgentCaptureAction::AppendEvidence,
                AgentCaptureAction::RecoverInterruptedTurn,
                AgentCaptureAction::CloseTurn {
                    outcome: AgentTurnOutcome::Interrupted,
                },
                AgentCaptureAction::CaptureBaseline,
                AgentCaptureAction::BeginTurn { synthetic: false },
                AgentCaptureAction::StartRootSpan,
            ],
            diagnostics: vec![AgentTransitionDiagnostic::PriorTurnInterrupted],
        };
    }

    if let Some(outcome) = event.terminal_outcome() {
        return AgentTransitionResult {
            new_phase: AgentCapturePhase::Finalizing,
            actions: vec![
                AgentCaptureAction::AppendEvidence,
                AgentCaptureAction::RequestTurnFinalization { outcome },
                AgentCaptureAction::ReconcileWorkdir,
                AgentCaptureAction::ImportTranscript,
            ],
            diagnostics: Vec::new(),
        };
    }

    if event == AgentLifecycleEventKind::SessionEnded {
        return AgentTransitionResult {
            new_phase: AgentCapturePhase::Finalizing,
            actions: vec![
                AgentCaptureAction::AppendEvidence,
                AgentCaptureAction::RequestSessionClose,
                AgentCaptureAction::RequestTurnFinalization {
                    outcome: AgentTurnOutcome::Interrupted,
                },
                AgentCaptureAction::ReconcileWorkdir,
                AgentCaptureAction::ImportTranscript,
            ],
            diagnostics: Vec::new(),
        };
    }

    if event == AgentLifecycleEventKind::FinalizationInterrupted {
        return AgentTransitionResult {
            new_phase: AgentCapturePhase::Interrupted,
            actions: vec![
                AgentCaptureAction::CloseTurn {
                    outcome: AgentTurnOutcome::Interrupted,
                },
                AgentCaptureAction::CloseSession { interrupted: true },
            ],
            diagnostics: vec![AgentTransitionDiagnostic::FinalizationInterrupted],
        };
    }

    if event == AgentLifecycleEventKind::FinalizationCompleted {
        return AgentTransitionResult::unchanged(AgentCapturePhase::Active);
    }

    let mut result = AgentTransitionResult::unchanged(AgentCapturePhase::Active)
        .append(AgentCaptureAction::AppendEvidence);
    append_event_span_actions(&mut result.actions, event, false);
    result
}

fn transition_finalizing(
    event: AgentLifecycleEventKind,
    context: AgentTransitionContext,
) -> AgentTransitionResult {
    if event == AgentLifecycleEventKind::FinalizationCompleted {
        let outcome = context
            .pending_turn_outcome
            .unwrap_or(AgentTurnOutcome::Interrupted);
        let mut actions = vec![AgentCaptureAction::CloseTurn { outcome }];
        let new_phase = if context.session_close_requested {
            actions.push(AgentCaptureAction::FinalizeAttestation);
            actions.push(AgentCaptureAction::CloseSession { interrupted: false });
            AgentCapturePhase::Ended
        } else {
            AgentCapturePhase::Idle
        };
        return AgentTransitionResult {
            new_phase,
            actions,
            diagnostics: Vec::new(),
        };
    }

    if event == AgentLifecycleEventKind::FinalizationInterrupted {
        let mut actions = vec![AgentCaptureAction::CloseTurn {
            outcome: AgentTurnOutcome::Interrupted,
        }];
        if context.session_close_requested {
            actions.push(AgentCaptureAction::CloseSession { interrupted: true });
        }
        return AgentTransitionResult {
            new_phase: AgentCapturePhase::Interrupted,
            actions,
            diagnostics: vec![AgentTransitionDiagnostic::FinalizationInterrupted],
        };
    }

    if event == AgentLifecycleEventKind::SessionEnded {
        return AgentTransitionResult::unchanged(AgentCapturePhase::Finalizing)
            .append(AgentCaptureAction::AppendEvidence)
            .append(AgentCaptureAction::RequestSessionClose)
            .append(AgentCaptureAction::ImportTranscript);
    }

    if event == AgentLifecycleEventKind::TurnStarted
        || event == AgentLifecycleEventKind::MessageUser
    {
        return AgentTransitionResult::unchanged(AgentCapturePhase::Finalizing)
            .append(AgentCaptureAction::AppendEvidence)
            .append(AgentCaptureAction::DeferUntilFinalized)
            .diagnose(AgentTransitionDiagnostic::EventDeferredDuringFinalization);
    }

    let mut result = AgentTransitionResult::unchanged(AgentCapturePhase::Finalizing)
        .append(AgentCaptureAction::AppendEvidence);
    append_event_span_actions(&mut result.actions, event, false);
    result
}

fn transition_closed(
    phase: AgentCapturePhase,
    event: AgentLifecycleEventKind,
) -> AgentTransitionResult {
    if matches!(
        event,
        AgentLifecycleEventKind::SessionStarted | AgentLifecycleEventKind::SessionResumed
    ) {
        return AgentTransitionResult {
            new_phase: AgentCapturePhase::Idle,
            actions: vec![
                AgentCaptureAction::CreateCaptureEpoch,
                AgentCaptureAction::AppendEvidence,
            ],
            diagnostics: Vec::new(),
        };
    }

    AgentTransitionResult::unchanged(phase)
        .append(AgentCaptureAction::AppendEvidence)
        .diagnose(AgentTransitionDiagnostic::LateEventAfterSessionClose)
}

fn append_event_span_actions(
    actions: &mut Vec<AgentCaptureAction>,
    event: AgentLifecycleEventKind,
    synthetic: bool,
) {
    match event {
        AgentLifecycleEventKind::ToolStarted => {
            actions.push(AgentCaptureAction::StartToolSpan { synthetic });
        }
        AgentLifecycleEventKind::ToolCompleted | AgentLifecycleEventKind::ToolFailed => {
            if synthetic {
                actions.push(AgentCaptureAction::StartToolSpan { synthetic: true });
            }
            actions.push(AgentCaptureAction::EndToolSpan);
        }
        AgentLifecycleEventKind::SubagentStarted => {
            actions.push(AgentCaptureAction::StartSubagentSpan { synthetic });
        }
        AgentLifecycleEventKind::SubagentCompleted | AgentLifecycleEventKind::SubagentFailed => {
            if synthetic {
                actions.push(AgentCaptureAction::StartSubagentSpan { synthetic: true });
            }
            actions.push(AgentCaptureAction::EndSubagentSpan);
        }
        AgentLifecycleEventKind::CompactionStarted => {
            actions.push(AgentCaptureAction::StartCompactionSpan);
        }
        AgentLifecycleEventKind::CompactionCompleted => {
            if synthetic {
                actions.push(AgentCaptureAction::StartCompactionSpan);
            }
            actions.push(AgentCaptureAction::EndCompactionSpan);
        }
        _ => {}
    }
}

impl fmt::Display for AgentLifecycleEventType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod agent_capture_model_tests {
    use super::*;

    const PHASES: [AgentCapturePhase; 5] = [
        AgentCapturePhase::Idle,
        AgentCapturePhase::Active,
        AgentCapturePhase::Finalizing,
        AgentCapturePhase::Ended,
        AgentCapturePhase::Interrupted,
    ];

    fn context() -> AgentTransitionContext {
        AgentTransitionContext {
            has_session: true,
            has_open_turn: true,
            duplicate: false,
            has_new_evidence: true,
            session_close_requested: false,
            pending_turn_outcome: Some(AgentTurnOutcome::Completed),
        }
    }

    #[test]
    fn normalized_vocabulary_round_trips_to_kinds() {
        for kind in AgentLifecycleEventKind::ALL {
            if matches!(
                kind,
                AgentLifecycleEventKind::Unknown
                    | AgentLifecycleEventKind::FinalizationCompleted
                    | AgentLifecycleEventKind::FinalizationInterrupted
            ) {
                continue;
            }
            let wire = AgentLifecycleEventType::from(kind);
            assert_eq!(wire.kind(), kind, "{}", wire.as_str());
        }
        assert_eq!(
            AgentLifecycleEventType::new("provider.codex.future_event").kind(),
            AgentLifecycleEventKind::Unknown
        );
    }

    #[test]
    fn lifecycle_cross_product_is_total_and_bounded() {
        for phase in PHASES {
            for event in AgentLifecycleEventKind::ALL {
                let result = transition_agent_capture(phase, event, context());
                assert!(result.actions.len() <= 10, "{phase:?} + {event:?}");
                assert!(result.diagnostics.len() <= 2, "{phase:?} + {event:?}");
            }
        }
    }

    #[test]
    fn idle_turn_start_captures_baseline_and_root_span() {
        let result = transition_agent_capture(
            AgentCapturePhase::Idle,
            AgentLifecycleEventKind::TurnStarted,
            AgentTransitionContext::default(),
        );
        assert_eq!(result.new_phase, AgentCapturePhase::Active);
        assert_eq!(
            result.actions,
            vec![
                AgentCaptureAction::EnsureSession,
                AgentCaptureAction::CaptureBaseline,
                AgentCaptureAction::BeginTurn { synthetic: false },
                AgentCaptureAction::StartRootSpan,
                AgentCaptureAction::AppendEvidence,
            ]
        );
    }

    #[test]
    fn missing_turn_start_is_recovered_before_terminal_finalization() {
        let result = transition_agent_capture(
            AgentCapturePhase::Idle,
            AgentLifecycleEventKind::TurnFailed,
            AgentTransitionContext::default(),
        );
        assert_eq!(result.new_phase, AgentCapturePhase::Finalizing);
        assert!(result
            .actions
            .contains(&AgentCaptureAction::BeginTurn { synthetic: true }));
        assert!(result
            .actions
            .contains(&AgentCaptureAction::RequestTurnFinalization {
                outcome: AgentTurnOutcome::Failed
            }));
        assert_eq!(
            result.diagnostics,
            vec![AgentTransitionDiagnostic::SyntheticTurnStart]
        );
    }

    #[test]
    fn idle_completion_callbacks_never_reopen_a_finalized_turn() {
        for event in [
            AgentLifecycleEventKind::MessageAssistantCompleted,
            AgentLifecycleEventKind::ToolCompleted,
            AgentLifecycleEventKind::ToolFailed,
            AgentLifecycleEventKind::ApprovalDecided,
            AgentLifecycleEventKind::SubagentCompleted,
            AgentLifecycleEventKind::SubagentFailed,
            AgentLifecycleEventKind::CompactionCompleted,
            AgentLifecycleEventKind::WorkspaceDiff,
            AgentLifecycleEventKind::WorkspaceFileChanged,
            AgentLifecycleEventKind::WorkspaceCheckpoint,
        ] {
            let result = transition_agent_capture(
                AgentCapturePhase::Idle,
                event,
                AgentTransitionContext {
                    has_session: true,
                    ..AgentTransitionContext::default()
                },
            );
            assert_eq!(result.new_phase, AgentCapturePhase::Idle, "{event:?}");
            assert_eq!(
                result.actions,
                vec![
                    AgentCaptureAction::EnsureSession,
                    AgentCaptureAction::AppendEvidence,
                ],
                "{event:?}"
            );
            assert!(result.diagnostics.is_empty(), "{event:?}");
        }
    }

    #[test]
    fn active_turn_start_interrupts_prior_turn() {
        let result = transition_agent_capture(
            AgentCapturePhase::Active,
            AgentLifecycleEventKind::TurnStarted,
            context(),
        );
        assert_eq!(result.new_phase, AgentCapturePhase::Active);
        assert!(result
            .actions
            .contains(&AgentCaptureAction::RecoverInterruptedTurn));
        assert!(result.actions.contains(&AgentCaptureAction::CloseTurn {
            outcome: AgentTurnOutcome::Interrupted
        }));
    }

    #[test]
    fn session_end_waits_for_active_turn_finalization() {
        let result = transition_agent_capture(
            AgentCapturePhase::Active,
            AgentLifecycleEventKind::SessionEnded,
            context(),
        );
        assert_eq!(result.new_phase, AgentCapturePhase::Finalizing);
        assert!(result
            .actions
            .contains(&AgentCaptureAction::RequestSessionClose));
        assert!(!result
            .actions
            .contains(&AgentCaptureAction::CloseSession { interrupted: false }));
    }

    #[test]
    fn finalization_completion_closes_session_only_when_requested() {
        let open = transition_agent_capture(
            AgentCapturePhase::Finalizing,
            AgentLifecycleEventKind::FinalizationCompleted,
            context(),
        );
        assert_eq!(open.new_phase, AgentCapturePhase::Idle);

        let closed = transition_agent_capture(
            AgentCapturePhase::Finalizing,
            AgentLifecycleEventKind::FinalizationCompleted,
            AgentTransitionContext {
                session_close_requested: true,
                ..context()
            },
        );
        assert_eq!(closed.new_phase, AgentCapturePhase::Ended);
        assert!(closed
            .actions
            .contains(&AgentCaptureAction::CloseSession { interrupted: false }));
    }

    #[test]
    fn duplicate_terminal_event_never_refinalizes() {
        let result = transition_agent_capture(
            AgentCapturePhase::Finalizing,
            AgentLifecycleEventKind::TurnCompleted,
            AgentTransitionContext {
                duplicate: true,
                has_new_evidence: true,
                ..context()
            },
        );
        assert_eq!(result.new_phase, AgentCapturePhase::Finalizing);
        assert_eq!(
            result.actions,
            vec![
                AgentCaptureAction::AppendEvidence,
                AgentCaptureAction::WarnDuplicate
            ]
        );
    }

    #[test]
    fn finalization_preserves_pending_failure_outcome() {
        let result = transition_agent_capture(
            AgentCapturePhase::Finalizing,
            AgentLifecycleEventKind::FinalizationCompleted,
            AgentTransitionContext {
                pending_turn_outcome: Some(AgentTurnOutcome::Failed),
                ..context()
            },
        );
        assert_eq!(result.new_phase, AgentCapturePhase::Idle);
        assert_eq!(
            result.actions,
            vec![AgentCaptureAction::CloseTurn {
                outcome: AgentTurnOutcome::Failed
            }]
        );
    }

    #[test]
    fn duplicate_events_never_schedule_semantic_mutations() {
        for phase in PHASES {
            for event in AgentLifecycleEventKind::ALL {
                let result = transition_agent_capture(
                    phase,
                    event,
                    AgentTransitionContext {
                        duplicate: true,
                        has_new_evidence: false,
                        ..context()
                    },
                );
                assert_eq!(result.new_phase, phase);
                assert_eq!(result.actions, vec![AgentCaptureAction::WarnDuplicate]);
            }
        }
    }

    #[test]
    fn unknown_event_is_inert_in_every_phase() {
        for phase in PHASES {
            let result =
                transition_agent_capture(phase, AgentLifecycleEventKind::Unknown, context());
            assert_eq!(result.new_phase, phase);
            assert_eq!(result.actions, vec![AgentCaptureAction::AppendEvidence]);
        }
    }

    #[test]
    fn closed_session_resumes_as_new_capture_epoch() {
        for phase in [AgentCapturePhase::Ended, AgentCapturePhase::Interrupted] {
            let result =
                transition_agent_capture(phase, AgentLifecycleEventKind::SessionResumed, context());
            assert_eq!(result.new_phase, AgentCapturePhase::Idle);
            assert_eq!(
                result.actions,
                vec![
                    AgentCaptureAction::CreateCaptureEpoch,
                    AgentCaptureAction::AppendEvidence
                ]
            );
        }
    }

    #[test]
    fn event_validation_rejects_wrong_schema_and_oversized_payload() {
        let mut event = AgentLifecycleEvent {
            schema: AGENT_LIFECYCLE_EVENT_SCHEMA.to_string(),
            version: AGENT_LIFECYCLE_EVENT_VERSION,
            event_id: "evt_1".to_string(),
            event_type: AgentLifecycleEventType::from(AgentLifecycleEventKind::ToolCompleted),
            occurred_at: Some(1),
            received_at: 2,
            provider: "codex".to_string(),
            provider_version: Some("1.0".to_string()),
            transport: AgentCaptureTransport::NativeHooks,
            workspace_id: "workspace_1".to_string(),
            lane_id: Some("lane_1".to_string()),
            capture_run_id: None,
            native: AgentNativeEventIdentity {
                event_name: "PostToolUse".to_string(),
                ..AgentNativeEventIdentity::default()
            },
            correlation: AgentEventCorrelation::default(),
            payload: serde_json::json!({"ok": true}),
            evidence: AgentEventEvidence {
                receipt_id: "receipt_1".to_string(),
                raw_digest: None,
                transcript_offset: None,
                confidence: AgentEvidenceConfidence::NativeStructured,
            },
        };
        event.validate().unwrap();

        event.schema = "wrong".to_string();
        assert!(event.validate().is_err());
        event.schema = AGENT_LIFECYCLE_EVENT_SCHEMA.to_string();
        event.payload =
            serde_json::Value::String("x".repeat(AGENT_LIFECYCLE_MAX_PAYLOAD_BYTES + 1));
        assert!(event.validate().is_err());
    }

    #[test]
    fn unknown_event_requires_provider_namespace() {
        let event_type = AgentLifecycleEventType::new("future.event");
        assert_eq!(event_type.kind(), AgentLifecycleEventKind::Unknown);

        let mut event = AgentLifecycleEvent {
            schema: AGENT_LIFECYCLE_EVENT_SCHEMA.to_string(),
            version: AGENT_LIFECYCLE_EVENT_VERSION,
            event_id: "evt_1".to_string(),
            event_type,
            occurred_at: None,
            received_at: 1,
            provider: "codex".to_string(),
            provider_version: None,
            transport: AgentCaptureTransport::NativeHooks,
            workspace_id: "workspace_1".to_string(),
            lane_id: None,
            capture_run_id: None,
            native: AgentNativeEventIdentity {
                event_name: "FutureEvent".to_string(),
                ..AgentNativeEventIdentity::default()
            },
            correlation: AgentEventCorrelation::default(),
            payload: serde_json::Value::Null,
            evidence: AgentEventEvidence {
                receipt_id: "receipt_1".to_string(),
                raw_digest: None,
                transcript_offset: None,
                confidence: AgentEvidenceConfidence::Heuristic,
            },
        };
        assert!(event.validate().is_err());
        event.event_type = AgentLifecycleEventType::new("provider.codex.future_event");
        event.validate().unwrap();
    }

    #[test]
    fn evidence_merge_is_monotonic_and_preserves_equal_rank_conflicts() {
        let reconstructed = AgentEvidenceFact {
            kind: AgentEvidenceFactKind::AssistantMessage,
            key: "message-1".to_string(),
            value: serde_json::json!("guessed"),
            source: AgentEvidenceSource::Reconstructed,
            confidence: AgentEvidenceConfidence::Heuristic,
            observed_at: Some(1),
        };
        let transcript = AgentEvidenceFact {
            value: serde_json::json!("canonical"),
            source: AgentEvidenceSource::NativeTranscript,
            confidence: AgentEvidenceConfidence::NativeStructured,
            observed_at: Some(2),
            ..reconstructed.clone()
        };
        let enriched = merge_agent_evidence_fact(Some(&reconstructed), transcript.clone()).unwrap();
        assert_eq!(enriched.decision, AgentEvidenceMergeDecision::Enrich);
        assert_eq!(enriched.fact, transcript);

        let conflict = AgentEvidenceFact {
            value: serde_json::json!("different canonical"),
            ..transcript.clone()
        };
        let preserved = merge_agent_evidence_fact(Some(&transcript), conflict).unwrap();
        assert_eq!(
            preserved.decision,
            AgentEvidenceMergeDecision::PreserveConflict
        );
        assert!(preserved.conflict.is_some());
    }

    #[test]
    fn observed_workdir_beats_protocol_diff_for_workspace_facts() {
        assert!(
            agent_evidence_precedence(
                AgentEvidenceFactKind::WorkspaceChange,
                AgentEvidenceSource::WorkdirObserved,
            ) > agent_evidence_precedence(
                AgentEvidenceFactKind::WorkspaceChange,
                AgentEvidenceSource::Acp,
            )
        );
    }
}
