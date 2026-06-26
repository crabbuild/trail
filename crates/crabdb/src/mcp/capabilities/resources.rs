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
            "uriTemplate": RESOURCE_AGENT_TEMPLATE,
            "name": "agent",
            "title": "Agent Details",
            "description": "Read one agent record and branch state by agent name or id.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_STATUS_TEMPLATE,
            "name": "agent-status",
            "title": "Agent Status",
            "description": "Read one agent's branch, workdir, queue, and latest gate status.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_REVIEW_TEMPLATE,
            "name": "agent-review",
            "title": "Agent Review Packet",
            "description": "Read one compact review packet with readiness, evidence summaries, gates, approvals, conflicts, operations, and next steps.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_CONTRIBUTION_TEMPLATE,
            "name": "agent-contribution",
            "title": "Agent Contribution",
            "description": "Read one review bundle for an agent: status, changed paths, operations, sessions, events, and approvals.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_GATES_TEMPLATE,
            "name": "agent-gates",
            "title": "Agent Gate History",
            "description": "Read recent test/eval gate results for one agent.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_READINESS_TEMPLATE,
            "name": "agent-readiness",
            "title": "Agent Readiness",
            "description": "Read one merge-readiness report with blockers, warnings, conflicts, approvals, and latest gates.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_HANDOFF_TEMPLATE,
            "name": "agent-handoff",
            "title": "Agent Handoff",
            "description": "Read one transfer packet with readiness, current session context, recent events, spans, operations, and next steps.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_DIFF_TEMPLATE,
            "name": "agent-diff",
            "title": "Agent Diff",
            "description": "Read one agent branch diff summary without unified patches.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_SESSION_TEMPLATE,
            "name": "session",
            "title": "Agent Session",
            "description": "Read one durable agent session with turns, messages, events, and operations.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_TURN_TEMPLATE,
            "name": "turn",
            "title": "Agent Turn",
            "description": "Read one durable agent turn with messages, events, and operations.",
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
            "name": "agent-run",
            "title": "Agent Run State",
            "description": "Read one durable paused/resumed agent run checkpoint.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_SPAN_TEMPLATE,
            "name": "trace-span",
            "title": "Trace Span",
            "description": "Read one derived agent trace span.",
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
            "uri": RESOURCE_AGENTS,
            "name": "agents",
            "title": "Agents",
            "description": "Current agent branches and lifecycle metadata.",
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
            "uri": RESOURCE_AGENT_WORKFLOWS,
            "name": "agent-workflows",
            "title": "CrabDB Agent Workflows",
            "description": "Guide for multi-agent coordinators and MCP hosts.",
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
