use std::collections::BTreeMap;

pub const ARTIFACT_RESOLUTION_PLAN_VERSION: u16 = 1;
pub const ARTIFACT_RESOLUTION_SNAPSHOT_VERSION: u16 = 1;
pub const ARTIFACT_RESOLUTION_SNAPSHOT_KIND: &str = "ArtifactResolutionSnapshot";
pub const ARTIFACT_RESOLUTION_CONTENT_KIND: &str = "ArtifactResolutionContent";
pub const ARTIFACT_RESOLUTION_PLAN_KIND: &str = "ArtifactResolutionPlan";
pub const ARTIFACT_RESOLUTION_CAPTURE_KIND: &str = "ArtifactResolutionCapture";
pub const ARTIFACT_RESOLUTION_FAILURE_KIND: &str = "ArtifactResolutionFailure";
pub const ARTIFACT_DIRECTORY_NODE_VERSION: u16 = 1;
pub const ARTIFACT_FILE_NODE_VERSION: u16 = 1;
pub const ARTIFACT_BLOB_VERSION: u16 = 1;
pub const ARTIFACT_CHUNK_LIST_VERSION: u16 = 1;
pub const ARTIFACT_CHUNK_VERSION: u16 = 1;
pub const ARTIFACT_TREE_ROOT_VERSION: u16 = 1;
pub const ARTIFACT_ENVELOPE_VERSION: u16 = 1;
pub const ARTIFACT_DIVERGENCE_EVIDENCE_VERSION: u16 = 1;
pub const ARTIFACT_VALIDATION_RECEIPT_VERSION: u16 = 1;
pub const ARTIFACT_ATTESTATION_VERSION: u16 = 1;

