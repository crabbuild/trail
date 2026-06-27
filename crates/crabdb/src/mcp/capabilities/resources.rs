use super::*;

pub(crate) fn resources_list_result() -> Value {
    json!({
        "resources": resources(),
        "ttlMs": 300000,
        "cacheScope": "public"
    })
}

pub(crate) fn resources_templates_list_result() -> Value {
    json!({
        "resourceTemplates": resource_templates()
    })
}

pub(crate) fn resource_templates() -> Value {
    json!([
        {
            "uriTemplate": RESOURCE_LANE_TEMPLATE,
            "name": "lane",
            "title": "Lane Details",
            "description": "Read one lane record and branch state by lane name or id.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_LANE_STATUS_TEMPLATE,
            "name": "lane-status",
            "title": "Lane Status",
            "description": "Read one lane's branch, workdir, queue, and latest gate status.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_LANE_REVIEW_TEMPLATE,
            "name": "lane-review",
            "title": "Lane Review Packet",
            "description": "Read one compact review packet with readiness, evidence summaries, gates, approvals, conflicts, operations, and next steps.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_LANE_CONTRIBUTION_TEMPLATE,
            "name": "lane-contribution",
            "title": "Lane Contribution",
            "description": "Read one review bundle for a lane: status, changed paths, operations, sessions, events, and approvals.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_LANE_GATES_TEMPLATE,
            "name": "lane-gates",
            "title": "Lane Gate History",
            "description": "Read recent test/eval gate results for one lane.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_LANE_READINESS_TEMPLATE,
            "name": "lane-readiness",
            "title": "Lane Readiness",
            "description": "Read one merge-readiness report with blockers, warnings, conflicts, approvals, and latest gates.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_LANE_HANDOFF_TEMPLATE,
            "name": "lane-handoff",
            "title": "Lane Handoff",
            "description": "Read one transfer packet with readiness, current session context, recent events, spans, operations, and next steps.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_LANE_DIFF_TEMPLATE,
            "name": "lane-diff",
            "title": "Lane Diff",
            "description": "Read one lane branch diff summary without unified patches.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_SESSION_TEMPLATE,
            "name": "session",
            "title": "Lane Session",
            "description": "Read one durable lane session with turns, messages, events, and operations.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_TURN_TEMPLATE,
            "name": "turn",
            "title": "Lane Turn",
            "description": "Read one durable lane turn with messages, events, and operations.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_CONFLICT_TEMPLATE,
            "name": "conflict",
            "title": "Conflict Set",
            "description": "Read one structured merge conflict set.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_APPROVAL_TEMPLATE,
            "name": "approval",
            "title": "Approval Gate",
            "description": "Read one durable human approval gate.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_RUN_TEMPLATE,
            "name": "lane-run",
            "title": "Lane Run State",
            "description": "Read one durable paused/resumed lane run checkpoint.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_SPAN_TEMPLATE,
            "name": "trace-span",
            "title": "Trace Span",
            "description": "Read one derived lane trace span.",
            "mimeType": "application/json"
        }
    ])
}

pub(crate) fn resources() -> Value {
    json!([
        {
            "uri": RESOURCE_STATUS,
            "name": "status",
            "title": "Workspace Status",
            "description": "Current branch, worktree state, and changed paths.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_DOCTOR,
            "name": "doctor",
            "title": "Workspace Diagnostics",
            "description": "Read-only operational health checks for the CrabDB workspace.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_LANES,
            "name": "lanes",
            "title": "Lanes",
            "description": "Current lane branches and lifecycle metadata.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_MERGE_QUEUE,
            "name": "merge-queue",
            "title": "Merge Queue",
            "description": "Current serialized merge queue entries.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_CONFLICTS,
            "name": "conflicts",
            "title": "Open Conflicts",
            "description": "Structured merge conflict sets known to the workspace.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_OPENAPI,
            "name": "openapi",
            "title": "OpenAPI Contract",
            "description": "The local CrabDB HTTP API OpenAPI 3.1 document.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_USER_GUIDE,
            "name": "user-guide",
            "title": "CrabDB User Guide",
            "description": "End-user guide for common CrabDB workflows.",
            "mimeType": "text/markdown"
        },
        {
            "uri": RESOURCE_LANE_WORKFLOWS,
            "name": "lane-workflows",
            "title": "CrabDB Lane Workflows",
            "description": "Guide for multi-lane coordinators and MCP hosts.",
            "mimeType": "text/markdown"
        },
        {
            "uri": RESOURCE_CLI_REFERENCE,
            "name": "cli-reference",
            "title": "CrabDB CLI Reference",
            "description": "Command reference for the CrabDB CLI and local API surfaces.",
            "mimeType": "text/markdown"
        }
    ])
}
