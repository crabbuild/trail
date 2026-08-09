//! Protocol-v3 wire types.
//!
//! These types deliberately describe plans and host evidence, not execution
//! authority. An adapter cannot mount a lane, publish an artifact, mint a
//! Trail attestation, resolve a quarantine, or write generated source.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    AdapterDependency, AdapterHost, AdapterOutput, AdapterPortability, PinnedFile, PROTOCOL_V3,
};

pub const MAX_V3_PROPOSALS: usize = 1_024;
pub const MAX_V3_REQUIREMENTS: usize = 256;
pub const MAX_V3_RECOVERY_ACTIONS: usize = 64;
pub const MAX_V3_INPUTS: usize = 100_000;
pub const MAX_V3_INPUT_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_V3_ACTIONS: usize = 64;
pub const MAX_V3_VALIDATIONS: usize = 64;
pub const MAX_V3_OUTPUTS: usize = 32;
pub const MAX_V3_SOURCE_EXPORTS: usize = 32;
pub const MAX_V3_AUTHORITIES: usize = 128;
pub const MAX_V3_IDENTITIES: usize = 1_024;
pub const MAX_V3_ATTESTATION_REFERENCES: usize = 256;
pub const MAX_V3_QUARANTINE_REFERENCES: usize = 64;
pub const MAX_V3_STRING_BYTES: usize = 4_096;
pub const MAX_V3_ARGV: usize = 256;
pub const MAX_V3_MAP_ENTRIES: usize = 1_024;
pub const MAX_V3_ACTION_TIMEOUT_MS: u64 = 60 * 60 * 1_000;
pub const MAX_V3_CAPTURE_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_V3_OUTPUT_ENTRIES: u64 = 1_000_000;
pub const MAX_V3_OUTPUT_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_V3_CHILD_PROCESSES: u32 = 256;

/// Host ceilings carried on every v3 request. A package may narrow these
/// values in its declaration, but an adapter response cannot widen them.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterProtocolLimitsV3 {
    pub max_input_files: u32,
    pub max_input_bytes: u64,
    pub max_actions: u16,
    pub max_validations: u16,
    pub max_outputs: u16,
    pub max_source_exports: u16,
    pub max_authorities: u16,
    pub max_response_bytes: u64,
}

