#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LaneWorkdirMode {
    Auto,
    Virtual,
    Sparse,
    NativeCow,
    PortableCopy,
    FuseCow,
    NfsCow,
    DokanCow,
}

impl LaneWorkdirMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            LaneWorkdirMode::Auto => "auto",
            LaneWorkdirMode::Virtual => "virtual",
            LaneWorkdirMode::Sparse => "sparse",
            LaneWorkdirMode::NativeCow => "native-cow",
            LaneWorkdirMode::PortableCopy => "portable-copy",
            LaneWorkdirMode::FuseCow => "fuse-cow",
            LaneWorkdirMode::NfsCow => "nfs-cow",
            LaneWorkdirMode::DokanCow => "dokan-cow",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(LaneWorkdirMode::Auto),
            "virtual" => Some(LaneWorkdirMode::Virtual),
            "sparse" => Some(LaneWorkdirMode::Sparse),
            "native-cow" | "native_cow" => Some(LaneWorkdirMode::NativeCow),
            "portable-copy" | "portable_copy" => Some(LaneWorkdirMode::PortableCopy),
            "fuse-cow" | "fuse_cow" => Some(LaneWorkdirMode::FuseCow),
            "nfs-cow" | "nfs_cow" => Some(LaneWorkdirMode::NfsCow),
            "dokan-cow" | "dokan_cow" => Some(LaneWorkdirMode::DokanCow),
            _ => None,
        }
    }

    pub fn materializes(&self) -> bool {
        !matches!(self, LaneWorkdirMode::Virtual)
    }

    pub fn default_backend(&self) -> Option<WorkdirBackend> {
        match self {
            LaneWorkdirMode::Auto | LaneWorkdirMode::PortableCopy => None,
            LaneWorkdirMode::Virtual => Some(WorkdirBackend::Virtual),
            LaneWorkdirMode::Sparse => None,
            LaneWorkdirMode::NativeCow => Some(WorkdirBackend::Clone),
            LaneWorkdirMode::FuseCow => Some(WorkdirBackend::Fuse),
            LaneWorkdirMode::NfsCow => Some(WorkdirBackend::Nfs),
            LaneWorkdirMode::DokanCow => Some(WorkdirBackend::Dokan),
        }
    }

    pub fn is_transparent_cow(&self) -> bool {
        matches!(
            self,
            LaneWorkdirMode::FuseCow | LaneWorkdirMode::NfsCow | LaneWorkdirMode::DokanCow
        )
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WorkdirBackend {
    Clone,
    Mixed,
    Copy,
    Fuse,
    Nfs,
    Dokan,
    Virtual,
}

/// Platform capability evidence used before admitting an automatic layered lane.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LayeredBackendPrerequisiteReport {
    pub platform: String,
    pub backend: Option<String>,
    pub required_service: String,
    pub mount_root: Option<String>,
    pub qualified: bool,
    pub remediation: Option<String>,
}

impl WorkdirBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            WorkdirBackend::Clone => "clone",
            WorkdirBackend::Mixed => "mixed",
            WorkdirBackend::Copy => "copy",
            WorkdirBackend::Fuse => "fuse",
            WorkdirBackend::Nfs => "nfs",
            WorkdirBackend::Dokan => "dokan",
            WorkdirBackend::Virtual => "virtual",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MaterializationFallbackReason {
    CloneUnsupported,
    CrossDevice,
    NativeSourceUnavailable,
}

