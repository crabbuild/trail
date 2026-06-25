use serde::Deserialize;
use serde_json::Value;

use crate::model::ConflictManualResolution;
use crate::PatchEdit;

pub(crate) const SERVER_NAME: &str = "crabdb";
pub(crate) const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
pub(crate) const RESOURCE_STATUS: &str = "crabdb://workspace/status";
pub(crate) const RESOURCE_DOCTOR: &str = "crabdb://workspace/doctor";
pub(crate) const RESOURCE_AGENTS: &str = "crabdb://workspace/agents";
pub(crate) const RESOURCE_MERGE_QUEUE: &str = "crabdb://workspace/merge-queue";
pub(crate) const RESOURCE_CONFLICTS: &str = "crabdb://workspace/conflicts";
pub(crate) const RESOURCE_OPENAPI: &str = "crabdb://workspace/openapi";
pub(crate) const RESOURCE_USER_GUIDE: &str = "crabdb://docs/user-guide";
pub(crate) const RESOURCE_AGENT_WORKFLOWS: &str = "crabdb://docs/agent-workflows";
pub(crate) const RESOURCE_CLI_REFERENCE: &str = "crabdb://docs/cli-reference";
pub(crate) const RESOURCE_AGENT_TEMPLATE: &str = "crabdb://workspace/agents/{agent}";
pub(crate) const RESOURCE_AGENT_STATUS_TEMPLATE: &str = "crabdb://workspace/agents/{agent}/status";
pub(crate) const RESOURCE_AGENT_CONTRIBUTION_TEMPLATE: &str =
    "crabdb://workspace/agents/{agent}/contribution";
pub(crate) const RESOURCE_AGENT_GATES_TEMPLATE: &str = "crabdb://workspace/agents/{agent}/gates";
pub(crate) const RESOURCE_AGENT_READINESS_TEMPLATE: &str =
    "crabdb://workspace/agents/{agent}/readiness";
pub(crate) const RESOURCE_AGENT_HANDOFF_TEMPLATE: &str =
    "crabdb://workspace/agents/{agent}/handoff";
pub(crate) const RESOURCE_AGENT_DIFF_TEMPLATE: &str = "crabdb://workspace/agents/{agent}/diff";
pub(crate) const RESOURCE_SESSION_TEMPLATE: &str = "crabdb://workspace/sessions/{session_id}";
pub(crate) const RESOURCE_TURN_TEMPLATE: &str = "crabdb://workspace/turns/{turn_id}";
pub(crate) const RESOURCE_CONFLICT_TEMPLATE: &str =
    "crabdb://workspace/conflicts/{conflict_set_id}";
pub(crate) const RESOURCE_APPROVAL_TEMPLATE: &str = "crabdb://workspace/approvals/{approval_id}";
pub(crate) const RESOURCE_RUN_TEMPLATE: &str = "crabdb://workspace/runs/{run_id}";
pub(crate) const RESOURCE_SPAN_TEMPLATE: &str = "crabdb://workspace/spans/{span_id}";
pub(crate) const PROMPT_AGENT_TASK: &str = "crabdb.agent_task";
pub(crate) const PROMPT_REVIEW_AGENT: &str = "crabdb.review_agent";
pub(crate) const PROMPT_RESOLVE_CONFLICT: &str = "crabdb.resolve_conflict";

pub(crate) const USER_GUIDE_MD: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/USER_GUIDE.md"
));
pub(crate) const AGENT_WORKFLOWS_MD: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/AGENT_WORKFLOWS.md"
));
pub(crate) const CLI_REFERENCE_MD: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/CLI_REFERENCE.md"
));

