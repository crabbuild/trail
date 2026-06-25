use serde_json::{json, Value};

pub(super) fn collaboration_schemas() -> Value {
    json!({
        "MergeAgentRequest": {
            "type": "object",
            "properties": {
                "agent_id": { "type": "string" },
                "agent": { "type": "string" },
                "name": { "type": "string" },
                "strategy": { "type": "string" },
                "dry_run": { "type": "boolean" },
                "dry-run": { "type": "boolean" }
            }
        },
        "SessionStartRequest": {
            "type": "object",
            "required": ["agent"],
            "properties": {
                "agent": { "type": "string" },
                "title": { "type": "string" },
                "id": { "type": "string" }
            }
        },
        "SessionEndRequest": {
            "type": "object",
            "properties": {
                "status": { "type": "string", "enum": ["completed", "failed", "cancelled", "archived"] }
            }
        },
        "ApprovalRequest": {
            "type": "object",
            "required": ["agent", "action", "summary"],
            "properties": {
                "agent": { "type": "string" },
                "action": { "type": "string" },
                "summary": { "type": "string" },
                "payload": { "type": "object", "additionalProperties": true },
                "session_id": { "type": "string" },
                "turn_id": { "type": "string" },
                "turn": { "type": "string" }
            }
        },
        "ApprovalDecisionRequest": {
            "type": "object",
            "required": ["decision"],
            "properties": {
                "decision": { "type": "string", "enum": ["approved", "rejected", "cancelled"] },
                "reviewer": { "type": "string" },
                "note": { "type": "string" }
            }
        },
        "LeaseAcquireRequest": {
            "type": "object",
            "required": ["agent"],
            "properties": {
                "agent": { "type": "string" },
                "path": { "type": "string" },
                "mode": { "type": "string", "enum": ["read", "write"] },
                "ttl_secs": { "type": "integer", "minimum": 1 },
                "ttl": { "type": "integer", "minimum": 1 }
            }
        },
        "AgentClaimRequest": {
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": { "type": "string" },
                "ttl_secs": { "type": "integer", "minimum": 1 },
                "ttl": { "type": "integer", "minimum": 1 }
            }
        },
        "AnchorCreateRequest": {
            "type": "object",
            "required": ["path_line", "label"],
            "properties": {
                "path_line": { "type": "string" },
                "label": { "type": "string" },
                "branch": { "type": "string" }
            }
        },
        "MergeQueueAddRequest": {
            "type": "object",
            "required": ["source", "target"],
            "properties": {
                "source": { "type": "string" },
                "target": { "type": "string" },
                "into": { "type": "string" },
                "target_branch": { "type": "string" },
                "priority": { "type": "integer" }
            }
        },
        "MergeQueueRunRequest": {
            "type": "object",
            "properties": { "limit": { "type": "integer", "minimum": 1 } }
        },
        "ConflictResolveRequest": {
            "type": "object",
            "properties": {
                "take": { "type": "string", "enum": ["source", "target"] },
                "manual": {
                    "type": "object",
                    "properties": {
                        "files": {
                            "type": "object",
                            "additionalProperties": {
                                "oneOf": [
                                    { "type": "string" },
                                    {
                                        "type": "object",
                                        "properties": {
                                            "content": { "type": "string" },
                                            "delete": { "type": "boolean" },
                                            "executable": { "type": "boolean" }
                                        }
                                    }
                                ]
                            }
                        }
                    }
                }
            }
        }
    })
}