impl MaterializationFallbackReason {
    pub fn as_str(self) -> &'static str {
        match self {
            MaterializationFallbackReason::CloneUnsupported => "clone-unsupported",
            MaterializationFallbackReason::CrossDevice => "cross-device",
            MaterializationFallbackReason::NativeSourceUnavailable => "native-source-unavailable",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MaterializationReport {
    pub cloned_files: u64,
    pub cloned_bytes: u64,
    pub copied_files: u64,
    pub copied_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<MaterializationFallbackReason>,
}

impl MaterializationReport {
    pub fn backend(&self) -> WorkdirBackend {
        match (self.cloned_files > 0, self.copied_files > 0) {
            (true, true) => WorkdirBackend::Mixed,
            (true, false) => WorkdirBackend::Clone,
            (false, true) => WorkdirBackend::Copy,
            (false, false) => WorkdirBackend::Clone,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneSpawnReport {
    #[serde(default)]
    pub initialization_id: String,
    #[serde(default)]
    pub request_fingerprint: String,
    #[serde(default = "default_completed_lane_initialization_phase")]
    pub phase: LaneInitializationPhase,
    #[serde(default = "default_true")]
    pub committed: bool,
    #[serde(default)]
    pub resumed: bool,
    #[serde(skip)]
    pub completed_deferred_initialization: bool,
    pub lane_id: String,
    pub ref_name: String,
    pub base_change: ChangeId,
    pub workdir: Option<String>,
    pub requested_workdir_mode: LaneWorkdirMode,
    pub workdir_mode: LaneWorkdirMode,
    pub workdir_backend: Option<WorkdirBackend>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub materialization: Option<MaterializationReport>,
    pub sparse_paths: Vec<String>,
    pub transparent_cow_available: bool,
    pub backend_prerequisites: LayeredBackendPrerequisiteReport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_inheritance: Option<EnvironmentInheritanceReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentOutputInheritanceDecisionReport {
    pub component_id: String,
    pub output_name: String,
    pub policy: EnvironmentOutputPolicy,
    pub decision: EnvironmentComponentDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer_id: Option<String>,
    pub storage_identity: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentInheritanceReport {
    pub parent_lane_id: String,
    pub parent_generation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_generation_id: Option<String>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    pub outputs: Vec<EnvironmentOutputInheritanceDecisionReport>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaneInitializationPhase {
    Reserved,
    Materialized,
    Associated,
    ObserverReady,
    RepairRequired,
}

fn default_completed_lane_initialization_phase() -> LaneInitializationPhase {
    LaneInitializationPhase::ObserverReady
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LaneInitializationReport {
    pub initialization_id: String,
    pub lane_name: String,
    pub lane_id: String,
    pub request_fingerprint: String,
    pub operation_id: String,
    pub phase: LaneInitializationPhase,
    pub workdir: Option<String>,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
    pub repair_command: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LaneWorkspaceViewReport {
    pub view_id: String,
    pub lane_id: String,
    pub base_change: ChangeId,
    pub base_root: ObjectId,
    pub backend: String,
    pub mountpoint: String,
    pub source_upper: String,
    pub generated_upper: String,
    pub scratch_upper: String,
    pub meta_dir: String,
    pub journal_path: String,
    pub generation: u64,
    pub checkpoint_seq: u64,
    pub checkpoint_root: Option<ObjectId>,
    pub status: String,
    pub owner_pid: Option<u32>,
    pub owner_start_token: Option<String>,
    pub heartbeat_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceLayerReport {
    pub layer_id: String,
    pub kind: String,
    pub cache_key: String,
    pub adapter: String,
    pub state: String,
    pub storage_path: String,
    pub logical_bytes: u64,
    pub physical_bytes: Option<u64>,
    pub entry_count: u64,
    pub portability_scope: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceEnvironmentReport {
    pub view_id: String,
    pub adapter: String,
    pub expected_key: String,
    pub attached_key: Option<String>,
    pub status: String,
    pub reason: Option<String>,
    pub updated_at: i64,
}

/// The stable, repository-local identity of an environment graph component.
///
/// Component identity is deliberately independent from the adapter that currently
/// implements it. This lets Trail upgrade or replace an adapter without changing
/// references to the logical component in status, policy, and dependency edges.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentComponentIdentityReport {
    pub component_id: String,
    pub kind: String,
}

/// The versioned identity of the adapter implementation responsible for a component.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentAdapterIdentityReport {
    pub namespace: String,
    pub name: String,
    pub contract_major: u32,
    pub implementation_version: String,
    pub distribution_digest: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentPluginProtocolCapabilitiesReport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_protocol: Option<String>,
    #[serde(default)]
    pub resolution_capable: bool,
    #[serde(default)]
    pub source_export_capable: bool,
    #[serde(default)]
    pub host_attestation_evidence_capable: bool,
    #[serde(default)]
    pub host_quarantine_evidence_capable: bool,
    #[serde(default)]
    pub certification_ceiling: String,
    #[serde(default)]
    pub content_policy: String,
    #[serde(default)]
    pub attestation_policy: String,
}

/// One adapter available to the environment host.
///
/// Catalog entries describe discovery and compatibility only. They never grant
/// an adapter permission to execute commands or mutate a lane.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentAdapterCatalogEntryReport {
    pub identity: EnvironmentAdapterIdentityReport,
    pub canonical_identity: String,
    pub selectors: Vec<String>,
    pub kind: String,
    pub layer_adapter_name: String,
    pub discovery_markers: Vec<String>,
    /// External planner protocols supported by the packaged executable.
    /// Built-ins and repository recipes use the in-process host contract and
    /// therefore report an empty list.
    pub protocols: Vec<String>,
    #[serde(default)]
    pub protocol_capabilities: EnvironmentPluginProtocolCapabilitiesReport,
    pub supported_operating_systems: Vec<String>,
    pub supported_architectures: Vec<String>,
    pub source: String,
    pub publisher: Option<String>,
    pub publisher_key_id: Option<String>,
    pub trust: String,
    pub certification_tier: String,
    pub stability: String,
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentAdapterCatalogReport {
    pub contract_major: u32,
    pub adapters: Vec<EnvironmentAdapterCatalogEntryReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentPluginInstallReport {
    pub canonical_identity: String,
    pub distribution_digest: String,
    pub executable_digest: String,
    pub package_path: String,
    pub replaced_distribution_digest: Option<String>,
    pub publisher: Option<String>,
    pub publisher_key_id: Option<String>,
    pub trust: String,
    pub certification_tier: String,
    #[serde(default)]
    pub protocol_capabilities: EnvironmentPluginProtocolCapabilitiesReport,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentPluginPackageInspectionReport {
    pub canonical_identity: String,
    pub payload_digest: String,
    pub executable_digest: String,
    pub distribution_digest: String,
    pub signature_present: bool,
    pub publisher: Option<String>,
    pub publisher_key_id: Option<String>,
    #[serde(default)]
    pub protocol_capabilities: EnvironmentPluginProtocolCapabilitiesReport,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentPluginRemoveReport {
    pub canonical_identity: String,
    pub removed_distribution_digest: Option<String>,
    #[serde(default)]
    pub protocol_capabilities: EnvironmentPluginProtocolCapabilitiesReport,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentStaleChangeReport {
    pub dimension: String,
    pub name: String,
    pub change: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentStaleExplanationReport {
    pub component_id: String,
    pub status: String,
    pub expected_key: String,
    pub attached_key: Option<String>,
    pub complete: bool,
    pub provenance_complete: bool,
    pub total_changes: u64,
    pub offset: u64,
    pub next_offset: Option<u64>,
    pub changes: Vec<EnvironmentStaleChangeReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentPublisherTrustEntryReport {
    pub publisher: String,
    pub key_id: String,
    pub public_key: String,
    pub trusted_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentPublisherTrustReport {
    pub keys: Vec<EnvironmentPublisherTrustEntryReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentPublisherTrustMutationReport {
    pub publisher: Option<String>,
    pub key_id: String,
    pub action: String,
}

/// Normalized environment state for one logical component in a workspace view.
///
/// `expected_key`, `attached_key`, and the status fields intentionally mirror
/// [`WorkspaceEnvironmentReport`] so legacy dependency state has a lossless report
/// projection while clients move to component-oriented APIs.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentComponentStateReport {
    pub view_id: String,
    pub component: EnvironmentComponentIdentityReport,
    pub adapter: EnvironmentAdapterIdentityReport,
    pub expected_key: String,
    pub attached_key: Option<String>,
    pub status: String,
    pub reason: Option<String>,
    pub updated_at: i64,
}

/// Storage and mutation semantics for one adapter-owned environment output.
///
/// The policy is framework-neutral: adapters describe whether bytes are
/// immutable, seeded with a private copy-on-write upper, lane-private, or
/// disposable. Trail owns the corresponding publication and mount behavior.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentOutputPolicy {
    ImmutableShared,
    #[default]
    ImmutableSeedPrivate,
    WritablePrivate,
    Disposable,
}

impl EnvironmentOutputPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ImmutableShared => "immutable_shared",
            Self::ImmutableSeedPrivate => "immutable_seed_private",
            Self::WritablePrivate => "writable_private",
            Self::Disposable => "disposable",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "immutable_shared" => Some(Self::ImmutableShared),
            "immutable_seed_private" => Some(Self::ImmutableSeedPrivate),
            "writable_private" => Some(Self::WritablePrivate),
            "disposable" => Some(Self::Disposable),
            _ => None,
        }
    }

    pub fn has_immutable_layer(self) -> bool {
        matches!(self, Self::ImmutableShared | Self::ImmutableSeedPrivate)
    }

    pub fn has_private_upper(self) -> bool {
        matches!(
            self,
            Self::ImmutableSeedPrivate | Self::WritablePrivate | Self::Disposable
        )
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentReuseMode {
    None,
    #[default]
    Exact,
    Compatible,
}

impl EnvironmentReuseMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Exact => "exact",
            Self::Compatible => "compatible",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "exact" => Some(Self::Exact),
            "compatible" => Some(Self::Compatible),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentSharingScope {
    Lane,
    #[default]
    Workspace,
    Host,
}

impl EnvironmentSharingScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lane => "lane",
            Self::Workspace => "workspace",
            Self::Host => "host",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "lane" => Some(Self::Lane),
            "workspace" => Some(Self::Workspace),
            "host" => Some(Self::Host),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentPublicationTrigger {
    #[default]
    Never,
    Manual,
    OnSync,
    SuccessfulGate,
}

impl EnvironmentPublicationTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::Manual => "manual",
            Self::OnSync => "on_sync",
            Self::SuccessfulGate => "successful_gate",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "never" => Some(Self::Never),
            "manual" => Some(Self::Manual),
            "on_sync" => Some(Self::OnSync),
            "successful_gate" => Some(Self::SuccessfulGate),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentComponentDecision {
    Reused,
    Built,
    Private,
    Rejected,
    Failed,
}

impl EnvironmentComponentDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reused => "reused",
            Self::Built => "built",
            Self::Private => "private",
            Self::Rejected => "rejected",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentRebuildReason {
    Missing,
    InputChanged,
    UpstreamChanged,
    ToolChanged,
    PolicyChanged,
    PlatformChanged,
    Corrupt,
    Revoked,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentGenerationOutputReport {
    pub name: String,
    pub policy: EnvironmentOutputPolicy,
    #[serde(default)]
    pub reuse: EnvironmentReuseMode,
    #[serde(default)]
    pub scope: EnvironmentSharingScope,
    #[serde(default)]
    pub publish: EnvironmentPublicationTrigger,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<String>,
    pub storage_identity: String,
    pub layer_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_object_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication_id: Option<String>,
    pub mount_path: String,
    pub layer_subpath: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentCacheReport {
    pub name: String,
    pub namespace_id: String,
    pub protocol: String,
    pub access: String,
    pub authority: String,
    pub scope: String,
    pub compatibility: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentExternalArtifactReport {
    pub name: String,
    pub artifact_type: String,
    pub provider: String,
    pub reference: String,
    pub digest: String,
    pub platform: String,
    pub cleanup_owner: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentRuntimeDeclarationReport {
    pub name: String,
    pub runtime_type: String,
    pub provider: String,
    pub artifact_name: String,
    pub container_port: u16,
    pub protocol: String,
    pub health_type: String,
    pub health_timeout_ms: u64,
    pub restart_policy: String,
    pub cleanup_owner: String,
    pub volume_target: Option<String>,
    #[serde(default)]
    pub secrets: Vec<EnvironmentSecretReferenceReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentSecretReferenceReport {
    pub name: String,
    pub provider: String,
    pub reference: String,
    pub version: Option<String>,
    pub purpose: String,
    pub injection: String,
    pub target: String,
    pub environment: Option<String>,
    pub required: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentSecretStatusReport {
    #[serde(flatten)]
    pub reference: EnvironmentSecretReferenceReport,
    pub status: String,
    pub reason: Option<String>,
    pub resolved_at: Option<i64>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentRuntimeResourceReport {
    #[serde(flatten)]
    pub declaration: EnvironmentRuntimeDeclarationReport,
    pub image_reference: String,
    pub image_digest: String,
    pub image_platform: String,
    pub allocation_id: String,
    pub provider_resource_id: Option<String>,
    pub container_name: String,
    pub network_name: String,
    pub volume_name: Option<String>,
    pub host_port: Option<u16>,
    pub status: String,
    pub health_status: String,
    pub reason: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub started_at: Option<i64>,
    pub stopped_at: Option<i64>,
    #[serde(default)]
    pub secret_statuses: Vec<EnvironmentSecretStatusReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentGenerationDependencyReport {
    pub component_id: String,
    pub component_key: String,
    #[serde(default = "default_environment_edge_type")]
    pub edge_type: String,
}

fn default_environment_edge_type() -> String {
    "build_requires".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentGenerationComponentReport {
    pub component_id: String,
    pub adapter_identity: String,
    pub kind: String,
    pub component_key: String,
    pub layer_id: Option<String>,
    pub mount_path: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<EnvironmentGenerationDependencyReport>,
    #[serde(default)]
    pub outputs: Vec<EnvironmentGenerationOutputReport>,
    #[serde(default)]
    pub caches: Vec<EnvironmentCacheReport>,
    #[serde(default)]
    pub external_artifacts: Vec<EnvironmentExternalArtifactReport>,
    #[serde(default)]
    pub runtime_resources: Vec<EnvironmentRuntimeResourceReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentGenerationReport {
    pub generation_id: String,
    pub view_id: String,
    pub generation_sequence: u64,
    pub source_root: ObjectId,
    pub specification_digest: String,
    pub predecessor_generation_id: Option<String>,
    pub state: String,
    pub components: Vec<EnvironmentGenerationComponentReport>,
    pub created_at: i64,
    pub activated_at: Option<i64>,
    pub retired_at: Option<i64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentComponentProposalStatus {
    #[default]
    Ready,
    Resolvable,
    Blocked,
    Unsupported,
    Ambiguous,
}

impl EnvironmentComponentProposalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Resolvable => "resolvable",
            Self::Blocked => "blocked",
            Self::Unsupported => "unsupported",
            Self::Ambiguous => "ambiguous",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentProposalReasonReport {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentRecoveryActionReport {
    pub code: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentDiscoveredComponentReport {
    pub component_id: String,
    pub component_root: String,
    pub kind: String,
    pub adapter_identity: String,
    #[serde(default)]
    pub status: EnvironmentComponentProposalStatus,
    #[serde(default)]
    pub reasons: Vec<EnvironmentProposalReasonReport>,
    #[serde(default)]
    pub recovery_actions: Vec<EnvironmentRecoveryActionReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentDiscoveryConflictReport {
    pub component_root: String,
    pub adapter_identities: Vec<String>,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentDiscoveryReport {
    pub source_root: ObjectId,
    pub components: Vec<EnvironmentDiscoveredComponentReport>,
    pub conflicts: Vec<EnvironmentDiscoveryConflictReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentGraphNodeReport {
    pub topological_index: u64,
    pub component_id: String,
    pub component_root: String,
    pub kind: String,
    pub adapter_identity: String,
    pub component_key: String,
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub caches: Vec<EnvironmentCacheReport>,
    #[serde(default)]
    pub external_artifacts: Vec<EnvironmentExternalArtifactReport>,
    #[serde(default)]
    pub runtime_resources: Vec<EnvironmentRuntimeDeclarationReport>,
    pub outputs: Vec<EnvironmentPlanOutputReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentGraphEdgeReport {
    pub source_component_id: String,
    pub source_component_key: String,
    pub target_component_id: String,
    pub edge_type: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentGraphReport {
    pub source_root: ObjectId,
    pub total_nodes: u64,
    pub total_edges: u64,
    pub offset: u64,
    pub next_offset: Option<u64>,
    pub nodes: Vec<EnvironmentGraphNodeReport>,
    pub edges: Vec<EnvironmentGraphEdgeReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentPlanInputReport {
    pub source_path: String,
    pub staging_path: String,
    pub content_hash: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentPlanCommandReport {
    /// `staging` or `mounted_initialization`.
    #[serde(default = "default_environment_command_phase")]
    pub phase: String,
    pub program: String,
    pub resolved_program: String,
    pub executable_identity: String,
    pub args: Vec<String>,
    pub working_directory: String,
    pub environment_names: Vec<String>,
}

fn default_environment_command_phase() -> String {
    "staging".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentCapabilityReport {
    pub filesystem_read: Vec<String>,
    pub filesystem_write: Vec<String>,
    pub process: Vec<String>,
    pub network: String,
    pub shell: String,
    pub scripts: String,
    pub secrets: String,
    pub sandbox: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentPlanOutputReport {
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentPlanReport {
    pub source_root: ObjectId,
    pub component_id: String,
    pub adapter_identity: String,
    pub kind: String,
    pub component_key: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub dependency_edges: Vec<EnvironmentGenerationDependencyReport>,
    #[serde(default)]
    pub caches: Vec<EnvironmentCacheReport>,
    #[serde(default)]
    pub external_artifacts: Vec<EnvironmentExternalArtifactReport>,
    #[serde(default)]
    pub runtime_resources: Vec<EnvironmentRuntimeDeclarationReport>,
    pub inputs: Vec<EnvironmentPlanInputReport>,
    pub tools: std::collections::BTreeMap<String, String>,
    pub commands: Vec<EnvironmentPlanCommandReport>,
    pub outputs: Vec<EnvironmentPlanOutputReport>,
    /// Compatibility projection of the first output.
    pub output_path: String,
    /// Compatibility projection of the first output.
    pub mount_path: String,
    pub portability_scope: String,
    pub capabilities: EnvironmentCapabilityReport,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentSyncReport {
    pub generation: EnvironmentGenerationReport,
    pub layers: Vec<WorkspaceLayerReport>,
    #[serde(default)]
    pub decisions: Vec<EnvironmentCacheDecisionReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentCacheDecisionReport {
    pub component_id: String,
    pub desired_key: String,
    pub storage_identity: Option<String>,
    pub decision: EnvironmentComponentDecision,
    pub decision_source: String,
    pub rebuild_reason: Option<EnvironmentRebuildReason>,
    #[serde(default)]
    pub identity_edges: Vec<EnvironmentStaleChangeReport>,
    pub bytes_avoided: Option<u64>,
    pub bytes_written: Option<u64>,
}

/// Durable result of turning one quiesced lane-private output into a reusable
/// immutable layer. The private source remains in place after publication.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentPromotionReport {
    pub publication_id: String,
    pub lane_id: String,
    pub view_id: String,
    pub component_id: String,
    pub output_name: String,
    pub trigger: EnvironmentPublicationTrigger,
    pub phase: String,
    pub predecessor_generation_id: String,
    pub successor_generation_id: String,
    pub source_root: ObjectId,
    pub output_identity: String,
    pub manifest_object_id: String,
    pub layer: WorkspaceLayerReport,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceCheckpointReport {
    pub view_id: String,
    pub operation: Option<ChangeId>,
    pub root_id: ObjectId,
    pub journal_sequence: u64,
    pub source_paths: Vec<String>,
    /// Generated/dependency paths changed in this authenticated view-journal
    /// interval. This is not a recursive inventory of the retained upper.
    pub generated_dirty_paths: u64,
    pub generated_path_accounting: String,
    #[serde(default)]
    pub upper_recovery_walks: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum PhysicalSharing {
    Verified,
    NotShared,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactStorageAccountingReport {
    /// Sum of logical file bytes across distinct artifact tree roots in scope.
    pub logical_bytes: u64,
    /// Encoded authoritative object bytes referenced by exactly one artifact.
    pub unique_authoritative_bytes: u64,
    /// Encoded authoritative object bytes referenced by multiple artifacts.
    pub cross_artifact_shared_bytes: u64,
    /// Filesystem-allocated bytes in reconstructible artifact/layer materializations.
    pub materialized_bytes: u64,
    /// Filesystem-allocated bytes owned by lane-private source/generated/scratch state.
    pub lane_private_bytes: u64,
    /// Persisted bytes created specifically by prefetch. OS page-cache warming is excluded.
    pub prefetched_bytes: u64,
    /// Filesystem-allocated bytes in content projections created on demand.
    pub demand_loaded_bytes: u64,
    /// Independently reclaimable cache bytes in the report scope.
    pub reclaimable_bytes: u64,
    /// Measured bytes whose artifact/cache ownership cannot be classified safely.
    pub unknown_bytes: u64,
    /// Exact byte bases and attribution boundary used by this report.
    pub accounting: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceSpaceReport {
    pub view_id: String,
    pub logical_visible_bytes: u64,
    pub shared_physical_bytes: u64,
    pub lane_exclusive_physical_bytes: u64,
    pub shared_extent_bytes: Option<u64>,
    pub reclaimable_cache_bytes: u64,
    pub uncheckpointed_source_bytes: u64,
    pub generated_upper_bytes: u64,
    pub scratch_upper_bytes: u64,
    pub physical_accounting: String,
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub logical_file_count: u64,
    #[serde(default)]
    pub filesystem_allocated_bytes: u64,
    #[serde(default)]
    pub changed_since_baseline_bytes: Option<u64>,
    #[serde(default)]
    pub clone_count: u64,
    #[serde(default)]
    pub physical_sharing: PhysicalSharing,
    #[serde(default)]
    pub physical_sharing_evidence: String,
    #[serde(default)]
    pub artifact_storage: ArtifactStorageAccountingReport,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceMountReport {
    pub view_id: String,
    pub backend: String,
    pub mountpoint: String,
    pub generation: u64,
    pub owner_pid: Option<u32>,
    pub owner_start_token: Option<String>,
    pub healthy: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceExecReport {
    pub view_id: String,
    pub lane_id: String,
    pub source_root: ObjectId,
    pub generation: u64,
    pub environment_generation: Option<String>,
    pub backend: String,
    pub command: Vec<String>,
    pub exit_code: i32,
    pub lifecycle: ManagedExecutionLifecycleReport,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedExecutionMissingResolutionPolicy {
    #[default]
    Explicit,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedExecutionResolutionPin {
    pub component_id: String,
    pub adapter_identity: String,
    pub status: EnvironmentComponentProposalStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_command: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedExecutionOutputPin {
    pub component_id: String,
    pub output_name: String,
    pub component_key: String,
    pub policy: EnvironmentOutputPolicy,
    pub storage_identity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_binding_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_envelope_id: Option<ArtifactEnvelopeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_tree_root_id: Option<ArtifactTreeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_binding_identity: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedExecutionPreparationReceipt {
    pub source_root: ObjectId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_generation: Option<u64>,
    pub missing_resolution_policy: ManagedExecutionMissingResolutionPolicy,
    pub resolution_pins: Vec<ManagedExecutionResolutionPin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_generation: Option<String>,
    pub output_pins: Vec<ManagedExecutionOutputPin>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedExecutionSealingDecision {
    pub component_id: String,
    pub output_name: String,
    pub policy: EnvironmentOutputPolicy,
    pub publication: EnvironmentPublicationTrigger,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<String>,
    pub decision: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedExecutionFinalizationReceipt {
    pub source_root_before: ObjectId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_root_after: Option<ObjectId>,
    pub source_changed: bool,
    pub checkpoint_status: String,
    pub disposal_status: String,
    pub unmount_status: String,
    pub complete: bool,
    pub sealing_decisions: Vec<ManagedExecutionSealingDecision>,
    pub errors: Vec<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactAdapterConformanceStatusV1 {
    Passed,
    Failed,
    Skipped,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactAdapterConformanceCheckV1 {
    pub stage: String,
    pub applicable: bool,
    pub status: ArtifactAdapterConformanceStatusV1,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactAdapterCertificationReportV1 {
    pub schema: String,
    pub producer_family: String,
    pub adapter_identity: String,
    pub trust_tier: ArtifactProducerTrustTierV1,
    pub status: ArtifactAdapterConformanceStatusV1,
    pub authority_effect: String,
    pub checks: Vec<ArtifactAdapterConformanceCheckV1>,
}

impl ArtifactAdapterCertificationReportV1 {
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.schema != "trail.artifact-adapter-certification/v1" {
            return Err("unsupported artifact adapter certification schema".to_string());
        }
        if self.authority_effect != "evidence_only" {
            return Err("artifact adapter certification cannot grant authority".to_string());
        }
        if self.producer_family.is_empty()
            || self.producer_family.len() > 64
            || self.adapter_identity.is_empty()
            || self.adapter_identity.len() > 256
        {
            return Err("artifact adapter certification identity is empty or oversized".to_string());
        }
        let expected = [
            "discovery",
            "resolution",
            "identity",
            "validation",
            "sealing",
            "cow",
            "recovery",
            "invalidation",
            "export",
            "retirement",
            "collection",
        ];
        if self.checks.len() != expected.len()
            || self
                .checks
                .iter()
                .zip(expected)
                .any(|(check, expected)| check.stage != expected)
        {
            return Err("artifact adapter certification checks are not complete and canonical"
                .to_string());
        }
        for check in &self.checks {
            if check.evidence.is_empty()
                || check.evidence.len() > 32
                || check
                    .evidence
                    .iter()
                    .any(|item| item.is_empty() || item.len() > 512)
            {
                return Err(format!(
                    "artifact adapter certification stage `{}` has empty or oversized evidence",
                    check.stage
                ));
            }
            if !check
                .evidence
                .windows(2)
                .all(|pair| pair[0] < pair[1])
            {
                return Err(format!(
                    "artifact adapter certification stage `{}` evidence is not canonical",
                    check.stage
                ));
            }
            if check.applicable
                && check.status != ArtifactAdapterConformanceStatusV1::Passed
                && self.status == ArtifactAdapterConformanceStatusV1::Passed
            {
                return Err(format!(
                    "artifact adapter certification cannot pass while required stage `{}` is not passed",
                    check.stage
                ));
            }
            if !check.applicable && check.status != ArtifactAdapterConformanceStatusV1::Skipped {
                return Err(format!(
                    "non-applicable artifact adapter certification stage `{}` must be skipped",
                    check.stage
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedExecutionPhaseReceipt {
    pub phase: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentHotPathEntry {
    pub layer_id: String,
    pub path: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct EnvironmentPrefetchReport {
    pub matched: bool,
    pub cancelled: bool,
    pub entries_considered: u64,
    pub entries_prefetched: u64,
    pub bytes_prefetched: u64,
    pub entry_limit: u64,
    pub byte_limit: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManagedExecutionLifecycleReport {
    pub execution_id: String,
    pub surface: String,
    pub command_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preparation: Option<ManagedExecutionPreparationReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_generation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<WorkspaceCheckpointReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disposal_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded: Option<LaneRecordReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalization: Option<ManagedExecutionFinalizationReceipt>,
    pub phases: Vec<ManagedExecutionPhaseReceipt>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceLayerKeyV1 {
    pub kind: String,
    pub adapter: String,
    pub adapter_version: u32,
    pub inputs: std::collections::BTreeMap<String, String>,
    pub tool_versions: std::collections::BTreeMap<String, String>,
    pub platform: String,
    pub architecture: String,
    pub portability_scope: String,
    pub strategy: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceGitShadowReport {
    pub view_id: String,
    pub git_dir: String,
    pub work_tree: String,
    pub policy: String,
    pub pinned_head: String,
    pub current_head: String,
    pub status: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceQuotaReport {
    pub view_id: String,
    pub upper_logical_bytes: u64,
    pub upper_file_count: u64,
    pub largest_file_bytes: u64,
    pub journal_bytes: u64,
    pub cache_physical_bytes: u64,
    pub exceeded: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceCacheGcEntry {
    pub kind: String,
    pub id: String,
    pub path: String,
    pub physical_bytes: u64,
    pub pinned: bool,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceCacheGcReport {
    pub dry_run: bool,
    pub retention_secs: u64,
    pub cache_physical_bytes_before: u64,
    pub reclaimable_bytes: u64,
    pub reclaimed_bytes: u64,
    pub candidates: Vec<WorkspaceCacheGcEntry>,
    pub deleted: Vec<WorkspaceCacheGcEntry>,
    #[serde(default)]
    pub artifact_storage: ArtifactStorageAccountingReport,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LanePatchReport {
    pub lane_id: String,
    pub operation: ChangeId,
    pub root_id: ObjectId,
    pub changed_paths: Vec<FileDiffSummary>,
    #[serde(default)]
    pub path_index: PathIndexMetricsReport,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneRecordReport {
    pub lane_id: String,
    pub operation: Option<ChangeId>,
    pub root_id: ObjectId,
    pub changed_paths: Vec<FileDiffSummary>,
    #[serde(default)]
    pub path_index: PathIndexMetricsReport,
    #[serde(default)]
    pub upper_recovery_walks: u64,
    #[serde(default)]
    pub generated_dirty_paths: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PathIndexMetricsReport {
    /// Path resolution used by this operation (`indexed` or `unknown`).
    #[serde(default = "default_path_index_mode")]
    pub mode: String,
    /// Number of unique folded keys looked up in the persisted path index.
    #[serde(default)]
    pub lookup_count: u64,
    /// Number of unbounded traversals that enumerate every persisted root path.
    #[serde(default)]
    pub full_root_path_load_count: u64,
    /// Number of unbounded repository-shaped filesystem validation walks.
    /// Explicitly selected sparse materializations are bounded and excluded.
    #[serde(default)]
    pub full_filesystem_path_scan_count: u64,
}

impl Default for PathIndexMetricsReport {
    fn default() -> Self {
        Self {
            mode: default_path_index_mode(),
            lookup_count: 0,
            full_root_path_load_count: 0,
            full_filesystem_path_scan_count: 0,
        }
    }
}

fn default_path_index_mode() -> String {
    "unknown".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneRecordPreviewReport {
    pub lane_id: String,
    pub workdir: String,
    pub head_change: ChangeId,
    pub root_id: ObjectId,
    pub clean: bool,
    pub changed_paths: Vec<FileDiffSummary>,
    pub ignored_paths: Vec<LaneWorkdirIgnoredPath>,
    pub risky_paths: Vec<LaneWorkdirRisk>,
    pub oversized_files: Vec<LaneRecordOversizedFile>,
    pub policy: LaneRecordPolicyPreview,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneWorkdirIgnoredPath {
    pub path: String,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneWorkdirRisk {
    pub path: String,
    pub kind: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneRecordOversizedFile {
    pub path: String,
    pub size_bytes: u64,
    pub limit_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneRecordPolicyPreview {
    pub allowed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneRefreshPreviewReport {
    pub lane_id: String,
    pub ref_name: String,
    pub base_change: ChangeId,
    pub lane_head_change: ChangeId,
    pub lane_head_root: ObjectId,
    pub target_ref: String,
    pub target_change: ChangeId,
    pub target_root: ObjectId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operations_behind: Option<u64>,
    pub clean: bool,
    pub conflicted: bool,
    pub changed_paths: Vec<FileDiffSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<String>,
    pub next_steps: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneRewindReport {
    pub lane_id: String,
    pub ref_name: String,
    pub target: String,
    pub previous_change: ChangeId,
    pub previous_root: ObjectId,
    pub target_change: ChangeId,
    pub target_root: ObjectId,
    pub operation: ChangeId,
    pub root_id: ObjectId,
    pub changed_paths: Vec<FileDiffSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_current: Option<ChangeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserved_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preserved_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
    pub workdir_synced: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneWorkdirReport {
    pub lane_id: String,
    pub workdir: Option<String>,
    pub requested_workdir_mode: LaneWorkdirMode,
    pub workdir_mode: LaneWorkdirMode,
    pub workdir_backend: Option<WorkdirBackend>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub materialization: Option<MaterializationReport>,
    pub sparse_paths: Vec<String>,
    pub transparent_cow_available: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneWorkdirSyncReport {
    pub lane_id: String,
    pub workdir: String,
    pub head_change: ChangeId,
    pub root_id: ObjectId,
    pub requested_workdir_mode: LaneWorkdirMode,
    pub workdir_mode: LaneWorkdirMode,
    pub workdir_backend: Option<WorkdirBackend>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub materialization: Option<MaterializationReport>,
    pub forced: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rescue_workdir: Option<String>,
    pub changed_paths: Vec<FileDiffSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneWatchReport {
    pub lane_id: String,
    pub iterations: u64,
    pub recorded_operations: Vec<ChangeId>,
    pub changed_paths: Vec<FileDiffSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneTestReport {
    pub lane_id: String,
    pub turn_id: String,
    pub session_id: Option<String>,
    pub workdir: String,
    pub source_root: ObjectId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub environment_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layer_ids: Vec<String>,
    pub command: Vec<String>,
    #[serde(default = "default_lane_gate_kind")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    pub status: String,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub duration_ms: u64,
    pub stdout_object: ObjectId,
    pub stderr_object: ObjectId,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub stdout_preview: String,
    pub stderr_preview: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub started_event_id: String,
    pub finished_event_id: String,
    pub lifecycle: ManagedExecutionLifecycleReport,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneTestSummary {
    pub event_id: String,
    pub turn_id: Option<String>,
    #[serde(default = "default_lane_gate_kind")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    pub status: String,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub duration_ms: u64,
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_root: Option<ObjectId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub environment_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layer_ids: Vec<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneGateHistoryReport {
    pub lane: LaneDetails,
    pub kind: String,
    pub limit: usize,
    pub gates: Vec<LaneTestSummary>,
}

fn default_lane_gate_kind() -> String {
    "test".to_string()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LaneGateOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
}

#[cfg(test)]
mod workdir_mode_tests {
    use super::*;

    #[test]
    fn cow_modes_use_backend_specific_names_and_reject_removed_aliases() {
        assert_eq!(
            LaneWorkdirMode::parse("native-cow"),
            Some(LaneWorkdirMode::NativeCow)
        );
        assert_eq!(
            LaneWorkdirMode::parse("native_cow"),
            Some(LaneWorkdirMode::NativeCow)
        );
        assert_eq!(
            LaneWorkdirMode::parse("fuse-cow"),
            Some(LaneWorkdirMode::FuseCow)
        );
        assert_eq!(
            LaneWorkdirMode::parse("fuse_cow"),
            Some(LaneWorkdirMode::FuseCow)
        );
        assert_eq!(
            LaneWorkdirMode::parse("dokan-cow"),
            Some(LaneWorkdirMode::DokanCow)
        );
        assert_eq!(
            LaneWorkdirMode::parse("dokan_cow"),
            Some(LaneWorkdirMode::DokanCow)
        );
        assert_eq!(LaneWorkdirMode::parse("overlay-cow"), None);
        assert_eq!(LaneWorkdirMode::parse("overlay_cow"), None);
        assert_eq!(LaneWorkdirMode::parse("full-cow"), None);
        assert_eq!(LaneWorkdirMode::parse("full_cow"), None);
        assert_eq!(LaneWorkdirMode::parse("auto"), Some(LaneWorkdirMode::Auto));
        assert_eq!(
            LaneWorkdirMode::parse("portable-copy"),
            Some(LaneWorkdirMode::PortableCopy)
        );
        assert_eq!(
            LaneWorkdirMode::parse("portable_copy"),
            Some(LaneWorkdirMode::PortableCopy)
        );
        assert_eq!(
            LaneWorkdirMode::NativeCow.default_backend(),
            Some(WorkdirBackend::Clone)
        );
        assert_eq!(LaneWorkdirMode::Auto.default_backend(), None);
        assert_eq!(LaneWorkdirMode::PortableCopy.default_backend(), None);
        assert_eq!(
            LaneWorkdirMode::FuseCow.default_backend(),
            Some(WorkdirBackend::Fuse)
        );
        assert_eq!(
            LaneWorkdirMode::NfsCow.default_backend(),
            Some(WorkdirBackend::Nfs)
        );
        assert_eq!(
            LaneWorkdirMode::DokanCow.default_backend(),
            Some(WorkdirBackend::Dokan)
        );
    }

    #[test]
    fn materialization_report_derives_actual_backend() {
        let mut report = MaterializationReport::default();
        assert_eq!(report.backend(), WorkdirBackend::Clone);
        report.copied_files = 1;
        assert_eq!(report.backend(), WorkdirBackend::Copy);
        report.cloned_files = 1;
        assert_eq!(report.backend(), WorkdirBackend::Mixed);
        report.copied_files = 0;
        assert_eq!(report.backend(), WorkdirBackend::Clone);
    }

    #[test]
    fn legacy_discovered_component_defaults_to_ready_proposal_status() {
        let report: EnvironmentDiscoveredComponentReport = serde_json::from_value(
            serde_json::json!({
                "component_id": "node",
                "component_root": "",
                "kind": "dependency",
                "adapter_identity": "trail/node@1"
            }),
        )
        .unwrap();
        assert_eq!(report.status, EnvironmentComponentProposalStatus::Ready);
        assert!(report.reasons.is_empty());
        assert!(report.recovery_actions.is_empty());

        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value["status"], "ready");
        assert_eq!(value["reasons"], serde_json::json!([]));
        assert_eq!(value["recovery_actions"], serde_json::json!([]));
    }

    #[test]
    fn managed_execution_receipts_are_additive_and_wire_stable() {
        let legacy: ManagedExecutionLifecycleReport = serde_json::from_value(
            serde_json::json!({
                "execution_id": "exec-legacy",
                "surface": "lane_exec",
                "command_fingerprint": "command",
                "phases": []
            }),
        )
        .unwrap();
        assert!(legacy.preparation.is_none());
        assert!(legacy.finalization.is_none());

        let current: ManagedExecutionLifecycleReport = serde_json::from_value(
            serde_json::json!({
                "execution_id": "exec-current",
                "surface": "lane_test",
                "command_fingerprint": "command",
                "preparation": {
                    "source_root": "object_source",
                    "view_id": "view-1",
                    "view_generation": 7,
                    "missing_resolution_policy": "explicit",
                    "resolution_pins": [{
                        "component_id": "node",
                        "adapter_identity": "trail/node@1",
                        "status": "ready",
                        "snapshot_id": "object_lock"
                    }],
                    "environment_generation": "generation-1",
                    "output_pins": [{
                        "component_id": "node",
                        "output_name": "dependencies",
                        "component_key": "desired-key",
                        "policy": "immutable_seed_private",
                        "storage_identity": "storage-key"
                    }]
                },
                "finalization": {
                    "source_root_before": "object_source",
                    "source_root_after": "object_after",
                    "source_changed": true,
                    "checkpoint_status": "succeeded",
                    "disposal_status": "succeeded",
                    "unmount_status": "succeeded",
                    "complete": true,
                    "sealing_decisions": [{
                        "component_id": "node",
                        "output_name": "dependencies",
                        "policy": "immutable_seed_private",
                        "publication": "never",
                        "decision": "retain_private_delta",
                        "reason": "private copy-on-write output is never published"
                    }],
                    "errors": []
                },
                "phases": []
            }),
        )
        .unwrap();
        let value = serde_json::to_value(current).unwrap();
        assert_eq!(
            value["preparation"]["missing_resolution_policy"],
            "explicit"
        );
        assert_eq!(
            value["preparation"]["resolution_pins"][0]["status"],
            "ready"
        );
        assert_eq!(
            value["finalization"]["sealing_decisions"][0]["decision"],
            "retain_private_delta"
        );
    }

    #[test]
    fn artifact_adapter_certification_is_complete_canonical_and_evidence_only() {
        let stages = [
            "discovery",
            "resolution",
            "identity",
            "validation",
            "sealing",
            "cow",
            "recovery",
            "invalidation",
            "export",
            "retirement",
            "collection",
        ];
        let report = ArtifactAdapterCertificationReportV1 {
            schema: "trail.artifact-adapter-certification/v1".into(),
            producer_family: "plugin_v3".into(),
            adapter_identity: "example/fixture@1".into(),
            trust_tier: ArtifactProducerTrustTierV1::LocallyTrustedPlugin,
            status: ArtifactAdapterConformanceStatusV1::Passed,
            authority_effect: "evidence_only".into(),
            checks: stages
                .into_iter()
                .map(|stage| ArtifactAdapterConformanceCheckV1 {
                    stage: stage.into(),
                    applicable: true,
                    status: ArtifactAdapterConformanceStatusV1::Passed,
                    evidence: vec![format!("fixture:{stage}")],
                })
                .collect(),
        };
        report.validate().unwrap();
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["status"], "passed");
        assert_eq!(value["authority_effect"], "evidence_only");
        assert_eq!(value["checks"][0]["stage"], "discovery");
        assert_eq!(value["checks"][10]["stage"], "collection");

        let mut skipped = report.clone();
        skipped.checks[5].status = ArtifactAdapterConformanceStatusV1::Skipped;
        assert!(skipped.validate().unwrap_err().contains("required stage `cow`"));
        let mut authority = report;
        authority.authority_effect = "certified_signed_plugin".into();
        assert!(authority
            .validate()
            .unwrap_err()
            .contains("cannot grant authority"));
    }

    #[test]
    fn legacy_patch_and_record_reports_default_path_index_metrics() {
        let patch: LanePatchReport = serde_json::from_value(serde_json::json!({
            "lane_id": "lane-1",
            "operation": "change-1",
            "root_id": "root-1",
            "changed_paths": []
        }))
        .unwrap();
        assert_eq!(patch.path_index, PathIndexMetricsReport::default());

        let record: LaneRecordReport = serde_json::from_value(serde_json::json!({
            "lane_id": "lane-1",
            "operation": null,
            "root_id": "root-1",
            "changed_paths": []
        }))
        .unwrap();
        assert_eq!(record.path_index, PathIndexMetricsReport::default());
    }

    #[test]
    fn legacy_workspace_space_reports_default_extended_accounting() {
        let report: WorkspaceSpaceReport = serde_json::from_value(serde_json::json!({
            "view_id": "view-1",
            "logical_visible_bytes": 10,
            "shared_physical_bytes": 2,
            "lane_exclusive_physical_bytes": 3,
            "shared_extent_bytes": null,
            "reclaimable_cache_bytes": 0,
            "uncheckpointed_source_bytes": 1,
            "generated_upper_bytes": 0,
            "scratch_upper_bytes": 0,
            "physical_accounting": "allocated-blocks"
        }))
        .unwrap();

        assert_eq!(report.backend, "");
        assert_eq!(report.logical_file_count, 0);
        assert_eq!(report.filesystem_allocated_bytes, 0);
        assert_eq!(report.changed_since_baseline_bytes, None);
        assert_eq!(report.clone_count, 0);
        assert_eq!(report.physical_sharing, PhysicalSharing::Unknown);
        assert_eq!(report.physical_sharing_evidence, "");
        assert_eq!(
            report.artifact_storage,
            ArtifactStorageAccountingReport::default()
        );

        let cache: WorkspaceCacheGcReport = serde_json::from_value(serde_json::json!({
            "dry_run": true,
            "retention_secs": 60,
            "cache_physical_bytes_before": 10,
            "reclaimable_bytes": 5,
            "reclaimed_bytes": 0,
            "candidates": [],
            "deleted": []
        }))
        .unwrap();
        assert_eq!(
            cache.artifact_storage,
            ArtifactStorageAccountingReport::default()
        );
    }

    #[test]
    fn artifact_storage_accounting_wire_fields_are_stable() {
        let report = ArtifactStorageAccountingReport {
            logical_bytes: 1,
            unique_authoritative_bytes: 2,
            cross_artifact_shared_bytes: 3,
            materialized_bytes: 4,
            lane_private_bytes: 5,
            prefetched_bytes: 6,
            demand_loaded_bytes: 7,
            reclaimable_bytes: 8,
            unknown_bytes: 9,
            accounting: "fixture".into(),
        };
        assert_eq!(
            serde_json::to_value(report).unwrap(),
            serde_json::json!({
                "logical_bytes": 1,
                "unique_authoritative_bytes": 2,
                "cross_artifact_shared_bytes": 3,
                "materialized_bytes": 4,
                "lane_private_bytes": 5,
                "prefetched_bytes": 6,
                "demand_loaded_bytes": 7,
                "reclaimable_bytes": 8,
                "unknown_bytes": 9,
                "accounting": "fixture"
            })
        );
    }

    #[test]
    fn environment_artifact_contract_enums_are_stable_and_reject_unknown_values() {
        for (kind, wire) in [
            (ArtifactValidationKindV1::Structural, "structural"),
            (ArtifactValidationKindV1::Loadability, "loadability"),
            (ArtifactValidationKindV1::Framework, "framework"),
            (ArtifactValidationKindV1::Policy, "policy"),
            (ArtifactValidationKindV1::Gate, "gate"),
            (
                ArtifactValidationKindV1::Reproducibility,
                "reproducibility",
            ),
            (ArtifactValidationKindV1::Ecosystem, "ecosystem"),
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), wire);
            assert_eq!(
                serde_json::from_value::<ArtifactValidationKindV1>(wire.into()).unwrap(),
                kind
            );
        }
        assert!(
            serde_json::from_value::<ArtifactValidationKindV1>("custom".into()).is_err()
        );
        for (tier, wire) in [
            (
                ArtifactProducerTrustTierV1::ReviewedBuiltin,
                "reviewed_builtin",
            ),
            (
                ArtifactProducerTrustTierV1::CertifiedSignedPlugin,
                "certified_signed_plugin",
            ),
            (
                ArtifactProducerTrustTierV1::LocallyTrustedPlugin,
                "locally_trusted_plugin",
            ),
            (
                ArtifactProducerTrustTierV1::RepositoryDeclaration,
                "repository_declaration",
            ),
        ] {
            assert_eq!(serde_json::to_value(tier).unwrap(), wire);
            assert_eq!(
                serde_json::from_value::<ArtifactProducerTrustTierV1>(wire.into()).unwrap(),
                tier
            );
        }
        assert!(
            serde_json::from_value::<ArtifactProducerTrustTierV1>("remote_trusted".into())
                .is_err()
        );
        for (phase, wire) in [
            (
                ArtifactExecutionPhaseV1::DiscoveryPlanning,
                "discovery_planning",
            ),
            (ArtifactExecutionPhaseV1::Resolve, "resolve"),
            (ArtifactExecutionPhaseV1::Construct, "construct"),
            (ArtifactExecutionPhaseV1::Validate, "validate"),
            (
                ArtifactExecutionPhaseV1::MountedExecution,
                "mounted_execution",
            ),
            (ArtifactExecutionPhaseV1::SourceExport, "source_export"),
        ] {
            assert_eq!(serde_json::to_value(phase).unwrap(), wire);
            assert_eq!(
                serde_json::from_value::<ArtifactExecutionPhaseV1>(wire.into()).unwrap(),
                phase
            );
        }
        assert!(
            serde_json::from_value::<ArtifactExecutionPhaseV1>("publish".into()).is_err()
        );

        for (policy, wire) in [
            (EnvironmentOutputPolicy::ImmutableShared, "immutable_shared"),
            (
                EnvironmentOutputPolicy::ImmutableSeedPrivate,
                "immutable_seed_private",
            ),
            (EnvironmentOutputPolicy::WritablePrivate, "writable_private"),
            (EnvironmentOutputPolicy::Disposable, "disposable"),
        ] {
            assert_eq!(policy.as_str(), wire);
            assert_eq!(EnvironmentOutputPolicy::parse(wire), Some(policy));
            assert_eq!(serde_json::to_value(policy).unwrap(), wire);
            assert_eq!(
                serde_json::from_value::<EnvironmentOutputPolicy>(wire.into()).unwrap(),
                policy
            );
        }
        assert_eq!(EnvironmentOutputPolicy::parse("shared_mutable"), None);
        assert!(
            serde_json::from_value::<EnvironmentOutputPolicy>("shared_mutable".into()).is_err()
        );

        for (mode, wire) in [
            (EnvironmentReuseMode::None, "none"),
            (EnvironmentReuseMode::Exact, "exact"),
            (EnvironmentReuseMode::Compatible, "compatible"),
        ] {
            assert_eq!(mode.as_str(), wire);
            assert_eq!(EnvironmentReuseMode::parse(wire), Some(mode));
        }
        for (scope, wire) in [
            (EnvironmentSharingScope::Lane, "lane"),
            (EnvironmentSharingScope::Workspace, "workspace"),
            (EnvironmentSharingScope::Host, "host"),
        ] {
            assert_eq!(scope.as_str(), wire);
            assert_eq!(EnvironmentSharingScope::parse(wire), Some(scope));
        }
        for (trigger, wire) in [
            (EnvironmentPublicationTrigger::Never, "never"),
            (EnvironmentPublicationTrigger::Manual, "manual"),
            (EnvironmentPublicationTrigger::OnSync, "on_sync"),
            (
                EnvironmentPublicationTrigger::SuccessfulGate,
                "successful_gate",
            ),
        ] {
            assert_eq!(trigger.as_str(), wire);
            assert_eq!(EnvironmentPublicationTrigger::parse(wire), Some(trigger));
        }
        assert!(EnvironmentReuseMode::parse("unsafe").is_none());
        assert!(EnvironmentSharingScope::parse("organization").is_none());
        assert!(EnvironmentPublicationTrigger::parse("always").is_none());
    }
}
