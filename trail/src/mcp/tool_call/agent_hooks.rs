use serde_json::{json, Value};

use crate::agent_hooks::AgentProviderRegistry;
use crate::{Result, Trail};

use super::{super::response::tool_result, super::types::*, parse_args};

pub(super) fn handle(db: &mut Trail, name: &str, arguments: &Value) -> Result<Option<Value>> {
    let value = match name {
        "trail.agent_integrations" => {
            let args: AgentIntegrationArgs = parse_args(arguments)?;
            let registry = AgentProviderRegistry::built_in()?;
            if let Some(provider) = args.provider {
                tool_result(registry.resolve(&provider)?)
            } else {
                tool_result(registry.list())
            }
        }
        "trail.agent_hook_installations" => {
            let args: AgentIntegrationArgs = parse_args(arguments)?;
            tool_result(db.list_agent_hook_installations(args.provider.as_deref())?)
        }
        "trail.agent_hook_receipts" => {
            let args: AgentReceiptListArgs = parse_args(arguments)?;
            tool_result(db.list_agent_hook_receipts_page(
                args.provider.as_deref(),
                args.status.as_deref(),
                args.offset.unwrap_or(0),
                args.limit.unwrap_or(100),
            )?)
        }
        "trail.agent_capture_runs" => {
            let args: AgentCaptureRunListArgs = parse_args(arguments)?;
            tool_result(db.list_agent_capture_runs_page(
                args.active_only,
                args.offset.unwrap_or(0),
                args.limit.unwrap_or(100),
            )?)
        }
        "trail.agent_artifacts" => {
            let args: AgentSessionEvidenceArgs = parse_args(arguments)?;
            tool_result(db.list_lane_artifacts_page(
                &args.session_id,
                args.turn_id.as_deref(),
                args.offset.unwrap_or(0),
                args.limit.unwrap_or(100),
            )?)
        }
        "trail.agent_provenance" => {
            let args: AgentSessionEvidenceArgs = parse_args(arguments)?;
            let (nodes, edges) = db.list_session_provenance_page(
                &args.session_id,
                args.offset.unwrap_or(0),
                args.limit.unwrap_or(1_000),
            )?;
            tool_result(json!({"session_id": args.session_id, "nodes": nodes, "edges": edges}))
        }
        "trail.agent_attestations" => {
            let args: AgentSessionEvidenceArgs = parse_args(arguments)?;
            tool_result(db.list_session_attestations_page(
                &args.session_id,
                args.offset.unwrap_or(0),
                args.limit.unwrap_or(100),
            )?)
        }
        "trail.agent_attestation_verify" => {
            let args: AgentAttestationArgs = parse_args(arguments)?;
            tool_result(db.verify_session_attestation(&args.attestation_id)?)
        }
        "trail.agent_learnings" => {
            let args: AgentLearningListArgs = parse_args(arguments)?;
            tool_result(db.list_learnings_page(
                args.session_id.as_deref(),
                args.status.as_deref(),
                args.offset.unwrap_or(0),
                args.limit.unwrap_or(100),
            )?)
        }
        "trail.agent_git_links" => {
            let args: AgentSessionEvidenceArgs = parse_args(arguments)?;
            tool_result(db.list_git_agent_links_page(
                &args.session_id,
                args.offset.unwrap_or(0),
                args.limit.unwrap_or(100),
            )?)
        }
        "trail.agent_trace" => {
            let args: AgentTraceArgs = parse_args(arguments)?;
            tool_result(db.export_agent_trace(&args.session_id, args.attachments)?)
        }
        _ => return Ok(None),
    };
    Ok(Some(value?))
}
