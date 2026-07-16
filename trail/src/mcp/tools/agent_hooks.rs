use serde_json::{json, Value};

use crate::mcp::response::object_schema;

pub(super) fn tools() -> Value {
    json!([
        {
            "name": "trail.agent_integrations",
            "title": "Agent Integration Capabilities",
            "description": "Read provider hook, transcript/export, and ACP capability contracts without invoking native capture.",
            "inputSchema": object_schema(json!({
                "provider": {"type": "string", "description": "Optional canonical provider name or alias."}
            }), vec![])
        },
        {
            "name": "trail.agent_hook_installations",
            "title": "Agent Hook Installations",
            "description": "List persisted ownership and drift metadata for native hook installations.",
            "inputSchema": object_schema(json!({
                "provider": {"type": "string"}
            }), vec![])
        },
        {
            "name": "trail.agent_hook_receipts",
            "title": "Agent Hook Receipt Diagnostics",
            "description": "List durable redacted hook receipts by provider or processing status.",
            "inputSchema": object_schema(json!({
                "provider": {"type": "string"},
                "status": {"type": "string"},
                "offset": {"type": "integer", "minimum": 0, "maximum": 1000000},
                "limit": {"type": "integer", "minimum": 1, "maximum": 1000}
            }), vec![])
        },
        {
            "name": "trail.agent_capture_runs",
            "title": "Agent Capture Runs",
            "description": "List managed capture-run correlation leases.",
            "inputSchema": object_schema(json!({
                "active_only": {"type": "boolean", "default": true},
                "offset": {"type": "integer", "minimum": 0, "maximum": 1000000},
                "limit": {"type": "integer", "minimum": 1, "maximum": 1000}
            }), vec![])
        },
        {
            "name": "trail.agent_artifacts",
            "title": "Agent Session Artifacts",
            "description": "List immutable native transcript, export, and evidence artifacts for a session or turn.",
            "inputSchema": object_schema(json!({
                "session_id": {"type": "string"},
                "turn_id": {"type": "string"},
                "offset": {"type": "integer", "minimum": 0, "maximum": 1000000},
                "limit": {"type": "integer", "minimum": 1, "maximum": 1000}
            }), vec!["session_id"])
        },
        {
            "name": "trail.agent_provenance",
            "title": "Agent Session Provenance",
            "description": "Read the causal provenance graph for one captured agent session.",
            "inputSchema": object_schema(json!({
                "session_id": {"type": "string"},
                "offset": {"type": "integer", "minimum": 0, "maximum": 1000000},
                "limit": {"type": "integer", "minimum": 1, "maximum": 10000}
            }), vec!["session_id"])
        },
        {
            "name": "trail.agent_attestations",
            "title": "Agent Session Attestations",
            "description": "List immutable chained attestation segments and exact turn coverage for one session.",
            "inputSchema": object_schema(json!({
                "session_id": {"type": "string"},
                "offset": {"type": "integer", "minimum": 0, "maximum": 1000000},
                "limit": {"type": "integer", "minimum": 1, "maximum": 1000}
            }), vec!["session_id"])
        },
        {
            "name": "trail.agent_attestation_verify",
            "title": "Verify Agent Attestation",
            "description": "Verify statement, evidence, predecessor chain, signature, and key revocation status.",
            "inputSchema": object_schema(json!({
                "attestation_id": {"type": "string"}
            }), vec!["attestation_id"])
        },
        {
            "name": "trail.agent_learnings",
            "title": "Agent Learnings",
            "description": "List proposed, accepted, or rejected reusable findings without changing provider context files.",
            "inputSchema": object_schema(json!({
                "session_id": {"type": "string"},
                "status": {"type": "string"},
                "offset": {"type": "integer", "minimum": 0, "maximum": 1000000},
                "limit": {"type": "integer", "minimum": 1, "maximum": 1000}
            }), vec![])
        },
        {
            "name": "trail.agent_git_links",
            "title": "Agent Session Git Links",
            "description": "List explicit mappings between exact Trail session changes and Git commits.",
            "inputSchema": object_schema(json!({
                "session_id": {"type": "string"},
                "offset": {"type": "integer", "minimum": 0, "maximum": 1000000},
                "limit": {"type": "integer", "minimum": 1, "maximum": 1000}
            }), vec!["session_id"])
        },
        {
            "name": "trail.agent_trace",
            "title": "Portable Agent Trace",
            "description": "Read a verified vendor-neutral trace projection for one session.",
            "inputSchema": object_schema(json!({
                "session_id": {"type": "string"},
                "attachments": {"type": "boolean", "default": false}
            }), vec!["session_id"])
        }
    ])
}
