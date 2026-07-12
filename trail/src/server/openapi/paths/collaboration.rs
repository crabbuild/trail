use serde_json::{json, Value};

use super::{
    openapi_operation, openapi_operation_with_response_schema, openapi_path_param, openapi_query,
};

pub(super) fn collaboration_paths() -> Value {
    json!({
        "/v1/sessions": {
            "get": openapi_operation("sessionList", "List sessions", "List durable lane sessions.", vec![
                openapi_query("lane", "string")
            ], None, true),
            "post": openapi_operation("sessionStart", "Start session", "Start an explicit durable lane session.", vec![], Some("SessionStartRequest"), true)
        },
        "/v1/sessions/current": {
            "get": openapi_operation("sessionCurrent", "Current sessions", "Read current lane branch session attachments.", vec![
                openapi_query("lane", "string")
            ], None, true)
        },
        "/v1/sessions/{session_id}": {
            "get": openapi_operation("sessionShow", "Show session", "Return a session with turns, messages, events, and operations.", vec![
                openapi_path_param("session_id", "string")
            ], None, true)
        },
        "/v1/sessions/{session_id}/context": {
            "get": openapi_operation("sessionContext", "Session context", "Return a bounded session context packet with total counts and recent messages, events, turns, and operations.", vec![
                openapi_path_param("session_id", "string"),
                openapi_query("limit", "integer")
            ], None, true)
        },
        "/v1/sessions/{session_id}/end": {
            "post": openapi_operation("sessionEnd", "End session", "End a durable lane session.", vec![
                openapi_path_param("session_id", "string")
            ], Some("SessionEndRequest"), true)
        },
        "/v1/approvals": {
            "get": openapi_operation("approvalList", "List approvals", "List durable human approval gates.", vec![
                openapi_query("lane", "string"),
                openapi_query("status", "string")
            ], None, true),
            "post": openapi_operation("approvalRequest", "Request approval", "Create a durable pending approval for a sensitive action.", vec![], Some("ApprovalRequest"), true)
        },
        "/v1/approvals/{approval_id}": {
            "get": openapi_operation("approvalShow", "Show approval", "Show one durable approval gate.", vec![
                openapi_path_param("approval_id", "string")
            ], None, true)
        },
        "/v1/approvals/{approval_id}/decision": {
            "post": openapi_operation("approvalDecide", "Decide approval", "Approve, reject, or cancel an approval gate.", vec![
                openapi_path_param("approval_id", "string")
            ], Some("ApprovalDecisionRequest"), true)
        },
        "/v1/leases": {
            "get": openapi_operation("leaseList", "List leases", "List active advisory leases, or all leases when requested.", vec![
                openapi_query("all", "boolean")
            ], None, true),
            "post": openapi_operation("leaseAcquire", "Acquire lease", "Acquire an advisory path lease.", vec![], Some("LeaseAcquireRequest"), true)
        },
        "/v1/leases/{lease_id}": {
            "delete": openapi_operation("leaseRelease", "Release lease", "Release an advisory path lease.", vec![
                openapi_path_param("lease_id", "string")
            ], None, true)
        },
        "/v1/lanes/{lane_or_id}/claims": {
            "post": openapi_operation("laneClaim", "Claim lane path", "Create an advisory path claim for a lane, or return active claim conflicts as a warning.", vec![
                openapi_path_param("lane_or_id", "string")
            ], Some("LaneClaimRequest"), true)
        },
        "/v1/anchors": {
            "get": openapi_operation("anchorList", "List anchors", "List durable line anchors.", vec![], None, true),
            "post": openapi_operation("anchorCreate", "Create anchor", "Create a durable line anchor.", vec![], Some("AnchorCreateRequest"), true)
        },
        "/v1/anchors/{anchor_id}": {
            "get": openapi_operation("anchorResolve", "Resolve anchor", "Resolve a durable line anchor.", vec![
                openapi_path_param("anchor_id", "string"),
                openapi_query("branch", "string")
            ], None, true),
            "delete": openapi_operation("anchorDelete", "Delete anchor", "Delete a durable line anchor.", vec![
                openapi_path_param("anchor_id", "string")
            ], None, true)
        },
        "/v1/merge-queue": {
            "get": openapi_operation("mergeQueueList", "List merge queue", "List merge queue entries.", vec![], None, true),
            "post": openapi_operation("mergeQueueAdd", "Queue merge", "Queue a lane or branch for serialized merge.", vec![], Some("MergeQueueAddRequest"), true)
        },
        "/v1/merge-queue/run": {
            "post": openapi_operation("mergeQueueRun", "Run merge queue", "Run queued merges serially.", vec![], Some("MergeQueueRunRequest"), true)
        },
        "/v1/merge-queue/explain": {
            "get": openapi_operation("mergeQueueExplainByQuery", "Explain merge queue entry", "Explain why a queued merge is ready or blocked.", vec![
                openapi_query("selector", "string")
            ], None, true)
        },
        "/v1/merge-queue/{selector}": {
            "delete": openapi_operation("mergeQueueRemove", "Remove queue entry", "Cancel a queued or conflicted merge queue entry.", vec![
                openapi_path_param("selector", "string")
            ], None, true)
        },
        "/v1/merge-queue/{selector}/explain": {
            "get": openapi_operation("mergeQueueExplain", "Explain merge queue entry", "Explain why a queued merge is ready or blocked.", vec![
                openapi_path_param("selector", "string")
            ], None, true)
        },
        "/v1/conflicts": {
            "get": openapi_operation("conflictList", "List conflicts", "List structured conflict sets.", vec![], None, true)
        },
        "/v1/conflicts/{conflict_set_id}": {
            "get": openapi_operation_with_response_schema("conflictShow", "Show conflict", "Show one structured conflict set with deterministic explanation evidence.", vec![
                openapi_path_param("conflict_set_id", "string"),
                openapi_query("limit", "integer")
            ], None, "ConflictSetSummary", true)
        },
        "/v1/conflicts/{conflict_set_id}/resolve": {
            "post": openapi_operation("conflictResolve", "Resolve conflict", "Resolve a conflict by taking source, target, or manual content.", vec![
                openapi_path_param("conflict_set_id", "string")
            ], Some("ConflictResolveRequest"), true)
        },
        "/v1/lanes/{lane}/merge": {
            "post": openapi_operation("laneMerge", "Merge lane", "Dry-run a lane merge or explicitly direct-merge this lane into the request target branch.", vec![
                openapi_path_param("lane", "string")
            ], Some("LaneMergeRequest"), true)
        }
    })
}
