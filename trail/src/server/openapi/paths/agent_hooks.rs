use serde_json::{json, Value};

use super::{openapi_operation, openapi_path_param, openapi_query};

pub(super) fn agent_hook_paths() -> Value {
    json!({
        "/v1/agent-integrations/capabilities": {
            "get": openapi_operation("agentIntegrationCapabilities", "Agent integration capabilities", "List the built-in native hook and ACP capability contracts for every supported provider.", vec![], None, true)
        },
        "/v1/agent-hooks/{provider}/{event}": {
            "post": openapi_operation("agentHookIngest", "Ingest native agent hook", "Durably journal one arbitrary provider payload before asynchronous semantic replay. This endpoint additionally requires the daemon to have token authentication configured.", vec![
                openapi_path_param("provider", "string"),
                openapi_path_param("event", "string"),
                openapi_query("installation", "string"),
                openapi_query("dedupe_key", "string")
            ], Some("AgentHookProviderPayload"), true)
        },
        "/v1/agent-hooks/installations": {
            "get": openapi_operation("agentHookInstallations", "Agent hook installations", "List Trail-owned native hook installations and their persisted ownership metadata.", vec![openapi_query("provider", "string")], None, true)
        },
        "/v1/agent-hooks/installations/{id}": {
            "get": openapi_operation("agentHookInstallation", "Agent hook installation", "Show one native hook installation record.", vec![openapi_path_param("id", "string")], None, true)
        },
        "/v1/agent-hooks/receipts": {
            "get": openapi_operation("agentHookReceipts", "Agent hook receipts", "List durable, redacted receipt journal rows for diagnostics and recovery.", vec![
                openapi_query("provider", "string"),
                openapi_query("status", "string"),
                openapi_query("offset", "integer"),
                openapi_query("limit", "integer")
            ], None, true)
        },
        "/v1/agent-hooks/receipts/{id}": {
            "get": openapi_operation("agentHookReceipt", "Agent hook receipt", "Show one durable redacted receipt journal row.", vec![openapi_path_param("id", "string")], None, true)
        },
        "/v1/agent-hooks/receipts/{id}/replay": {
            "post": openapi_operation("agentHookReceiptReplay", "Replay agent hook receipt", "Replay one durable receipt through the provider parser and shared lifecycle coordinator.", vec![openapi_path_param("id", "string")], None, true)
        },
        "/v1/agent-hooks/receipts/{id}/retry": {
            "post": openapi_operation("agentHookReceiptRetry", "Retry agent hook receipt", "Move one retrying or quarantined receipt back to the replay queue.", vec![openapi_path_param("id", "string")], None, true)
        },
        "/v1/agent-hooks/receipts/{id}/discard": {
            "post": openapi_operation("agentHookReceiptDiscard", "Discard agent hook receipt", "Explicitly discard one received, retrying, or quarantined receipt while retaining its audit row.", vec![openapi_path_param("id", "string")], None, true)
        },
        "/v1/agent-capture-runs": {
            "get": openapi_operation("agentCaptureRuns", "Agent capture runs", "List active or historical managed capture runs.", vec![openapi_query("active_only", "boolean"), openapi_query("offset", "integer"), openapi_query("limit", "integer")], None, true),
            "post": openapi_operation("agentCaptureRunBegin", "Begin agent capture run", "Declare a leased managed run used to correlate nested native or ACP sessions by owner, executor, and canonical workdir.", vec![], Some("AgentCaptureRunRequest"), true)
        },
        "/v1/agent-capture-runs/{id}/renew": {
            "post": openapi_operation("agentCaptureRunRenew", "Renew agent capture run", "Renew a managed capture run using its exact owner identity.", vec![openapi_path_param("id", "string")], Some("AgentCaptureRunLeaseRequest"), true)
        },
        "/v1/agent-capture-runs/{id}": {
            "get": openapi_operation("agentCaptureRun", "Agent capture run", "Show one managed capture-run lease and ownership record.", vec![openapi_path_param("id", "string")], None, true)
        },
        "/v1/agent-capture-runs/reconcile": {
            "post": openapi_operation("agentCaptureRunReconcile", "Reconcile expired capture runs", "Expire abandoned managed-run leases and close their open turns and sessions as interrupted.", vec![], None, true)
        },
        "/v1/agent-capture-runs/{id}/end": {
            "post": openapi_operation("agentCaptureRunEnd", "End agent capture run", "Idempotently end a managed capture run using its exact owner identity.", vec![openapi_path_param("id", "string")], Some("AgentCaptureRunLeaseRequest"), true)
        },
        "/v1/agent-sessions/{id}/artifacts": {
            "get": openapi_operation("agentSessionArtifacts", "Agent session artifacts", "List immutable transcript, export, and evidence artifacts for one session.", vec![openapi_path_param("id", "string"), openapi_query("turn", "string"), openapi_query("offset", "integer"), openapi_query("limit", "integer")], None, true)
        },
        "/v1/agent-artifacts/{id}": {
            "get": openapi_operation("agentArtifact", "Agent artifact", "Show immutable artifact metadata without returning its potentially sensitive attachment bytes.", vec![openapi_path_param("id", "string")], None, true)
        },
        "/v1/agent-artifacts/{id}/redact": {
            "post": openapi_operation("agentArtifactRedact", "Redact agent artifact attachment", "Remove attachment access while preserving immutable digest, manifest, and attestation identity.", vec![openapi_path_param("id", "string")], Some("AgentArtifactRedactRequest"), true)
        },
        "/v1/agent-turns/{id}/evidence": {
            "get": openapi_operation("agentTurnEvidence", "Agent turn evidence manifest", "Show and verify the deterministic immutable evidence manifest for one completed turn.", vec![openapi_path_param("id", "string")], None, true)
        },
        "/v1/agent-sessions/{id}/provenance": {
            "get": openapi_operation("agentSessionProvenance", "Agent session provenance", "Return the queryable causal provenance nodes and edges for one session.", vec![openapi_path_param("id", "string"), openapi_query("offset", "integer"), openapi_query("limit", "integer")], None, true)
        },
        "/v1/agent-sessions/{id}/attestations": {
            "get": openapi_operation("agentSessionAttestations", "Agent session attestations", "List immutable chained attestation segments for one session.", vec![openapi_path_param("id", "string"), openapi_query("offset", "integer"), openapi_query("limit", "integer")], None, true),
            "post": openapi_operation("agentSessionAttestationCreate", "Create agent session attestation", "Create the next idempotent attestation segment over completed turns not covered by the predecessor.", vec![openapi_path_param("id", "string")], Some("AgentAttestationCreateRequest"), true)
        },
        "/v1/agent-attestations/{id}": {
            "get": openapi_operation("agentAttestation", "Agent attestation", "Show one immutable session attestation and its exact turn coverage.", vec![openapi_path_param("id", "string")], None, true)
        },
        "/v1/agent-attestations/{id}/verify": {
            "post": openapi_operation("agentAttestationVerify", "Verify agent attestation", "Verify statement, evidence, predecessor chain, signature, and revocation status.", vec![openapi_path_param("id", "string")], None, true)
        },
        "/v1/agent-sessions/{id}/export": {
            "get": openapi_operation("agentSessionExport", "Export portable agent trace", "Project one session into the verified vendor-neutral agent-trace representation.", vec![openapi_path_param("id", "string"), openapi_query("format", "string"), openapi_query("attachments", "boolean")], None, true)
        },
        "/v1/agent-learnings": {
            "get": openapi_operation("agentLearnings", "Agent learnings", "List reviewable reusable findings without injecting them into provider files.", vec![openapi_query("session", "string"), openapi_query("status", "string"), openapi_query("offset", "integer"), openapi_query("limit", "integer")], None, true),
            "post": openapi_operation("agentLearningPropose", "Propose agent learning", "Create a redacted, evidence-linked learning proposal requiring explicit review before context use.", vec![], Some("AgentLearningRequest"), true)
        },
        "/v1/agent-learnings/{id}": {
            "get": openapi_operation("agentLearning", "Agent learning", "Show one reviewable learning record.", vec![openapi_path_param("id", "string")], None, true)
        },
        "/v1/agent-learnings/{id}/accept": {
            "post": openapi_operation("agentLearningAccept", "Accept agent learning", "Accept one proposed learning for explicit, bounded context use.", vec![openapi_path_param("id", "string")], Some("AgentLearningReviewRequest"), true)
        },
        "/v1/agent-learnings/{id}/reject": {
            "post": openapi_operation("agentLearningReject", "Reject agent learning", "Reject one proposed learning without deleting its audit record.", vec![openapi_path_param("id", "string")], Some("AgentLearningReviewRequest"), true)
        },
        "/v1/agent-sessions/{id}/git-links": {
            "get": openapi_operation("agentSessionGitLinks", "Agent session Git links", "List explicit mappings between a session's exact Trail changes and Git commits.", vec![openapi_path_param("id", "string"), openapi_query("offset", "integer"), openapi_query("limit", "integer")], None, true)
        },
        "/v1/agent-git-links": {
            "post": openapi_operation("agentGitLinkCreate", "Create agent Git link", "Create an explicit exact association between a Git commit, Trail session, turn, and change boundary.", vec![], Some("AgentGitLinkRequest"), true)
        },
        "/v1/agent-git-links/{id}": {
            "get": openapi_operation("agentGitLink", "Agent Git link", "Show one exact Git-to-Trail association.", vec![openapi_path_param("id", "string")], None, true)
        }
    })
}
