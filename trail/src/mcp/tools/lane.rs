use serde_json::{json, Value};

use crate::mcp::response::object_schema;

pub(super) fn tools() -> Value {
    json!([
        {
            "name": "trail.lane_spawn",
            "title": "Spawn Lane Branch",
            "description": "Create or reuse an isolated lane branch, optionally materializing its workdir.",
            "inputSchema": object_schema(json!({
                "name": { "type": "string" },
                "from_ref": { "type": "string" },
                "materialize": { "type": "boolean" },
                "workdir_mode": { "type": "string", "enum": ["auto", "virtual", "sparse", "full-cow", "fuse-cow", "nfs-cow", "dokan-cow"] },
                "workdir": { "type": "string" },
                "workdir_path": { "type": "string" },
                "paths": { "type": "array", "items": { "type": "string" } },
                "include_neighbors": { "type": "boolean" },
                "include_neighborhood": { "type": "boolean" },
                "provider": { "type": "string" },
                "model": { "type": "string" }
            }), vec!["name"])
        },
        {
            "name": "trail.lane_hydrate",
            "title": "Hydrate Lane Workdir Paths",
            "description": "Hydrate selected paths into a sparse lane workdir before filesystem edits.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "paths": { "type": "array", "items": { "type": "string" }, "minItems": 1 },
                "force": { "type": "boolean" },
                "include_neighbors": { "type": "boolean" },
                "include_neighborhood": { "type": "boolean" }
            }), vec!["lane", "paths"])
        },
        {
            "name": "trail.lane_claim",
            "title": "Claim Lane Path",
            "description": "Create a soft advisory write claim for a lane path, returning conflicts as warnings instead of hard failures.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "path": { "type": "string" },
                "ttl_secs": { "type": "integer", "minimum": 1 },
                "ttl": { "type": "integer", "minimum": 1 }
            }), vec!["lane", "path"])
        },
        {
            "name": "trail.lane_list",
            "title": "List Lanes",
            "description": "List lane metadata and branch state for coordinator discovery.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "trail.lane_show",
            "title": "Show Lane",
            "description": "Show one lane's metadata and branch state by name or lane id.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.lane_status",
            "title": "Lane Status",
            "description": "Show one lane branch status, including workdir and latest test state.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.lane_review",
            "title": "Lane Review Packet",
            "description": "Produce a compact read-only review packet for one lane branch with readiness, evidence summaries, gates, approvals, conflicts, operations, and next steps.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1 }
            }), vec!["lane"])
        },
        {
            "name": "trail.lane_contribution",
            "title": "Lane Contribution",
            "description": "Summarize one lane branch for review with status, changed paths, operations, sessions, events, approvals, and latest gates.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1 }
            }), vec!["lane"])
        },
        {
            "name": "trail.gate_history",
            "title": "Lane Gate History",
            "description": "List recent durable test/eval gate results for one lane branch, optionally filtered by kind.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "kind": { "type": "string", "enum": ["all", "test", "tests", "eval", "evals"] },
                "limit": { "type": "integer", "minimum": 1 }
            }), vec!["lane"])
        },
        {
            "name": "trail.lane_readiness",
            "title": "Lane Readiness",
            "description": "Assess whether one lane branch is ready to merge by checking conflicts, approvals, workdir state, tests, and evals.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.lane_refresh_preview",
            "title": "Lane Refresh Preview",
            "description": "Preview refreshing one lane onto a target branch, including operations-behind, incoming changed paths, conflicts, and next steps, without mutating refs or recording conflict state.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "target": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.lane_update",
            "title": "Update Layered Lane",
            "description": "Three-way merge a source branch into a clean, unmounted layered lane and atomically advance its pinned view generation.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "from": { "type": "string" },
                "source": { "type": "string" },
                "checkpoint": { "type": "boolean" }
            }), vec!["lane"])
        },
        {
            "name": "trail.lane_handoff",
            "title": "Lane Handoff",
            "description": "Package one lane branch for transfer with readiness, current session context, recent events, spans, operations, and next steps.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1 }
            }), vec!["lane"])
        },
        {
            "name": "trail.lane_remove",
            "title": "Remove Lane",
            "description": "Remove a lane branch and materialized workdir. Requires force when the branch has unmerged changes.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "force": { "type": "boolean" }
            }), vec!["lane"])
        },
        {
            "name": "trail.lane_rewind",
            "title": "Rewind Lane",
            "description": "Move a lane branch back to a known-good change or root, optionally preserving the current head and syncing the materialized workdir.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "to": { "type": "string" },
                "target": { "type": "string" },
                "record_current": { "type": "boolean" },
                "sync_workdir": { "type": "boolean" }
            }), vec!["lane", "to"])
        },
        {
            "name": "trail.lane_workspace",
            "title": "Lane Workspace View",
            "description": "Show the persisted layered workspace view, backend, uppers, generation, checkpoint, and mount owner for one lane.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.lane_space",
            "title": "Lane Workspace Space",
            "description": "Report shared cache bytes and lane-exclusive source, generated, scratch, journal, and physical storage.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.lane_mount",
            "title": "Mount Lane Workspace",
            "description": "Start a daemon-owned layered mount worker and return its owner, backend, mountpoint, and generation.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.lane_unmount",
            "title": "Unmount Lane Workspace",
            "description": "Request graceful teardown from the active mount owner and wait for its lease release.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.lane_checkpoint",
            "title": "Checkpoint Lane Workspace",
            "description": "Checkpoint only durable source-upper mutations into the lane ref under a mutation barrier.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "message": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.lane_exec",
            "title": "Execute In Lane Workspace",
            "description": "Mount a layered lane for one open-world command with isolated cache and target variables.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "command": { "type": "array", "items": { "type": "string" }, "minItems": 1 }
            }), vec!["lane", "command"])
        },
        {
            "name": "trail.deps_status",
            "title": "Dependency Environment Status",
            "description": "Show expected, attached, ready, stale, building, or failed workspace environments for one lane.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.deps_sync",
            "title": "Synchronize Dependencies",
            "description": "Build or reuse a frozen dependency layer, then bulk-replace private dependency state in an unmounted lane.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "path": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.env_adapters",
            "title": "Workspace Environment Adapters",
            "description": "List registered adapters, selectors, component kinds, discovery markers, provenance, and stability without probing tools or repository code.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "trail.env_status",
            "title": "Workspace Environment Status",
            "description": "Show normalized component and versioned adapter state for one layered lane.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.env_discover",
            "title": "Discover Workspace Environments",
            "description": "Detect built-in environment components without running package managers, compilers, network providers, or repository code.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "path": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.env_graph",
            "title": "Workspace Environment Graph",
            "description": "Return the validated desired component DAG, deterministic topological order, output ownership, component keys, and ordering/invalidation edges without executing tools or mutating state.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "path": { "type": "string" },
                "offset": { "type": "integer", "minimum": 0, "default": 0 },
                "limit": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 256 }
            }), vec!["lane"])
        },
        {
            "name": "trail.env_generation",
            "title": "Active Environment Generation",
            "description": "Show the exact component keys, layers, mounts, source root, and predecessor active for one lane.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.env_explain",
            "title": "Explain Workspace Environment Staleness",
            "description": "List every canonical input, tool, platform, architecture, portability, and policy edge that differs from the attached component artifact.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "component": { "type": "string" },
                "offset": { "type": "integer", "minimum": 0, "default": 0 },
                "limit": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 256 }
            }), vec!["lane", "component"])
        },
        {
            "name": "trail.env_plan",
            "title": "Plan Workspace Environment",
            "description": "Return the normalized component key, declared inputs, argv actions, output, and capability grants without executing or mutating state.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "adapter": { "type": "string", "default": "auto" },
                "component": { "type": "string" },
                "path": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.env_sync",
            "title": "Synchronize Workspace Environment",
            "description": "Prepare one adapter-owned environment component and atomically activate its shared and/or writable-private outputs for an unmounted lane.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "adapter": { "type": "string", "default": "auto" },
                "component": { "type": "string" },
                "path": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.env_sync_all",
            "title": "Synchronize All Workspace Environments",
            "description": "Build every discovered component first, then atomically activate all mounts as one environment generation.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" },
                "path": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.env_runtime_status",
            "title": "Environment Runtime Status",
            "description": "Show persisted container, network, volume, port, lifecycle, and health state for the active lane generation without contacting a provider.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.env_runtime_reconcile",
            "title": "Reconcile Environment Runtime",
            "description": "Idempotently create or adopt declared lane-private OCI resources and wait for their health contracts.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.env_runtime_stop",
            "title": "Stop Environment Runtime",
            "description": "Stop the active generation's Trail-owned containers while retaining private networks and volumes for restart.",
            "inputSchema": object_schema(json!({
                "lane": { "type": "string" }
            }), vec!["lane"])
        },
        {
            "name": "trail.cache_list",
            "title": "List Workspace Cache",
            "description": "List immutable workspace layers and their logical and physical storage accounting.",
            "inputSchema": object_schema(json!({}), vec![])
        },
        {
            "name": "trail.cache_inspect",
            "title": "Inspect Workspace Layer",
            "description": "Inspect and integrity-check one immutable workspace layer.",
            "inputSchema": object_schema(json!({
                "layer": { "type": "string" }
            }), vec!["layer"])
        },
        {
            "name": "trail.cache_verify",
            "title": "Verify Workspace Layer",
            "description": "Verify one immutable workspace layer against its content-addressed manifest.",
            "inputSchema": object_schema(json!({
                "layer": { "type": "string" }
            }), vec!["layer"])
        },
        {
            "name": "trail.cache_gc",
            "title": "Collect Workspace Cache",
            "description": "Preview or reclaim unpinned immutable layers and rematerializable projections.",
            "inputSchema": object_schema(json!({
                "dry_run": { "type": "boolean" },
                "retention_secs": { "type": "integer", "minimum": 0 }
            }), vec![])
        }
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lane_spawn_schema_uses_the_hard_cutover_modes() {
        let declarations = tools();
        let lane_spawn = declarations
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "trail.lane_spawn")
            .unwrap();
        let modes = lane_spawn["inputSchema"]["properties"]["workdir_mode"]["enum"]
            .as_array()
            .unwrap();

        assert!(modes.iter().any(|mode| mode == "full-cow"));
        assert!(modes.iter().any(|mode| mode == "fuse-cow"));
        assert!(modes.iter().any(|mode| mode == "dokan-cow"));
        assert!(!modes.iter().any(|mode| mode == "overlay-cow"));
    }
}
