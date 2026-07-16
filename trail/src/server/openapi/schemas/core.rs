use serde_json::{json, Value};

pub(super) fn core_schemas() -> Value {
    json!({
        "JsonValue": {
            "description": "Trail typed JSON report. See CLI reference for the concrete report shape.",
            "oneOf": [
                { "type": "object", "additionalProperties": true },
                { "type": "array", "items": true },
                { "type": "string" },
                { "type": "number" },
                { "type": "boolean" },
                { "type": "null" }
            ]
        },
        "FileDiffSummary": {
            "type": "object",
            "required": ["path", "kind", "additions", "deletions"],
            "additionalProperties": false,
            "properties": {
                "path": { "type": "string" },
                "old_path": { "type": ["string", "null"] },
                "kind": {
                    "type": "string",
                    "enum": ["Added", "Modified", "Deleted", "Renamed", "TypeChanged"]
                },
                "before_hash": { "type": ["string", "null"] },
                "after_hash": { "type": ["string", "null"] },
                "additions": { "type": "integer" },
                "deletions": { "type": "integer" },
                "line_changes": {
                    "type": "array",
                    "items": { "$ref": "#/components/schemas/JsonValue" }
                },
                "patch": { "type": ["string", "null"] }
            }
        },
        "ErrorBody": {
            "type": "object",
            "required": ["error"],
            "additionalProperties": false,
            "properties": {
                "error": {
                    "type": "object",
                    "required": ["code", "status", "exit", "message", "scope", "state", "reason", "recovery"],
                    "additionalProperties": false,
                    "properties": {
                        "message": { "type": "string" },
                        "code": { "type": "string" },
                        "status": { "type": "integer" },
                        "exit": { "type": "integer" },
                        "scope": { "type": ["string", "null"] },
                        "state": { "type": ["string", "null"] },
                        "reason": { "type": ["string", "null"] },
                        "recovery": {
                            "oneOf": [
                                { "$ref": "#/components/schemas/StructuredRecovery" },
                                { "type": "null" }
                            ]
                        }
                    }
                }
            }
        },
        "StructuredRecovery": {
            "type": "object",
            "required": ["command"],
            "additionalProperties": false,
            "properties": { "command": { "type": "string" } }
        },
        "IndexReconcileRequest": {
            "type": "object",
            "additionalProperties": false,
            "properties": { "lane": { "type": ["string", "null"] } }
        },
        "ChangeLedgerReconcileReport": {
            "type": "object",
            "required": ["scope_id", "scope_kind", "previous_state", "reason", "observed_paths", "candidates", "resulting_epoch", "resulting_state"],
            "additionalProperties": false,
            "properties": {
                "scope_id": { "type": "string" },
                "scope_kind": { "type": "string", "enum": ["workspace", "materialized_lane"] },
                "previous_state": { "type": "string" },
                "reason": { "type": "string" },
                "observed_paths": { "type": "integer", "minimum": 0 },
                "candidates": { "type": "integer", "minimum": 0 },
                "resulting_epoch": { "type": "integer", "minimum": 0 },
                "resulting_state": { "type": "string" }
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
        "RecordRequest": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "ref_name": { "type": "string" },
                "branch": { "type": "string" },
                "message": { "type": "string" },
                "paths": {
                    "type": "array",
                    "items": { "type": "string" }
                },
                "kind": {
                    "type": "string",
                    "enum": ["file-edit", "multi-file-edit", "format", "manual-checkpoint", "manual-record"]
                },
                "session_id": { "type": "string" },
                "session": { "type": "string" },
                "allow_ignored": { "type": "boolean" }
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
                "lane": { "type": "string" },
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
