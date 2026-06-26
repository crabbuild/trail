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
pub(crate) const RESOURCE_AGENT_REVIEW_TEMPLATE: &str = "crabdb://workspace/agents/{agent}/review";
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
