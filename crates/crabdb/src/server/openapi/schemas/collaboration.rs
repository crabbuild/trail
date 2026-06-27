use serde_json::{json, Value};

pub(super) fn collaboration_schemas() -> Value {
    json!({
        "MergeLaneRequest": {
            "type": "object",
            "properties": {
                "lane_id": { "type": "string" },
                "lane": { "type": "string" },
                "name": { "type": "string" },
                "strategy": { "type": "string" },
                "dry_run": { "type": "boolean" },
                "dry-run": { "type": "boolean" }
            }
        },
        "SessionStartRequest": {
            "type": "object",
            "required": ["lane"],
            "properties": {
                "lane": { "type": "string" },
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
            "required": ["lane", "action", "summary"],
            "properties": {
                "lane": { "type": "string" },
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
            "required": ["lane"],
            "properties": {
                "lane": { "type": "string" },
                "path": { "type": "string" },
                "mode": { "type": "string", "enum": ["read", "write"] },
                "ttl_secs": { "type": "integer", "minimum": 1 },
                "ttl": { "type": "integer", "minimum": 1 }
            }
        },
        "LaneClaimRequest": {
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
        "ConflictSetSummary": {
            "type": "object",
            "required": ["conflict_set_id", "status", "details", "created_at"],
            "properties": {
                "conflict_set_id": { "type": "string" },
                "merge_id": { "type": "string" },
                "source_ref": { "type": "string" },
                "target_ref": { "type": "string" },
                "status": { "type": "string" },
                "details": { "type": "array", "items": { "type": "string" } },
                "created_at": { "type": "integer" },
                "explanation": { "$ref": "#/components/schemas/ConflictExplanation" }
            }
        },
        "ConflictExplanation": {
            "type": "object",
            "required": ["merge", "paths", "recommendations", "next_steps"],
            "properties": {
                "merge": { "$ref": "#/components/schemas/ConflictMergeContext" },
                "paths": { "type": "array", "items": { "$ref": "#/components/schemas/ConflictPathExplanation" } },
                "recommendations": { "type": "array", "items": { "$ref": "#/components/schemas/ConflictResolutionCandidate" } },
                "next_steps": { "type": "array", "items": { "type": "string" } }
            }
        },
        "ConflictMergeContext": {
            "type": "object",
            "required": ["merge_id", "source_ref", "target_ref", "base_change", "target_change", "source_change"],
            "properties": {
                "merge_id": { "type": "string" },
                "queue_id": { "type": "string" },
                "source_ref": { "type": "string" },
                "target_ref": { "type": "string" },
                "base_change": { "type": "string" },
                "target_change": { "type": "string" },
                "source_change": { "type": "string" }
            }
        },
        "ConflictPathExplanation": {
            "type": "object",
            "required": ["path", "summary", "reason", "lines", "recommendation"],
            "properties": {
                "path": { "type": "string" },
                "summary": { "type": "string" },
                "reason": { "type": "string" },
                "target": { "$ref": "#/components/schemas/ConflictSideProvenance" },
                "source": { "$ref": "#/components/schemas/ConflictSideProvenance" },
                "lines": { "type": "array", "items": { "$ref": "#/components/schemas/ConflictLineExplanation" } },
                "recommendation": { "$ref": "#/components/schemas/ConflictResolutionCandidate" }
            }
        },
        "ConflictSideProvenance": {
            "type": "object",
            "required": ["side", "change_id", "kind", "branch", "actor_id", "created_at"],
            "properties": {
                "side": { "type": "string" },
                "change_id": { "type": "string" },
                "kind": { "type": "string" },
                "branch": { "type": "string" },
                "actor_id": { "type": "string" },
                "session_id": { "type": "string" },
                "message": { "type": "string" },
                "created_at": { "type": "integer" }
            }
        },
        "ConflictLineExplanation": {
            "type": "object",
            "required": ["line_id", "reason"],
            "properties": {
                "line_id": { "type": "string" },
                "base": { "type": "string" },
                "target": { "type": "string" },
                "source": { "type": "string" },
                "target_change": { "$ref": "#/components/schemas/ConflictSideProvenance" },
                "source_change": { "$ref": "#/components/schemas/ConflictSideProvenance" },
                "reason": { "type": "string" }
            }
        },
        "ConflictResolutionCandidate": {
            "type": "object",
            "required": ["resolution", "confidence", "reason"],
            "properties": {
                "resolution": { "type": "string", "enum": ["source", "target", "manual"] },
                "confidence": { "type": "string" },
                "reason": { "type": "string" }
            }
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
