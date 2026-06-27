use serde_json::{json, Value};

pub(super) fn lane_schemas() -> Value {
    json!({
        "LaneReviewEvidenceSummary": {
            "type": "object",
            "required": [
                "operations",
                "sessions",
                "events",
                "spans",
                "approvals",
                "pending_approvals",
                "conflicts",
                "queued_merges",
                "gates"
            ],
            "properties": {
                "operations": { "type": "integer" },
                "sessions": { "type": "integer" },
                "events": { "type": "integer" },
                "spans": { "type": "integer" },
                "approvals": { "type": "integer" },
                "pending_approvals": { "type": "integer" },
                "conflicts": { "type": "integer" },
                "queued_merges": { "type": "integer" },
                "gates": { "type": "integer" }
            }
        },
        "LaneReviewPacketReport": {
            "type": "object",
            "required": [
                "lane",
                "readiness",
                "changed_paths",
                "workdir_state",
                "evidence_summary",
                "latest_test",
                "recent_gates",
                "recent_operations",
                "recent_sessions",
                "recent_events",
                "recent_spans",
                "recent_approvals",
                "conflicts",
                "next_steps"
            ],
            "properties": {
                "lane": { "$ref": "#/components/schemas/JsonValue" },
                "readiness": { "$ref": "#/components/schemas/JsonValue" },
                "changed_paths": { "type": "array", "items": { "$ref": "#/components/schemas/JsonValue" } },
                "workdir_state": { "$ref": "#/components/schemas/JsonValue" },
                "evidence_summary": { "$ref": "#/components/schemas/LaneReviewEvidenceSummary" },
                "latest_test": { "$ref": "#/components/schemas/JsonValue" },
                "latest_eval": { "$ref": "#/components/schemas/JsonValue" },
                "recent_gates": { "type": "array", "items": { "$ref": "#/components/schemas/JsonValue" } },
                "recent_operations": { "type": "array", "items": { "$ref": "#/components/schemas/JsonValue" } },
                "recent_sessions": { "type": "array", "items": { "$ref": "#/components/schemas/JsonValue" } },
                "recent_events": { "type": "array", "items": { "$ref": "#/components/schemas/JsonValue" } },
                "recent_spans": { "type": "array", "items": { "$ref": "#/components/schemas/JsonValue" } },
                "recent_approvals": { "type": "array", "items": { "$ref": "#/components/schemas/JsonValue" } },
                "conflicts": { "type": "array", "items": { "$ref": "#/components/schemas/JsonValue" } },
                "next_steps": { "type": "array", "items": { "type": "string" } }
            }
        },
        "SpawnLaneRequest": {
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
            "required": ["lane"],
            "properties": {
                "lane": { "type": "string" },
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
        "LaneRunPauseRequest": {
            "type": "object",
            "required": ["lane", "reason", "summary"],
            "properties": {
                "lane": { "type": "string" },
                "reason": { "type": "string" },
                "summary": { "type": "string" },
                "state": { "type": "object", "additionalProperties": true },
                "interruption": { "type": "object", "additionalProperties": true },
                "session_id": { "type": "string" },
                "turn_id": { "type": "string" },
                "turn": { "type": "string" }
            }
        },
        "LaneRunResumeRequest": {
            "type": "object",
            "properties": {
                "reviewer": { "type": "string" },
                "note": { "type": "string" }
            }
        },
        "LaneTestRequest": {
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
        "LaneReadFileRequest": {
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
        "LaneRecordRequest": {
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            }
        },
        "LaneRewindRequest": {
            "type": "object",
            "required": ["to"],
            "properties": {
                "to": { "type": "string" },
                "target": { "type": "string" },
                "record_current": { "type": "boolean" },
                "sync_workdir": { "type": "boolean" }
            }
        }
    })
}
