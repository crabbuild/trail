use serde_json::{json, Value};

pub(super) fn agent_hook_schemas() -> Value {
    json!({
        "AgentHookProviderPayload": {
            "type": "object",
            "description": "Arbitrary provider-native hook payload. Provider contracts determine its fields.",
            "additionalProperties": true
        },
        "AgentCaptureRunRequest": {
            "type": "object",
            "required": ["workdir", "owner_agent", "owner_session_id", "lease_ms"],
            "properties": {
                "lane": {"type": ["string", "null"]},
                "workdir": {"type": "string"},
                "owner_agent": {"type": "string"},
                "owner_session_id": {"type": "string"},
                "executor_agent": {"type": ["string", "null"]},
                "work_item_id": {"type": ["string", "null"]},
                "lease_ms": {"type": "integer", "minimum": 1000},
                "metadata": {"type": ["object", "null"], "additionalProperties": true}
            }
        },
        "AgentCaptureRunLeaseRequest": {
            "type": "object",
            "required": ["owner_agent", "owner_session_id"],
            "properties": {
                "owner_agent": {"type": "string"},
                "owner_session_id": {"type": "string"},
                "lease_ms": {"type": ["integer", "null"], "minimum": 1000}
            }
        },
        "AgentAttestationCreateRequest": {
            "type": "object",
            "properties": {
                "capture_policy": {"type": "string", "default": "native-agent-hooks/v1"},
                "metadata": {"type": ["object", "null"], "additionalProperties": true}
            }
        },
        "AgentLearningReviewRequest": {
            "type": "object",
            "required": ["reviewer"],
            "properties": {"reviewer": {"type": "string"}}
        },
        "AgentArtifactRedactRequest": {
            "type": "object",
            "required": ["reason"],
            "properties": {"reason": {"type": "string", "maxLength": 512}}
        },
        "AgentLearningRequest": {
            "type": "object",
            "required": ["session_id", "scope", "body"],
            "properties": {
                "session_id": {"type": "string"},
                "turn_id": {"type": ["string", "null"]},
                "scope": {"type": "string"},
                "body": {"type": "string"},
                "confidence": {"type": ["number", "null"], "minimum": 0, "maximum": 1},
                "source_artifact_id": {"type": ["string", "null"]},
                "anchor": {"type": ["object", "null"], "additionalProperties": true},
                "expires_at": {"type": ["integer", "null"]},
                "metadata": {"type": ["object", "null"], "additionalProperties": true}
            }
        },
        "AgentGitLinkRequest": {
            "type": "object",
            "required": ["session_id", "git_commit", "confidence", "source"],
            "properties": {
                "session_id": {"type": "string"},
                "turn_id": {"type": ["string", "null"]},
                "git_commit": {"type": "string"},
                "from_change": {"type": ["string", "null"]},
                "through_change": {"type": ["string", "null"]},
                "confidence": {"type": "string"},
                "source": {"type": "string"},
                "metadata": {"type": ["object", "null"], "additionalProperties": true}
            }
        }
    })
}
