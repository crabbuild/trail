pub const TURN_EVIDENCE_MANIFEST_SCHEMA: &str = "trail.turn_evidence_manifest";
pub const TURN_EVIDENCE_MANIFEST_VERSION: u16 = 1;
pub const SESSION_ATTESTATION_SCHEMA: &str = "trail.session_attestation";
pub const SESSION_ATTESTATION_VERSION: u16 = 1;
pub const AGENT_TRACE_SCHEMA: &str = "trail.agent_trace";
pub const AGENT_TRACE_VERSION: u16 = 1;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TurnEvidenceCoverage {
    pub receipt_ids: Vec<String>,
    pub event_ids: Vec<String>,
    pub message_ids: Vec<String>,
    pub artifact_ids: Vec<String>,
    pub change_ids: Vec<ChangeId>,
    pub tool_span_ids: Vec<String>,
    pub approval_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TurnEvidenceStatement {
    pub schema: String,
    pub version: u16,
    pub workspace_id: String,
    pub lane_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub before_change: ChangeId,
    pub after_change: ChangeId,
    pub turn_status: String,
    pub coverage: TurnEvidenceCoverage,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TurnEvidenceManifest {
    pub manifest_id: String,
    pub lane_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub schema_version: u16,
    pub object_id: ObjectId,
    pub digest: String,
    pub created_at: i64,
    pub statement: TurnEvidenceStatement,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProvenanceNodeInput {
    pub session_id: String,
    pub turn_id: Option<String>,
    pub node_kind: String,
    pub summary: String,
    pub event_id: Option<String>,
    pub span_id: Option<String>,
    pub message_id: Option<String>,
    pub change_id: Option<String>,
    pub artifact_id: Option<String>,
    pub source_confidence: String,
    pub classifier_version: Option<String>,
    pub attributes: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProvenanceNode {
    pub provenance_node_id: String,
    pub lane_id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub node_kind: String,
    pub summary: String,
    pub event_id: Option<String>,
    pub span_id: Option<String>,
    pub message_id: Option<String>,
    pub change_id: Option<String>,
    pub artifact_id: Option<String>,
    pub source_confidence: String,
    pub classifier_version: Option<String>,
    pub created_at: i64,
    pub attributes: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProvenanceEdgeInput {
    pub from_node_id: String,
    pub to_node_id: String,
    pub relation: String,
    pub source_confidence: String,
    pub receipt_id: Option<String>,
    pub attributes: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProvenanceEdge {
    pub provenance_edge_id: String,
    pub lane_id: String,
    pub session_id: String,
    pub from_node_id: String,
    pub to_node_id: String,
    pub relation: String,
    pub source_confidence: String,
    pub receipt_id: Option<String>,
    pub created_at: i64,
    pub attributes: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ActivityClassificationReport {
    pub session_id: String,
    pub classifier_version: String,
    pub nodes: Vec<ProvenanceNode>,
    pub edges: Vec<ProvenanceEdge>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionAttestationTurn {
    pub turn_id: String,
    pub change_id: Option<String>,
    pub evidence_manifest_id: String,
    pub evidence_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionAttestationStatement {
    pub schema: String,
    pub version: u16,
    pub workspace_id: String,
    pub lane_id: String,
    pub session_id: String,
    pub capture_run_id: Option<String>,
    pub previous_attestation_id: Option<String>,
    pub turns: Vec<SessionAttestationTurn>,
    pub capture_policy: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SessionAttestation {
    pub attestation_id: String,
    pub lane_id: String,
    pub session_id: String,
    pub capture_run_id: Option<String>,
    pub previous_attestation_id: Option<String>,
    pub statement_object_id: ObjectId,
    pub statement_digest: String,
    pub signature: Option<serde_json::Value>,
    pub status: String,
    pub created_at: i64,
    pub superseded_by: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub turns: Vec<SessionAttestationTurn>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AttestationSignature {
    pub algorithm: String,
    pub key_id: String,
    pub public_key_hex: String,
    pub signature_hex: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AttestationKeyRevocation {
    pub key_id: String,
    pub public_key_hex: String,
    pub reason: String,
    pub revoked_at: i64,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AttestationVerificationReport {
    pub attestation_id: String,
    pub statement_digest_valid: bool,
    pub evidence_digests_valid: bool,
    pub chain_valid: bool,
    pub signature_status: String,
    pub valid: bool,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearningInput {
    pub session_id: String,
    pub turn_id: Option<String>,
    pub scope: String,
    pub body: String,
    pub confidence: Option<f64>,
    pub source_artifact_id: Option<String>,
    pub anchor: Option<serde_json::Value>,
    pub expires_at: Option<i64>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Learning {
    pub learning_id: String,
    pub lane_id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub scope: String,
    pub body: String,
    pub status: String,
    pub confidence: Option<f64>,
    pub source_artifact_id: Option<String>,
    pub anchor: Option<serde_json::Value>,
    pub created_at: i64,
    pub reviewed_at: Option<i64>,
    pub reviewer: Option<String>,
    pub expires_at: Option<i64>,
    pub superseded_by: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GitAgentLinkInput {
    pub session_id: String,
    pub turn_id: Option<String>,
    pub git_commit: String,
    pub from_change: Option<String>,
    pub through_change: Option<String>,
    pub confidence: String,
    pub source: String,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GitAgentLink {
    pub git_agent_link_id: String,
    pub git_commit: String,
    pub lane_id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub from_change: Option<String>,
    pub through_change: Option<String>,
    pub confidence: String,
    pub source: String,
    pub created_at: i64,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PortableAgentArtifact {
    pub artifact_id: String,
    pub provider: String,
    pub artifact_kind: String,
    pub format: String,
    pub source: AgentEvidenceSource,
    pub content_digest: String,
    pub size_bytes: u64,
    pub start_offset: Option<u64>,
    pub end_offset: Option<u64>,
    pub trust: String,
    pub retention_status: String,
    pub content: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PortableAgentTrace {
    pub schema: String,
    pub version: u16,
    pub source_workspace_id: String,
    pub lane_id: String,
    pub session_id: String,
    pub session_status: String,
    pub evidence_manifests: Vec<TurnEvidenceManifest>,
    pub artifacts: Vec<PortableAgentArtifact>,
    pub provenance_nodes: Vec<ProvenanceNode>,
    pub provenance_edges: Vec<ProvenanceEdge>,
    pub attestations: Vec<SessionAttestation>,
    pub learnings: Vec<Learning>,
    pub git_links: Vec<GitAgentLink>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentTraceVerificationReport {
    pub schema_valid: bool,
    pub ordering_valid: bool,
    pub artifact_digests_valid: bool,
    pub evidence_references_valid: bool,
    pub attestation_references_valid: bool,
    pub valid: bool,
    pub diagnostics: Vec<String>,
}

impl PortableAgentTrace {
    pub fn to_canonical_json(&self) -> crate::Result<Vec<u8>> {
        let mut bytes = serde_json::to_vec(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    pub fn from_json(bytes: &[u8]) -> crate::Result<Self> {
        const MAX_TRACE_BYTES: usize = 256 * 1024 * 1024;
        if bytes.len() > MAX_TRACE_BYTES {
            return Err(crate::Error::InvalidInput(format!(
                "portable agent trace is {} bytes; maximum is {MAX_TRACE_BYTES}",
                bytes.len()
            )));
        }
        let trace: Self = serde_json::from_slice(bytes)?;
        let verification = trace.verify();
        if !verification.valid {
            return Err(crate::Error::InvalidInput(format!(
                "portable agent trace failed verification: {}",
                verification.diagnostics.join("; ")
            )));
        }
        Ok(trace)
    }

    pub fn verify(&self) -> AgentTraceVerificationReport {
        use sha2::{Digest, Sha256};

        let schema_valid = self.schema == AGENT_TRACE_SCHEMA && self.version == AGENT_TRACE_VERSION;
        let mut diagnostics = Vec::new();
        if !schema_valid {
            diagnostics.push(format!(
                "unsupported trace schema `{}` version {}",
                self.schema, self.version
            ));
        }
        let ordering_valid = sorted_unique_by(&self.evidence_manifests, |value| &value.turn_id)
            && sorted_unique_by(&self.artifacts, |value| &value.artifact_id)
            && sorted_unique_by(&self.provenance_nodes, |value| &value.provenance_node_id)
            && sorted_unique_by(&self.provenance_edges, |value| &value.provenance_edge_id)
            && sorted_unique_by(&self.attestations, |value| &value.attestation_id)
            && sorted_unique_by(&self.learnings, |value| &value.learning_id)
            && sorted_unique_by(&self.git_links, |value| &value.git_agent_link_id);
        if !ordering_valid {
            diagnostics.push("trace collections are not canonically sorted and unique".to_string());
        }
        let artifact_digests_valid = self.artifacts.iter().all(|artifact| {
            artifact.content.as_ref().is_none_or(|content| {
                artifact.size_bytes == content.len() as u64
                    && artifact.content_digest
                        == format!("sha256:{}", hex::encode(Sha256::digest(content)))
            })
        });
        if !artifact_digests_valid {
            diagnostics.push("one or more attachment digests are invalid".to_string());
        }
        let artifact_ids = self
            .artifacts
            .iter()
            .map(|artifact| artifact.artifact_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        let evidence_references_valid = self.evidence_manifests.iter().all(|manifest| {
            manifest.session_id == self.session_id
                && manifest.lane_id == self.lane_id
                && manifest
                    .statement
                    .coverage
                    .artifact_ids
                    .iter()
                    .all(|artifact_id| artifact_ids.contains(artifact_id.as_str()))
        });
        if !evidence_references_valid {
            diagnostics
                .push("an evidence manifest has a missing or cross-session reference".to_string());
        }
        let manifests = self
            .evidence_manifests
            .iter()
            .map(|manifest| (manifest.manifest_id.as_str(), manifest.digest.as_str()))
            .collect::<std::collections::BTreeMap<_, _>>();
        let attestation_references_valid = self.attestations.iter().all(|attestation| {
            attestation.session_id == self.session_id
                && attestation.lane_id == self.lane_id
                && attestation.turns.iter().all(|turn| {
                    manifests
                        .get(turn.evidence_manifest_id.as_str())
                        .is_some_and(|digest| *digest == turn.evidence_digest)
                })
        });
        if !attestation_references_valid {
            diagnostics
                .push("an attestation references missing or mismatched evidence".to_string());
        }
        AgentTraceVerificationReport {
            schema_valid,
            ordering_valid,
            artifact_digests_valid,
            evidence_references_valid,
            attestation_references_valid,
            valid: schema_valid
                && ordering_valid
                && artifact_digests_valid
                && evidence_references_valid
                && attestation_references_valid,
            diagnostics,
        }
    }
}

fn sorted_unique_by<T>(values: &[T], key: impl Fn(&T) -> &str) -> bool {
    values.windows(2).all(|pair| key(&pair[0]) < key(&pair[1]))
}