#[derive(Debug, Deserialize)]
pub(crate) struct ToolCall {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) arguments: Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ResourceReadArgs {
    pub(crate) uri: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PromptGetArgs {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) arguments: Value,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CompletionArgs {
    #[serde(rename = "ref")]
    pub(crate) reference: CompletionReference,
    pub(crate) argument: CompletionArgument,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CompletionReference {
    #[serde(rename = "type")]
    pub(crate) reference_type: String,
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) uri: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CompletionArgument {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) value: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StatusArgs {
    #[serde(default)]
    pub(crate) branch: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DiffArgs {
    #[serde(default)]
    pub(crate) range: Option<String>,
    #[serde(default)]
    pub(crate) root: Option<String>,
    #[serde(default)]
    pub(crate) dirty: bool,
    #[serde(default)]
    pub(crate) patch: bool,
    #[serde(default, alias = "show-line-ids")]
    pub(crate) show_line_ids: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TimelineArgs {
    #[serde(default)]
    pub(crate) branch: Option<String>,
    #[serde(default)]
    pub(crate) session: Option<String>,
    #[serde(default)]
    pub(crate) agent: Option<String>,
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConfigKeyArgs {
    pub(crate) key: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConfigSetArgs {
    pub(crate) key: String,
    pub(crate) value: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SessionStartArgs {
    pub(crate) agent: String,
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SessionListArgs {
    #[serde(default)]
    pub(crate) agent: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SessionCurrentArgs {
    #[serde(default)]
    pub(crate) agent: Option<String>,
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
    pub(crate) agent: String,
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
    pub(crate) agent: Option<String>,
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
pub(crate) struct AgentRunPauseArgs {
    pub(crate) agent: String,
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
pub(crate) struct AgentRunListArgs {
    #[serde(default)]
    pub(crate) agent: Option<String>,
    #[serde(default)]
    pub(crate) status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentRunShowArgs {
    pub(crate) run_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentRunResumeArgs {
    pub(crate) run_id: String,
    #[serde(default)]
    pub(crate) reviewer: Option<String>,
    #[serde(default)]
    pub(crate) note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LeaseAcquireArgs {
    pub(crate) agent: String,
    #[serde(default)]
    pub(crate) path: Option<String>,
    #[serde(default)]
    pub(crate) mode: Option<String>,
    #[serde(default, alias = "ttl")]
    pub(crate) ttl_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LeaseListArgs {
    #[serde(default)]
    pub(crate) all: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct LeaseReleaseArgs {
    pub(crate) lease_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WhyArgs {
    #[serde(default)]
    pub(crate) path_line: Option<String>,
    #[serde(default)]
    pub(crate) line_id: Option<String>,
    #[serde(default)]
    pub(crate) branch: Option<String>,
    #[serde(default)]
    pub(crate) at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HistoryArgs {
    #[serde(default)]
    pub(crate) selector: Option<String>,
    #[serde(default)]
    pub(crate) path: Option<String>,
    #[serde(default)]
    pub(crate) file_id: Option<String>,
    #[serde(default)]
    pub(crate) line_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CodeFromArgs {
    pub(crate) selector: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentSpawnArgs {
    pub(crate) name: String,
    #[serde(default, alias = "from", alias = "branch")]
    pub(crate) from_ref: Option<String>,
    #[serde(default = "default_true")]
    pub(crate) materialize: bool,
    #[serde(default, alias = "workdir_path")]
    pub(crate) workdir: Option<String>,
    #[serde(default)]
    pub(crate) provider: Option<String>,
    #[serde(default)]
    pub(crate) model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentClaimArgs {
    pub(crate) agent: String,
    pub(crate) path: String,
    #[serde(default, alias = "ttl")]
    pub(crate) ttl_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentHandleArgs {
    #[serde(alias = "agent_or_id", alias = "name")]
    pub(crate) agent: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentContributionArgs {
    #[serde(alias = "agent_or_id", alias = "name")]
    pub(crate) agent: String,
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentRemoveArgs {
    #[serde(alias = "agent_or_id", alias = "name")]
    pub(crate) agent: String,
    #[serde(default)]
    pub(crate) force: bool,
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

#[derive(Debug, Deserialize)]
pub(crate) struct MergeQueueAddArgs {
    pub(crate) source: String,
    #[serde(alias = "into", alias = "target_branch")]
    pub(crate) target: String,
    #[serde(default)]
    pub(crate) priority: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MergeQueueRunArgs {
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct MergeQueueRemoveArgs {
    pub(crate) selector: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConflictIdArgs {
    pub(crate) conflict_set_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ConflictResolveArgs {
    pub(crate) conflict_set_id: String,
    #[serde(default)]
    pub(crate) take: Option<String>,
    #[serde(default)]
    pub(crate) manual: Option<ConflictManualResolution>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BeginTurnArgs {
    pub(crate) agent: String,
    #[serde(default)]
    pub(crate) branch: Option<String>,
    #[serde(default)]
    pub(crate) session_title: Option<String>,
    #[serde(default)]
    pub(crate) base_change: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TurnIdArgs {
    pub(crate) turn_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AddMessageArgs {
    pub(crate) turn_id: String,
    pub(crate) role: String,
    #[serde(default)]
    pub(crate) content: Option<String>,
    #[serde(default)]
    pub(crate) text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AddEventArgs {
    pub(crate) turn_id: String,
    #[serde(alias = "type")]
    pub(crate) event_type: String,
    #[serde(default)]
    pub(crate) payload: Option<Value>,
    #[serde(default)]
    pub(crate) change_id: Option<String>,
    #[serde(default)]
    pub(crate) message_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EventListArgs {
    #[serde(default)]
    pub(crate) agent: Option<String>,
    #[serde(default)]
    pub(crate) session: Option<String>,
    #[serde(default, alias = "turn")]
    pub(crate) turn_id: Option<String>,
    #[serde(default, alias = "type")]
    pub(crate) event_type: Option<String>,
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SpanStartArgs {
    pub(crate) turn_id: String,
    #[serde(alias = "type")]
    pub(crate) span_type: String,
    pub(crate) name: String,
    #[serde(default, alias = "parent_span_id")]
    pub(crate) parent: Option<String>,
    #[serde(default, alias = "trace_id")]
    pub(crate) trace: Option<String>,
    #[serde(default, alias = "attributes_json")]
    pub(crate) attributes: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SpanEndArgs {
    pub(crate) span_id: String,
    #[serde(default = "default_completed_status")]
    pub(crate) status: String,
    #[serde(default, alias = "result_json")]
    pub(crate) result: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SpanListArgs {
    #[serde(default)]
    pub(crate) agent: Option<String>,
    #[serde(default)]
    pub(crate) session: Option<String>,
    #[serde(default, alias = "turn")]
    pub(crate) turn_id: Option<String>,
    #[serde(default, alias = "trace")]
    pub(crate) trace_id: Option<String>,
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SpanSummaryArgs {
    #[serde(default)]
    pub(crate) agent: Option<String>,
    #[serde(default)]
    pub(crate) session: Option<String>,
    #[serde(default, alias = "turn")]
    pub(crate) turn_id: Option<String>,
    #[serde(default, alias = "trace")]
    pub(crate) trace_id: Option<String>,
    #[serde(default, alias = "slowest_limit")]
    pub(crate) slowest: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SpanShowArgs {
    pub(crate) span_id: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EndTurnArgs {
    pub(crate) turn_id: String,
    #[serde(default = "default_completed_status")]
    pub(crate) status: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DiffAgentArgs {
    pub(crate) agent: String,
    #[serde(default)]
    pub(crate) patch: bool,
    #[serde(default, alias = "show-line-ids")]
    pub(crate) show_line_ids: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GateHistoryArgs {
    pub(crate) agent: String,
    #[serde(default)]
    pub(crate) kind: Option<String>,
    #[serde(default)]
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RunTestArgs {
    pub(crate) agent: String,
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
pub(crate) struct SyncWorkdirArgs {
    pub(crate) agent: String,
    #[serde(default)]
    pub(crate) force: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct IgnorePatternArgs {
    pub(crate) pattern: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct IgnoreCheckArgs {
    pub(crate) path: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GuardrailCheckArgs {
    pub(crate) agent: Option<String>,
    pub(crate) action: String,
    pub(crate) summary: Option<String>,
    pub(crate) payload: Option<Value>,
    #[serde(default)]
    pub(crate) paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApplyPatchArgs {
    pub(crate) turn_id: String,
    #[serde(default)]
    pub(crate) base_change: Option<String>,
    #[serde(default)]
    pub(crate) message: Option<String>,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) allow_ignored: bool,
    #[serde(default)]
    pub(crate) edits: Vec<PatchEdit>,
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

pub(crate) fn default_true() -> bool {
    true
}
