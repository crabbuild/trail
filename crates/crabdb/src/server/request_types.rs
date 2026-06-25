use serde::Deserialize;

use crate::model::ConflictManualResolution;

#[derive(Debug, Deserialize)]
pub(crate) struct SpawnAgentRequest {
    pub(crate) name: String,
    #[serde(default, alias = "from_ref", alias = "branch")]
    pub(crate) from: Option<String>,
    #[serde(default)]
    pub(crate) materialize: Option<bool>,
    #[serde(default, alias = "workdir_path")]
    pub(crate) workdir: Option<String>,
    #[serde(default)]
    pub(crate) provider: Option<String>,
    #[serde(default)]
    pub(crate) model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MergeAgentRequest {
    #[serde(default, alias = "agent", alias = "name")]
    pub(crate) agent_id: Option<String>,
    #[serde(default)]
    pub(crate) strategy: Option<String>,
    #[serde(default, alias = "dry-run")]
    pub(crate) dry_run: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BeginTurnRequest {
    pub(crate) agent: String,
    #[serde(default)]
    pub(crate) branch: Option<String>,
    #[serde(default)]
    pub(crate) session_title: Option<String>,
    #[serde(default)]
    pub(crate) base_change: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AddMessageRequest {
    pub(crate) role: String,
    #[serde(default)]
    pub(crate) content: Option<String>,
    #[serde(default)]
    pub(crate) text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AddEventRequest {
    #[serde(alias = "type")]
    pub(crate) event_type: String,
    #[serde(default)]
    pub(crate) payload: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) change_id: Option<String>,
    #[serde(default)]
    pub(crate) message_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StartSpanRequest {
    #[serde(alias = "type")]
    pub(crate) span_type: String,
    pub(crate) name: String,
    #[serde(default, alias = "parent_span_id")]
    pub(crate) parent: Option<String>,
    #[serde(default, alias = "trace_id")]
    pub(crate) trace: Option<String>,
    #[serde(default, alias = "attributes_json")]
    pub(crate) attributes: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EndSpanRequest {
    #[serde(default = "default_completed_status")]
    pub(crate) status: String,
    #[serde(default, alias = "result_json")]
    pub(crate) result: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EndTurnRequest {
    #[serde(default = "default_completed_status")]
    pub(crate) status: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentTestRequest {
    pub(crate) command: Vec<String>,
    #[serde(default, alias = "turn")]
    pub(crate) turn_id: Option<String>,
    #[serde(default, alias = "timeout_seconds")]
    pub(crate) timeout_secs: Option<u64>,
    #[serde(default)]
    pub(crate) suite: Option<String>,
    #[serde(default)]
    pub(crate) score: Option<f64>,
    #[serde(default)]
    pub(crate) threshold: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SyncWorkdirRequest {
    #[serde(default)]
    pub(crate) force: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct IgnorePatternRequest {
    pub(crate) pattern: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct IgnoreCheckRequest {
    pub(crate) path: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GuardrailCheckRequest {
    pub(crate) agent: Option<String>,
    pub(crate) action: String,
    pub(crate) summary: Option<String>,
    pub(crate) payload: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConfigSetRequest {
    pub(crate) key: String,
    pub(crate) value: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SessionStartRequest {
    pub(crate) agent: String,
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SessionEndRequest {
    #[serde(default = "default_completed_status")]
    pub(crate) status: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApprovalRequest {
    pub(crate) agent: String,
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
pub(crate) struct ApprovalDecisionRequest {
    pub(crate) decision: String,
    #[serde(default)]
    pub(crate) reviewer: Option<String>,
    #[serde(default)]
    pub(crate) note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentRunPauseRequest {
    pub(crate) agent: String,
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
pub(crate) struct AgentRunResumeRequest {
    #[serde(default)]
    pub(crate) reviewer: Option<String>,
    #[serde(default)]
    pub(crate) note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LeaseAcquireRequest {
    pub(crate) agent: String,
    #[serde(default)]
    pub(crate) path: Option<String>,
    #[serde(default)]
    pub(crate) mode: Option<String>,
    #[serde(default, alias = "ttl")]
    pub(crate) ttl_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentClaimRequest {
    pub(crate) path: String,
    #[serde(default, alias = "ttl")]
    pub(crate) ttl_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnchorCreateRequest {
    pub(crate) path_line: String,
    pub(crate) label: String,
    #[serde(default)]
    pub(crate) branch: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MergeQueueAddRequest {
    pub(crate) source: String,
    #[serde(alias = "into", alias = "target_branch")]
    pub(crate) target: String,
    #[serde(default)]
    pub(crate) priority: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MergeQueueRunRequest {
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConflictResolveRequest {
    #[serde(default)]
    pub(crate) take: Option<String>,
    #[serde(default)]
    pub(crate) manual: Option<ConflictManualResolution>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApiPatchRequest {
    #[serde(default)]
    pub(crate) base_change: Option<String>,
    #[serde(default)]
    pub(crate) message: Option<String>,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) allow_ignored: bool,
    #[serde(default)]
    pub(crate) edits: Vec<crate::PatchEdit>,
    #[serde(default)]
    pub(crate) files: Vec<ApiPatchFile>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ApiPatchFile {
    AddText {
        path: String,
        content: String,
        #[serde(default)]
        executable: bool,
    },
    ModifyText {
        path: String,
        edits: Vec<ApiTextEdit>,
    },
    WriteBytes {
        path: String,
        bytes_hex: String,
        #[serde(default)]
        executable: bool,
    },
    Delete {
        path: String,
    },
    Rename {
        from: String,
        to: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ApiTextEdit {
    ModifyLine {
        line_id: String,
        #[serde(default)]
        expected_text: Option<String>,
        new_text: String,
    },
}

pub(crate) fn default_completed_status() -> String {
    "completed".to_string()
}

pub(crate) fn default_lease_mode() -> String {
    "write".to_string()
}
