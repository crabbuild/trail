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
            LaneWorkdirMode::Sparse | LaneWorkdirMode::NativeCow => {
                Some(WorkdirBackend::Clone)
            }
            LaneWorkdirMode::FuseCow => Some(WorkdirBackend::Fuse),
            LaneWorkdirMode::NfsCow => Some(WorkdirBackend::Nfs),
            LaneWorkdirMode::DokanCow => Some(WorkdirBackend::Dokan),
        }
    }

    pub fn cow_backend(&self) -> Option<&'static str> {
        self.default_backend().map(WorkdirBackend::as_str)
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
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentPluginRemoveReport {
    pub canonical_identity: String,
    pub removed_distribution_digest: Option<String>,
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentGenerationOutputReport {
    pub name: String,
    pub policy: String,
    pub storage_identity: String,
    pub layer_id: Option<String>,
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentDiscoveredComponentReport {
    pub component_id: String,
    pub component_root: String,
    pub kind: String,
    pub adapter_identity: String,
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
    pub policy: String,
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
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceCheckpointReport {
    pub view_id: String,
    pub operation: Option<ChangeId>,
    pub root_id: ObjectId,
    pub journal_sequence: u64,
    pub source_paths: Vec<String>,
    pub generated_dirty_paths: u64,
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceExecReport {
    pub view_id: String,
    pub lane_id: String,
    pub source_root: ObjectId,
    pub generation: u64,
    pub environment_generation: Option<String>,
    pub backend: String,
    pub command: Vec<String>,
    pub exit_code: i32,
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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LanePatchReport {
    pub lane_id: String,
    pub operation: ChangeId,
    pub root_id: ObjectId,
    pub changed_paths: Vec<FileDiffSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LaneRecordReport {
    pub lane_id: String,
    pub operation: Option<ChangeId>,
    pub root_id: ObjectId,
    pub changed_paths: Vec<FileDiffSummary>,
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
        assert_eq!(LaneWorkdirMode::parse("fuse-cow"), Some(LaneWorkdirMode::FuseCow));
        assert_eq!(LaneWorkdirMode::parse("fuse_cow"), Some(LaneWorkdirMode::FuseCow));
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
        assert_eq!(LaneWorkdirMode::NativeCow.cow_backend(), Some("clone"));
        assert_eq!(LaneWorkdirMode::Auto.cow_backend(), None);
        assert_eq!(LaneWorkdirMode::PortableCopy.cow_backend(), None);
        assert_eq!(LaneWorkdirMode::FuseCow.cow_backend(), Some("fuse"));
        assert_eq!(LaneWorkdirMode::NfsCow.cow_backend(), Some("nfs"));
        assert_eq!(LaneWorkdirMode::DokanCow.cow_backend(), Some("dokan"));
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
}