pub const ARTIFACT_DIRECTORY_NODE_KIND: &str = "ArtifactDirectoryNode";
pub const ARTIFACT_FILE_NODE_KIND: &str = "ArtifactFileNode";
pub const ARTIFACT_BLOB_KIND: &str = "ArtifactBlob";
pub const ARTIFACT_CHUNK_LIST_KIND: &str = "ArtifactChunkList";
pub const ARTIFACT_CHUNK_KIND: &str = "ArtifactChunk";
pub const ARTIFACT_TREE_ROOT_KIND: &str = "ArtifactTreeRoot";
pub const ARTIFACT_ENVELOPE_KIND: &str = "ArtifactEnvelope";
pub const ARTIFACT_DIVERGENCE_EVIDENCE_KIND: &str = "ArtifactDivergenceEvidence";
pub const ARTIFACT_VALIDATION_RECEIPT_KIND: &str = "ArtifactValidationReceipt";
pub const ARTIFACT_ATTESTATION_KIND: &str = "ArtifactAttestation";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArtifactDirectoryEntryTargetV1 {
    Directory { node_id: ArtifactTreeId },
    File { node_id: ArtifactFileId },
    Symlink { target: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArtifactDirectoryEntryV1 {
    pub name: String,
    pub target: ArtifactDirectoryEntryTargetV1,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactDirectoryNodeV1 {
    pub version: u16,
    pub entries: Vec<ArtifactDirectoryEntryV1>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArtifactFileContentV1 {
    Blob { blob_id: ArtifactBlobId },
    Chunks { chunk_list_id: ArtifactChunkListId },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactFileNodeV1 {
    pub version: u16,
    pub mode: u32,
    pub executable: bool,
    pub size_bytes: u64,
    pub content_sha256: String,
    pub content: ArtifactFileContentV1,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactBlobV1 {
    pub version: u16,
    pub content_sha256: String,
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactChunkV1 {
    pub version: u16,
    pub content_sha256: String,
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactChunkRefV1 {
    pub chunk_id: ArtifactChunkId,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactChunkListV1 {
    pub version: u16,
    pub algorithm: String,
    pub file_size_bytes: u64,
    pub file_sha256: String,
    pub chunks: Vec<ArtifactChunkRefV1>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactTreeRootV1 {
    pub version: u16,
    pub root_directory_id: ArtifactTreeId,
    pub logical_bytes: u64,
    pub entry_count: u64,
    pub path_normalizer: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "identity_version", rename_all = "snake_case")]
pub enum ArtifactDesiredIdentityV1 {
    WorkspaceLayerV1 {
        cache_key: String,
        canonical_key: WorkspaceLayerKeyV1,
    },
    ArtifactDesiredV2 {
        desired_key: ArtifactDesiredKeyV2,
    },
}

impl ArtifactDesiredIdentityV1 {
    pub fn desired_key_v2(&self) -> Option<&ArtifactDesiredKeyV2> {
        match self {
            Self::WorkspaceLayerV1 { .. } => None,
            Self::ArtifactDesiredV2 { desired_key } => Some(desired_key),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactEnvelopeV1 {
    pub version: u16,
    pub desired_identity: ArtifactDesiredIdentityV1,
    pub tree_root_id: ArtifactTreeId,
    pub component_id: String,
    pub output_name: String,
    pub output_policy: EnvironmentOutputPolicy,
    pub portability_scope: String,
    pub trust_scope: String,
    #[serde(default, skip_serializing_if = "ArtifactSecretTaintV1::is_clear")]
    pub secret_taint: ArtifactSecretTaintV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution_snapshot_id: Option<ObjectId>,
    #[serde(default)]
    pub validation_receipt_ids: Vec<ObjectId>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactAttestationSignatureV1 {
    pub algorithm: String,
    pub key_id: String,
    pub public_key_hex: String,
    pub signature_hex: String,
}

/// Deterministic host statement about one sealed artifact envelope.
///
/// Local storage paths and wall-clock observations are intentionally absent.
/// Publisher and package fields identify trust evidence but do not grant trust;
/// attachment rechecks their current local status.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactAttestationStatementV1 {
    pub version: u16,
    pub envelope_id: ArtifactEnvelopeId,
    pub desired_identity: ArtifactDesiredIdentityV1,
    pub tree_root_id: ArtifactTreeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_root: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution_snapshot_id: Option<ObjectId>,
    #[serde(default)]
    pub upstream_identities: BTreeMap<String, String>,
    pub producer_identity: String,
    pub producer_trust: ArtifactProducerTrustTierV1,
    pub adapter_implementation_version: String,
    pub adapter_distribution_digest: String,
    pub adapter_protocol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_key_id: Option<String>,
    #[serde(default)]
    pub executable_identities: BTreeMap<String, String>,
    pub platform: String,
    pub architecture: String,
    pub abi: String,
    pub capability_ceiling: ArtifactCapabilityCeilingV1,
    pub sandbox_enforcement: String,
    pub network_policy: String,
    pub script_policy: ArtifactScriptPolicyV1,
    pub output_name: String,
    pub output_policy: EnvironmentOutputPolicy,
    pub portability_scope: String,
    pub trust_scope: String,
    #[serde(default)]
    pub validation_receipt_ids: Vec<ObjectId>,
    #[serde(default, skip_serializing_if = "ArtifactSecretTaintV1::is_clear")]
    pub secret_taint: ArtifactSecretTaintV1,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactAttestationV1 {
    pub statement: ArtifactAttestationStatementV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<ArtifactAttestationSignatureV1>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactAttestationReportV1 {
    pub attestation_id: ArtifactAttestationId,
    pub object_id: ObjectId,
    pub state: String,
    pub attestation: ArtifactAttestationV1,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactAttestationVerificationReportV1 {
    pub attestation_id: ArtifactAttestationId,
    pub envelope_id: ArtifactEnvelopeId,
    pub state: String,
    pub content_identity_valid: bool,
    pub envelope_binding_valid: bool,
    pub producer_trusted: bool,
    pub signature_status: String,
    pub valid: bool,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactResolutionContentV1 {
    pub version: u16,
    pub content_sha256: String,
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
}

/// One pinned source input that a resolver may read.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArtifactResolutionInputV1 {
    pub source_path: String,
    pub content_hash: String,
    pub size_bytes: u64,
}

/// The semantic role of an environment variable made visible to a resolver.
///
/// Values are never part of this contract. Credential material is supplied by
/// an opaque handle at execution time and must not enter durable objects.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactEnvironmentRoleV1 {
    Identity,
    Runtime,
    CredentialHandle,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactScriptPolicyV1 {
    Deny,
    AllowDeclared,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactProducerTrustTierV1 {
    ReviewedBuiltin,
    CertifiedSignedPlugin,
    LocallyTrustedPlugin,
    RepositoryDeclaration,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactExecutionPhaseV1 {
    DiscoveryPlanning,
    Resolve,
    Construct,
    Validate,
    MountedExecution,
    SourceExport,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactNetworkCapabilityV1 {
    Deny,
    ExactAuthorities,
    ReviewedBuiltinManaged,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactFilesystemReadCapabilityV1 {
    None,
    DeclaredInputs,
    PinnedSourceClosure,
    ArtifactCandidate,
    LaneView,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactFilesystemWriteCapabilityV1 {
    None,
    IsolatedCandidate,
    CandidateAndHostCache,
    ValidationReceipt,
    LaneBindings,
    SourceExportDestination,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactProcessCapabilityV1 {
    Deny,
    DeclaredExecutable,
    ReviewedBuiltinGraph,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactSecretCapabilityV1 {
    Deny,
    OpaqueHandles,
    RuntimeInjection,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ArtifactSecretTaintV1 {
    #[default]
    Clear,
    Tainted { channels: Vec<String> },
}

impl ArtifactSecretTaintV1 {
    pub fn is_clear(&self) -> bool {
        matches!(self, Self::Clear)
    }

    pub fn channels(&self) -> &[String] {
        match self {
            Self::Clear => &[],
            Self::Tainted { channels } => channels,
        }
    }
}

/// Maximum authority available to one producer tier in one execution phase.
///
/// This is a host policy result, not an adapter request. Repository and plugin
/// declarations may narrow it but cannot widen it, and publication authority
/// is intentionally absent from every executable phase.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactCapabilityCeilingV1 {
    pub producer_trust: ArtifactProducerTrustTierV1,
    pub phase: ArtifactExecutionPhaseV1,
    pub network: ArtifactNetworkCapabilityV1,
    pub filesystem_read: ArtifactFilesystemReadCapabilityV1,
    pub filesystem_write: ArtifactFilesystemWriteCapabilityV1,
    pub processes: ArtifactProcessCapabilityV1,
    pub secrets: ArtifactSecretCapabilityV1,
    pub publication_authority: bool,
}

impl ArtifactCapabilityCeilingV1 {
    pub fn for_phase(
        producer_trust: ArtifactProducerTrustTierV1,
        phase: ArtifactExecutionPhaseV1,
    ) -> Self {
        use ArtifactExecutionPhaseV1 as Phase;
        use ArtifactFilesystemReadCapabilityV1 as Read;
        use ArtifactFilesystemWriteCapabilityV1 as Write;
        use ArtifactNetworkCapabilityV1 as Network;
        use ArtifactProcessCapabilityV1 as Process;
        use ArtifactProducerTrustTierV1 as Trust;
        use ArtifactSecretCapabilityV1 as Secrets;

        let (network, filesystem_read, filesystem_write, processes, secrets) = match phase {
            Phase::DiscoveryPlanning => (
                Network::Deny,
                Read::None,
                Write::None,
                Process::Deny,
                Secrets::Deny,
            ),
            Phase::Resolve => (
                Network::ExactAuthorities,
                if producer_trust == Trust::ReviewedBuiltin {
                    Read::PinnedSourceClosure
                } else {
                    Read::DeclaredInputs
                },
                if producer_trust == Trust::ReviewedBuiltin {
                    Write::CandidateAndHostCache
                } else {
                    Write::IsolatedCandidate
                },
                if producer_trust == Trust::ReviewedBuiltin {
                    Process::ReviewedBuiltinGraph
                } else {
                    Process::DeclaredExecutable
                },
                Secrets::OpaqueHandles,
            ),
            Phase::Construct => match producer_trust {
                Trust::ReviewedBuiltin => (
                    Network::ReviewedBuiltinManaged,
                    Read::PinnedSourceClosure,
                    Write::CandidateAndHostCache,
                    Process::ReviewedBuiltinGraph,
                    Secrets::Deny,
                ),
                Trust::CertifiedSignedPlugin | Trust::LocallyTrustedPlugin => (
                    Network::Deny,
                    Read::DeclaredInputs,
                    Write::CandidateAndHostCache,
                    Process::DeclaredExecutable,
                    Secrets::Deny,
                ),
                Trust::RepositoryDeclaration => (
                    Network::Deny,
                    Read::DeclaredInputs,
                    Write::IsolatedCandidate,
                    Process::DeclaredExecutable,
                    Secrets::Deny,
                ),
            },
            Phase::Validate => (
                Network::Deny,
                Read::ArtifactCandidate,
                Write::ValidationReceipt,
                if producer_trust == Trust::ReviewedBuiltin {
                    Process::ReviewedBuiltinGraph
                } else {
                    Process::DeclaredExecutable
                },
                Secrets::Deny,
            ),
            Phase::MountedExecution => match producer_trust {
                Trust::RepositoryDeclaration => (
                    Network::Deny,
                    Read::None,
                    Write::None,
                    Process::Deny,
                    Secrets::Deny,
                ),
                Trust::ReviewedBuiltin => (
                    Network::Deny,
                    Read::LaneView,
                    Write::LaneBindings,
                    Process::ReviewedBuiltinGraph,
                    Secrets::RuntimeInjection,
                ),
                Trust::CertifiedSignedPlugin | Trust::LocallyTrustedPlugin => (
                    Network::Deny,
                    Read::LaneView,
                    Write::LaneBindings,
                    Process::DeclaredExecutable,
                    Secrets::RuntimeInjection,
                ),
            },
            Phase::SourceExport => (
                Network::Deny,
                Read::ArtifactCandidate,
                Write::SourceExportDestination,
                Process::Deny,
                Secrets::Deny,
            ),
        };
        Self {
            producer_trust,
            phase,
            network,
            filesystem_read,
            filesystem_write,
            processes,
            secrets,
            publication_authority: false,
        }
    }
}

/// Finite host-enforced ceilings for a resolver attempt.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactActionLimitsV1 {
    pub timeout_ms: u64,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub candidate_bytes: u64,
    pub candidate_entries: u64,
    pub child_processes: u32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactValidationKindV1 {
    Structural,
    Loadability,
    Framework,
    Policy,
    Gate,
    Reproducibility,
    /// Legacy wire value retained for exact compatibility with existing plans.
    Ecosystem,
}

/// A deterministic validation declaration applied before snapshot publication.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArtifactValidationV1 {
    pub name: String,
    pub kind: ArtifactValidationKindV1,
    pub required: bool,
    #[serde(default)]
    pub parameters: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactValidationOutcomeV1 {
    Passed,
    Failed,
}

/// Canonical, path-independent evidence for one host-run validation.
///
/// Wall-clock time and local filesystem paths are intentionally absent so the
/// same declaration, inputs, validator, result, and bounded evidence produce
/// the same object identity.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactValidationReceiptV1 {
    pub version: u16,
    pub declaration: ArtifactValidationV1,
    pub desired_identity: ArtifactDesiredIdentityV1,
    pub tree_root_id: ArtifactTreeId,
    pub validator_identity: String,
    pub validated_input_digest: String,
    pub outcome: ArtifactValidationOutcomeV1,
    #[serde(default)]
    pub evidence: BTreeMap<String, String>,
}

/// A complete, host-validated contract for an optional dependency resolver.
///
/// The structure contains data only. It grants no authority until Trail checks
/// it against workspace policy and launches the exact executable itself.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactResolutionPlanV1 {
    pub version: u16,
    pub proposal_key: String,
    pub source_root: ObjectId,
    pub component_id: String,
    pub adapter_identity: String,
    pub policy_identity: String,
    pub program: String,
    pub resolved_program: String,
    pub executable_identity: String,
    pub argv: Vec<String>,
    pub working_directory: String,
    pub readable_inputs: Vec<ArtifactResolutionInputV1>,
    pub candidate_output: String,
    #[serde(default)]
    pub allowed_authorities: Vec<String>,
    #[serde(default)]
    pub credential_handles: Vec<String>,
    pub script_policy: ArtifactScriptPolicyV1,
    #[serde(default)]
    pub environment_roles: BTreeMap<String, ArtifactEnvironmentRoleV1>,
    pub limits: ArtifactActionLimitsV1,
    pub snapshot_format: String,
    #[serde(default)]
    pub validations: Vec<ArtifactValidationV1>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactResolutionVerificationStateV1 {
    Verified,
    Rejected,
}

/// Immutable provenance envelope for one resolver-produced snapshot.
///
/// `content_object_id` owns the exact snapshot bytes while this object records
/// why those bytes are valid for a proposal. Wall-clock time deliberately does
/// not participate, so identical evidence has identical content identity.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactResolutionSnapshotV1 {
    pub version: u16,
    pub proposal_key: String,
    pub source_root: ObjectId,
    pub component_id: String,
    pub adapter_identity: String,
    pub snapshot_format: String,
    pub content_object_id: ObjectId,
    pub content_sha256: String,
    #[serde(default)]
    pub resolved_identities: BTreeMap<String, String>,
    #[serde(default)]
    pub checksums: BTreeMap<String, String>,
    pub resolver_executable_identity: String,
    pub policy_identity: String,
    #[serde(default)]
    pub contacted_authorities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predecessor_snapshot_id: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "ArtifactSecretTaintV1::is_clear")]
    pub secret_taint: ArtifactSecretTaintV1,
    pub verification_state: ArtifactResolutionVerificationStateV1,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactResolutionAttemptStatusV1 {
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Abandoned,
}

/// Durable non-secret evidence describing the authority boundary of one
/// resolver attempt. Credential handles are names only; their values are
/// supplied late and never enter this object or an attempt row.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactResolutionAuthorityEvidenceV1 {
    pub allowed_authorities: Vec<String>,
    pub contacted_authorities: Vec<String>,
    pub credential_handles: Vec<String>,
    pub credential_values_redacted: bool,
}

/// One bounded stream captured from a resolver. `original_bytes` allows the
/// failure receipt to explain a limit violation without retaining excess data.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactResolutionCaptureV1 {
    pub version: u16,
    pub original_bytes: u64,
    pub truncated: bool,
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactResolutionFailureReceiptV1 {
    pub version: u16,
    pub attempt_id: ArtifactAttemptId,
    pub proposal_key: String,
    pub source_root: ObjectId,
    pub code: String,
    pub message: String,
    pub authority_evidence: ArtifactResolutionAuthorityEvidenceV1,
    #[serde(default, skip_serializing_if = "ArtifactSecretTaintV1::is_clear")]
    pub secret_taint: ArtifactSecretTaintV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_object_id: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_object_id: Option<ObjectId>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactResolutionAttemptReportV1 {
    pub attempt_id: ArtifactAttemptId,
    pub proposal_key: String,
    pub source_root: ObjectId,
    pub plan_object_id: ObjectId,
    pub owner_generation: u64,
    pub owner_pid: u32,
    pub status: ArtifactResolutionAttemptStatusV1,
    pub cancel_requested: bool,
    pub authority_evidence: ArtifactResolutionAuthorityEvidenceV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_object_id: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_object_id: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_receipt_object_id: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_message: Option<String>,
    pub started_at: i64,
    pub heartbeat_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    pub recovery_command: Vec<String>,
}

/// Ephemeral output returned by an authorized resolver executor.
///
/// This value is intentionally not serializable. `redactions` can contain
/// credential bytes used only to scrub bounded diagnostics before Trail makes
/// attempt evidence durable.
#[derive(Clone, PartialEq, Eq)]
pub struct ArtifactResolutionCandidateV1 {
    pub snapshot_bytes: Vec<u8>,
    pub resolved_identities: BTreeMap<String, String>,
    pub checksums: BTreeMap<String, String>,
    pub contacted_authorities: Vec<String>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub redactions: Vec<Vec<u8>>,
}

impl std::fmt::Debug for ArtifactResolutionCandidateV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ArtifactResolutionCandidateV1")
            .field("snapshot_bytes", &self.snapshot_bytes.len())
            .field("resolved_identities", &self.resolved_identities)
            .field("checksums", &self.checksums)
            .field("contacted_authorities", &self.contacted_authorities)
            .field("stdout_bytes", &self.stdout.len())
            .field("stderr_bytes", &self.stderr.len())
            .field("redactions", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactResolutionRequestV1 {
    pub plan: ArtifactResolutionPlanV1,
    pub candidate: ArtifactResolutionCandidateV1,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactResolutionDecisionV1 {
    Resolved,
    Reused,
    Refreshed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactResolutionComponentReportV1 {
    pub component_id: String,
    pub proposal_key: String,
    pub source_root: ObjectId,
    pub snapshot_id: ObjectId,
    pub snapshot: ArtifactResolutionSnapshotV1,
    pub decision: ArtifactResolutionDecisionV1,
    pub refresh_requested: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<ArtifactResolutionAttemptReportV1>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactResolutionBatchReportV1 {
    pub source_root: ObjectId,
    pub refresh_requested: bool,
    pub components: Vec<ArtifactResolutionComponentReportV1>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactActionPhaseV2 {
    Resolve,
    Construct,
    Validate,
    Finalize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArtifactActionIdentityV2 {
    pub name: String,
    pub phase: ArtifactActionPhaseV2,
    pub executable_identity: String,
    pub argv: Vec<String>,
    pub working_directory: String,
    pub environment_names: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactSourceClosureV2 {
    pub normalizer_version: String,
    pub certified_complete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complete_source_root: Option<ObjectId>,
    pub declared_inputs: Vec<ArtifactResolutionInputV1>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArtifactOutputContractV2 {
    pub name: String,
    pub output_path: String,
    pub mount_path: String,
    pub policy: EnvironmentOutputPolicy,
    pub reuse: EnvironmentReuseMode,
    pub scope: EnvironmentSharingScope,
    pub publish: EnvironmentPublicationTrigger,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArtifactSourceExportContractV2 {
    pub name: String,
    pub artifact_subpath: String,
    pub destination: String,
    pub collision_policy: String,
    pub required_validation: String,
}

/// Canonical identity inputs for the framework-neutral artifact pipeline.
///
/// Secret values and mutable provider allocation IDs have no representation in
/// this structure. Callers must pass only non-secret build environment values.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactDesiredKeyMaterialV2 {
    pub version: u16,
    pub component_id: String,
    pub adapter_identity: String,
    pub adapter_implementation_version: String,
    pub adapter_distribution_digest: String,
    pub adapter_protocol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution_snapshot_id: Option<ObjectId>,
    pub source_closure: ArtifactSourceClosureV2,
    pub upstream_identities: BTreeMap<String, String>,
    pub actions: Vec<ArtifactActionIdentityV2>,
    pub outputs: Vec<ArtifactOutputContractV2>,
    pub validations: Vec<ArtifactValidationV1>,
    pub source_exports: Vec<ArtifactSourceExportContractV2>,
    pub build_environment: BTreeMap<String, String>,
    pub target: String,
    pub platform: String,
    pub architecture: String,
    pub abi: String,
    /// True only when validation evidence permits reuse outside the producing
    /// lane. Missing fields decode conservatively as unproven portability.
    #[serde(default)]
    pub portability_certified: bool,
    pub portability_scope: String,
    pub trust_scope: String,
    pub network_policy: String,
    pub script_policy: ArtifactScriptPolicyV1,
    pub sandbox_policy: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArtifactInvalidationEdgeV2 {
    pub dimension: String,
    pub name: String,
    pub change: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactDesiredKeyDiffV2 {
    pub previous_key: ArtifactDesiredKeyV2,
    pub current_key: ArtifactDesiredKeyV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<ArtifactInvalidationEdgeV2>,
    pub edges: Vec<ArtifactInvalidationEdgeV2>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactDivergenceEvidenceV1 {
    pub version: u16,
    pub trust_scope: String,
    pub desired_key: String,
    pub incumbent_envelope_id: ArtifactEnvelopeId,
    pub incumbent_tree_root_id: ArtifactTreeId,
    pub candidate_envelope_id: ArtifactEnvelopeId,
    pub candidate_tree_root_id: ArtifactTreeId,
    pub reason_code: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactQuarantineResolutionV1 {
    RetainPrivate,
    AcceptIncumbent,
    AcceptCandidate,
    RetireAll,
}

impl ArtifactQuarantineResolutionV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RetainPrivate => "retain_private",
            Self::AcceptIncumbent => "accept_incumbent",
            Self::AcceptCandidate => "accept_candidate",
            Self::RetireAll => "retire_all",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactQuarantineRecordV1 {
    pub quarantine_id: ArtifactQuarantineId,
    pub trust_scope: String,
    pub desired_key: String,
    pub incumbent_envelope_id: Option<ArtifactEnvelopeId>,
    pub candidate_envelope_id: ArtifactEnvelopeId,
    pub reason_code: String,
    pub evidence_object_id: ObjectId,
    pub state: String,
    pub resolution: Option<String>,
    pub created_at: i64,
    pub resolved_at: Option<i64>,
}
