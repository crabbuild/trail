use serde_json::{json, Value};

pub(super) fn patch_schemas() -> Value {
    json!({
        "PatchRequest": {
            "type": "object",
            "description": "Native CrabDB PatchDocument or design-style files patch. Provide exactly one non-empty edit source: edits or files. replace_line/modify_line edits must include expected_text.",
            "oneOf": [
                { "required": ["edits"], "not": { "required": ["files"] } },
                { "required": ["files"], "not": { "required": ["edits"] } }
            ],
            "properties": {
                "base_change": { "type": "string" },
                "message": { "type": "string" },
                "session_id": { "type": "string" },
                "allow_ignored": { "type": "boolean" },
                "allow_stale": { "type": "boolean" },
                "edits": { "type": "array", "minItems": 1, "items": { "$ref": "#/components/schemas/PatchEdit" } },
                "files": { "type": "array", "minItems": 1, "items": { "$ref": "#/components/schemas/ApiPatchFile" } }
            }
        },
        "PatchEdit": {
            "oneOf": [
                { "$ref": "#/components/schemas/PatchEditWrite" },
                { "$ref": "#/components/schemas/PatchEditWriteBytes" },
                { "$ref": "#/components/schemas/PatchEditReplaceLine" },
                { "$ref": "#/components/schemas/PatchEditDelete" },
                { "$ref": "#/components/schemas/PatchEditRename" }
            ]
        },
        "PatchEditWrite": {
            "type": "object",
            "additionalProperties": false,
            "required": ["op", "path", "content"],
            "properties": {
                "op": { "type": "string", "enum": ["write"] },
                "path": { "type": "string" },
                "content": { "type": "string" },
                "executable": { "type": "boolean" }
            }
        },
        "PatchEditWriteBytes": {
            "type": "object",
            "additionalProperties": false,
            "required": ["op", "path", "bytes_hex"],
            "properties": {
                "op": { "type": "string", "enum": ["write_bytes"] },
                "path": { "type": "string" },
                "bytes_hex": { "type": "string" },
                "executable": { "type": "boolean" }
            }
        },
        "PatchEditReplaceLine": {
            "type": "object",
            "additionalProperties": false,
            "required": ["op", "path", "line_id", "expected_text", "new_text"],
            "properties": {
                "op": { "type": "string", "enum": ["replace_line"] },
                "path": { "type": "string" },
                "line_id": { "type": "string" },
                "expected_text": { "type": "string" },
                "new_text": { "type": "string" }
            }
        },
        "PatchEditDelete": {
            "type": "object",
            "additionalProperties": false,
            "required": ["op", "path"],
            "properties": {
                "op": { "type": "string", "enum": ["delete"] },
                "path": { "type": "string" }
            }
        },
        "PatchEditRename": {
            "type": "object",
            "additionalProperties": false,
            "required": ["op", "from", "to"],
            "properties": {
                "op": { "type": "string", "enum": ["rename"] },
                "from": { "type": "string" },
                "to": { "type": "string" }
            }
        },
        "ApiPatchFile": {
            "oneOf": [
                { "$ref": "#/components/schemas/ApiPatchFileAddText" },
                { "$ref": "#/components/schemas/ApiPatchFileModifyText" },
                { "$ref": "#/components/schemas/ApiPatchFileWriteBytes" },
                { "$ref": "#/components/schemas/ApiPatchFileDelete" },
                { "$ref": "#/components/schemas/ApiPatchFileRename" }
            ]
        },
        "ApiPatchFileAddText": {
            "type": "object",
            "additionalProperties": false,
            "required": ["type", "path", "content"],
            "properties": {
                "type": { "type": "string", "enum": ["add_text"] },
                "path": { "type": "string" },
                "content": { "type": "string" },
                "executable": { "type": "boolean" }
            }
        },
        "ApiPatchFileModifyText": {
            "type": "object",
            "additionalProperties": false,
            "required": ["type", "path", "edits"],
            "properties": {
                "type": { "type": "string", "enum": ["modify_text"] },
                "path": { "type": "string" },
                "edits": { "type": "array", "items": { "$ref": "#/components/schemas/ApiTextEdit" } }
            }
        },
        "ApiPatchFileWriteBytes": {
            "type": "object",
            "additionalProperties": false,
            "required": ["type", "path", "bytes_hex"],
            "properties": {
                "type": { "type": "string", "enum": ["write_bytes"] },
                "path": { "type": "string" },
                "bytes_hex": { "type": "string" },
                "executable": { "type": "boolean" }
            }
        },
        "ApiPatchFileDelete": {
            "type": "object",
            "additionalProperties": false,
            "required": ["type", "path"],
            "properties": {
                "type": { "type": "string", "enum": ["delete"] },
                "path": { "type": "string" }
            }
        },
        "ApiPatchFileRename": {
            "type": "object",
            "additionalProperties": false,
            "required": ["type", "from", "to"],
            "properties": {
                "type": { "type": "string", "enum": ["rename"] },
                "from": { "type": "string" },
                "to": { "type": "string" }
            }
        },
        "ApiTextEdit": {
            "oneOf": [
                { "$ref": "#/components/schemas/ApiTextEditModifyLine" }
            ]
        },
        "ApiTextEditModifyLine": {
            "type": "object",
            "additionalProperties": false,
            "required": ["type", "line_id", "expected_text", "new_text"],
            "properties": {
                "type": { "type": "string", "enum": ["modify_line"] },
                "line_id": { "type": "string" },
                "expected_text": { "type": "string" },
                "new_text": { "type": "string" }
            }
        }
    })
}
