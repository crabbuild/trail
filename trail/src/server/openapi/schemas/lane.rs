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
                "changed_paths": { "type": "array", "items": { "$ref": "#/components/schemas/FileDiffSummary" } },
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
        "LaneRefreshPreviewReport": {
            "type": "object",
            "required": [
                "lane_id",
                "ref_name",
                "base_change",
                "lane_head_change",
                "lane_head_root",
                "target_ref",
                "target_change",
                "target_root",
                "clean",
                "conflicted",
                "changed_paths",
                "conflicts",
                "next_steps"
            ],
            "properties": {
                "lane_id": { "type": "string" },
                "ref_name": { "type": "string" },
                "base_change": { "type": "string" },
                "lane_head_change": { "type": "string" },
                "lane_head_root": { "type": "string" },
                "target_ref": { "type": "string" },
                "target_change": { "type": "string" },
                "target_root": { "type": "string" },
                "operations_behind": { "type": "integer" },
                "clean": { "type": "boolean" },
                "conflicted": { "type": "boolean" },
                "changed_paths": { "type": "array", "items": { "$ref": "#/components/schemas/FileDiffSummary" } },
                "conflicts": { "type": "array", "items": { "type": "string" } },
                "next_steps": { "type": "array", "items": { "type": "string" } }
            }
        },
        "LaneRecordWorkdirResponse": {
            "oneOf": [
                { "$ref": "#/components/schemas/LaneRecordReport" },
                { "$ref": "#/components/schemas/LaneRecordPreviewReport" }
            ]
        },
        "LaneRecordReport": {
            "type": "object",
            "required": ["lane_id", "operation", "root_id", "changed_paths"],
            "additionalProperties": false,
            "properties": {
                "lane_id": { "type": "string" },
                "operation": { "type": ["string", "null"] },
                "root_id": { "type": "string" },
                "changed_paths": { "type": "array", "items": { "$ref": "#/components/schemas/FileDiffSummary" } }
            }
        },
        "LaneRecordPreviewReport": {
            "type": "object",
            "required": [
                "lane_id",
                "workdir",
                "head_change",
                "root_id",
                "clean",
                "changed_paths",
                "ignored_paths",
                "risky_paths",
                "oversized_files",
                "policy"
            ],
            "additionalProperties": false,
            "properties": {
                "lane_id": { "type": "string" },
                "workdir": { "type": "string" },
                "head_change": { "type": "string" },
                "root_id": { "type": "string" },
                "clean": { "type": "boolean" },
                "changed_paths": { "type": "array", "items": { "$ref": "#/components/schemas/FileDiffSummary" } },
                "ignored_paths": { "type": "array", "items": { "$ref": "#/components/schemas/LaneWorkdirIgnoredPath" } },
                "risky_paths": { "type": "array", "items": { "$ref": "#/components/schemas/LaneWorkdirRisk" } },
                "oversized_files": { "type": "array", "items": { "$ref": "#/components/schemas/LaneRecordOversizedFile" } },
                "policy": { "$ref": "#/components/schemas/LaneRecordPolicyPreview" }
            }
        },
        "LaneWorkdirIgnoredPath": {
            "type": "object",
            "required": ["path", "source"],
            "additionalProperties": false,
            "properties": {
                "path": { "type": "string" },
                "source": { "type": "string", "enum": ["hardcoded", "workdir"] }
            }
        },
        "LaneWorkdirRisk": {
            "type": "object",
            "required": ["path", "kind", "message"],
            "additionalProperties": false,
            "properties": {
                "path": { "type": "string" },
                "kind": {
                    "type": "string",
                    "enum": ["nested_git", "nested_trail", "symlink", "hardlink", "external_mount"]
                },
                "message": { "type": "string" }
            }
        },
        "LaneRecordOversizedFile": {
            "type": "object",
            "required": ["path", "size_bytes", "limit_bytes"],
            "additionalProperties": false,
            "properties": {
                "path": { "type": "string" },
                "size_bytes": { "type": "integer" },
                "limit_bytes": { "type": "integer" }
            }
        },
        "LaneRecordPolicyPreview": {
            "type": "object",
            "required": ["allowed"],
            "additionalProperties": false,
            "properties": {
                "allowed": { "type": "boolean" },
                "warnings": { "type": "array", "items": { "type": "string" } },
                "error": { "type": ["string", "null"] }
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
                "workdir_mode": { "type": "string", "enum": ["auto", "virtual", "sparse", "full-cow", "overlay-cow", "nfs-cow"] },
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
                "message": { "type": "string" },
                "preview": { "type": "boolean" }
            }
        },
        "WorkspaceCheckpointRequest": {
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            }
        },
        "LaneUpdateRequest": {
            "type": "object",
            "properties": {
                "from": { "type": "string", "default": "main" },
                "source": { "type": "string" },
                "checkpoint": { "type": "boolean", "default": false }
            }
        },
        "WorkspaceExecRequest": {
            "type": "object",
            "required": ["command"],
            "properties": {
                "command": {
                    "type": "array",
                    "items": { "type": "string" },
                    "minItems": 1
                }
            }
        },
        "DependencySyncRequest": {
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            }
        },
        "EnvironmentSyncRequest": {
            "type": "object",
            "properties": {
                "adapter": { "type": "string", "default": "auto" },
                "component": { "type": "string" },
                "path": { "type": "string" }
            }
        },
        "EnvironmentAdapterIdentityReport": {
            "type": "object",
            "required": ["namespace", "name", "contract_major", "implementation_version"],
            "additionalProperties": false,
            "properties": {
                "namespace": { "type": "string" },
                "name": { "type": "string" },
                "contract_major": { "type": "integer", "minimum": 1 },
                "implementation_version": { "type": "string" },
                "distribution_digest": { "type": ["string", "null"] }
            }
        },
        "EnvironmentAdapterCatalogEntryReport": {
            "type": "object",
            "required": ["identity", "canonical_identity", "selectors", "kind", "layer_adapter_name", "discovery_markers", "protocols", "supported_operating_systems", "supported_architectures", "source", "publisher", "publisher_key_id", "trust", "certification_tier", "stability", "description"],
            "additionalProperties": false,
            "properties": {
                "identity": { "$ref": "#/components/schemas/EnvironmentAdapterIdentityReport" },
                "canonical_identity": { "type": "string" },
                "selectors": { "type": "array", "items": { "type": "string" } },
                "kind": { "type": "string" },
                "layer_adapter_name": { "type": "string" },
                "discovery_markers": { "type": "array", "items": { "type": "string" } },
                "protocols": { "type": "array", "items": { "type": "string", "enum": ["trail.environment-adapter/v1", "trail.environment-adapter/v2"] } },
                "supported_operating_systems": { "type": "array", "items": { "type": "string", "enum": ["linux", "macos", "windows"] } },
                "supported_architectures": { "type": "array", "items": { "type": "string", "enum": ["aarch64", "x86_64"] } },
                "source": { "type": "string", "enum": ["builtin", "recipe", "plugin"] },
                "publisher": { "type": ["string", "null"] },
                "publisher_key_id": { "type": ["string", "null"] },
                "trust": { "type": "string", "enum": ["builtin", "local_unsigned", "publisher_signed"] },
                "certification_tier": { "type": "string" },
                "stability": { "type": "string" },
                "description": { "type": "string" }
            }
        },
        "EnvironmentAdapterCatalogReport": {
            "type": "object",
            "required": ["contract_major", "adapters"],
            "additionalProperties": false,
            "properties": {
                "contract_major": { "type": "integer", "minimum": 1 },
                "adapters": {
                    "type": "array",
                    "items": { "$ref": "#/components/schemas/EnvironmentAdapterCatalogEntryReport" }
                }
            }
        },
        "EnvironmentComponentStateReport": {
            "type": "object",
            "required": ["view_id", "component", "adapter", "expected_key", "status", "updated_at"],
            "properties": {
                "view_id": { "type": "string" },
                "component": {
                    "type": "object",
                    "required": ["component_id", "kind"],
                    "properties": {
                        "component_id": { "type": "string" },
                        "kind": { "type": "string" }
                    }
                },
                "adapter": {
                    "type": "object",
                    "required": ["namespace", "name", "contract_major", "implementation_version"],
                    "properties": {
                        "namespace": { "type": "string" },
                        "name": { "type": "string" },
                        "contract_major": { "type": "integer", "minimum": 0 },
                        "implementation_version": { "type": "string" },
                        "distribution_digest": { "type": ["string", "null"] }
                    }
                },
                "expected_key": { "type": "string" },
                "attached_key": { "type": ["string", "null"] },
                "status": { "type": "string", "enum": ["building", "ready", "stale", "failed"] },
                "reason": { "type": ["string", "null"] },
                "updated_at": { "type": "integer" }
            }
        },
        "EnvironmentComponentStateReportList": {
            "type": "array",
            "items": { "$ref": "#/components/schemas/EnvironmentComponentStateReport" }
        },
        "EnvironmentStaleChangeReport": {
            "type": "object",
            "required": ["dimension", "name", "change"],
            "additionalProperties": false,
            "properties": {
                "dimension": { "type": "string", "enum": ["input", "tool", "policy", "canonical_key", "provenance", "component", "attachment"] },
                "name": { "type": "string" },
                "change": { "type": "string", "enum": ["added", "removed", "modified", "not_attached", "removed_or_adapter_unavailable", "unavailable_for_legacy_or_missing_layer"] }
            }
        },
        "EnvironmentStaleExplanationReport": {
            "type": "object",
            "required": ["component_id", "status", "expected_key", "attached_key", "complete", "provenance_complete", "total_changes", "offset", "next_offset", "changes"],
            "additionalProperties": false,
            "properties": {
                "component_id": { "type": "string" },
                "status": { "type": "string", "enum": ["ready", "stale"] },
                "expected_key": { "type": "string" },
                "attached_key": { "type": ["string", "null"] },
                "complete": { "type": "boolean" },
                "provenance_complete": { "type": "boolean" },
                "total_changes": { "type": "integer", "minimum": 0 },
                "offset": { "type": "integer", "minimum": 0 },
                "next_offset": { "type": ["integer", "null"], "minimum": 0 },
                "changes": { "type": "array", "items": { "$ref": "#/components/schemas/EnvironmentStaleChangeReport" } }
            }
        },
        "WorkspaceLayerReport": {
            "type": "object",
            "required": ["layer_id", "kind", "cache_key", "adapter", "state", "storage_path", "logical_bytes", "entry_count", "portability_scope"],
            "properties": {
                "layer_id": { "type": "string" },
                "kind": { "type": "string" },
                "cache_key": { "type": "string" },
                "adapter": { "type": "string" },
                "state": { "type": "string" },
                "storage_path": { "type": "string" },
                "logical_bytes": { "type": "integer", "minimum": 0 },
                "physical_bytes": { "type": ["integer", "null"], "minimum": 0 },
                "entry_count": { "type": "integer", "minimum": 0 },
                "portability_scope": { "type": "string" }
            }
        },
        "EnvironmentDiscoveredComponentReport": {
            "type": "object",
            "required": ["component_id", "component_root", "kind", "adapter_identity"],
            "additionalProperties": false,
            "properties": {
                "component_id": { "type": "string" },
                "component_root": { "type": "string" },
                "kind": { "type": "string" },
                "adapter_identity": { "type": "string" }
            }
        },
        "EnvironmentDiscoveryConflictReport": {
            "type": "object",
            "required": ["component_root", "adapter_identities", "reason"],
            "additionalProperties": false,
            "properties": {
                "component_root": { "type": "string" },
                "adapter_identities": { "type": "array", "items": { "type": "string" } },
                "reason": { "type": "string" }
            }
        },
        "EnvironmentDiscoveryReport": {
            "type": "object",
            "required": ["source_root", "components", "conflicts"],
            "additionalProperties": false,
            "properties": {
                "source_root": { "type": "string" },
                "components": {
                    "type": "array",
                    "items": { "$ref": "#/components/schemas/EnvironmentDiscoveredComponentReport" }
                },
                "conflicts": {
                    "type": "array",
                    "items": { "$ref": "#/components/schemas/EnvironmentDiscoveryConflictReport" }
                }
            }
        },
        "EnvironmentGraphNodeReport": {
            "type": "object",
            "required": ["topological_index", "component_id", "component_root", "kind", "adapter_identity", "component_key", "dependencies", "caches", "external_artifacts", "runtime_resources", "outputs"],
            "additionalProperties": false,
            "properties": {
                "topological_index": { "type": "integer", "minimum": 0 },
                "component_id": { "type": "string" },
                "component_root": { "type": "string" },
                "kind": { "type": "string" },
                "adapter_identity": { "type": "string" },
                "component_key": { "type": "string" },
                "dependencies": { "type": "array", "items": { "type": "string" } },
                "caches": { "type": "array", "items": { "$ref": "#/components/schemas/EnvironmentCacheReport" } },
                "external_artifacts": { "type": "array", "items": { "$ref": "#/components/schemas/EnvironmentExternalArtifactReport" } },
                "runtime_resources": { "type": "array", "items": { "$ref": "#/components/schemas/EnvironmentRuntimeDeclarationReport" } },
                "outputs": { "type": "array", "items": { "$ref": "#/components/schemas/EnvironmentPlanOutputReport" } }
            }
        },
        "EnvironmentGraphEdgeReport": {
            "type": "object",
            "required": ["source_component_id", "source_component_key", "target_component_id", "edge_type"],
            "additionalProperties": false,
            "properties": {
                "source_component_id": { "type": "string" },
                "source_component_key": { "type": "string" },
                "target_component_id": { "type": "string" },
                "edge_type": { "type": "string", "enum": ["build_requires", "runtime_requires", "binds_after", "invalidates_with"] }
            }
        },
        "EnvironmentGraphReport": {
            "type": "object",
            "required": ["source_root", "total_nodes", "total_edges", "offset", "next_offset", "nodes", "edges"],
            "additionalProperties": false,
            "properties": {
                "source_root": { "type": "string" },
                "total_nodes": { "type": "integer", "minimum": 0 },
                "total_edges": { "type": "integer", "minimum": 0 },
                "offset": { "type": "integer", "minimum": 0 },
                "next_offset": { "type": ["integer", "null"], "minimum": 0 },
                "nodes": { "type": "array", "items": { "$ref": "#/components/schemas/EnvironmentGraphNodeReport" } },
                "edges": { "type": "array", "items": { "$ref": "#/components/schemas/EnvironmentGraphEdgeReport" } }
            }
        },
        "EnvironmentPlanInputReport": {
            "type": "object",
            "required": ["source_path", "staging_path", "content_hash", "size_bytes"],
            "additionalProperties": false,
            "properties": {
                "source_path": { "type": "string" },
                "staging_path": { "type": "string" },
                "content_hash": { "type": "string" },
                "size_bytes": { "type": "integer", "minimum": 0 }
            }
        },
        "EnvironmentPlanCommandReport": {
            "type": "object",
            "required": ["phase", "program", "resolved_program", "executable_identity", "args", "working_directory", "environment_names"],
            "additionalProperties": false,
            "properties": {
                "phase": { "type": "string", "enum": ["staging", "mounted_initialization"] },
                "program": { "type": "string" },
                "resolved_program": { "type": "string" },
                "executable_identity": { "type": "string" },
                "args": { "type": "array", "items": { "type": "string" } },
                "working_directory": { "type": "string" },
                "environment_names": { "type": "array", "items": { "type": "string" } }
            }
        },
        "EnvironmentCapabilityReport": {
            "type": "object",
            "required": ["filesystem_read", "filesystem_write", "process", "network", "shell", "scripts", "secrets", "sandbox"],
            "additionalProperties": false,
            "properties": {
                "filesystem_read": { "type": "array", "items": { "type": "string" } },
                "filesystem_write": { "type": "array", "items": { "type": "string" } },
                "process": { "type": "array", "items": { "type": "string" } },
                "network": { "type": "string" },
                "shell": { "type": "string" },
                "scripts": { "type": "string" },
                "secrets": { "type": "string" },
                "sandbox": { "type": "string" }
            }
        },
        "EnvironmentPlanReport": {
            "type": "object",
            "required": ["source_root", "component_id", "adapter_identity", "kind", "component_key", "dependencies", "dependency_edges", "caches", "external_artifacts", "runtime_resources", "inputs", "tools", "commands", "outputs", "output_path", "mount_path", "portability_scope", "capabilities"],
            "additionalProperties": false,
            "properties": {
                "source_root": { "type": "string" },
                "component_id": { "type": "string" },
                "adapter_identity": { "type": "string" },
                "kind": { "type": "string" },
                "component_key": { "type": "string" },
                "dependencies": { "type": "array", "items": { "type": "string" } },
                "dependency_edges": { "type": "array", "items": { "$ref": "#/components/schemas/EnvironmentGenerationDependencyReport" } },
                "caches": { "type": "array", "items": { "$ref": "#/components/schemas/EnvironmentCacheReport" } },
                "external_artifacts": { "type": "array", "items": { "$ref": "#/components/schemas/EnvironmentExternalArtifactReport" } },
                "runtime_resources": { "type": "array", "items": { "$ref": "#/components/schemas/EnvironmentRuntimeDeclarationReport" } },
                "inputs": { "type": "array", "items": { "$ref": "#/components/schemas/EnvironmentPlanInputReport" } },
                "tools": { "type": "object", "additionalProperties": { "type": "string" } },
                "commands": { "type": "array", "items": { "$ref": "#/components/schemas/EnvironmentPlanCommandReport" } },
                "outputs": { "type": "array", "items": { "$ref": "#/components/schemas/EnvironmentPlanOutputReport" } },
                "output_path": { "type": "string" },
                "mount_path": { "type": "string" },
                "portability_scope": { "type": "string" },
                "capabilities": { "$ref": "#/components/schemas/EnvironmentCapabilityReport" }
            }
        },
        "EnvironmentPlanOutputReport": {
            "type": "object",
            "required": ["name", "output_path", "mount_path", "policy"],
            "additionalProperties": false,
            "properties": {
                "name": { "type": "string" },
                "output_path": { "type": "string" },
                "mount_path": { "type": "string" },
                "policy": { "type": "string", "enum": ["immutable_seed_private", "writable_private"] }
            }
        },
        "EnvironmentGenerationOutputReport": {
            "type": "object",
            "required": ["name", "policy", "storage_identity", "layer_id", "mount_path", "layer_subpath"],
            "additionalProperties": false,
            "properties": {
                "name": { "type": "string" },
                "policy": { "type": "string", "enum": ["immutable_seed_private", "writable_private"] },
                "storage_identity": { "type": "string" },
                "layer_id": { "type": ["string", "null"] },
                "mount_path": { "type": "string" },
                "layer_subpath": { "type": "string" }
            }
        },
        "EnvironmentCacheReport": {
            "type": "object",
            "required": ["name", "namespace_id", "protocol", "access", "authority", "scope", "compatibility"],
            "additionalProperties": false,
            "properties": {
                "name": { "type": "string" },
                "namespace_id": { "type": "string", "pattern": "^cache_[0-9a-f]{64}$" },
                "protocol": { "type": "string", "enum": ["content_store", "compiler_cache", "locked_index"] },
                "access": { "type": "string", "enum": ["tool_concurrent", "host_exclusive"] },
                "authority": { "type": "string", "enum": ["performance_only"] },
                "scope": { "type": "string", "enum": ["workspace"] },
                "compatibility": { "type": "object", "additionalProperties": { "type": "string" } }
            }
        },
        "EnvironmentExternalArtifactReport": {
            "type": "object",
            "required": ["name", "artifact_type", "provider", "reference", "digest", "platform", "cleanup_owner"],
            "additionalProperties": false,
            "properties": {
                "name": { "type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[A-Za-z0-9._-]+$" },
                "artifact_type": { "type": "string", "enum": ["oci_image"] },
                "provider": { "type": "string", "enum": ["oci"] },
                "reference": { "type": "string", "minLength": 1, "maxLength": 2120 },
                "digest": { "type": "string", "pattern": "^sha256:[0-9a-f]{64}$" },
                "platform": { "type": "string", "enum": ["linux/amd64", "linux/arm64", "windows/amd64", "windows/arm64"] },
                "cleanup_owner": { "type": "string", "enum": ["external"] }
            }
        },
        "EnvironmentRuntimeDeclarationReport": {
            "type": "object",
            "required": ["name", "runtime_type", "provider", "artifact_name", "container_port", "protocol", "health_type", "health_timeout_ms", "restart_policy", "cleanup_owner", "volume_target", "secrets"],
            "additionalProperties": false,
            "properties": {
                "name": { "type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[A-Za-z0-9._-]+$" },
                "runtime_type": { "type": "string", "enum": ["container"] },
                "provider": { "type": "string", "enum": ["oci"] },
                "artifact_name": { "type": "string", "minLength": 1 },
                "container_port": { "type": "integer", "minimum": 1, "maximum": 65535 },
                "protocol": { "type": "string", "enum": ["tcp"] },
                "health_type": { "type": "string", "enum": ["tcp"] },
                "health_timeout_ms": { "type": "integer", "minimum": 1000, "maximum": 300000 },
                "restart_policy": { "type": "string", "enum": ["never", "on_failure", "always"] },
                "cleanup_owner": { "type": "string", "enum": ["trail"] },
                "volume_target": { "type": ["string", "null"] },
                "secrets": { "type": "array", "items": { "$ref": "#/components/schemas/EnvironmentSecretReferenceReport" }, "maxItems": 16 }
            }
        },
        "EnvironmentSecretReferenceReport": {
            "type": "object",
            "required": ["name", "provider", "reference", "version", "purpose", "injection", "target", "environment", "required"],
            "additionalProperties": false,
            "properties": {
                "name": { "type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[A-Za-z0-9._-]+$" },
                "provider": { "type": "string", "enum": ["file", "environment_file"] },
                "reference": { "type": "string", "minLength": 1, "maxLength": 4096 },
                "version": { "type": ["string", "null"], "minLength": 1, "maxLength": 256 },
                "purpose": { "type": "string", "minLength": 1, "maxLength": 256 },
                "injection": { "type": "string", "enum": ["file"] },
                "target": { "type": "string", "pattern": "^/run/secrets/[^/]+(?:/[^/]+)*$" },
                "environment": { "type": ["string", "null"], "pattern": "^[A-Z_][A-Z0-9_]{0,127}$" },
                "required": { "type": "boolean" }
            }
        },
        "EnvironmentSecretStatusReport": {
            "type": "object",
            "required": ["name", "provider", "reference", "version", "purpose", "injection", "target", "environment", "required", "status", "reason", "resolved_at", "updated_at"],
            "additionalProperties": false,
            "properties": {
                "name": { "type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[A-Za-z0-9._-]+$" },
                "provider": { "type": "string", "enum": ["file", "environment_file"] },
                "reference": { "type": "string", "minLength": 1, "maxLength": 4096 },
                "version": { "type": ["string", "null"], "minLength": 1, "maxLength": 256 },
                "purpose": { "type": "string", "minLength": 1, "maxLength": 256 },
                "injection": { "type": "string", "enum": ["file"] },
                "target": { "type": "string", "pattern": "^/run/secrets/[^/]+(?:/[^/]+)*$" },
                "environment": { "type": ["string", "null"], "pattern": "^[A-Z_][A-Z0-9_]{0,127}$" },
                "required": { "type": "boolean" },
                "status": { "type": "string", "enum": ["pending", "available", "unavailable"] },
                "reason": { "type": ["string", "null"] },
                "resolved_at": { "type": ["integer", "null"] },
                "updated_at": { "type": "integer" }
            }
        },
        "EnvironmentRuntimeResourceReport": {
            "type": "object",
            "required": ["name", "runtime_type", "provider", "artifact_name", "container_port", "protocol", "health_type", "health_timeout_ms", "restart_policy", "cleanup_owner", "volume_target", "secrets", "image_reference", "image_digest", "image_platform", "allocation_id", "provider_resource_id", "container_name", "network_name", "volume_name", "host_port", "status", "health_status", "reason", "created_at", "updated_at", "started_at", "stopped_at", "secret_statuses"],
            "additionalProperties": false,
            "properties": {
                "name": { "type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[A-Za-z0-9._-]+$" },
                "runtime_type": { "type": "string", "enum": ["container"] },
                "provider": { "type": "string", "enum": ["oci"] },
                "artifact_name": { "type": "string", "minLength": 1 },
                "container_port": { "type": "integer", "minimum": 1, "maximum": 65535 },
                "protocol": { "type": "string", "enum": ["tcp"] },
                "health_type": { "type": "string", "enum": ["tcp"] },
                "health_timeout_ms": { "type": "integer", "minimum": 1000, "maximum": 300000 },
                "restart_policy": { "type": "string", "enum": ["never", "on_failure", "always"] },
                "cleanup_owner": { "type": "string", "enum": ["trail"] },
                "volume_target": { "type": ["string", "null"] },
                "secrets": { "type": "array", "items": { "$ref": "#/components/schemas/EnvironmentSecretReferenceReport" }, "maxItems": 16 },
                "image_reference": { "type": "string" },
                "image_digest": { "type": "string", "pattern": "^sha256:[0-9a-f]{64}$" },
                "image_platform": { "type": "string" },
                "allocation_id": { "type": "string", "pattern": "^runtime_[0-9a-f]{32}$" },
                "provider_resource_id": { "type": ["string", "null"] },
                "container_name": { "type": "string" },
                "network_name": { "type": "string" },
                "volume_name": { "type": ["string", "null"] },
                "host_port": { "type": ["integer", "null"], "minimum": 1, "maximum": 65535 },
                "status": { "type": "string", "enum": ["pending", "allocating", "running", "failed", "stopping", "stopped", "orphaned"] },
                "health_status": { "type": "string", "enum": ["pending", "starting", "healthy", "unhealthy", "stopped", "unknown"] },
                "reason": { "type": ["string", "null"] },
                "created_at": { "type": "integer" },
                "updated_at": { "type": "integer" },
                "started_at": { "type": ["integer", "null"] },
                "stopped_at": { "type": ["integer", "null"] },
                "secret_statuses": { "type": "array", "items": { "$ref": "#/components/schemas/EnvironmentSecretStatusReport" }, "maxItems": 16 }
            }
        },
        "EnvironmentGenerationDependencyReport": {
            "type": "object",
            "required": ["component_id", "component_key", "edge_type"],
            "additionalProperties": false,
            "properties": {
                "component_id": { "type": "string" },
                "component_key": { "type": "string" },
                "edge_type": { "type": "string", "enum": ["build_requires", "runtime_requires", "binds_after", "invalidates_with"] }
            }
        },
        "EnvironmentGenerationComponentReport": {
            "type": "object",
            "required": ["component_id", "adapter_identity", "kind", "component_key", "dependencies", "outputs", "caches", "external_artifacts", "runtime_resources"],
            "additionalProperties": false,
            "properties": {
                "component_id": { "type": "string" },
                "adapter_identity": { "type": "string" },
                "kind": { "type": "string" },
                "component_key": { "type": "string" },
                "layer_id": { "type": ["string", "null"] },
                "mount_path": { "type": ["string", "null"] },
                "dependencies": { "type": "array", "items": { "$ref": "#/components/schemas/EnvironmentGenerationDependencyReport" } },
                "outputs": { "type": "array", "items": { "$ref": "#/components/schemas/EnvironmentGenerationOutputReport" } },
                "caches": { "type": "array", "items": { "$ref": "#/components/schemas/EnvironmentCacheReport" } },
                "external_artifacts": { "type": "array", "items": { "$ref": "#/components/schemas/EnvironmentExternalArtifactReport" } },
                "runtime_resources": { "type": "array", "items": { "$ref": "#/components/schemas/EnvironmentRuntimeResourceReport" } }
            }
        },
        "EnvironmentGenerationReport": {
            "type": "object",
            "required": ["generation_id", "view_id", "generation_sequence", "source_root", "specification_digest", "state", "components", "created_at"],
            "additionalProperties": false,
            "properties": {
                "generation_id": { "type": "string" },
                "view_id": { "type": "string" },
                "generation_sequence": { "type": "integer", "minimum": 1 },
                "source_root": { "type": "string" },
                "specification_digest": { "type": "string" },
                "predecessor_generation_id": { "type": ["string", "null"] },
                "state": { "type": "string", "enum": ["active", "retired", "failed"] },
                "components": {
                    "type": "array",
                    "items": { "$ref": "#/components/schemas/EnvironmentGenerationComponentReport" }
                },
                "created_at": { "type": "integer" },
                "activated_at": { "type": ["integer", "null"] },
                "retired_at": { "type": ["integer", "null"] }
            }
        },
        "EnvironmentGenerationReportNullable": {
            "oneOf": [
                { "$ref": "#/components/schemas/EnvironmentGenerationReport" },
                { "type": "null" }
            ]
        },
        "EnvironmentSyncReport": {
            "type": "object",
            "required": ["generation", "layers"],
            "additionalProperties": false,
            "properties": {
                "generation": { "$ref": "#/components/schemas/EnvironmentGenerationReport" },
                "layers": {
                    "type": "array",
                    "items": { "$ref": "#/components/schemas/WorkspaceLayerReport" }
                }
            }
        },
        "CacheGcRequest": {
            "type": "object",
            "properties": {
                "dry_run": { "type": "boolean" },
                "retention_secs": { "type": "integer", "minimum": 0 }
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
