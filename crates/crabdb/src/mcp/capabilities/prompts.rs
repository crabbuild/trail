use super::*;

pub(crate) fn prompts_list_result() -> Value {
    json!({
        "prompts": prompts(),
        "ttlMs": 300000,
        "cacheScope": "public"
    })
}

pub(crate) fn prompts() -> Value {
    json!([
        {
            "name": PROMPT_AGENT_TASK,
            "title": "Run a CrabDB Agent Task",
            "description": "Guide an MCP host through a safe CrabDB agent task with turn tracking, patching, gates, and merge handoff.",
            "arguments": [
                {
                    "name": "agent",
                    "description": "Agent branch name to use or create.",
                    "required": true
                },
                {
                    "name": "task",
                    "description": "User-visible task objective.",
                    "required": true
                },
                {
                    "name": "branch",
                    "description": "Base branch, defaulting to main.",
                    "required": false
                }
            ]
        },
        {
            "name": PROMPT_REVIEW_AGENT,
            "title": "Review a CrabDB Agent",
            "description": "Guide a host through reviewing an agent branch before merge.",
            "arguments": [
                {
                    "name": "agent",
                    "description": "Agent branch name or id to review.",
                    "required": true
                }
            ]
        },
        {
            "name": PROMPT_RESOLVE_CONFLICT,
            "title": "Resolve a CrabDB Conflict",
            "description": "Guide a host through inspecting and resolving a structured CrabDB merge conflict.",
            "arguments": [
                {
                    "name": "conflict_set_id",
                    "description": "Conflict set id from CrabDB.",
                    "required": true
                }
            ]
        }
    ])
}
