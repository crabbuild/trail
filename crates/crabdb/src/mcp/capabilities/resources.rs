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
            "uriTemplate": RESOURCE_AGENT_SUMMARY_TEMPLATE,
            "name": "agent-task-summary",
            "title": "Agent Task Summary",
            "description": "Read the one-page post-run cockpit with readiness, risk, validation, receipt Markdown, PR draft, and next commands.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_DIAGNOSE_TEMPLATE,
            "name": "agent-task-diagnose",
            "title": "Agent Task Diagnose",
            "description": "Read a recovery-oriented diagnosis with likely issue, evidence, recovery targets, and safe next commands.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_TEST_PLAN_TEMPLATE,
            "name": "agent-task-test-plan",
            "title": "Agent Task Test Plan",
            "description": "Read the prioritized test/eval checklist for an agent task.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_CONFIDENCE_TEMPLATE,
            "name": "agent-task-confidence",
            "title": "Agent Task Confidence",
            "description": "Read one go/no-go verdict from review freshness, validation, risk, and apply preflight.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_REVIEW_MAP_TEMPLATE,
            "name": "agent-task-review-map",
            "title": "Agent Task Review Map",
            "description": "Read the file-by-file review checklist grouped by changed area.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_REVIEW_TEMPLATE,
            "name": "agent-task-review",
            "title": "Agent Task Review",
            "description": "Read the agent task review dashboard with readiness, risk, prioritized files, blockers, warnings, and next commands.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_REVIEW_DATA_TEMPLATE,
            "name": "agent-task-review-data",
            "title": "Agent Review Data",
            "description": "Read one editor-friendly review packet with file review progress, focus file, changes by file, confidence, validation, risk, and readiness.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_CHANGES_TEMPLATE,
            "name": "agent-task-changes",
            "title": "Agent Task Changes",
            "description": "Read high-level agent change cards plus turn or operation checkpoint groups.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_TIMELINE_TEMPLATE,
            "name": "agent-task-timeline",
            "title": "Agent Task Timeline",
            "description": "Read the chronological prompt/operation timeline with checkpoints, tools, changed files, and follow-up commands.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_FILES_TEMPLATE,
            "name": "agent-task-files",
            "title": "Agent Task Files",
            "description": "Read changed files with the turns, prompts, checkpoints, and focused diff commands behind each file.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_REPORT_TEMPLATE,
            "name": "agent-task-report",
            "title": "Agent Task Report",
            "description": "Read the shareable agent task report bundle with story, risk, readiness, transcript, suggestions, and Markdown.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_RECEIPT_TEMPLATE,
            "name": "agent-task-receipt",
            "title": "Agent Task Receipt",
            "description": "Read the copyable post-run receipt with summary, validation, changed files, turns, risk, checkpoint, and next command.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_HANDOFF_TEMPLATE,
            "name": "agent-task-handoff",
            "title": "Agent Task Handoff",
            "description": "Read the copyable handoff packet for another human or agent.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_PR_TEMPLATE,
            "name": "agent-task-pr",
            "title": "Agent PR Draft",
            "description": "Read a pull request draft title and body for an agent task without creating a remote PR.",
            "mimeType": "application/json"
        },
        {
            "uriTemplate": RESOURCE_AGENT_FOCUS_TEMPLATE,
            "name": "agent-task-focus",
            "title": "Agent Task Focus",
            "description": "Read the next file to inspect with its review priority, explanation, and focused diff summary.",
            "mimeType": "application/json"
        },
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
            "uri": RESOURCE_AGENT_INBOX,
            "name": "agent-tasks",
            "title": "Agent Task Inbox",
            "description": "Grouped agent tasks and the one next useful action.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_AGENT_LATEST_SUMMARY,
            "name": "latest-agent-summary",
            "title": "Latest Agent Summary",
            "description": "One-page post-run cockpit for the latest agent task.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_AGENT_LATEST_DIAGNOSE,
            "name": "latest-agent-diagnose",
            "title": "Latest Agent Diagnose",
            "description": "Recovery-oriented diagnosis for the latest agent task.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_AGENT_LATEST_TEST_PLAN,
            "name": "latest-agent-test-plan",
            "title": "Latest Agent Test Plan",
            "description": "Prioritized test/eval checklist for the latest agent task.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_AGENT_LATEST_CONFIDENCE,
            "name": "latest-agent-confidence",
            "title": "Latest Agent Confidence",
            "description": "Go/no-go verdict for the latest agent task.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_AGENT_LATEST_REVIEW_MAP,
            "name": "latest-agent-review-map",
            "title": "Latest Agent Review Map",
            "description": "File-by-file review checklist grouped by changed area for the latest agent task.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_AGENT_LATEST_REVIEW,
            "name": "latest-agent-review",
            "title": "Latest Agent Review",
            "description": "Review dashboard for the latest agent task.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_AGENT_LATEST_REVIEW_DATA,
            "name": "latest-agent-review-data",
            "title": "Latest Agent Review Data",
            "description": "Editor-friendly review packet for the latest agent task.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_AGENT_LATEST_CHANGES,
            "name": "latest-agent-changes",
            "title": "Latest Agent Changes",
            "description": "High-level change cards for the latest agent task.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_AGENT_LATEST_TIMELINE,
            "name": "latest-agent-timeline",
            "title": "Latest Agent Timeline",
            "description": "Chronological prompt/operation timeline for the latest agent task.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_AGENT_LATEST_FILES,
            "name": "latest-agent-files",
            "title": "Latest Agent Files",
            "description": "Changed-file provenance for the latest agent task.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_AGENT_LATEST_FOCUS,
            "name": "latest-agent-focus",
            "title": "Latest Agent Focus",
            "description": "Next file to inspect for the latest agent task.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_AGENT_LATEST_RECEIPT,
            "name": "latest-agent-receipt",
            "title": "Latest Agent Receipt",
            "description": "Copyable receipt for the latest agent task.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_AGENT_LATEST_HANDOFF,
            "name": "latest-agent-handoff",
            "title": "Latest Agent Handoff",
            "description": "Copyable handoff packet for another human or agent.",
            "mimeType": "application/json"
        },
        {
            "uri": RESOURCE_AGENT_LATEST_PR,
            "name": "latest-agent-pr",
            "title": "Latest Agent PR Draft",
            "description": "Pull request draft for the latest agent task.",
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
