use serde_json::{json, Value};

pub(super) fn patch_schemas() -> Value {
    json!({
        "PatchRequest": {
            "type": "object",
            "description": "Native CrabDB PatchDocument or design-style files patch.",
            "properties": {
                "base_change": { "type": "string" },
                "message": { "type": "string" },
                "session_id": { "type": "string" },
                "allow_ignored": { "type": "boolean" },
                "allow_stale": { "type": "boolean" },
                "edits": { "type": "array", "items": { "type": "object", "additionalProperties": true } },
                "files": { "type": "array", "items": { "type": "object", "additionalProperties": true } }
            }
        }
    })
}
