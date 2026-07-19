use serde_json::{json, Value};

use super::{
    openapi_operation, openapi_operation_with_response_schema, openapi_path_param, openapi_query,
    openapi_required_query,
};

pub(super) fn lane_paths() -> Value {
    json!({
        "/v1/environment/adapters": {
            "get": openapi_operation_with_response_schema("environmentAdapterCatalog", "Workspace environment adapters", "List registered adapters and their side-effect-free discovery metadata, provenance, and stability.", vec![], None, "EnvironmentAdapterCatalogReport", true)
        },
        "/v1/lanes": {
            "get": openapi_operation("laneList", "List lanes", "List lane branches with metadata and branch state.", vec![], None, true),
            "post": lane_spawn_operation()
        },
        "/v1/lanes/{lane_or_id}": {
            "get": openapi_operation("laneShow", "Show lane", "Show lane metadata and branch state.", vec![
                openapi_path_param("lane_or_id", "string")
            ], None, true),
            "delete": openapi_operation("laneRemove", "Remove lane", "Remove a lane branch and its materialized workdir. Requires force when the branch has unmerged changes.", vec![
                openapi_path_param("lane_or_id", "string"),
                openapi_query("force", "boolean")
            ], None, true)
        },
        "/v1/lanes/{lane_or_id}/status": {
            "get": openapi_operation("laneStatus", "Lane status", "Show a lane branch status.", vec![
                openapi_path_param("lane_or_id", "string")
            ], None, true)
        },
        "/v1/lanes/{lane_or_id}/repair-initialization": {
            "post": openapi_operation_with_response_schema("laneRepairInitialization", "Repair lane initialization", "Validate and idempotently finish a committed lane initialization.", vec![
                openapi_path_param("lane_or_id", "string")
            ], None, "LaneSpawnReport", true)
        },
        "/v1/lanes/{lane_or_id}/review": {
            "get": openapi_operation_with_response_schema("laneReview", "Lane review packet", "Produce a compact review packet for one lane branch with readiness, evidence summaries, gates, approvals, conflicts, operations, and next steps.", vec![
                openapi_path_param("lane_or_id", "string"),
                openapi_query("limit", "integer")
            ], None, "LaneReviewPacketReport", true)
        },
        "/v1/lanes/{lane_or_id}/contribution": {
            "get": openapi_operation("laneContribution", "Lane contribution", "Summarize a lane branch for review with status, changed paths, operations, sessions, events, and approvals.", vec![
                openapi_path_param("lane_or_id", "string"),
                openapi_query("limit", "integer")
            ], None, true)
        },
        "/v1/lanes/{lane_or_id}/gates": {
            "get": openapi_operation("laneGates", "Lane gate history", "List recent durable test/eval gate results for one lane branch.", vec![
                openapi_path_param("lane_or_id", "string"),
                openapi_query("kind", "string"),
                openapi_query("limit", "integer")
            ], None, true)
        },
        "/v1/lanes/{lane_or_id}/readiness": {
            "get": openapi_operation("laneReadiness", "Lane readiness", "Assess whether a lane branch is ready to merge by checking conflicts, approvals, workdir state, tests, and evals.", vec![
                openapi_path_param("lane_or_id", "string")
            ], None, true)
        },
        "/v1/lanes/{lane_or_id}/refresh-preview": {
            "get": openapi_operation_with_response_schema("laneRefreshPreview", "Lane refresh preview", "Preview refreshing a lane branch onto a target branch before merge.", vec![
                openapi_path_param("lane_or_id", "string"),
                openapi_query("target", "string")
            ], None, "LaneRefreshPreviewReport", true)
        },
        "/v1/lanes/{lane_or_id}/update": {
            "post": openapi_operation("laneUpdate", "Update layered lane", "Three-way merge a source branch into a clean, unmounted layered lane and advance its pinned view generation.", vec![
                openapi_path_param("lane_or_id", "string")
            ], Some("LaneUpdateRequest"), true)
        },
        "/v1/lanes/{lane_or_id}/handoff": {
            "get": openapi_operation("laneHandoff", "Lane handoff", "Package lane branch, readiness, current session context, recent events, spans, operations, and next steps for transfer to another lane or reviewer.", vec![
                openapi_path_param("lane_or_id", "string"),
                openapi_query("limit", "integer")
            ], None, true)
        },
        "/v1/lanes/{lane_or_id}/workdir": {
            "get": openapi_operation("laneWorkdir", "Lane workdir", "Return the materialized workdir path for a lane, if one exists.", vec![
                openapi_path_param("lane_or_id", "string")
            ], None, true)
        },
        "/v1/lanes/{lane_or_id}/workspace": {
            "get": openapi_operation("laneWorkspace", "Lane workspace view", "Return the persisted layered workspace view and mount/checkpoint state.", vec![
                openapi_path_param("lane_or_id", "string")
            ], None, true)
        },
        "/v1/lanes/{lane_or_id}/space": {
            "get": openapi_operation("laneWorkspaceSpace", "Lane workspace space", "Report shared and lane-exclusive workspace-view storage.", vec![
                openapi_path_param("lane_or_id", "string")
            ], None, true)
        },
        "/v1/lanes/{lane_or_id}/mount": {
            "post": openapi_operation("laneWorkspaceMount", "Mount lane workspace", "Start a daemon-owned layered mount worker and return its mount report.", vec![
                openapi_path_param("lane_or_id", "string")
            ], None, true)
        },
        "/v1/lanes/{lane_or_id}/unmount": {
            "post": openapi_operation("laneWorkspaceUnmount", "Unmount lane workspace", "Ask the active mount owner to release the native backend and lease safely.", vec![
                openapi_path_param("lane_or_id", "string")
            ], None, true)
        },
        "/v1/lanes/{lane_or_id}/dependencies": {
            "get": openapi_operation("laneDependencyStatus", "Dependency environment status", "Return expected, attached, ready, stale, building, or failed dependency environments.", vec![
                openapi_path_param("lane_or_id", "string")
            ], None, true)
        },
        "/v1/lanes/{lane_or_id}/dependencies/sync": {
            "post": openapi_operation("laneDependencySync", "Synchronize dependencies", "Build or reuse a frozen dependency layer, then bulk-replace private dependency state in an unmounted lane.", vec![
                openapi_path_param("lane_or_id", "string")
            ], Some("DependencySyncRequest"), true)
        },
        "/v1/lanes/{lane_or_id}/environment": {
            "get": openapi_operation_with_response_schema("laneEnvironmentStatus", "Workspace environment status", "Return normalized logical component and versioned adapter state for a layered lane.", vec![
                openapi_path_param("lane_or_id", "string")
            ], None, "EnvironmentComponentStateReportList", true)
        },
        "/v1/lanes/{lane_or_id}/environment/discover": {
            "get": openapi_operation_with_response_schema("laneEnvironmentDiscover", "Discover workspace environments", "Detect built-in environment components without executing tools, network providers, or repository code.", vec![
                openapi_path_param("lane_or_id", "string"),
                openapi_query("path", "string")
            ], None, "EnvironmentDiscoveryReport", true)
        },
        "/v1/lanes/{lane_or_id}/environment/graph": {
            "get": openapi_operation_with_response_schema("laneEnvironmentGraph", "Desired environment graph", "Return the validated component DAG, deterministic topological order, output ownership, component keys, and ordering/invalidation edges without executing tools or mutating state.", vec![
                openapi_path_param("lane_or_id", "string"),
                openapi_query("path", "string"),
                openapi_query("offset", "integer"),
                openapi_query("limit", "integer")
            ], None, "EnvironmentGraphReport", true)
        },
        "/v1/lanes/{lane_or_id}/environment/generation": {
            "get": openapi_operation_with_response_schema("laneEnvironmentGeneration", "Active environment generation", "Return the exact source root, component keys, layers, mounts, and predecessor atomically active for a lane.", vec![
                openapi_path_param("lane_or_id", "string")
            ], None, "EnvironmentGenerationReportNullable", true)
        },
        "/v1/lanes/{lane_or_id}/environment/explain": {
            "get": openapi_operation_with_response_schema("laneEnvironmentExplain", "Explain environment staleness", "Return every canonical input, tool, platform, and policy edge that differs from the attached component artifact.", vec![
                openapi_path_param("lane_or_id", "string"),
                openapi_required_query("component", "string"),
                openapi_query("offset", "integer"),
                openapi_query("limit", "integer")
            ], None, "EnvironmentStaleExplanationReport", true)
        },
        "/v1/lanes/{lane_or_id}/environment/plan": {
            "get": openapi_operation_with_response_schema("laneEnvironmentPlan", "Plan workspace environment", "Return the normalized key, inputs, argv actions, output, and capability grants without executing or mutating state.", vec![
                openapi_path_param("lane_or_id", "string"),
                openapi_query("adapter", "string"),
                openapi_query("component", "string"),
                openapi_query("path", "string")
            ], None, "EnvironmentPlanReport", true)
        },
        "/v1/lanes/{lane_or_id}/environment/sync": {
            "post": openapi_operation_with_response_schema("laneEnvironmentSync", "Synchronize workspace environment", "Prepare one adapter-owned environment component and atomically activate its shared and/or writable-private outputs for an unmounted lane.", vec![
                openapi_path_param("lane_or_id", "string")
            ], Some("EnvironmentSyncRequest"), "EnvironmentSyncReport", true)
        },
        "/v1/lanes/{lane_or_id}/environment/sync-all": {
            "post": openapi_operation_with_response_schema("laneEnvironmentSyncAll", "Synchronize all workspace environments", "Build all discovered components before atomically activating one complete generation.", vec![
                openapi_path_param("lane_or_id", "string")
            ], Some("DependencySyncRequest"), "EnvironmentSyncReport", true)
        },
        "/v1/lanes/{lane_or_id}/environment/runtime/status": {
            "get": openapi_operation_with_response_schema("laneEnvironmentRuntimeStatus", "Environment runtime status", "Return persisted container, network, volume, port, lifecycle, and health state without contacting the runtime provider.", vec![
                openapi_path_param("lane_or_id", "string")
            ], None, "EnvironmentGenerationReportNullable", true)
        },
        "/v1/lanes/{lane_or_id}/environment/runtime/reconcile": {
            "post": openapi_operation_with_response_schema("laneEnvironmentRuntimeReconcile", "Reconcile environment runtime", "Idempotently create or adopt declared lane-private OCI resources and wait for health.", vec![
                openapi_path_param("lane_or_id", "string")
            ], None, "EnvironmentGenerationReport", true)
        },
        "/v1/lanes/{lane_or_id}/environment/runtime/stop": {
            "post": openapi_operation_with_response_schema("laneEnvironmentRuntimeStop", "Stop environment runtime", "Stop active Trail-owned containers while retaining private networks and volumes for restart.", vec![
                openapi_path_param("lane_or_id", "string")
            ], None, "EnvironmentGenerationReport", true)
        },
        "/v1/lanes/{lane_or_id}/checkpoint": {
            "post": openapi_operation("laneWorkspaceCheckpoint", "Checkpoint lane workspace", "Checkpoint source-upper mutations into the lane ref under a mutation barrier.", vec![
                openapi_path_param("lane_or_id", "string")
            ], Some("WorkspaceCheckpointRequest"), true)
        },
        "/v1/lanes/{lane_or_id}/exec": {
            "post": openapi_operation("laneWorkspaceExec", "Execute in lane workspace", "Mount the layered lane for one open-world command with isolated cache and target variables.", vec![
                openapi_path_param("lane_or_id", "string")
            ], Some("WorkspaceExecRequest"), true)
        },
        "/v1/lanes/{lane_or_id}/diff": {
            "get": openapi_operation("laneDiff", "Lane diff", "Show the diff from a lane branch base to head.", vec![
                openapi_path_param("lane_or_id", "string"),
                openapi_query("patch", "boolean"),
                openapi_query("show_line_ids", "boolean"),
                openapi_query("show-line-ids", "boolean")
            ], None, true)
        },
        "/v1/lanes/{lane_or_id}/read-file": {
            "post": openapi_operation("laneReadFile", "Read lane file", "Read one file from a lane branch. Sparse workdirs hydrate lazily when hydrate is omitted; pass hydrate=false for a side-effect-free read.", vec![
                openapi_path_param("lane_or_id", "string")
            ], Some("LaneReadFileRequest"), true)
        },
        "/v1/lanes/{lane_or_id}/sync-workdir": {
            "post": openapi_operation("laneSyncWorkdir", "Sync lane workdir", "Refresh a materialized lane workdir.", vec![
                openapi_path_param("lane_or_id", "string")
            ], Some("SyncWorkdirRequest"), true)
        },
        "/v1/lanes/{lane_or_id}/hydrate": {
            "post": openapi_operation("laneHydrateWorkdir", "Hydrate lane workdir paths", "Hydrate selected paths into a sparse lane workdir before editing.", vec![
                openapi_path_param("lane_or_id", "string")
            ], Some("SyncWorkdirRequest"), true)
        },
        "/v1/lanes/{lane_or_id}/record": {
            "post": openapi_operation_with_response_schema("laneRecordWorkdir", "Record lane workdir", "Record materialized lane workdir changes into the lane branch, or return a non-mutating preview when preview=true.", vec![
                openapi_path_param("lane_or_id", "string")
            ], Some("LaneRecordRequest"), "LaneRecordWorkdirResponse", true)
        },
        "/v1/lanes/{lane_or_id}/rewind": {
            "post": openapi_operation("laneRewind", "Rewind lane", "Rewind a lane branch to a known-good change or root, optionally preserving the current head and syncing its workdir.", vec![
                openapi_path_param("lane_or_id", "string")
            ], Some("LaneRewindRequest"), true)
        },
        "/v1/lanes/{lane_or_id}/tests": {
            "post": openapi_operation("laneRunTest", "Run lane test", "Run a command in a lane workdir and record test events.", vec![
                openapi_path_param("lane_or_id", "string")
            ], Some("LaneTestRequest"), true)
        },
        "/v1/lanes/{lane_or_id}/evals": {
            "post": openapi_operation("laneRunEval", "Run lane eval", "Run an evaluation command in a lane workdir and record eval events.", vec![
                openapi_path_param("lane_or_id", "string")
            ], Some("LaneTestRequest"), true)
        },
        "/v1/lanes/{lane_or_id}/patches": {
            "post": openapi_operation_with_response_schema("laneApplyPatch", "Apply lane patch", "Apply a patch directly to a lane branch.", vec![
                openapi_path_param("lane_or_id", "string")
            ], Some("PatchRequest"), "LanePatchReport", true)
        }
    })
}

fn lane_spawn_operation() -> Value {
    let mut operation = openapi_operation_with_response_schema(
        "laneSpawn",
        "Spawn lane",
        "Create or resume a lane branch. First completion returns 201, replay returns 200, and committed repair remains a structured 409.",
        vec![],
        Some("SpawnLaneRequest"),
        "LaneSpawnReport",
        true,
    );
    operation["responses"]["201"] = operation["responses"]["200"].clone();
    operation["responses"]["201"]["description"] =
        Value::String("Lane initialization completed for the first time".into());
    operation["responses"]["409"] = json!({ "$ref": "#/components/responses/Error" });
    operation
}
