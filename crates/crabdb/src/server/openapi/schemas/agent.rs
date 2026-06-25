use serde_json::{json, Value};

pub(super) fn agent_schemas() -> Value {
    json!({
        "SpawnAgentRequest": {
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": { "type": "string" },
                "from": { "type": "string" },
                "from_ref": { "type": "string" },
                "branch": { "type": "string" },
                "materialize": { "type": "boolean" },
                "workdir": { "type": "string" },
                "workdir_path": { "type": "string" },
                "paths": { "type": "array", "items": { "type": "string" } },
                "include_neighbors": { "type": "boolean" },
                "include_neighborhood": { "type": "boolean" },
                "provider": { "type": "string" },
                "model": { "type": "string" }
            }
        },
        "BeginTurnRequest": {
            "type": "object",
            "required": ["agent"],
            "properties": {
                "agent": { "type": "string" },
                "branch": { "type": "string" },
                "session_title": { "type": "string" },
                "base_change": { "type": "string" }
            }
        },
        "AddMessageRequest": {
            "type": "object",
            "required": ["role"],
            "properties": {
                "role": { "type": "string" },
                "content": { "type": "string" },
                "text": { "type": "string" }
            }
        },
        "AddEventRequest": {
            "type": "object",
            "required": ["event_type"],
            "properties": {
                "event_type": { "type": "string" },
                "type": { "type": "string" },
                "payload": { "type": "object", "additionalProperties": true },
                "change_id": { "type": "string" },
                "message_id": { "type": "string" }
            }
        },
        "StartSpanRequest": {
            "type": "object",
            "required": ["span_type", "name"],
            "properties": {
                "span_type": { "type": "string" },
                "type": { "type": "string" },
                "name": { "type": "string" },
                "parent": { "type": "string" },
                "parent_span_id": { "type": "string" },
                "trace": { "type": "string" },
                "trace_id": { "type": "string" },
                "attributes": { "type": "object", "additionalProperties": true },
                "attributes_json": { "type": "object", "additionalProperties": true }
            }
        },
        "EndSpanRequest": {
            "type": "object",
            "properties": {
                "status": { "type": "string" },
                "result": { "type": "object", "additionalProperties": true },
                "result_json": { "type": "object", "additionalProperties": true }
            }
        },
        "EndTurnRequest": {
            "type": "object",
            "properties": {
                "status": { "type": "string", "enum": ["completed", "failed", "cancelled", "archived"] }
            }
        },
        "AgentRunPauseRequest": {
            "type": "object",
            "required": ["agent", "reason", "summary"],
            "properties": {
                "agent": { "type": "string" },
                "reason": { "type": "string" },
                "summary": { "type": "string" },
                "state": { "type": "object", "additionalProperties": true },
                "interruption": { "type": "object", "additionalProperties": true },
                "session_id": { "type": "string" },
                "turn_id": { "type": "string" },
                "turn": { "type": "string" }
            }
        },
        "AgentRunResumeRequest": {
            "type": "object",
            "properties": {
                "reviewer": { "type": "string" },
                "note": { "type": "string" }
            }
        },
        "AgentTestRequest": {
            "type": "object",
            "required": ["command"],
            "properties": {
                "command": { "type": "array", "items": { "type": "string" } },
                "turn_id": { "type": "string" },
                "turn": { "type": "string" },
                "timeout_secs": { "type": "integer", "minimum": 1 },
                "timeout_seconds": { "type": "integer", "minimum": 1 },
                "suite": { "type": "string" },
                "score": { "type": "number" },
                "threshold": { "type": "number" }
            }
        },
        "AgentReadFileRequest": {
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": { "type": "string" },
                "hydrate": { "type": "boolean" },
                "force": { "type": "boolean" },
                "include_neighbors": { "type": "boolean" },
                "include_neighborhood": { "type": "boolean" }
            }
        },
        "SyncWorkdirRequest": {
            "type": "object",
            "properties": {
                "force": { "type": "boolean" },
                "paths": { "type": "array", "items": { "type": "string" } },
                "include_neighbors": { "type": "boolean" },
                "include_neighborhood": { "type": "boolean" }
            }
        },
        "AgentRecordRequest": {
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            }
        }
    })
}
