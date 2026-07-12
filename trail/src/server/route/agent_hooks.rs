use sha2::{Digest, Sha256};

use crate::agent_hooks::AgentProviderRegistry;
use crate::server::request_types::{
    AgentArtifactRedactRequest, AgentAttestationCreateRequest, AgentCaptureRunLeaseRequest,
    AgentCaptureRunRequest, AgentLearningReviewRequest,
};
use crate::server::route::utils::{json_response, query_usize, query_value};
use crate::server::transport::{HttpRequest, HttpResponse};
use crate::{
    AgentCaptureRunInput, AgentCaptureTransport, AgentHookReceiptInput, GitAgentLinkInput,
    LearningInput, Result, Trail,
};

pub(super) fn handle_agent_hook_route(
    db: &mut Trail,
    request: &HttpRequest,
    path: &str,
    query: &str,
    parts: &[&str],
) -> Result<Option<HttpResponse>> {
    if request.method == "GET" && path == "/v1/agent-integrations/capabilities" {
        let registry = AgentProviderRegistry::built_in()?;
        return Ok(Some(json_response(200, "OK", &registry.list())?));
    }

    if request.method == "GET" && path == "/v1/agent-hooks/installations" {
        let reports = db.list_agent_hook_installations(query_value(query, "provider"))?;
        return Ok(Some(json_response(200, "OK", &reports)?));
    }
    if request.method == "GET" && parts_match(parts, &["v1", "agent-hooks", "installations", "*"]) {
        return Ok(Some(json_response(
            200,
            "OK",
            &db.agent_hook_installation(parts[3])?,
        )?));
    }
    if request.method == "GET" && path == "/v1/agent-hooks/receipts" {
        let reports = db.list_agent_hook_receipts_page(
            query_value(query, "provider"),
            query_value(query, "status"),
            query_usize(query, "offset", 0)?,
            query_usize(query, "limit", 100)?,
        )?;
        return Ok(Some(json_response(200, "OK", &reports)?));
    }
    if request.method == "GET" && parts_match(parts, &["v1", "agent-hooks", "receipts", "*"]) {
        return Ok(Some(json_response(
            200,
            "OK",
            &db.agent_hook_receipt(parts[3])?,
        )?));
    }
    if request.method == "POST"
        && parts_match(parts, &["v1", "agent-hooks", "receipts", "*", "replay"])
    {
        return Ok(Some(json_response(
            200,
            "OK",
            &db.replay_agent_hook_receipt(parts[3])?,
        )?));
    }
    if request.method == "POST"
        && parts_match(parts, &["v1", "agent-hooks", "receipts", "*", "retry"])
    {
        return Ok(Some(json_response(
            200,
            "OK",
            &db.retry_agent_hook_receipt(parts[3])?,
        )?));
    }
    if request.method == "POST"
        && parts_match(parts, &["v1", "agent-hooks", "receipts", "*", "discard"])
    {
        return Ok(Some(json_response(
            200,
            "OK",
            &db.discard_agent_hook_receipt(parts[3])?,
        )?));
    }
    if request.method == "POST" && parts.len() == 4 && parts[0] == "v1" && parts[1] == "agent-hooks"
    {
        let registry = AgentProviderRegistry::built_in()?;
        let provider = registry.resolve(parts[2])?.provider.clone();
        let payload: serde_json::Value = serde_json::from_slice(&request.body)?;
        let native_session_id = payload_string(
            &payload,
            &[
                "session_id",
                "sessionId",
                "sessionID",
                "conversation_id",
                "conversationId",
                "thread_id",
                "threadId",
            ],
        );
        let native_turn_id = payload_string(&payload, &["turn_id", "turnId"]);
        let occurred_at = payload_i64(&payload, &["timestamp", "occurred_at", "occurredAt"]);
        let dedupe_key = query_value(query, "dedupe_key")
            .map(ToString::to_string)
            .unwrap_or_else(|| {
                hook_dedupe_key(
                    &provider,
                    parts[3],
                    native_session_id.as_deref(),
                    native_turn_id.as_deref(),
                    &payload,
                )
            });
        let response_provider = provider.clone();
        let report = db.persist_agent_hook_receipt(AgentHookReceiptInput {
            installation_id: query_value(query, "installation").map(ToString::to_string),
            provider,
            native_event: parts[3].to_string(),
            native_session_id,
            native_turn_id,
            transport: AgentCaptureTransport::NativeHooks,
            dedupe_key,
            payload,
            occurred_at,
        })?;
        let response_body =
            if response_provider == "codex" && matches!(parts[3], "Stop" | "SubagentStop") {
                serde_json::json!({"continue": true})
            } else if response_provider == "gemini" {
                serde_json::json!({})
            } else {
                serde_json::to_value(&report)?
            };
        let mut response = json_response(200, "OK", &response_body)?;
        response
            .extra_headers
            .push(("X-Trail-Receipt-Id", report.receipt.receipt_id.clone()));
        return Ok(Some(response));
    }

    if request.method == "GET" && path == "/v1/agent-capture-runs" {
        let active_only = query_value(query, "active_only")
            .map(|value| !matches!(value, "0" | "false" | "no"))
            .unwrap_or(true);
        let reports = db.list_agent_capture_runs_page(
            active_only,
            query_usize(query, "offset", 0)?,
            query_usize(query, "limit", 100)?,
        )?;
        return Ok(Some(json_response(200, "OK", &reports)?));
    }
    if request.method == "GET" && parts_match(parts, &["v1", "agent-capture-runs", "*"]) {
        return Ok(Some(json_response(
            200,
            "OK",
            &db.agent_capture_run(parts[2])?,
        )?));
    }
    if request.method == "POST" && path == "/v1/agent-capture-runs" {
        let body: AgentCaptureRunRequest = serde_json::from_slice(&request.body)?;
        let report = db.begin_agent_capture_run(AgentCaptureRunInput {
            lane: body.lane,
            workdir: body.workdir,
            owner_agent: body.owner_agent,
            owner_session_id: body.owner_session_id,
            executor_agent: body.executor_agent,
            work_item_id: body.work_item_id,
            lease_ms: body.lease_ms,
            metadata_json: body.metadata.map(|value| value.to_string()),
        })?;
        return Ok(Some(json_response(201, "Created", &report)?));
    }
    if request.method == "POST" && path == "/v1/agent-capture-runs/reconcile" {
        return Ok(Some(json_response(
            200,
            "OK",
            &db.reconcile_expired_agent_capture_runs()?,
        )?));
    }
    if request.method == "POST" && parts_match(parts, &["v1", "agent-capture-runs", "*", "renew"]) {
        let body: AgentCaptureRunLeaseRequest = serde_json::from_slice(&request.body)?;
        let report = db.renew_agent_capture_run(
            parts[2],
            &body.owner_agent,
            &body.owner_session_id,
            body.lease_ms.unwrap_or(300_000),
        )?;
        return Ok(Some(json_response(200, "OK", &report)?));
    }
    if request.method == "POST" && parts_match(parts, &["v1", "agent-capture-runs", "*", "end"]) {
        let body: AgentCaptureRunLeaseRequest = serde_json::from_slice(&request.body)?;
        let report =
            db.end_agent_capture_run(parts[2], &body.owner_agent, &body.owner_session_id)?;
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if request.method == "GET" && parts_match(parts, &["v1", "agent-sessions", "*", "artifacts"]) {
        let reports = db.list_lane_artifacts_page(
            parts[2],
            query_value(query, "turn"),
            query_usize(query, "offset", 0)?,
            query_usize(query, "limit", 100)?,
        )?;
        return Ok(Some(json_response(200, "OK", &reports)?));
    }
    if request.method == "GET" && parts_match(parts, &["v1", "agent-artifacts", "*"]) {
        return Ok(Some(json_response(
            200,
            "OK",
            &db.lane_artifact(parts[2])?,
        )?));
    }
    if request.method == "POST" && parts_match(parts, &["v1", "agent-artifacts", "*", "redact"]) {
        let body: AgentArtifactRedactRequest = serde_json::from_slice(&request.body)?;
        return Ok(Some(json_response(
            200,
            "OK",
            &db.redact_lane_artifact(parts[2], &body.reason)?,
        )?));
    }
    if request.method == "GET" && parts_match(parts, &["v1", "agent-turns", "*", "evidence"]) {
        return Ok(Some(json_response(
            200,
            "OK",
            &db.turn_evidence_manifest(parts[2])?,
        )?));
    }
    if request.method == "GET" && parts_match(parts, &["v1", "agent-sessions", "*", "provenance"]) {
        let (nodes, edges) = db.list_session_provenance_page(
            parts[2],
            query_usize(query, "offset", 0)?,
            query_usize(query, "limit", 1_000)?,
        )?;
        return Ok(Some(json_response(
            200,
            "OK",
            &serde_json::json!({"session_id": parts[2], "nodes": nodes, "edges": edges}),
        )?));
    }
    if request.method == "GET" && parts_match(parts, &["v1", "agent-sessions", "*", "attestations"])
    {
        return Ok(Some(json_response(
            200,
            "OK",
            &db.list_session_attestations_page(
                parts[2],
                query_usize(query, "offset", 0)?,
                query_usize(query, "limit", 100)?,
            )?,
        )?));
    }
    if request.method == "POST"
        && parts_match(parts, &["v1", "agent-sessions", "*", "attestations"])
    {
        let body: AgentAttestationCreateRequest = if request.body.is_empty() {
            serde_json::from_value(serde_json::json!({}))?
        } else {
            serde_json::from_slice(&request.body)?
        };
        let report =
            db.create_session_attestation(parts[2], &body.capture_policy, body.metadata)?;
        return Ok(Some(json_response(201, "Created", &report)?));
    }
    if request.method == "POST" && parts_match(parts, &["v1", "agent-attestations", "*", "verify"])
    {
        return Ok(Some(json_response(
            200,
            "OK",
            &db.verify_session_attestation(parts[2])?,
        )?));
    }
    if request.method == "GET" && parts_match(parts, &["v1", "agent-attestations", "*"]) {
        return Ok(Some(json_response(
            200,
            "OK",
            &db.session_attestation(parts[2])?,
        )?));
    }
    if request.method == "GET" && parts_match(parts, &["v1", "agent-sessions", "*", "export"]) {
        if query_value(query, "format").is_some_and(|format| format != "agent-trace") {
            return Err(crate::Error::InvalidInput(
                "only `format=agent-trace` is supported".to_string(),
            ));
        }
        let attachments = query_value(query, "attachments")
            .is_some_and(|value| matches!(value, "1" | "true" | "yes"));
        return Ok(Some(json_response(
            200,
            "OK",
            &db.export_agent_trace(parts[2], attachments)?,
        )?));
    }

    if request.method == "GET" && path == "/v1/agent-learnings" {
        let reports = db.list_learnings_page(
            query_value(query, "session"),
            query_value(query, "status"),
            query_usize(query, "offset", 0)?,
            query_usize(query, "limit", 100)?,
        )?;
        return Ok(Some(json_response(200, "OK", &reports)?));
    }
    if request.method == "POST" && path == "/v1/agent-learnings" {
        let body: LearningInput = serde_json::from_slice(&request.body)?;
        return Ok(Some(json_response(
            201,
            "Created",
            &db.propose_learning(body)?,
        )?));
    }
    if request.method == "GET" && parts_match(parts, &["v1", "agent-learnings", "*"]) {
        return Ok(Some(json_response(200, "OK", &db.learning(parts[2])?)?));
    }
    if request.method == "POST"
        && parts.len() == 4
        && parts[0] == "v1"
        && parts[1] == "agent-learnings"
        && matches!(parts[3], "accept" | "reject")
    {
        let body: AgentLearningReviewRequest = serde_json::from_slice(&request.body)?;
        let report = db.review_learning(parts[2], parts[3] == "accept", &body.reviewer)?;
        return Ok(Some(json_response(200, "OK", &report)?));
    }

    if request.method == "GET" && parts_match(parts, &["v1", "agent-sessions", "*", "git-links"]) {
        return Ok(Some(json_response(
            200,
            "OK",
            &db.list_git_agent_links_page(
                parts[2],
                query_usize(query, "offset", 0)?,
                query_usize(query, "limit", 100)?,
            )?,
        )?));
    }
    if request.method == "POST" && path == "/v1/agent-git-links" {
        let body: GitAgentLinkInput = serde_json::from_slice(&request.body)?;
        return Ok(Some(json_response(
            201,
            "Created",
            &db.link_git_commit_to_agent(body)?,
        )?));
    }
    if request.method == "GET" && parts_match(parts, &["v1", "agent-git-links", "*"]) {
        return Ok(Some(json_response(
            200,
            "OK",
            &db.git_agent_link(parts[2])?,
        )?));
    }

    Ok(None)
}

fn parts_match(parts: &[&str], pattern: &[&str]) -> bool {
    parts.len() == pattern.len()
        && parts
            .iter()
            .zip(pattern)
            .all(|(part, expected)| *expected == "*" || part == expected)
}

fn payload_string(payload: &serde_json::Value, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        payload
            .get(*name)
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string)
    })
}

fn payload_i64(payload: &serde_json::Value, names: &[&str]) -> Option<i64> {
    names.iter().find_map(|name| {
        let value = payload.get(*name)?;
        value
            .as_i64()
            .or_else(|| value.as_str().and_then(|raw| raw.parse().ok()))
    })
}

fn hook_dedupe_key(
    provider: &str,
    native_event: &str,
    session_id: Option<&str>,
    turn_id: Option<&str>,
    payload: &serde_json::Value,
) -> String {
    let bytes = serde_json::to_vec(payload).unwrap_or_default();
    let digest = hex::encode(Sha256::digest(bytes));
    format!(
        "http:{provider}:{native_event}:{}:{}:{digest}",
        session_id.unwrap_or("none"),
        turn_id.unwrap_or("none")
    )
}
