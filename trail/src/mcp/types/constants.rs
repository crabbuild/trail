pub(crate) const SERVER_NAME: &str = "trail";
pub(crate) const MCP_PROTOCOL_VERSION: &str = "2025-11-25";

pub(crate) const RESOURCE_STATUS: &str = "trail://workspace/status";
pub(crate) const RESOURCE_DOCTOR: &str = "trail://workspace/doctor";
pub(crate) const RESOURCE_LANES: &str = "trail://workspace/lanes";
pub(crate) const RESOURCE_MERGE_QUEUE: &str = "trail://workspace/merge-queue";
pub(crate) const RESOURCE_CONFLICTS: &str = "trail://workspace/conflicts";
pub(crate) const RESOURCE_OPENAPI: &str = "trail://workspace/openapi";
pub(crate) const RESOURCE_AGENT_INBOX: &str = "trail://workspace/agent-tasks";
pub(crate) const RESOURCE_AGENT_LATEST_SUMMARY: &str =
    "trail://workspace/agent-tasks/latest/summary";
pub(crate) const RESOURCE_AGENT_LATEST_DIAGNOSE: &str =
    "trail://workspace/agent-tasks/latest/diagnose";
pub(crate) const RESOURCE_AGENT_LATEST_TEST_PLAN: &str =
    "trail://workspace/agent-tasks/latest/test-plan";
pub(crate) const RESOURCE_AGENT_LATEST_CONFIDENCE: &str =
    "trail://workspace/agent-tasks/latest/confidence";
pub(crate) const RESOURCE_AGENT_LATEST_REVIEW_MAP: &str =
    "trail://workspace/agent-tasks/latest/review-map";
pub(crate) const RESOURCE_AGENT_LATEST_REVIEW: &str = "trail://workspace/agent-tasks/latest/review";
pub(crate) const RESOURCE_AGENT_LATEST_REVIEW_DATA: &str =
    "trail://workspace/agent-tasks/latest/review-data";
pub(crate) const RESOURCE_AGENT_LATEST_CHANGES: &str =
    "trail://workspace/agent-tasks/latest/changes";
pub(crate) const RESOURCE_AGENT_LATEST_TIMELINE: &str =
    "trail://workspace/agent-tasks/latest/timeline";
pub(crate) const RESOURCE_AGENT_LATEST_FILES: &str = "trail://workspace/agent-tasks/latest/files";
pub(crate) const RESOURCE_AGENT_LATEST_FOCUS: &str = "trail://workspace/agent-tasks/latest/focus";
pub(crate) const RESOURCE_AGENT_LATEST_RECEIPT: &str =
    "trail://workspace/agent-tasks/latest/receipt";
pub(crate) const RESOURCE_AGENT_LATEST_HANDOFF: &str =
    "trail://workspace/agent-tasks/latest/handoff";
pub(crate) const RESOURCE_AGENT_LATEST_PR: &str = "trail://workspace/agent-tasks/latest/pr";
pub(crate) const RESOURCE_USER_GUIDE: &str = "trail://docs/user-guide";
pub(crate) const RESOURCE_LANE_WORKFLOWS: &str = "trail://docs/lane-workflows";
pub(crate) const RESOURCE_CLI_REFERENCE: &str = "trail://docs/cli-reference";
pub(crate) const RESOURCE_AGENT_REVIEW_TEMPLATE: &str =
    "trail://workspace/agent-tasks/{selector}/review";
pub(crate) const RESOURCE_AGENT_REVIEW_DATA_TEMPLATE: &str =
    "trail://workspace/agent-tasks/{selector}/review-data";
pub(crate) const RESOURCE_AGENT_SUMMARY_TEMPLATE: &str =
    "trail://workspace/agent-tasks/{selector}/summary";
pub(crate) const RESOURCE_AGENT_DIAGNOSE_TEMPLATE: &str =
    "trail://workspace/agent-tasks/{selector}/diagnose";
pub(crate) const RESOURCE_AGENT_TEST_PLAN_TEMPLATE: &str =
    "trail://workspace/agent-tasks/{selector}/test-plan";
