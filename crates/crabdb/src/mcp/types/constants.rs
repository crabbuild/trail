pub(crate) const SERVER_NAME: &str = "crabdb";
pub(crate) const MCP_PROTOCOL_VERSION: &str = "2025-11-25";

pub(crate) const RESOURCE_STATUS: &str = "crabdb://workspace/status";
pub(crate) const RESOURCE_DOCTOR: &str = "crabdb://workspace/doctor";
pub(crate) const RESOURCE_LANES: &str = "crabdb://workspace/lanes";
pub(crate) const RESOURCE_MERGE_QUEUE: &str = "crabdb://workspace/merge-queue";
pub(crate) const RESOURCE_CONFLICTS: &str = "crabdb://workspace/conflicts";
pub(crate) const RESOURCE_OPENAPI: &str = "crabdb://workspace/openapi";
pub(crate) const RESOURCE_AGENT_INBOX: &str = "crabdb://workspace/agent-tasks";
pub(crate) const RESOURCE_AGENT_LATEST_SUMMARY: &str =
    "crabdb://workspace/agent-tasks/latest/summary";
pub(crate) const RESOURCE_AGENT_LATEST_DIAGNOSE: &str =
    "crabdb://workspace/agent-tasks/latest/diagnose";
pub(crate) const RESOURCE_AGENT_LATEST_TEST_PLAN: &str =
    "crabdb://workspace/agent-tasks/latest/test-plan";
pub(crate) const RESOURCE_AGENT_LATEST_CONFIDENCE: &str =
    "crabdb://workspace/agent-tasks/latest/confidence";
pub(crate) const RESOURCE_AGENT_LATEST_REVIEW_MAP: &str =
    "crabdb://workspace/agent-tasks/latest/review-map";
pub(crate) const RESOURCE_AGENT_LATEST_REVIEW: &str =
    "crabdb://workspace/agent-tasks/latest/review";
pub(crate) const RESOURCE_AGENT_LATEST_REVIEW_DATA: &str =
    "crabdb://workspace/agent-tasks/latest/review-data";
pub(crate) const RESOURCE_AGENT_LATEST_CHANGES: &str =
    "crabdb://workspace/agent-tasks/latest/changes";
pub(crate) const RESOURCE_AGENT_LATEST_TIMELINE: &str =
    "crabdb://workspace/agent-tasks/latest/timeline";
pub(crate) const RESOURCE_AGENT_LATEST_FILES: &str = "crabdb://workspace/agent-tasks/latest/files";
pub(crate) const RESOURCE_AGENT_LATEST_FOCUS: &str = "crabdb://workspace/agent-tasks/latest/focus";
pub(crate) const RESOURCE_AGENT_LATEST_RECEIPT: &str =
    "crabdb://workspace/agent-tasks/latest/receipt";
pub(crate) const RESOURCE_AGENT_LATEST_HANDOFF: &str =
    "crabdb://workspace/agent-tasks/latest/handoff";
pub(crate) const RESOURCE_AGENT_LATEST_PR: &str = "crabdb://workspace/agent-tasks/latest/pr";
pub(crate) const RESOURCE_USER_GUIDE: &str = "crabdb://docs/user-guide";
pub(crate) const RESOURCE_LANE_WORKFLOWS: &str = "crabdb://docs/lane-workflows";
pub(crate) const RESOURCE_CLI_REFERENCE: &str = "crabdb://docs/cli-reference";
pub(crate) const RESOURCE_AGENT_REVIEW_TEMPLATE: &str =
    "crabdb://workspace/agent-tasks/{selector}/review";
pub(crate) const RESOURCE_AGENT_REVIEW_DATA_TEMPLATE: &str =
    "crabdb://workspace/agent-tasks/{selector}/review-data";
pub(crate) const RESOURCE_AGENT_SUMMARY_TEMPLATE: &str =
    "crabdb://workspace/agent-tasks/{selector}/summary";
pub(crate) const RESOURCE_AGENT_DIAGNOSE_TEMPLATE: &str =
    "crabdb://workspace/agent-tasks/{selector}/diagnose";
pub(crate) const RESOURCE_AGENT_TEST_PLAN_TEMPLATE: &str =
    "crabdb://workspace/agent-tasks/{selector}/test-plan";
pub(crate) const RESOURCE_AGENT_CONFIDENCE_TEMPLATE: &str =
    "crabdb://workspace/agent-tasks/{selector}/confidence";