impl Default for AdapterProtocolLimitsV3 {
    fn default() -> Self {
        Self {
            max_input_files: 4_096,
            max_input_bytes: MAX_V3_INPUT_BYTES,
            max_actions: MAX_V3_ACTIONS as u16,
            max_validations: MAX_V3_VALIDATIONS as u16,
            max_outputs: MAX_V3_OUTPUTS as u16,
            max_source_exports: MAX_V3_SOURCE_EXPORTS as u16,
            max_authorities: MAX_V3_AUTHORITIES as u16,
            max_response_bytes: 4 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterRequestV3 {
    pub protocol: String,
    pub request_id: String,
    pub adapter_identity: String,
    pub distribution_digest: String,
    pub host: AdapterHost,
    pub source_root: String,
    pub limits: AdapterProtocolLimitsV3,
    pub operation: AdapterOperationV3,
}

impl AdapterRequestV3 {
    pub fn new(
        request_id: impl Into<String>,
        adapter_identity: impl Into<String>,
        distribution_digest: impl Into<String>,
        host: AdapterHost,
        source_root: impl Into<String>,
        operation: AdapterOperationV3,
    ) -> Self {
        Self {
            protocol: PROTOCOL_V3.to_string(),
            request_id: request_id.into(),
            adapter_identity: adapter_identity.into(),
            distribution_digest: distribution_digest.into(),
            host,
            source_root: source_root.into(),
            limits: AdapterProtocolLimitsV3::default(),
            operation,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum AdapterOperationV3 {
    Propose {
        component_root: String,
        files: Vec<PinnedFile>,
    },
    Plan {
        proposal: Box<AdapterComponentProposalV3>,
        files: Vec<AdapterInputV3>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolution_snapshot: Option<Box<AdapterResolutionSnapshotV3>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host_evidence: Option<Box<AdapterHostEvidenceV3>>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterResponseV3 {
    pub protocol: String,
    pub request_id: String,
    pub result: AdapterResultV3,
}

impl AdapterResponseV3 {
    pub fn for_request(request: &AdapterRequestV3, result: AdapterResultV3) -> Self {
        Self {
            protocol: request.protocol.clone(),
            request_id: request.request_id.clone(),
            result,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub enum AdapterResultV3 {
    Proposed {
        component: Option<AdapterComponentProposalV3>,
    },
    Planned {
        pipeline: Box<AdapterPipelineV3>,
    },
    Error {
        code: String,
        message: String,
        #[serde(default)]
        recovery_actions: Vec<AdapterRecoveryActionV3>,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AdapterProposalStatusV3 {
    Ready,
    Resolvable,
    Blocked,
    Unsupported,
    Ambiguous,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct AdapterMissingRequirementV3 {
    pub code: String,
    pub message: String,
    pub requirement_type: AdapterRequirementTypeV3,
    pub resolvable: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AdapterRequirementTypeV3 {
    SourceInput,
    ResolutionSnapshot,
    Tool,
    PlatformCapability,
    PolicyApproval,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct AdapterRecoveryActionV3 {
    pub code: String,
    pub message: String,
    pub operation: AdapterRecoveryOperationV3,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AdapterRecoveryOperationV3 {
    Resolve,
    InstallTool,
    SelectComponent,
    GrantPolicy,
    EditSource,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterComponentProposalV3 {
    pub component_id: String,
    pub component_root: String,
    pub kind: String,
    pub status: AdapterProposalStatusV3,
    pub proposal_key: String,
    #[serde(default)]
    pub missing_requirements: Vec<AdapterMissingRequirementV3>,
    #[serde(default)]
    pub recovery_actions: Vec<AdapterRecoveryActionV3>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AdapterInputRoleV3 {
    Identity,
    Resolution,
    Construction,
    Validation,
    Runtime,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct AdapterInputV3 {
    pub path: String,
    pub content_hash: String,
    pub size_bytes: u64,
    pub executable: bool,
    pub role: AdapterInputRoleV3,
    pub format: String,
    pub required: bool,
    #[serde(with = "serde_bytes")]
    pub content: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterResolutionPlanV3 {
    pub name: String,
    pub program: String,
    pub argv: Vec<String>,
    pub working_directory: String,
    pub readable_inputs: Vec<String>,
    pub candidate_output: String,
    pub snapshot_format: String,
    #[serde(default)]
    pub allowed_authorities: Vec<String>,
    #[serde(default)]
    pub credential_handles: Vec<String>,
    pub capabilities: AdapterCapabilityProfileV3,
    pub limits: AdapterActionLimitsV3,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterResolutionSnapshotV3 {
    pub snapshot_id: String,
    pub proposal_key: String,
    pub format: String,
    pub content_sha256: String,
    #[serde(with = "serde_bytes")]
    pub content: Vec<u8>,
    #[serde(default)]
    pub resolved_identities: BTreeMap<String, String>,
    #[serde(default)]
    pub checksums: BTreeMap<String, String>,
    pub verified: bool,
    pub secret_taint: AdapterSecretTaintV3,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AdapterActionPhaseV3 {
    Construct,
    Validate,
    Finalize,
    MountedInitialization,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterActionV3 {
    pub name: String,
    pub phase: AdapterActionPhaseV3,
    pub program: String,
    pub argv: Vec<String>,
    pub working_directory: String,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    pub capabilities: AdapterCapabilityProfileV3,
    pub limits: AdapterActionLimitsV3,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterActionLimitsV3 {
    pub timeout_ms: u64,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub output_entries: u64,
    pub output_bytes: u64,
    pub child_processes: u32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AdapterNetworkCapabilityV3 {
    Deny,
    ExactAuthorities,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AdapterFilesystemCapabilityV3 {
    Deny,
    DeclaredInputs,
    IsolatedCandidate,
    DeclaredOutputs,
    LanePrivateBindings,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AdapterProcessCapabilityV3 {
    Deny,
    DeclaredExecutable,
    ReviewedBuiltinGraph,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AdapterSecretCapabilityV3 {
    Deny,
    OpaqueHandles,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AdapterPublicationCapabilityV3 {
    Deny,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterCapabilityProfileV3 {
    pub network: AdapterNetworkCapabilityV3,
    #[serde(default)]
    pub network_authorities: Vec<String>,
    pub filesystem_read: AdapterFilesystemCapabilityV3,
    pub filesystem_write: AdapterFilesystemCapabilityV3,
    pub process: AdapterProcessCapabilityV3,
    pub child_processes: u32,
    pub secrets: AdapterSecretCapabilityV3,
    pub publication: AdapterPublicationCapabilityV3,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AdapterValidationKindV3 {
    PathContract,
    Checksum,
    Command,
    Relocatability,
    Reproducibility,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct AdapterValidationV3 {
    pub name: String,
    pub kind: AdapterValidationKindV3,
    pub path: String,
    pub required: bool,
    #[serde(default)]
    pub parameters: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterIdentityContractV3 {
    pub normalizer_version: String,
    pub source_closure_complete: bool,
    #[serde(default)]
    pub semantic_identities: BTreeMap<String, String>,
    pub target: String,
    pub platform: String,
    pub architecture: String,
    pub abi: String,
    pub portability: AdapterPortability,
    pub portability_certified: bool,
    pub portability_scope: String,
    pub trust_scope: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AdapterSourceExportCollisionV3 {
    Fail,
    Replace,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct AdapterSourceExportV3 {
    pub name: String,
    pub output_name: String,
    pub artifact_subpath: String,
    pub destination: String,
    pub collision: AdapterSourceExportCollisionV3,
    pub required_validation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_gate: Option<String>,
    pub authorization: AdapterSourceExportAuthorizationV3,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AdapterSourceExportAuthorizationV3 {
    ExplicitUser,
}

/// Adapter-declared evidence requirements. Trail alone constructs and signs
/// the resulting attestation after sealing an artifact.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterAttestationRequirementsV3 {
    #[serde(default)]
    pub required_validations: Vec<String>,
    pub require_sandbox_evidence: bool,
    pub require_executable_identities: bool,
    pub signature_policy: AdapterAttestationSignaturePolicyV3,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AdapterAttestationSignaturePolicyV3 {
    OptionalLocal,
    RequiredTrustedPublisher,
}

/// Host-authored evidence that may be supplied back to a planner. The issuer
/// tag is not proof by itself; Trail validates every referenced object before
/// including this structure in a request.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterAttestationEvidenceV3 {
    pub issuer: AdapterEvidenceIssuerV3,
    pub attestation_id: String,
    pub envelope_id: String,
    pub desired_key: String,
    pub tree_root_id: String,
    pub verification_state: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterQuarantineEvidenceV3 {
    pub issuer: AdapterEvidenceIssuerV3,
    pub quarantine_id: String,
    pub desired_key: String,
    pub incumbent_envelope_id: Option<String>,
    pub candidate_envelope_id: String,
    pub reason_code: String,
    pub state: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AdapterEvidenceIssuerV3 {
    TrailHost,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterHostEvidenceV3 {
    #[serde(default)]
    pub attestations: Vec<AdapterAttestationEvidenceV3>,
    #[serde(default)]
    pub quarantines: Vec<AdapterQuarantineEvidenceV3>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterPipelineV3 {
    pub proposal: AdapterComponentProposalV3,
    #[serde(default)]
    pub dependencies: Vec<AdapterDependency>,
    pub inputs: Vec<AdapterInputV3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<AdapterResolutionPlanV3>,
    pub actions: Vec<AdapterActionV3>,
    pub validations: Vec<AdapterValidationV3>,
    pub capabilities: AdapterCapabilityProfileV3,
    pub identity: AdapterIdentityContractV3,
    pub outputs: Vec<AdapterOutput>,
    #[serde(default)]
    pub source_exports: Vec<AdapterSourceExportV3>,
    pub attestation: AdapterAttestationRequirementsV3,
    pub secret_taint: AdapterSecretTaintV3,
    pub quarantine_policy: AdapterQuarantinePolicyV3,
    pub stale_reason: String,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AdapterSecretTaintV3 {
    #[default]
    Clear,
    Credential,
    RuntimeSecret,
    Unknown,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AdapterQuarantinePolicyV3 {
    FailClosed,
    RetainPrivate,
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum AdapterProtocolV3BoundsError {
    #[error("protocol-v3 field `{field}` is required")]
    Empty { field: &'static str },
    #[error("protocol-v3 field `{field}` is {actual} bytes; maximum is {maximum}")]
    StringTooLong {
        field: &'static str,
        actual: usize,
        maximum: usize,
    },
    #[error("protocol-v3 collection `{field}` has {actual} entries; maximum is {maximum}")]
    CollectionTooLarge {
        field: &'static str,
        actual: usize,
        maximum: usize,
    },
    #[error("protocol-v3 byte collection `{field}` has {actual} bytes; maximum is {maximum}")]
    BytesTooLarge {
        field: &'static str,
        actual: u64,
        maximum: u64,
    },
    #[error("protocol-v3 request declares invalid host limit `{field}` = {actual}; maximum is {maximum}")]
    InvalidLimit {
        field: &'static str,
        actual: u64,
        maximum: u64,
    },
    #[error("protocol-v3 input `{path}` declares {declared} bytes but carries {actual}")]
    InputSizeMismatch {
        path: String,
        declared: u64,
        actual: u64,
    },
    #[error("expected protocol `{PROTOCOL_V3}`, received `{actual}`")]
    Protocol { actual: String },
}

impl AdapterRequestV3 {
    /// Enforce allocation and collection ceilings before an adapter receives
    /// a request. Semantic and trust validation remains host-owned.
    pub fn validate_bounds(&self) -> Result<(), AdapterProtocolV3BoundsError> {
        check_protocol(&self.protocol)?;
        check_string(&self.request_id, "request_id")?;
        check_string(&self.adapter_identity, "adapter_identity")?;
        check_string(&self.distribution_digest, "distribution_digest")?;
        check_string(&self.source_root, "source_root")?;
        self.limits.validate()?;
        match &self.operation {
            AdapterOperationV3::Propose {
                component_root,
                files,
            } => {
                check_string_allow_empty(component_root, "component_root")?;
                validate_pinned_files(files, &self.limits)?;
            }
            AdapterOperationV3::Plan {
                proposal,
                files,
                resolution_snapshot,
                host_evidence,
            } => {
                proposal.validate_bounds()?;
                check_count(
                    files.len(),
                    "inputs",
                    (self.limits.max_input_files as usize).min(MAX_V3_INPUTS),
                )?;
                let mut total = 0u64;
                for input in files {
                    input.validate_bounds()?;
                    let actual = input.content.len() as u64;
                    if actual != input.size_bytes {
                        return Err(AdapterProtocolV3BoundsError::InputSizeMismatch {
                            path: input.path.clone(),
                            declared: input.size_bytes,
                            actual,
                        });
                    }
                    total = total.saturating_add(actual);
                }
                check_bytes(
                    total,
                    "inputs",
                    self.limits.max_input_bytes.min(MAX_V3_INPUT_BYTES),
                )?;
                if let Some(snapshot) = resolution_snapshot {
                    snapshot.validate_bounds()?;
                }
                if let Some(evidence) = host_evidence {
                    evidence.validate_bounds()?;
                }
            }
        }
        Ok(())
    }
}

impl AdapterResponseV3 {
    /// Enforce the request's negotiated ceilings before host normalization.
    pub fn validate_bounds(
        &self,
        limits: &AdapterProtocolLimitsV3,
    ) -> Result<(), AdapterProtocolV3BoundsError> {
        check_protocol(&self.protocol)?;
        check_string(&self.request_id, "request_id")?;
        limits.validate()?;
        match &self.result {
            AdapterResultV3::Proposed { component } => {
                if let Some(component) = component {
                    component.validate_bounds()?;
                }
            }
            AdapterResultV3::Planned { pipeline } => pipeline.validate_bounds(limits)?,
            AdapterResultV3::Error {
                code,
                message,
                recovery_actions,
            } => {
                check_string(code, "error.code")?;
                check_string(message, "error.message")?;
                check_count(
                    recovery_actions.len(),
                    "error.recovery_actions",
                    MAX_V3_RECOVERY_ACTIONS,
                )?;
                for action in recovery_actions {
                    action.validate_bounds()?;
                }
            }
        }
        Ok(())
    }
}

impl AdapterProtocolLimitsV3 {
    fn validate(&self) -> Result<(), AdapterProtocolV3BoundsError> {
        check_limit(
            self.max_input_files as u64,
            "max_input_files",
            MAX_V3_INPUTS as u64,
        )?;
        check_limit(self.max_input_bytes, "max_input_bytes", MAX_V3_INPUT_BYTES)?;
        check_limit(
            self.max_actions as u64,
            "max_actions",
            MAX_V3_ACTIONS as u64,
        )?;
        check_limit(
            self.max_validations as u64,
            "max_validations",
            MAX_V3_VALIDATIONS as u64,
        )?;
        check_limit(
            self.max_outputs as u64,
            "max_outputs",
            MAX_V3_OUTPUTS as u64,
        )?;
        check_limit(
            self.max_source_exports as u64,
            "max_source_exports",
            MAX_V3_SOURCE_EXPORTS as u64,
        )?;
        check_limit(
            self.max_authorities as u64,
            "max_authorities",
            MAX_V3_AUTHORITIES as u64,
        )?;
        check_limit(
            self.max_response_bytes,
            "max_response_bytes",
            crate::MAX_FRAME_BYTES as u64,
        )
    }
}

impl AdapterComponentProposalV3 {
    fn validate_bounds(&self) -> Result<(), AdapterProtocolV3BoundsError> {
        check_string(&self.component_id, "proposal.component_id")?;
        check_string_allow_empty(&self.component_root, "proposal.component_root")?;
        check_string(&self.kind, "proposal.kind")?;
        check_string(&self.proposal_key, "proposal.proposal_key")?;
        check_count(
            self.missing_requirements.len(),
            "proposal.missing_requirements",
            MAX_V3_REQUIREMENTS,
        )?;
        for requirement in &self.missing_requirements {
            check_string(&requirement.code, "requirement.code")?;
            check_string(&requirement.message, "requirement.message")?;
        }
        check_count(
            self.recovery_actions.len(),
            "proposal.recovery_actions",
            MAX_V3_RECOVERY_ACTIONS,
        )?;
        for action in &self.recovery_actions {
            action.validate_bounds()?;
        }
        Ok(())
    }
}

impl AdapterRecoveryActionV3 {
    fn validate_bounds(&self) -> Result<(), AdapterProtocolV3BoundsError> {
        check_string(&self.code, "recovery_action.code")?;
        check_string(&self.message, "recovery_action.message")
    }
}

impl AdapterInputV3 {
    fn validate_bounds(&self) -> Result<(), AdapterProtocolV3BoundsError> {
        check_string(&self.path, "input.path")?;
        check_string(&self.content_hash, "input.content_hash")?;
        check_string(&self.format, "input.format")
    }
}

impl AdapterResolutionSnapshotV3 {
    fn validate_bounds(&self) -> Result<(), AdapterProtocolV3BoundsError> {
        check_string(&self.snapshot_id, "resolution_snapshot.snapshot_id")?;
        check_string(&self.proposal_key, "resolution_snapshot.proposal_key")?;
        check_string(&self.format, "resolution_snapshot.format")?;
        check_string(&self.content_sha256, "resolution_snapshot.content_sha256")?;
        check_bytes(
            self.content.len() as u64,
            "resolution_snapshot.content",
            MAX_V3_INPUT_BYTES,
        )?;
        check_map(
            &self.resolved_identities,
            "resolution_snapshot.resolved_identities",
        )?;
        check_map(&self.checksums, "resolution_snapshot.checksums")
    }
}

impl AdapterHostEvidenceV3 {
    fn validate_bounds(&self) -> Result<(), AdapterProtocolV3BoundsError> {
        check_count(
            self.attestations.len(),
            "host_evidence.attestations",
            MAX_V3_ATTESTATION_REFERENCES,
        )?;
        check_count(
            self.quarantines.len(),
            "host_evidence.quarantines",
            MAX_V3_QUARANTINE_REFERENCES,
        )?;
        for attestation in &self.attestations {
            check_string(&attestation.attestation_id, "attestation.attestation_id")?;
            check_string(&attestation.envelope_id, "attestation.envelope_id")?;
            check_string(&attestation.desired_key, "attestation.desired_key")?;
            check_string(&attestation.tree_root_id, "attestation.tree_root_id")?;
            check_string(
                &attestation.verification_state,
                "attestation.verification_state",
            )?;
        }
        for quarantine in &self.quarantines {
            check_string(&quarantine.quarantine_id, "quarantine.quarantine_id")?;
            check_string(&quarantine.desired_key, "quarantine.desired_key")?;
            if let Some(envelope) = &quarantine.incumbent_envelope_id {
                check_string(envelope, "quarantine.incumbent_envelope_id")?;
            }
            check_string(
                &quarantine.candidate_envelope_id,
                "quarantine.candidate_envelope_id",
            )?;
            check_string(&quarantine.reason_code, "quarantine.reason_code")?;
            check_string(&quarantine.state, "quarantine.state")?;
        }
        Ok(())
    }
}

impl AdapterPipelineV3 {
    fn validate_bounds(
        &self,
        limits: &AdapterProtocolLimitsV3,
    ) -> Result<(), AdapterProtocolV3BoundsError> {
        self.proposal.validate_bounds()?;
        check_count(
            self.dependencies.len(),
            "pipeline.dependencies",
            MAX_V3_IDENTITIES,
        )?;
        for dependency in &self.dependencies {
            check_string(&dependency.component_id, "pipeline.dependencies")?;
        }
        check_count(self.inputs.len(), "pipeline.inputs", MAX_V3_INPUTS)?;
        check_count(
            self.actions.len(),
            "pipeline.actions",
            (limits.max_actions as usize).min(MAX_V3_ACTIONS),
        )?;
        check_count(
            self.validations.len(),
            "pipeline.validations",
            (limits.max_validations as usize).min(MAX_V3_VALIDATIONS),
        )?;
        check_count(
            self.outputs.len(),
            "pipeline.outputs",
            (limits.max_outputs as usize).min(MAX_V3_OUTPUTS),
        )?;
        check_count(
            self.source_exports.len(),
            "pipeline.source_exports",
            (limits.max_source_exports as usize).min(MAX_V3_SOURCE_EXPORTS),
        )?;
        for input in &self.inputs {
            input.validate_bounds()?;
        }
        if let Some(resolution) = &self.resolution {
            validate_resolution_bounds(resolution, limits)?;
        }
        for action in &self.actions {
            check_string(&action.name, "action.name")?;
            check_string(&action.program, "action.program")?;
            check_string(&action.working_directory, "action.working_directory")?;
            check_string_slice(&action.argv, "action.argv", MAX_V3_ARGV)?;
            check_map(&action.environment, "action.environment")?;
            validate_capability_bounds(&action.capabilities, limits)?;
            validate_action_limits(&action.limits)?;
        }
        for validation in &self.validations {
            check_string(&validation.name, "validation.name")?;
            check_string_allow_empty(&validation.path, "validation.path")?;
            check_map(&validation.parameters, "validation.parameters")?;
        }
        validate_capability_bounds(&self.capabilities, limits)?;
        check_map(
            &self.identity.semantic_identities,
            "identity.semantic_identities",
        )?;
        check_string(
            &self.identity.normalizer_version,
            "identity.normalizer_version",
        )?;
        check_string(&self.identity.target, "identity.target")?;
        check_string(&self.identity.platform, "identity.platform")?;
        check_string(&self.identity.architecture, "identity.architecture")?;
        check_string(&self.identity.abi, "identity.abi")?;
        check_string(
            &self.identity.portability_scope,
            "identity.portability_scope",
        )?;
        check_string(&self.identity.trust_scope, "identity.trust_scope")?;
        for export in &self.source_exports {
            check_string(&export.name, "source_export.name")?;
            check_string(&export.output_name, "source_export.output_name")?;
            check_string(&export.artifact_subpath, "source_export.artifact_subpath")?;
            check_string(&export.destination, "source_export.destination")?;
            check_string(
                &export.required_validation,
                "source_export.required_validation",
            )?;
            if let Some(gate) = &export.required_gate {
                check_string(gate, "source_export.required_gate")?;
            }
        }
        for output in &self.outputs {
            check_string(&output.name, "output.name")?;
            check_string(&output.source, "output.source")?;
            check_string(&output.target, "output.target")?;
            if let Some(gate) = &output.gate {
                check_string(gate, "output.gate")?;
            }
        }
        check_string_slice(
            &self.attestation.required_validations,
            "attestation.required_validations",
            MAX_V3_ATTESTATION_REFERENCES,
        )?;
        check_string(&self.stale_reason, "pipeline.stale_reason")
    }
}

fn validate_resolution_bounds(
    resolution: &AdapterResolutionPlanV3,
    limits: &AdapterProtocolLimitsV3,
) -> Result<(), AdapterProtocolV3BoundsError> {
    check_string(&resolution.name, "resolution.name")?;
    check_string(&resolution.program, "resolution.program")?;
    check_string(
        &resolution.working_directory,
        "resolution.working_directory",
    )?;
    check_string(&resolution.candidate_output, "resolution.candidate_output")?;
    check_string(&resolution.snapshot_format, "resolution.snapshot_format")?;
    check_string_slice(&resolution.argv, "resolution.argv", MAX_V3_ARGV)?;
    check_string_slice(
        &resolution.readable_inputs,
        "resolution.readable_inputs",
        MAX_V3_INPUTS,
    )?;
    check_string_slice(
        &resolution.allowed_authorities,
        "resolution.allowed_authorities",
        (limits.max_authorities as usize).min(MAX_V3_AUTHORITIES),
    )?;
    check_string_slice(
        &resolution.credential_handles,
        "resolution.credential_handles",
        MAX_V3_AUTHORITIES,
    )?;
    validate_capability_bounds(&resolution.capabilities, limits)?;
    validate_action_limits(&resolution.limits)
}

fn validate_capability_bounds(
    capability: &AdapterCapabilityProfileV3,
    limits: &AdapterProtocolLimitsV3,
) -> Result<(), AdapterProtocolV3BoundsError> {
    check_string_slice(
        &capability.network_authorities,
        "capabilities.network_authorities",
        (limits.max_authorities as usize).min(MAX_V3_AUTHORITIES),
    )?;
    check_limit_allow_zero(
        capability.child_processes as u64,
        "capabilities.child_processes",
        MAX_V3_CHILD_PROCESSES as u64,
    )
}

fn validate_action_limits(
    limits: &AdapterActionLimitsV3,
) -> Result<(), AdapterProtocolV3BoundsError> {
    check_limit(
        limits.timeout_ms,
        "action_limits.timeout_ms",
        MAX_V3_ACTION_TIMEOUT_MS,
    )?;
    check_limit(
        limits.stdout_bytes,
        "action_limits.stdout_bytes",
        MAX_V3_CAPTURE_BYTES,
    )?;
    check_limit(
        limits.stderr_bytes,
        "action_limits.stderr_bytes",
        MAX_V3_CAPTURE_BYTES,
    )?;
    check_limit(
        limits.output_entries,
        "action_limits.output_entries",
        MAX_V3_OUTPUT_ENTRIES,
    )?;
    check_limit(
        limits.output_bytes,
        "action_limits.output_bytes",
        MAX_V3_OUTPUT_BYTES,
    )?;
    check_limit_allow_zero(
        limits.child_processes as u64,
        "action_limits.child_processes",
        MAX_V3_CHILD_PROCESSES as u64,
    )
}

fn validate_pinned_files(
    files: &[PinnedFile],
    limits: &AdapterProtocolLimitsV3,
) -> Result<(), AdapterProtocolV3BoundsError> {
    check_count(
        files.len(),
        "pinned_files",
        (limits.max_input_files as usize).min(MAX_V3_INPUTS),
    )?;
    let mut total = 0u64;
    for file in files {
        check_string(&file.path, "pinned_file.path")?;
        check_string(&file.content_hash, "pinned_file.content_hash")?;
        total = total.saturating_add(file.content.len() as u64);
    }
    check_bytes(
        total,
        "pinned_files",
        limits.max_input_bytes.min(MAX_V3_INPUT_BYTES),
    )
}

fn check_protocol(protocol: &str) -> Result<(), AdapterProtocolV3BoundsError> {
    if protocol == PROTOCOL_V3 {
        Ok(())
    } else {
        Err(AdapterProtocolV3BoundsError::Protocol {
            actual: protocol.to_string(),
        })
    }
}

fn check_limit(
    actual: u64,
    field: &'static str,
    maximum: u64,
) -> Result<(), AdapterProtocolV3BoundsError> {
    if actual == 0 || actual > maximum {
        Err(AdapterProtocolV3BoundsError::InvalidLimit {
            field,
            actual,
            maximum,
        })
    } else {
        Ok(())
    }
}

fn check_limit_allow_zero(
    actual: u64,
    field: &'static str,
    maximum: u64,
) -> Result<(), AdapterProtocolV3BoundsError> {
    if actual > maximum {
        Err(AdapterProtocolV3BoundsError::InvalidLimit {
            field,
            actual,
            maximum,
        })
    } else {
        Ok(())
    }
}

fn check_count(
    actual: usize,
    field: &'static str,
    maximum: usize,
) -> Result<(), AdapterProtocolV3BoundsError> {
    if actual > maximum {
        Err(AdapterProtocolV3BoundsError::CollectionTooLarge {
            field,
            actual,
            maximum,
        })
    } else {
        Ok(())
    }
}

fn check_bytes(
    actual: u64,
    field: &'static str,
    maximum: u64,
) -> Result<(), AdapterProtocolV3BoundsError> {
    if actual > maximum {
        Err(AdapterProtocolV3BoundsError::BytesTooLarge {
            field,
            actual,
            maximum,
        })
    } else {
        Ok(())
    }
}

fn check_string(value: &str, field: &'static str) -> Result<(), AdapterProtocolV3BoundsError> {
    if value.is_empty() {
        return Err(AdapterProtocolV3BoundsError::Empty { field });
    }
    check_string_allow_empty(value, field)
}

fn check_string_allow_empty(
    value: &str,
    field: &'static str,
) -> Result<(), AdapterProtocolV3BoundsError> {
    if value.len() > MAX_V3_STRING_BYTES {
        Err(AdapterProtocolV3BoundsError::StringTooLong {
            field,
            actual: value.len(),
            maximum: MAX_V3_STRING_BYTES,
        })
    } else {
        Ok(())
    }
}

fn check_string_slice(
    values: &[String],
    field: &'static str,
    maximum: usize,
) -> Result<(), AdapterProtocolV3BoundsError> {
    check_count(values.len(), field, maximum)?;
    for value in values {
        check_string(value, field)?;
    }
    Ok(())
}

fn check_map(
    values: &BTreeMap<String, String>,
    field: &'static str,
) -> Result<(), AdapterProtocolV3BoundsError> {
    check_count(values.len(), field, MAX_V3_MAP_ENTRIES)?;
    for (key, value) in values {
        check_string(key, field)?;
        check_string(value, field)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AdapterDependencyType, AdapterOutput, AdapterPortability, AdapterSharingScope};

    fn proposal() -> AdapterComponentProposalV3 {
        AdapterComponentProposalV3 {
            component_id: "example.codegen".into(),
            component_root: ".".into(),
            kind: "generated".into(),
            status: AdapterProposalStatusV3::Ready,
            proposal_key: "sha256:proposal".into(),
            missing_requirements: Vec::new(),
            recovery_actions: Vec::new(),
        }
    }

    fn limits() -> AdapterActionLimitsV3 {
        AdapterActionLimitsV3 {
            timeout_ms: 30_000,
            stdout_bytes: 1024 * 1024,
            stderr_bytes: 1024 * 1024,
            output_entries: 10_000,
            output_bytes: 64 * 1024 * 1024,
            child_processes: 1,
        }
    }

    fn capabilities() -> AdapterCapabilityProfileV3 {
        AdapterCapabilityProfileV3 {
            network: AdapterNetworkCapabilityV3::Deny,
            network_authorities: Vec::new(),
            filesystem_read: AdapterFilesystemCapabilityV3::DeclaredInputs,
            filesystem_write: AdapterFilesystemCapabilityV3::IsolatedCandidate,
            process: AdapterProcessCapabilityV3::DeclaredExecutable,
            child_processes: 1,
            secrets: AdapterSecretCapabilityV3::Deny,
            publication: AdapterPublicationCapabilityV3::Deny,
        }
    }

    fn input() -> AdapterInputV3 {
        AdapterInputV3 {
            path: "schema.json".into(),
            content_hash: "sha256:input".into(),
            size_bytes: 3,
            executable: false,
            role: AdapterInputRoleV3::Identity,
            format: "application/json".into(),
            required: true,
            content: b"{}\n".to_vec(),
        }
    }

    #[test]
    fn v3_plan_request_round_trips_resolution_and_host_evidence() {
        let request = AdapterRequestV3::new(
            "request-v3",
            "example/codegen@1",
            "sha256:distribution",
            AdapterHost {
                operating_system: "linux".into(),
                architecture: "x86_64".into(),
            },
            "root:v3",
            AdapterOperationV3::Plan {
                proposal: Box::new(proposal()),
                files: vec![input()],
                resolution_snapshot: Some(Box::new(AdapterResolutionSnapshotV3 {
                    snapshot_id: "object:snapshot".into(),
                    proposal_key: "sha256:proposal".into(),
                    format: "application/vnd.example.lock".into(),
                    content_sha256: "sha256:snapshot".into(),
                    content: b"locked\n".to_vec(),
                    resolved_identities: BTreeMap::from([("dependency".into(), "1.0.0".into())]),
                    checksums: BTreeMap::from([("dependency".into(), "sha256:dep".into())]),
                    verified: true,
                    secret_taint: AdapterSecretTaintV3::Clear,
                })),
                host_evidence: Some(Box::new(AdapterHostEvidenceV3 {
                    attestations: vec![AdapterAttestationEvidenceV3 {
                        issuer: AdapterEvidenceIssuerV3::TrailHost,
                        attestation_id: "attestation:one".into(),
                        envelope_id: "envelope:one".into(),
                        desired_key: "desired:one".into(),
                        tree_root_id: "tree:one".into(),
                        verification_state: "verified".into(),
                    }],
                    quarantines: vec![AdapterQuarantineEvidenceV3 {
                        issuer: AdapterEvidenceIssuerV3::TrailHost,
                        quarantine_id: "quarantine:one".into(),
                        desired_key: "desired:other".into(),
                        incumbent_envelope_id: None,
                        candidate_envelope_id: "envelope:other".into(),
                        reason_code: "divergent_content".into(),
                        state: "open".into(),
                    }],
                })),
            },
        );
        request.validate_bounds().unwrap();
        let decoded: AdapterRequestV3 =
            serde_cbor::from_slice(&serde_cbor::to_vec(&request).unwrap()).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn v3_pipeline_response_covers_common_artifact_contract() {
        let pipeline = AdapterPipelineV3 {
            proposal: proposal(),
            dependencies: vec![AdapterDependency::new(
                "example.compiler",
                AdapterDependencyType::BuildRequires,
            )],
            inputs: vec![input()],
            resolution: Some(AdapterResolutionPlanV3 {
                name: "lock".into(),
                program: "resolver".into(),
                argv: vec!["resolve".into(), "--output".into(), "generated.lock".into()],
                working_directory: ".".into(),
                readable_inputs: vec!["schema.json".into()],
                candidate_output: "generated.lock".into(),
                snapshot_format: "application/vnd.example.lock".into(),
                allowed_authorities: vec!["registry.example:443".into()],
                credential_handles: Vec::new(),
                capabilities: AdapterCapabilityProfileV3 {
                    network: AdapterNetworkCapabilityV3::ExactAuthorities,
                    network_authorities: vec!["registry.example:443".into()],
                    ..capabilities()
                },
                limits: limits(),
            }),
            actions: vec![AdapterActionV3 {
                name: "construct".into(),
                phase: AdapterActionPhaseV3::Construct,
                program: "generator".into(),
                argv: vec!["build".into()],
                working_directory: ".".into(),
                environment: BTreeMap::new(),
                capabilities: capabilities(),
                limits: limits(),
            }],
            validations: vec![AdapterValidationV3 {
                name: "path-contract".into(),
                kind: AdapterValidationKindV3::PathContract,
                path: "generated".into(),
                required: true,
                parameters: BTreeMap::new(),
            }],
            capabilities: capabilities(),
            identity: AdapterIdentityContractV3 {
                normalizer_version: "trail-path-v1".into(),
                source_closure_complete: true,
                semantic_identities: BTreeMap::from([("mode".into(), "production".into())]),
                target: "host".into(),
                platform: "linux".into(),
                architecture: "x86_64".into(),
                abi: "gnu".into(),
                portability: AdapterPortability::Platform,
                portability_certified: false,
                portability_scope: "workspace".into(),
                trust_scope: "local_plugin".into(),
            },
            outputs: vec![AdapterOutput::immutable_seed_private(
                "generated",
                "generated",
                ".trail-generated/generated",
            )],
            source_exports: vec![AdapterSourceExportV3 {
                name: "client".into(),
                output_name: "generated".into(),
                artifact_subpath: "client".into(),
                destination: "src/generated".into(),
                collision: AdapterSourceExportCollisionV3::Fail,
                required_validation: "path-contract".into(),
                required_gate: None,
                authorization: AdapterSourceExportAuthorizationV3::ExplicitUser,
            }],
            attestation: AdapterAttestationRequirementsV3 {
                required_validations: vec!["path-contract".into()],
                require_sandbox_evidence: true,
                require_executable_identities: true,
                signature_policy: AdapterAttestationSignaturePolicyV3::OptionalLocal,
            },
            secret_taint: AdapterSecretTaintV3::Clear,
            quarantine_policy: AdapterQuarantinePolicyV3::FailClosed,
            stale_reason: "input, resolver, generator, or platform changed".into(),
        };
        assert_eq!(pipeline.outputs[0].scope, AdapterSharingScope::Workspace);
        let response = AdapterResponseV3 {
            protocol: PROTOCOL_V3.into(),
            request_id: "request-v3".into(),
            result: AdapterResultV3::Planned {
                pipeline: Box::new(pipeline),
            },
        };
        response
            .validate_bounds(&AdapterProtocolLimitsV3::default())
            .unwrap();
        let decoded: AdapterResponseV3 =
            serde_cbor::from_slice(&serde_cbor::to_vec(&response).unwrap()).unwrap();
        assert_eq!(decoded, response);
    }

    #[test]
    fn v3_bounds_reject_limit_widening_and_input_mismatch() {
        let mut request = AdapterRequestV3::new(
            "request-v3",
            "example/codegen@1",
            "sha256:distribution",
            AdapterHost {
                operating_system: "linux".into(),
                architecture: "x86_64".into(),
            },
            "root:v3",
            AdapterOperationV3::Plan {
                proposal: Box::new(proposal()),
                files: vec![input()],
                resolution_snapshot: None,
                host_evidence: None,
            },
        );
        request.limits.max_actions = (MAX_V3_ACTIONS + 1) as u16;
        assert!(matches!(
            request.validate_bounds(),
            Err(AdapterProtocolV3BoundsError::InvalidLimit {
                field: "max_actions",
                ..
            })
        ));

        request.limits = AdapterProtocolLimitsV3::default();
        let AdapterOperationV3::Plan { files, .. } = &mut request.operation else {
            unreachable!();
        };
        files[0].size_bytes += 1;
        assert!(matches!(
            request.validate_bounds(),
            Err(AdapterProtocolV3BoundsError::InputSizeMismatch { .. })
        ));
    }
}