pub(crate) const RESOURCE_AGENT_CONFIDENCE_TEMPLATE: &str =
    "trail://workspace/agent-tasks/{selector}/confidence";
pub(crate) const RESOURCE_AGENT_REVIEW_MAP_TEMPLATE: &str =
    "trail://workspace/agent-tasks/{selector}/review-map";
pub(crate) const RESOURCE_AGENT_CHANGES_TEMPLATE: &str =
    "trail://workspace/agent-tasks/{selector}/changes";
pub(crate) const RESOURCE_AGENT_TIMELINE_TEMPLATE: &str =
    "trail://workspace/agent-tasks/{selector}/timeline";
pub(crate) const RESOURCE_AGENT_FILES_TEMPLATE: &str =
    "trail://workspace/agent-tasks/{selector}/files";
pub(crate) const RESOURCE_AGENT_REPORT_TEMPLATE: &str =
    "trail://workspace/agent-tasks/{selector}/report";
pub(crate) const RESOURCE_AGENT_RECEIPT_TEMPLATE: &str =
    "trail://workspace/agent-tasks/{selector}/receipt";
pub(crate) const RESOURCE_AGENT_HANDOFF_TEMPLATE: &str =
    "trail://workspace/agent-tasks/{selector}/handoff";
pub(crate) const RESOURCE_AGENT_PR_TEMPLATE: &str = "trail://workspace/agent-tasks/{selector}/pr";
pub(crate) const RESOURCE_AGENT_FOCUS_TEMPLATE: &str =
    "trail://workspace/agent-tasks/{selector}/focus";
pub(crate) const RESOURCE_LANE_TEMPLATE: &str = "trail://workspace/lanes/{lane}";
pub(crate) const RESOURCE_LANE_STATUS_TEMPLATE: &str = "trail://workspace/lanes/{lane}/status";
pub(crate) const RESOURCE_LANE_REVIEW_TEMPLATE: &str = "trail://workspace/lanes/{lane}/review";
pub(crate) const RESOURCE_LANE_CONTRIBUTION_TEMPLATE: &str =
    "trail://workspace/lanes/{lane}/contribution";
pub(crate) const RESOURCE_LANE_GATES_TEMPLATE: &str = "trail://workspace/lanes/{lane}/gates";
pub(crate) const RESOURCE_LANE_READINESS_TEMPLATE: &str =
    "trail://workspace/lanes/{lane}/readiness";
pub(crate) const RESOURCE_LANE_HANDOFF_TEMPLATE: &str = "trail://workspace/lanes/{lane}/handoff";
pub(crate) const RESOURCE_LANE_DIFF_TEMPLATE: &str = "trail://workspace/lanes/{lane}/diff";
pub(crate) const RESOURCE_SESSION_TEMPLATE: &str = "trail://workspace/sessions/{session_id}";
pub(crate) const RESOURCE_TURN_TEMPLATE: &str = "trail://workspace/turns/{turn_id}";
pub(crate) const RESOURCE_CONFLICT_TEMPLATE: &str = "trail://workspace/conflicts/{conflict_set_id}";
pub(crate) const RESOURCE_APPROVAL_TEMPLATE: &str = "trail://workspace/approvals/{approval_id}";
pub(crate) const RESOURCE_RUN_TEMPLATE: &str = "trail://workspace/runs/{run_id}";
pub(crate) const RESOURCE_SPAN_TEMPLATE: &str = "trail://workspace/spans/{span_id}";

pub(crate) const PROMPT_LANE_TASK: &str = "trail.lane_task";
pub(crate) const PROMPT_REVIEW_LANE: &str = "trail.review_lane";
pub(crate) const PROMPT_RESOLVE_CONFLICT: &str = "trail.resolve_conflict";
pub(crate) const PROMPT_REVIEW_AGENT: &str = "trail.review_agent";
pub(crate) const PROMPT_RECOVER_AGENT: &str = "trail.recover_agent";
pub(crate) const PROMPT_APPLY_AGENT: &str = "trail.apply_agent";

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
