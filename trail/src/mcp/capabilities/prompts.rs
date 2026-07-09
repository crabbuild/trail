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
            "name": PROMPT_LANE_TASK,
            "title": "Run a Trail Lane Task",
            "description": "Guide an MCP host through a safe Trail lane task with turn tracking, patching, gates, and merge handoff.",
            "arguments": [
                {
                    "name": "lane",
                    "description": "Lane branch name to use or create.",
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
            "name": PROMPT_REVIEW_LANE,
            "title": "Review a Trail Lane",
            "description": "Guide a host through reviewing a lane branch before merge.",
            "arguments": [
                {
                    "name": "lane",
                    "description": "Lane branch name or id to review.",
                    "required": true
                }
            ]
        },
        {
            "name": PROMPT_RESOLVE_CONFLICT,
            "title": "Resolve a Trail Conflict",
            "description": "Guide a host through inspecting and resolving a structured Trail merge conflict.",
            "arguments": [
                {
                    "name": "conflict_set_id",
                    "description": "Conflict set id from Trail.",
                    "required": true
                }
            ]
        },
        {
            "name": PROMPT_REVIEW_AGENT,
            "title": "Review a Trail Agent Task",
            "description": "Guide a host through reviewing an agent task using the high-level agent tools.",
            "arguments": [
                {
                    "name": "selector",
                    "description": "Agent task, lane, session, ACP session, or latest. Defaults to latest.",
                    "required": false
                }
            ]
        },
        {
            "name": PROMPT_RECOVER_AGENT,
            "title": "Recover a Trail Agent Task",
            "description": "Guide a host through safe agent undo/rewind using friendly checkpoint targets.",
            "arguments": [
                {
                    "name": "selector",
                    "description": "Agent task, lane, session, ACP session, or latest. Defaults to latest.",
                    "required": false
                }
            ]
        },
        {
            "name": PROMPT_APPLY_AGENT,
            "title": "Apply a Trail Agent Task",
            "description": "Guide a host through testing, dry-run apply, and confirmed safe apply for an agent task.",
            "arguments": [
                {
                    "name": "selector",
                    "description": "Agent task, lane, session, ACP session, or latest. Defaults to latest.",
                    "required": false
                }
            ]
        }
    ])
}