pub(crate) const RESOURCE_AGENT_REVIEW_MAP_TEMPLATE: &str =
    "crabdb://workspace/agent-tasks/{selector}/review-map";
pub(crate) const RESOURCE_AGENT_CHANGES_TEMPLATE: &str =
    "crabdb://workspace/agent-tasks/{selector}/changes";
pub(crate) const RESOURCE_AGENT_TIMELINE_TEMPLATE: &str =
    "crabdb://workspace/agent-tasks/{selector}/timeline";
pub(crate) const RESOURCE_AGENT_FILES_TEMPLATE: &str =
    "crabdb://workspace/agent-tasks/{selector}/files";
pub(crate) const RESOURCE_AGENT_REPORT_TEMPLATE: &str =
    "crabdb://workspace/agent-tasks/{selector}/report";
pub(crate) const RESOURCE_AGENT_RECEIPT_TEMPLATE: &str =
    "crabdb://workspace/agent-tasks/{selector}/receipt";
pub(crate) const RESOURCE_AGENT_HANDOFF_TEMPLATE: &str =
    "crabdb://workspace/agent-tasks/{selector}/handoff";
pub(crate) const RESOURCE_AGENT_PR_TEMPLATE: &str = "crabdb://workspace/agent-tasks/{selector}/pr";
pub(crate) const RESOURCE_AGENT_FOCUS_TEMPLATE: &str =
    "crabdb://workspace/agent-tasks/{selector}/focus";
pub(crate) const RESOURCE_LANE_TEMPLATE: &str = "crabdb://workspace/lanes/{lane}";
pub(crate) const RESOURCE_LANE_STATUS_TEMPLATE: &str = "crabdb://workspace/lanes/{lane}/status";
pub(crate) const RESOURCE_LANE_REVIEW_TEMPLATE: &str = "crabdb://workspace/lanes/{lane}/review";
pub(crate) const RESOURCE_LANE_CONTRIBUTION_TEMPLATE: &str =
    "crabdb://workspace/lanes/{lane}/contribution";
pub(crate) const RESOURCE_LANE_GATES_TEMPLATE: &str = "crabdb://workspace/lanes/{lane}/gates";
pub(crate) const RESOURCE_LANE_READINESS_TEMPLATE: &str =
    "crabdb://workspace/lanes/{lane}/readiness";
pub(crate) const RESOURCE_LANE_HANDOFF_TEMPLATE: &str = "crabdb://workspace/lanes/{lane}/handoff";
pub(crate) const RESOURCE_LANE_DIFF_TEMPLATE: &str = "crabdb://workspace/lanes/{lane}/diff";
pub(crate) const RESOURCE_SESSION_TEMPLATE: &str = "crabdb://workspace/sessions/{session_id}";
pub(crate) const RESOURCE_TURN_TEMPLATE: &str = "crabdb://workspace/turns/{turn_id}";
pub(crate) const RESOURCE_CONFLICT_TEMPLATE: &str =
    "crabdb://workspace/conflicts/{conflict_set_id}";
pub(crate) const RESOURCE_APPROVAL_TEMPLATE: &str = "crabdb://workspace/approvals/{approval_id}";
pub(crate) const RESOURCE_RUN_TEMPLATE: &str = "crabdb://workspace/runs/{run_id}";
pub(crate) const RESOURCE_SPAN_TEMPLATE: &str = "crabdb://workspace/spans/{span_id}";

pub(crate) const PROMPT_LANE_TASK: &str = "crabdb.lane_task";
pub(crate) const PROMPT_REVIEW_LANE: &str = "crabdb.review_lane";
pub(crate) const PROMPT_RESOLVE_CONFLICT: &str = "crabdb.resolve_conflict";
pub(crate) const PROMPT_REVIEW_AGENT: &str = "crabdb.review_agent";
pub(crate) const PROMPT_RECOVER_AGENT: &str = "crabdb.recover_agent";
pub(crate) const PROMPT_APPLY_AGENT: &str = "crabdb.apply_agent";

pub(crate) const USER_GUIDE_MD: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/USER_GUIDE.md"
));
pub(crate) const LANE_WORKFLOWS_MD: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/LANE_WORKFLOWS.md"
));
pub(crate) const CLI_REFERENCE_MD: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/CLI_REFERENCE.md"
));
