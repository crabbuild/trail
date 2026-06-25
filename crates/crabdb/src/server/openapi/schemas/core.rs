use serde_json::{json, Value};

pub(super) fn core_schemas() -> Value {
    json!({
        "JsonValue": {
            "description": "CrabDB typed JSON report. See CLI reference for the concrete report shape.",
            "oneOf": [
                { "type": "object", "additionalProperties": true },
                { "type": "array", "items": true },
                { "type": "string" },
                { "type": "number" },
                { "type": "boolean" },
                { "type": "null" }
            ]
        },
        "ErrorBody": {
            "type": "object",
            "required": ["error"],
            "properties": {
                "error": {
                    "type": "object",
                    "required": ["message", "code"],
                    "properties": {
                        "message": { "type": "string" },
                        "code": { "type": "integer" }
                    }
                }
            }
        },
        "ConfigSetRequest": {
            "type": "object",
            "required": ["key", "value"],
            "additionalProperties": false,
            "properties": {
                "key": { "type": "string" },
                "value": { "type": "string" }
            }
        },
        "IgnorePatternRequest": {
            "type": "object",
            "required": ["pattern"],
            "additionalProperties": false,
            "properties": { "pattern": { "type": "string" } }
        },
        "IgnoreCheckRequest": {
            "type": "object",
            "required": ["path"],
            "additionalProperties": false,
            "properties": { "path": { "type": "string" } }
        },
        "GuardrailCheckRequest": {
            "type": "object",
            "required": ["action"],
            "additionalProperties": false,
            "properties": {
                "agent": { "type": "string" },
                "action": { "type": "string" },
                "summary": { "type": "string" },
                "payload": { "type": "object" },
                "paths": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            }
        }
    })
}
