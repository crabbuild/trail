//! Protocol and authoring helpers for isolated Trail environment adapters.
//!
//! An adapter is a planner, not an executor. Trail sends bounded bytes from a
//! pinned source root; the adapter returns discovery or a normalized action
//! proposal. The Trail host owns tool resolution, sandboxing, execution,
//! validation, publication, bindings, state, and recovery.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

pub const PROTOCOL_V1: &str = "trail.environment-adapter/v1";
/// Adds host-sandboxed actions that execute against Trail's ephemeral mounted
/// candidate view. V1 remains the default for packages that do not declare a
/// protocol list, so existing adapters keep their exact behavior.
pub const PROTOCOL_V2: &str = "trail.environment-adapter/v2";
pub const PACKAGE_SCHEMA_V1: &str = "trail.environment-adapter-package/v1";
pub const PACKAGE_SIGNATURE_SCHEMA_V1: &str = "trail.environment-adapter-signature/v1";
pub const TRUSTED_PUBLISHER_KEY_SCHEMA_V1: &str = "trail.environment-adapter-publisher-key/v1";
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterPackageManifest {
    pub schema: String,
    pub adapter: AdapterMetadata,
    pub executable: AdapterExecutable,
    #[serde(default)]
    pub permissions: AdapterPermissions,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterMetadata {
    pub canonical_identity: String,
    pub implementation_version: String,
    pub selectors: Vec<String>,
    pub kind: String,
    pub layer_adapter_name: String,
    pub discovery_markers: Vec<String>,
    /// Protocols understood by the packaged executable. Missing metadata is
    /// deliberately interpreted as v1 for package compatibility.
    #[serde(
        default = "default_adapter_protocols",
        skip_serializing_if = "is_default_v1_protocols"
    )]
    pub protocols: Vec<String>,
    #[serde(default = "default_supported_operating_systems")]
    pub supported_operating_systems: Vec<String>,
    #[serde(default = "default_supported_architectures")]
    pub supported_architectures: Vec<String>,
    pub stability: String,
    pub description: String,
}

fn default_adapter_protocols() -> Vec<String> {
    vec![PROTOCOL_V1.to_string()]
}

fn is_default_v1_protocols(protocols: &[String]) -> bool {
    protocols == [PROTOCOL_V1]
}

fn default_supported_operating_systems() -> Vec<String> {
    ["linux", "macos", "windows"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn default_supported_architectures() -> Vec<String> {
    ["aarch64", "x86_64"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterExecutable {
    pub path: String,
    pub sha256: String,
}

/// Detached publisher signature stored as `trail-adapter.sig` beside a
/// package. The signature authenticates `payload_digest`, the canonical
/// manifest-plus-executable digest calculated by Trail.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterPackageSignature {
    pub schema: String,
    pub publisher: String,
    pub key_id: String,
    pub payload_digest: String,
    pub signature: String,
}

/// Public key document accepted by `trail env plugin trust add`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterPublisherKey {
    pub schema: String,
    pub publisher: String,
    pub public_key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct AdapterPermissions {
    pub read_patterns: Vec<String>,
    pub max_input_files: u32,
    pub max_input_bytes: u64,
    pub timeout_ms: u64,
    pub max_response_bytes: u64,
}

impl Default for AdapterPermissions {
    fn default() -> Self {
        Self {
            read_patterns: Vec::new(),
            max_input_files: 4_096,
            max_input_bytes: 8 * 1024 * 1024,
            timeout_ms: 5_000,
            max_response_bytes: 4 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterRequest {
    pub protocol: String,
    pub request_id: String,
    pub adapter_identity: String,
    pub distribution_digest: String,
    pub host: AdapterHost,
    pub source_root: String,
    pub operation: AdapterOperation,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterHost {
    pub operating_system: String,
    pub architecture: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum AdapterOperation {
    Discover {
        component_root: String,
        files: Vec<PinnedFile>,
    },
    Plan {
        component_id: String,
        component_root: String,
        files: Vec<PinnedFile>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PinnedFile {
    pub path: String,
    pub content_hash: String,
    pub executable: bool,
    #[serde(with = "serde_bytes")]
    pub content: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterResponse {
    pub protocol: String,
    pub request_id: String,
    pub result: AdapterResult,
}

impl AdapterResponse {
    /// Build a protocol-matched response for one host request.
    pub fn for_request(request: &AdapterRequest, result: AdapterResult) -> Self {
        Self {
            protocol: request.protocol.clone(),
            request_id: request.request_id.clone(),
            result,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "result", rename_all = "snake_case")]
// Keep the v1 constructor shape source-compatible for adapter authors. Boxing
// `Planned::plan` would not change CBOR but would require every existing
// plugin to wrap its plan solely to optimize this short-lived message enum.
#[allow(clippy::large_enum_variant)]
pub enum AdapterResult {
    Discovered {
        component: Option<DiscoveredComponent>,
    },
    Planned {
        plan: AdapterPlan,
    },
    PlannedV2 {
        plan: AdapterPlanV2,
    },
    Error {
        code: String,
        message: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredComponent {
    pub component_id: String,
    pub kind: String,
}

impl DiscoveredComponent {
    pub fn new(component_id: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            component_id: component_id.into(),
            kind: kind.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterPlan {
    pub component_id: String,
    pub kind: String,
    /// Stable logical component IDs that must precede this component. This is
    /// optional on the wire for backward compatibility with early v1 plans.
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub identity_inputs: Vec<String>,
    #[serde(default)]
    pub semantic_inputs: BTreeMap<String, String>,
    pub command: AdapterCommand,
    pub outputs: Vec<AdapterOutput>,
    pub portability: AdapterPortability,
    pub stale_reason: String,
}

/// Protocol-v2 plan with explicitly phased host actions. Keeping this type
/// separate preserves the public construction and exact wire shape of
/// `AdapterPlan` for every existing protocol-v1 adapter.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterPlanV2 {
    pub component_id: String,
    pub kind: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// Typed dependency edges. `dependencies` remains the protocol-v1-shaped
    /// compatibility field and is interpreted as `build_requires`; a
    /// component ID may appear in only one of the two collections.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependency_edges: Vec<AdapterDependency>,
    pub identity_inputs: Vec<String>,
    #[serde(default)]
    pub semantic_inputs: BTreeMap<String, String>,
    /// Performance-only caches used by the single staging action. Trail owns
    /// namespace identity, storage, locking, sandbox projection, leases, and
    /// garbage collection; adapters receive only the declared environment
    /// bindings. Mounted actions never receive cache access.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caches: Vec<AdapterCache>,
    /// Provider-owned immutable identities. A plan that declares external
    /// artifacts is metadata-only: it must not also declare actions, caches,
    /// or filesystem outputs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_artifacts: Vec<AdapterExternalArtifact>,
    /// Per-lane runtime resources derived from declared immutable artifacts.
    /// Provider allocation IDs and host ports are assigned by Trail after the
    /// environment generation commits.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_resources: Vec<AdapterRuntimeResource>,
    pub actions: Vec<AdapterAction>,
    pub outputs: Vec<AdapterOutput>,
    pub portability: AdapterPortability,
    pub stale_reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterExternalArtifact {
    pub name: String,
    pub artifact_type: String,
    pub provider: String,
    pub reference: String,
    pub digest: String,
    pub platform: String,
    pub cleanup_owner: String,
}

impl AdapterExternalArtifact {
    pub fn pinned_oci_image(
        name: impl Into<String>,
        reference: impl Into<String>,
        platform: impl Into<String>,
    ) -> Self {
        let reference = reference.into();
        let digest = reference
            .rsplit_once('@')
            .map(|(_, digest)| digest.to_string())
            .unwrap_or_default();
        Self {
            name: name.into(),
            artifact_type: "oci_image".to_string(),
            provider: "oci".to_string(),
            reference,
            digest,
            platform: platform.into(),
            cleanup_owner: "external".to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterRuntimeResource {
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_target: Option<String>,
    /// Opaque provider references resolved by Trail only while starting the
    /// resource. Values never cross the adapter protocol.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<AdapterSecretReference>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterSecretReference {
    pub name: String,
    pub provider: String,
    pub reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub purpose: String,
    pub injection: String,
    pub target: String,
    /// Optional environment variable receiving the non-secret target file
    /// path (for example `POSTGRES_PASSWORD_FILE`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    pub required: bool,
}

impl AdapterSecretReference {
    pub fn file(
        name: impl Into<String>,
        provider: impl Into<String>,
        reference: impl Into<String>,
        target: impl Into<String>,
        purpose: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            provider: provider.into(),
            reference: reference.into(),
            version: None,
            purpose: purpose.into(),
            injection: "file".to_string(),
            target: target.into(),
            environment: None,
            required: true,
        }
    }

    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    pub fn environment_variable(mut self, name: impl Into<String>) -> Self {
        self.environment = Some(name.into());
        self
    }
}

impl AdapterRuntimeResource {
    pub fn oci_container(
        name: impl Into<String>,
        artifact_name: impl Into<String>,
        container_port: u16,
    ) -> Self {
        Self {
            name: name.into(),
            runtime_type: "container".to_string(),
            provider: "oci".to_string(),
            artifact_name: artifact_name.into(),
            container_port,
            protocol: "tcp".to_string(),
            health_type: "tcp".to_string(),
            health_timeout_ms: 30_000,
            restart_policy: "on_failure".to_string(),
            cleanup_owner: "trail".to_string(),
            volume_target: None,
            secrets: Vec::new(),
        }
    }

    pub fn health_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.health_timeout_ms = timeout_ms;
        self
    }

    pub fn restart_policy(mut self, policy: impl Into<String>) -> Self {
        self.restart_policy = policy.into();
        self
    }

    pub fn volume_target(mut self, target: impl Into<String>) -> Self {
        self.volume_target = Some(target.into());
        self
    }

    pub fn secret(mut self, secret: AdapterSecretReference) -> Self {
        self.secrets.push(secret);
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct AdapterDependency {
    pub component_id: String,
    pub edge_type: AdapterDependencyType,
}

impl AdapterDependency {
    pub fn new(component_id: impl Into<String>, edge_type: AdapterDependencyType) -> Self {
        Self {
            component_id: component_id.into(),
            edge_type,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AdapterDependencyType {
    BuildRequires,
    RuntimeRequires,
    BindsAfter,
    InvalidatesWith,
}

/// A host-owned performance cache requested by a protocol-v2 adapter.
///
/// Cache contents must never be required for correctness: Trail may evict a
/// namespace at any time when it has no live users. External adapters are
/// initially authorized only for `HostExclusive` access; `ToolConcurrent` is
/// represented on the wire so independently certified adapters do not need a
/// future protocol shape change.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterCache {
    pub name: String,
    pub protocol: AdapterCacheProtocol,
    pub access: AdapterCacheAccess,
    /// Adapter-specific, non-secret compatibility dimensions. Trail adds the
    /// authenticated distribution, protocol, platform, and architecture.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub compatibility: BTreeMap<String, String>,
    /// Environment variable to relative namespace subpath. `.` binds the
    /// namespace root. Trail resolves and injects absolute paths only after it
    /// has validated and acquired the namespace.
    pub environment: BTreeMap<String, String>,
}

impl AdapterCache {
    pub fn host_exclusive(name: impl Into<String>, protocol: AdapterCacheProtocol) -> Self {
        Self {
            name: name.into(),
            protocol,
            access: AdapterCacheAccess::HostExclusive,
            compatibility: BTreeMap::new(),
            environment: BTreeMap::new(),
        }
    }

    pub fn tool_concurrent(name: impl Into<String>, protocol: AdapterCacheProtocol) -> Self {
        Self {
            name: name.into(),
            protocol,
            access: AdapterCacheAccess::ToolConcurrent,
            compatibility: BTreeMap::new(),
            environment: BTreeMap::new(),
        }
    }

    pub fn compatibility_dimension(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.compatibility.insert(name.into(), value.into());
        self
    }

    pub fn environment_variable(
        mut self,
        name: impl Into<String>,
        relative_subpath: impl Into<String>,
    ) -> Self {
        self.environment
            .insert(name.into(), relative_subpath.into());
        self
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdapterCacheProtocol {
    ContentStore,
    CompilerCache,
    LockedIndex,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdapterCacheAccess {
    HostExclusive,
    ToolConcurrent,
}

impl AdapterPlanV2 {
    pub fn builder(
        component_id: impl Into<String>,
        kind: impl Into<String>,
    ) -> AdapterPlanV2Builder {
        AdapterPlanV2Builder::new(component_id, kind)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "phase", content = "command", rename_all = "snake_case")]
pub enum AdapterAction {
    Staging(AdapterCommand),
    MountedInitialization(AdapterCommand),
}

impl AdapterPlan {
    /// Start a deterministic plan builder. This is an authoring convenience;
    /// the resulting value has exactly the same v1 wire representation as a
    /// directly constructed `AdapterPlan`.
    pub fn builder(component_id: impl Into<String>, kind: impl Into<String>) -> AdapterPlanBuilder {
        AdapterPlanBuilder::new(component_id, kind)
    }
}

/// Validating builder for the normalized v1 plan returned by an adapter.
///
/// Trail still performs authoritative host-side validation. The builder gives
/// adapter authors fast local failures for the structural mistakes that most
/// often otherwise appear only during `trail env plan`.
#[derive(Clone, Debug)]
pub struct AdapterPlanBuilder {
    component_id: String,
    kind: String,
    dependencies: Vec<String>,
    identity_inputs: Vec<String>,
    semantic_inputs: BTreeMap<String, String>,
    command: Option<AdapterCommand>,
    outputs: Vec<AdapterOutput>,
    portability: AdapterPortability,
    stale_reason: Option<String>,
}

impl AdapterPlanBuilder {
    pub fn new(component_id: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            component_id: component_id.into(),
            kind: kind.into(),
            dependencies: Vec::new(),
            identity_inputs: Vec::new(),
            semantic_inputs: BTreeMap::new(),
            command: None,
            outputs: Vec::new(),
            portability: AdapterPortability::Host,
            stale_reason: None,
        }
    }

    pub fn dependency(mut self, component_id: impl Into<String>) -> Self {
        self.dependencies.push(component_id.into());
        self
    }

    pub fn dependencies<I, S>(mut self, component_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.dependencies
            .extend(component_ids.into_iter().map(Into::into));
        self
    }

    pub fn identity_input(mut self, path: impl Into<String>) -> Self {
        self.identity_inputs.push(path.into());
        self
    }

    pub fn identity_inputs<I, S>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.identity_inputs
            .extend(paths.into_iter().map(Into::into));
        self
    }

    pub fn semantic_input(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.semantic_inputs.insert(name.into(), value.into());
        self
    }

    pub fn command(mut self, command: AdapterCommand) -> Self {
        self.command = Some(command);
        self
    }

    pub fn output(mut self, output: AdapterOutput) -> Self {
        self.outputs.push(output);
        self
    }

    pub fn outputs<I>(mut self, outputs: I) -> Self
    where
        I: IntoIterator<Item = AdapterOutput>,
    {
        self.outputs.extend(outputs);
        self
    }

    pub fn portability(mut self, portability: AdapterPortability) -> Self {
        self.portability = portability;
        self
    }

    pub fn stale_reason(mut self, reason: impl Into<String>) -> Self {
        self.stale_reason = Some(reason.into());
        self
    }

    pub fn build(mut self) -> Result<AdapterPlan, AdapterPlanBuildError> {
        require_non_empty(&self.component_id, "component_id")?;
        require_non_empty(&self.kind, "kind")?;
        for dependency in &self.dependencies {
            require_non_empty(dependency, "dependencies")?;
            if dependency == &self.component_id {
                return Err(AdapterPlanBuildError::SelfDependency {
                    component_id: self.component_id,
                });
            }
        }
        sort_and_reject_duplicates(&mut self.dependencies, "dependencies")?;
        for path in &self.identity_inputs {
            require_non_empty(path, "identity_inputs")?;
        }
        sort_and_reject_duplicates(&mut self.identity_inputs, "identity_inputs")?;
        for (name, value) in &self.semantic_inputs {
            require_non_empty(name, "semantic_inputs key")?;
            if value.contains('\0') {
                return Err(AdapterPlanBuildError::NulValue {
                    field: "semantic_inputs value",
                });
            }
        }
        let command = self
            .command
            .ok_or(AdapterPlanBuildError::MissingField { field: "command" })?;
        validate_adapter_command(&command, "command")?;
        if self.outputs.is_empty() || self.outputs.len() > 32 {
            return Err(AdapterPlanBuildError::OutputCount {
                actual: self.outputs.len(),
            });
        }
        let mut output_names = Vec::with_capacity(self.outputs.len());
        let mut output_targets = Vec::with_capacity(self.outputs.len());
        for output in &self.outputs {
            require_non_empty(&output.name, "outputs.name")?;
            require_non_empty(&output.source, "outputs.source")?;
            require_non_empty(&output.target, "outputs.target")?;
            if !output.create_if_missing {
                return Err(AdapterPlanBuildError::OutputCreationRequired {
                    output: output.name.clone(),
                });
            }
            output_names.push(output.name.clone());
            output_targets.push(output.target.clone());
        }
        sort_and_reject_duplicates(&mut output_names, "outputs.name")?;
        sort_and_reject_duplicates(&mut output_targets, "outputs.target")?;
        let stale_reason = self
            .stale_reason
            .ok_or(AdapterPlanBuildError::MissingField {
                field: "stale_reason",
            })?;
        require_non_empty(&stale_reason, "stale_reason")?;

        Ok(AdapterPlan {
            component_id: self.component_id,
            kind: self.kind,
            dependencies: self.dependencies,
            identity_inputs: self.identity_inputs,
            semantic_inputs: self.semantic_inputs,
            command,
            outputs: self.outputs,
            portability: self.portability,
            stale_reason,
        })
    }
}

/// Deterministic validating builder for protocol-v2 action plans.
#[derive(Clone, Debug)]
pub struct AdapterPlanV2Builder {
    component_id: String,
    kind: String,
    dependencies: Vec<String>,
    dependency_edges: Vec<AdapterDependency>,
    identity_inputs: Vec<String>,
    semantic_inputs: BTreeMap<String, String>,
    caches: Vec<AdapterCache>,
    external_artifacts: Vec<AdapterExternalArtifact>,
    runtime_resources: Vec<AdapterRuntimeResource>,
    actions: Vec<AdapterAction>,
    outputs: Vec<AdapterOutput>,
    portability: AdapterPortability,
    stale_reason: Option<String>,
}

impl AdapterPlanV2Builder {
    pub fn new(component_id: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            component_id: component_id.into(),
            kind: kind.into(),
            dependencies: Vec::new(),
            dependency_edges: Vec::new(),
            identity_inputs: Vec::new(),
            semantic_inputs: BTreeMap::new(),
            caches: Vec::new(),
            external_artifacts: Vec::new(),
            runtime_resources: Vec::new(),
            actions: Vec::new(),
            outputs: Vec::new(),
            portability: AdapterPortability::Host,
            stale_reason: None,
        }
    }

    pub fn dependency(mut self, component_id: impl Into<String>) -> Self {
        self.dependencies.push(component_id.into());
        self
    }

    pub fn dependencies<I, S>(mut self, component_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.dependencies
            .extend(component_ids.into_iter().map(Into::into));
        self
    }

    pub fn dependency_edge(
        mut self,
        component_id: impl Into<String>,
        edge_type: AdapterDependencyType,
    ) -> Self {
        self.dependency_edges
            .push(AdapterDependency::new(component_id, edge_type));
        self
    }

    pub fn build_requires(self, component_id: impl Into<String>) -> Self {
        self.dependency_edge(component_id, AdapterDependencyType::BuildRequires)
    }

    pub fn runtime_requires(self, component_id: impl Into<String>) -> Self {
        self.dependency_edge(component_id, AdapterDependencyType::RuntimeRequires)
    }

    pub fn binds_after(self, component_id: impl Into<String>) -> Self {
        self.dependency_edge(component_id, AdapterDependencyType::BindsAfter)
    }

    pub fn invalidates_with(self, component_id: impl Into<String>) -> Self {
        self.dependency_edge(component_id, AdapterDependencyType::InvalidatesWith)
    }

    pub fn identity_input(mut self, path: impl Into<String>) -> Self {
        self.identity_inputs.push(path.into());
        self
    }

    pub fn identity_inputs<I, S>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.identity_inputs
            .extend(paths.into_iter().map(Into::into));
        self
    }

    pub fn semantic_input(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.semantic_inputs.insert(name.into(), value.into());
        self
    }

    pub fn cache(mut self, cache: AdapterCache) -> Self {
        self.caches.push(cache);
        self
    }

    pub fn caches<I>(mut self, caches: I) -> Self
    where
        I: IntoIterator<Item = AdapterCache>,
    {
        self.caches.extend(caches);
        self
    }

    pub fn external_artifact(mut self, artifact: AdapterExternalArtifact) -> Self {
        self.external_artifacts.push(artifact);
        self
    }

    pub fn external_artifacts<I>(mut self, artifacts: I) -> Self
    where
        I: IntoIterator<Item = AdapterExternalArtifact>,
    {
        self.external_artifacts.extend(artifacts);
        self
    }

    pub fn runtime_resource(mut self, resource: AdapterRuntimeResource) -> Self {
        self.runtime_resources.push(resource);
        self
    }

    pub fn runtime_resources<I>(mut self, resources: I) -> Self
    where
        I: IntoIterator<Item = AdapterRuntimeResource>,
    {
        self.runtime_resources.extend(resources);
        self
    }

    pub fn staging_command(mut self, command: AdapterCommand) -> Self {
        self.actions.push(AdapterAction::Staging(command));
        self
    }

    pub fn mounted_command(mut self, command: AdapterCommand) -> Self {
        self.actions
            .push(AdapterAction::MountedInitialization(command));
        self
    }

    pub fn action(mut self, action: AdapterAction) -> Self {
        self.actions.push(action);
        self
    }

    pub fn output(mut self, output: AdapterOutput) -> Self {
        self.outputs.push(output);
        self
    }

    pub fn outputs<I>(mut self, outputs: I) -> Self
    where
        I: IntoIterator<Item = AdapterOutput>,
    {
        self.outputs.extend(outputs);
        self
    }

    pub fn portability(mut self, portability: AdapterPortability) -> Self {
        self.portability = portability;
        self
    }

    pub fn stale_reason(mut self, reason: impl Into<String>) -> Self {
        self.stale_reason = Some(reason.into());
        self
    }

    pub fn build(mut self) -> Result<AdapterPlanV2, AdapterPlanBuildError> {
        require_non_empty(&self.component_id, "component_id")?;
        require_non_empty(&self.kind, "kind")?;
        for dependency in &self.dependencies {
            require_non_empty(dependency, "dependencies")?;
            if dependency == &self.component_id {
                return Err(AdapterPlanBuildError::SelfDependency {
                    component_id: self.component_id,
                });
            }
        }
        sort_and_reject_duplicates(&mut self.dependencies, "dependencies")?;
        self.dependency_edges
            .sort_by(|left, right| left.component_id.cmp(&right.component_id));
        let mut all_dependency_ids = self.dependencies.iter().cloned().collect::<BTreeSet<_>>();
        for dependency in &self.dependency_edges {
            require_non_empty(&dependency.component_id, "dependency_edges.component_id")?;
            if dependency.component_id == self.component_id {
                return Err(AdapterPlanBuildError::SelfDependency {
                    component_id: self.component_id,
                });
            }
            if !all_dependency_ids.insert(dependency.component_id.clone()) {
                return Err(AdapterPlanBuildError::DuplicateValue {
                    field: "dependency_edges.component_id",
                    value: dependency.component_id.clone(),
                });
            }
        }
        for path in &self.identity_inputs {
            require_non_empty(path, "identity_inputs")?;
        }
        sort_and_reject_duplicates(&mut self.identity_inputs, "identity_inputs")?;
        for (name, value) in &self.semantic_inputs {
            require_non_empty(name, "semantic_inputs key")?;
            if value.contains('\0') {
                return Err(AdapterPlanBuildError::NulValue {
                    field: "semantic_inputs value",
                });
            }
        }
        let metadata_only = !self.external_artifacts.is_empty();
        if metadata_only
            && (self.kind != "external"
                || !self.actions.is_empty()
                || !self.caches.is_empty()
                || !self.outputs.is_empty())
        {
            return Err(AdapterPlanBuildError::ExternalArtifactPlanConflict);
        }
        if !metadata_only && self.actions.is_empty() {
            return Err(AdapterPlanBuildError::MissingAction);
        }
        if self.actions.len() > 9 {
            return Err(AdapterPlanBuildError::ActionCount {
                actual: self.actions.len(),
            });
        }
        let staging_count = self
            .actions
            .iter()
            .filter(|action| matches!(action, AdapterAction::Staging(_)))
            .count();
        if staging_count > 1 {
            return Err(AdapterPlanBuildError::StagingCommandCount {
                actual: staging_count,
            });
        }
        validate_adapter_caches(&mut self.caches, staging_count)?;
        validate_adapter_external_artifacts(&mut self.external_artifacts)?;
        validate_adapter_runtime_resources(&mut self.runtime_resources, &self.external_artifacts)?;
        for action in &self.actions {
            let (field, command) = match action {
                AdapterAction::Staging(command) => ("actions.staging", command),
                AdapterAction::MountedInitialization(command) => {
                    ("actions.mounted_initialization", command)
                }
            };
            validate_adapter_command(command, field)?;
        }
        if !metadata_only {
            validate_adapter_outputs(&self.outputs)?;
        }
        let stale_reason = self
            .stale_reason
            .ok_or(AdapterPlanBuildError::MissingField {
                field: "stale_reason",
            })?;
        require_non_empty(&stale_reason, "stale_reason")?;
        Ok(AdapterPlanV2 {
            component_id: self.component_id,
            kind: self.kind,
            dependencies: self.dependencies,
            dependency_edges: self.dependency_edges,
            identity_inputs: self.identity_inputs,
            semantic_inputs: self.semantic_inputs,
            caches: self.caches,
            external_artifacts: self.external_artifacts,
            runtime_resources: self.runtime_resources,
            actions: self.actions,
            outputs: self.outputs,
            portability: self.portability,
            stale_reason,
        })
    }
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum AdapterPlanBuildError {
    #[error("adapter plan field `{field}` is required")]
    MissingField { field: &'static str },
    #[error("adapter plan requires at least one staging or mounted action")]
    MissingAction,
    #[error("adapter plan field `{field}` contains an empty value")]
    EmptyValue { field: &'static str },
    #[error("adapter plan field `{field}` contains a NUL byte")]
    NulValue { field: &'static str },
    #[error("adapter plan field `{field}` contains duplicate value `{value}`")]
    DuplicateValue { field: &'static str, value: String },
    #[error("adapter component `{component_id}` cannot depend on itself")]
    SelfDependency { component_id: String },
    #[error("adapter plans require 1-32 outputs; received {actual}")]
    OutputCount { actual: usize },
    #[error("adapter output `{output}` must allow Trail to create a missing directory")]
    OutputCreationRequired { output: String },
    #[error("adapter protocol-v2 plans support at most nine actions; received {actual}")]
    ActionCount { actual: usize },
    #[error("adapter protocol-v2 plans support at most one staging action; received {actual}")]
    StagingCommandCount { actual: usize },
    #[error("adapter protocol-v2 plans support at most sixteen caches; received {actual}")]
    CacheCount { actual: usize },
    #[error(
        "external-artifact plans require kind `external` and cannot mix actions, caches, or filesystem outputs"
    )]
    ExternalArtifactPlanConflict,
    #[error("adapter protocol-v2 plans support at most 32 external artifacts; received {actual}")]
    ExternalArtifactCount { actual: usize },
    #[error("adapter external artifact `{artifact}` is invalid")]
    InvalidExternalArtifact { artifact: String },
    #[error("adapter protocol-v2 plans support at most 32 runtime resources; received {actual}")]
    RuntimeResourceCount { actual: usize },
    #[error("adapter runtime resource `{resource}` is invalid")]
    InvalidRuntimeResource { resource: String },
    #[error("adapter cache `{cache}` requires at least one environment binding")]
    CacheBindingRequired { cache: String },
    #[error("adapter cache `{cache}` has invalid relative subpath `{path}`")]
    InvalidCacheSubpath { cache: String, path: String },
}

fn validate_adapter_external_artifacts(
    artifacts: &mut [AdapterExternalArtifact],
) -> Result<(), AdapterPlanBuildError> {
    if artifacts.len() > 32 {
        return Err(AdapterPlanBuildError::ExternalArtifactCount {
            actual: artifacts.len(),
        });
    }
    artifacts.sort_by(|left, right| left.name.cmp(&right.name));
    for (index, artifact) in artifacts.iter().enumerate() {
        let digest = artifact.digest.strip_prefix("sha256:");
        let valid_digest = digest.is_some_and(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        });
        let valid_reference =
            artifact
                .reference
                .rsplit_once('@')
                .is_some_and(|(repository, digest)| {
                    !repository.is_empty()
                        && !repository.contains('@')
                        && digest == artifact.digest
                        && repository
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || b"._-/:".contains(&byte))
                });
        if artifact.name.is_empty()
            || artifact.name.len() > 128
            || !artifact.name.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
            })
            || index > 0 && artifacts[index - 1].name == artifact.name
            || artifact.artifact_type != "oci_image"
            || artifact.provider != "oci"
            || artifact.cleanup_owner != "external"
            || !valid_digest
            || !valid_reference
            || !matches!(
                artifact.platform.as_str(),
                "linux/amd64" | "linux/arm64" | "windows/amd64" | "windows/arm64"
            )
        {
            return Err(AdapterPlanBuildError::InvalidExternalArtifact {
                artifact: artifact.name.clone(),
            });
        }
    }
    Ok(())
}

fn validate_adapter_runtime_resources(
    resources: &mut [AdapterRuntimeResource],
    artifacts: &[AdapterExternalArtifact],
) -> Result<(), AdapterPlanBuildError> {
    if resources.len() > 32 {
        return Err(AdapterPlanBuildError::RuntimeResourceCount {
            actual: resources.len(),
        });
    }
    let artifact_names = artifacts
        .iter()
        .map(|artifact| artifact.name.as_str())
        .collect::<BTreeSet<_>>();
    resources.sort_by(|left, right| left.name.cmp(&right.name));
    for resource in resources.iter_mut() {
        resource
            .secrets
            .sort_by(|left, right| left.name.cmp(&right.name));
    }
    for (index, resource) in resources.iter().enumerate() {
        let valid_volume = resource.volume_target.as_deref().is_none_or(|target| {
            target.starts_with('/')
                && target.len() <= 4096
                && !target.contains('\\')
                && !target.chars().any(char::is_control)
                && target
                    .split('/')
                    .skip(1)
                    .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
                && !["/proc", "/sys", "/dev", "/run", "/etc"]
                    .iter()
                    .any(|reserved| {
                        target == *reserved || target.starts_with(&format!("{reserved}/"))
                    })
        });
        if resource.name.is_empty()
            || resource.name.len() > 128
            || !resource.name.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
            })
            || index > 0 && resources[index - 1].name == resource.name
            || resource.runtime_type != "container"
            || resource.provider != "oci"
            || !artifact_names.contains(resource.artifact_name.as_str())
            || resource.container_port == 0
            || resource.protocol != "tcp"
            || resource.health_type != "tcp"
            || !(1_000..=300_000).contains(&resource.health_timeout_ms)
            || !matches!(
                resource.restart_policy.as_str(),
                "never" | "on_failure" | "always"
            )
            || resource.cleanup_owner != "trail"
            || !valid_volume
            || !valid_adapter_secret_references(&resource.secrets)
        {
            return Err(AdapterPlanBuildError::InvalidRuntimeResource {
                resource: resource.name.clone(),
            });
        }
    }
    Ok(())
}

fn valid_adapter_secret_references(secrets: &[AdapterSecretReference]) -> bool {
    if secrets.len() > 16 {
        return false;
    }
    let mut names = BTreeSet::new();
    let mut targets = BTreeSet::new();
    secrets.iter().all(|secret| {
        !secret.name.is_empty()
            && secret.name.len() <= 128
            && secret.name.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
            })
            && names.insert(secret.name.clone())
            && matches!(secret.provider.as_str(), "file" | "environment_file")
            && !secret.reference.is_empty()
            && secret.reference.len() <= 4096
            && !secret.reference.chars().any(char::is_control)
            && !secret.purpose.is_empty()
            && secret.purpose.len() <= 256
            && !secret.purpose.chars().any(char::is_control)
            && secret.injection == "file"
            && secret.target.starts_with("/run/secrets/")
            && secret.target.len() <= 4096
            && !secret.target.contains('\\')
            && !secret.target.chars().any(char::is_control)
            && secret
                .target
                .split('/')
                .skip(1)
                .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
            && targets.insert(secret.target.clone())
            && secret.environment.as_deref().is_none_or(|name| {
                !name.is_empty()
                    && name.len() <= 128
                    && !name.as_bytes()[0].is_ascii_digit()
                    && name.bytes().all(|byte| {
                        byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                    })
            })
            && secret.version.as_deref().is_none_or(|version| {
                !version.is_empty()
                    && version.len() <= 256
                    && !version.chars().any(char::is_control)
            })
            && (secret.provider != "file" || secret.reference.starts_with('/'))
            && (secret.provider != "environment_file"
                || secret.reference.chars().all(|character| {
                    character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
                }))
    })
}

fn validate_adapter_caches(
    caches: &mut [AdapterCache],
    staging_count: usize,
) -> Result<(), AdapterPlanBuildError> {
    if caches.len() > 16 {
        return Err(AdapterPlanBuildError::CacheCount {
            actual: caches.len(),
        });
    }
    if !caches.is_empty() && staging_count != 1 {
        return Err(AdapterPlanBuildError::MissingField {
            field: "cache staging action",
        });
    }
    caches.sort_by(|left, right| left.name.cmp(&right.name));
    let mut previous_name: Option<&str> = None;
    let mut environment_names = BTreeSet::new();
    for cache in caches {
        require_non_empty(&cache.name, "caches.name")?;
        if cache.name.len() > 64
            || !cache.name.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
            })
        {
            return Err(AdapterPlanBuildError::EmptyValue {
                field: "caches.name",
            });
        }
        if previous_name == Some(cache.name.as_str()) {
            return Err(AdapterPlanBuildError::DuplicateValue {
                field: "caches.name",
                value: cache.name.clone(),
            });
        }
        previous_name = Some(&cache.name);
        if cache.compatibility.len() > 24 {
            return Err(AdapterPlanBuildError::CacheCount {
                actual: cache.compatibility.len(),
            });
        }
        for (name, value) in &cache.compatibility {
            require_non_empty(name, "caches.compatibility key")?;
            require_non_empty(value, "caches.compatibility value")?;
            if name.len() > 128 || value.len() > 4096 || name.contains('\0') || value.contains('\0')
            {
                return Err(AdapterPlanBuildError::NulValue {
                    field: "caches.compatibility",
                });
            }
        }
        if cache.environment.is_empty() {
            return Err(AdapterPlanBuildError::CacheBindingRequired {
                cache: cache.name.clone(),
            });
        }
        if cache.environment.len() > 16 {
            return Err(AdapterPlanBuildError::CacheCount {
                actual: cache.environment.len(),
            });
        }
        for (name, path) in &cache.environment {
            if name.is_empty()
                || name.len() > 128
                || !name
                    .chars()
                    .all(|character| character == '_' || character.is_ascii_alphanumeric())
                || !environment_names.insert(name.clone())
            {
                return Err(AdapterPlanBuildError::DuplicateValue {
                    field: "caches.environment",
                    value: name.clone(),
                });
            }
            if !valid_cache_subpath(path) {
                return Err(AdapterPlanBuildError::InvalidCacheSubpath {
                    cache: cache.name.clone(),
                    path: path.clone(),
                });
            }
        }
    }
    Ok(())
}

fn valid_cache_subpath(path: &str) -> bool {
    if path == "." {
        return true;
    }
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn validate_adapter_command(
    command: &AdapterCommand,
    field: &'static str,
) -> Result<(), AdapterPlanBuildError> {
    require_non_empty(&command.program, field)?;
    require_non_empty(&command.working_directory, field)?;
    if command
        .args
        .iter()
        .chain(command.environment.keys())
        .chain(command.environment.values())
        .any(|value| value.contains('\0'))
    {
        return Err(AdapterPlanBuildError::NulValue { field });
    }
    Ok(())
}

fn validate_adapter_outputs(outputs: &[AdapterOutput]) -> Result<(), AdapterPlanBuildError> {
    if outputs.is_empty() || outputs.len() > 32 {
        return Err(AdapterPlanBuildError::OutputCount {
            actual: outputs.len(),
        });
    }
    let mut output_names = Vec::with_capacity(outputs.len());
    let mut output_targets = Vec::with_capacity(outputs.len());
    for output in outputs {
        require_non_empty(&output.name, "outputs.name")?;
        require_non_empty(&output.source, "outputs.source")?;
        require_non_empty(&output.target, "outputs.target")?;
        if !output.create_if_missing {
            return Err(AdapterPlanBuildError::OutputCreationRequired {
                output: output.name.clone(),
            });
        }
        output_names.push(output.name.clone());
        output_targets.push(output.target.clone());
    }
    sort_and_reject_duplicates(&mut output_names, "outputs.name")?;
    sort_and_reject_duplicates(&mut output_targets, "outputs.target")
}

fn require_non_empty(value: &str, field: &'static str) -> Result<(), AdapterPlanBuildError> {
    if value.trim().is_empty() {
        Err(AdapterPlanBuildError::EmptyValue { field })
    } else {
        Ok(())
    }
}

fn sort_and_reject_duplicates(
    values: &mut [String],
    field: &'static str,
) -> Result<(), AdapterPlanBuildError> {
    values.sort();
    if let Some(duplicate) = values.windows(2).find(|pair| pair[0] == pair[1]) {
        return Err(AdapterPlanBuildError::DuplicateValue {
            field,
            value: duplicate[0].clone(),
        });
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterCommand {
    pub program: String,
    pub args: Vec<String>,
    pub working_directory: String,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
}

impl AdapterCommand {
    pub fn new<I, S>(program: impl Into<String>, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            working_directory: ".".to_string(),
            environment: BTreeMap::new(),
        }
    }

    pub fn in_directory(mut self, working_directory: impl Into<String>) -> Self {
        self.working_directory = working_directory.into();
        self
    }

    pub fn environment_variable(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.environment.insert(name.into(), value.into());
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterOutput {
    pub name: String,
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub policy: AdapterOutputPolicy,
    #[serde(default = "default_create_if_missing")]
    pub create_if_missing: bool,
}

impl AdapterOutput {
    pub fn immutable_seed_private(
        name: impl Into<String>,
        source: impl Into<String>,
        target: impl Into<String>,
    ) -> Self {
        Self::new(
            name,
            source,
            target,
            AdapterOutputPolicy::ImmutableSeedPrivate,
        )
    }

    pub fn writable_private(
        name: impl Into<String>,
        source: impl Into<String>,
        target: impl Into<String>,
    ) -> Self {
        Self::new(name, source, target, AdapterOutputPolicy::WritablePrivate)
    }

    pub fn with_create_if_missing(mut self, create_if_missing: bool) -> Self {
        self.create_if_missing = create_if_missing;
        self
    }

    fn new(
        name: impl Into<String>,
        source: impl Into<String>,
        target: impl Into<String>,
        policy: AdapterOutputPolicy,
    ) -> Self {
        Self {
            name: name.into(),
            source: source.into(),
            target: target.into(),
            policy,
            create_if_missing: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdapterOutputPolicy {
    #[default]
    ImmutableSeedPrivate,
    WritablePrivate,
}

fn default_create_if_missing() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdapterPortability {
    Host,
    Platform,
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("adapter protocol I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("adapter protocol serialization failed: {0}")]
    Cbor(#[from] serde_cbor::Error),
    #[error("adapter protocol frame is {actual} bytes; maximum is {maximum}")]
    FrameTooLarge { actual: usize, maximum: usize },
    #[error("adapter protocol stream ended before a complete frame was read")]
    Truncated,
}

pub fn read_frame<T: for<'de> Deserialize<'de>>(
    reader: &mut impl Read,
    maximum: usize,
) -> Result<T, ProtocolError> {
    let mut header = [0u8; 4];
    reader.read_exact(&mut header).map_err(|error| {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            ProtocolError::Truncated
        } else {
            ProtocolError::Io(error)
        }
    })?;
    let length = u32::from_be_bytes(header) as usize;
    if length > maximum {
        return Err(ProtocolError::FrameTooLarge {
            actual: length,
            maximum,
        });
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).map_err(|error| {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            ProtocolError::Truncated
        } else {
            ProtocolError::Io(error)
        }
    })?;
    Ok(serde_cbor::from_slice(&body)?)
}

pub fn write_frame<T: Serialize>(
    writer: &mut impl Write,
    value: &T,
    maximum: usize,
) -> Result<(), ProtocolError> {
    let body = serde_cbor::to_vec(value)?;
    if body.len() > maximum || body.len() > u32::MAX as usize {
        return Err(ProtocolError::FrameTooLarge {
            actual: body.len(),
            maximum: maximum.min(u32::MAX as usize),
        });
    }
    writer.write_all(&(body.len() as u32).to_be_bytes())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

pub fn serve_once(
    handler: impl FnOnce(AdapterRequest) -> AdapterResponse,
) -> Result<(), ProtocolError> {
    let request = read_frame(&mut io::stdin().lock(), MAX_FRAME_BYTES)?;
    let response = handler(request);
    write_frame(&mut io::stdout().lock(), &response, MAX_FRAME_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framed_protocol_round_trips_binary_pinned_files() {
        let request = AdapterRequest {
            protocol: PROTOCOL_V1.to_string(),
            request_id: "request-1".to_string(),
            adapter_identity: "example/test@1".to_string(),
            distribution_digest: "sha256:abc".to_string(),
            host: AdapterHost {
                operating_system: "test".to_string(),
                architecture: "test".to_string(),
            },
            source_root: "root".to_string(),
            operation: AdapterOperation::Discover {
                component_root: String::new(),
                files: vec![PinnedFile {
                    path: "manifest.bin".to_string(),
                    content_hash: "hash".to_string(),
                    executable: false,
                    content: vec![0, 1, 0xff],
                }],
            },
        };
        let mut wire = Vec::new();
        write_frame(&mut wire, &request, MAX_FRAME_BYTES).unwrap();
        let decoded: AdapterRequest = read_frame(&mut wire.as_slice(), MAX_FRAME_BYTES).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn framed_protocol_rejects_oversized_frames_before_allocating_body() {
        let mut wire = ((MAX_FRAME_BYTES + 1) as u32).to_be_bytes().to_vec();
        wire.extend([0u8; 8]);
        assert!(matches!(
            read_frame::<AdapterRequest>(&mut wire.as_slice(), MAX_FRAME_BYTES),
            Err(ProtocolError::FrameTooLarge { .. })
        ));
    }

    #[test]
    fn output_policy_is_backward_compatible_and_round_trips_writable_private() {
        #[derive(Serialize)]
        struct LegacyOutput<'a> {
            name: &'a str,
            source: &'a str,
            target: &'a str,
            create_if_missing: bool,
        }
        let legacy = serde_cbor::to_vec(&LegacyOutput {
            name: "generated",
            source: "generated",
            target: "build",
            create_if_missing: true,
        })
        .unwrap();
        let legacy: AdapterOutput = serde_cbor::from_slice(&legacy).unwrap();
        assert_eq!(legacy.policy, AdapterOutputPolicy::ImmutableSeedPrivate);

        let private = AdapterOutput {
            name: "build-tree".to_string(),
            source: "build".to_string(),
            target: "build".to_string(),
            policy: AdapterOutputPolicy::WritablePrivate,
            create_if_missing: true,
        };
        let private: AdapterOutput =
            serde_cbor::from_slice(&serde_cbor::to_vec(&private).unwrap()).unwrap();
        assert_eq!(private.policy, AdapterOutputPolicy::WritablePrivate);
    }

    #[test]
    fn dependency_edges_are_backward_compatible_and_round_trip() {
        #[derive(Serialize)]
        struct LegacyPlan {
            component_id: String,
            kind: String,
            identity_inputs: Vec<String>,
            semantic_inputs: BTreeMap<String, String>,
            command: AdapterCommand,
            outputs: Vec<AdapterOutput>,
            portability: AdapterPortability,
            stale_reason: String,
        }
        let legacy = LegacyPlan {
            component_id: "generated".to_string(),
            kind: "generated".to_string(),
            identity_inputs: vec!["schema.json".to_string()],
            semantic_inputs: BTreeMap::new(),
            command: AdapterCommand {
                program: "generator".to_string(),
                args: Vec::new(),
                working_directory: ".".to_string(),
                environment: BTreeMap::new(),
            },
            outputs: vec![AdapterOutput {
                name: "generated".to_string(),
                source: "generated".to_string(),
                target: "generated".to_string(),
                policy: AdapterOutputPolicy::ImmutableSeedPrivate,
                create_if_missing: true,
            }],
            portability: AdapterPortability::Host,
            stale_reason: "input changed".to_string(),
        };
        let decoded: AdapterPlan =
            serde_cbor::from_slice(&serde_cbor::to_vec(&legacy).unwrap()).unwrap();
        assert!(decoded.dependencies.is_empty());

        let mut current = decoded;
        current.dependencies = vec!["toolchain".to_string()];
        let current: AdapterPlan =
            serde_cbor::from_slice(&serde_cbor::to_vec(&current).unwrap()).unwrap();
        assert_eq!(current.dependencies, ["toolchain"]);
    }

    #[test]
    fn v2_builder_supports_mounted_only_initialization() {
        let plan = AdapterPlanV2::builder("python", "dependency")
            .identity_input("pyproject.toml")
            .mounted_command(
                AdapterCommand::new("python3", ["-m", "venv", ".venv"])
                    .in_directory("services/api"),
            )
            .output(AdapterOutput::writable_private("venv", ".venv", ".venv"))
            .stale_reason("Python or the dependency manifest changed")
            .build()
            .unwrap();

        assert_eq!(plan.actions.len(), 1);
        let AdapterAction::MountedInitialization(command) = &plan.actions[0] else {
            panic!("v2 builder emitted the wrong action phase");
        };
        assert_eq!(command.working_directory, "services/api");
        let decoded: AdapterPlanV2 =
            serde_cbor::from_slice(&serde_cbor::to_vec(&plan).unwrap()).unwrap();
        assert_eq!(decoded, plan);
    }

    #[test]
    fn v2_builder_supports_metadata_only_pinned_oci_artifacts() {
        let digest = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let plan = AdapterPlanV2::builder("images", "external")
            .identity_input("images.lock")
            .external_artifact(AdapterExternalArtifact::pinned_oci_image(
                "web",
                format!("ghcr.io/example/web@{digest}"),
                "linux/amd64",
            ))
            .runtime_resource(
                AdapterRuntimeResource::oci_container("web-service", "web", 8080)
                    .health_timeout_ms(45_000)
                    .volume_target("/var/lib/web")
                    .secret(
                        AdapterSecretReference::file(
                            "api-token",
                            "environment_file",
                            "WEB_API_TOKEN_FILE",
                            "/run/secrets/api-token",
                            "authenticate the web service",
                        )
                        .version("rotation-7")
                        .environment_variable("WEB_API_TOKEN_FILE"),
                    ),
            )
            .stale_reason("pinned OCI declaration changed")
            .build()
            .unwrap();

        assert!(plan.actions.is_empty());
        assert!(plan.outputs.is_empty());
        assert_eq!(plan.external_artifacts[0].digest, digest);
        assert_eq!(plan.runtime_resources[0].container_port, 8080);
        assert_eq!(plan.runtime_resources[0].secrets[0].name, "api-token");
        assert_eq!(
            plan.runtime_resources[0].secrets[0].reference,
            "WEB_API_TOKEN_FILE"
        );
        assert_eq!(
            plan.runtime_resources[0].secrets[0].environment.as_deref(),
            Some("WEB_API_TOKEN_FILE")
        );
        let decoded: AdapterPlanV2 =
            serde_cbor::from_slice(&serde_cbor::to_vec(&plan).unwrap()).unwrap();
        assert_eq!(decoded, plan);

        let mixed = AdapterPlanV2::builder("images", "external")
            .external_artifact(AdapterExternalArtifact::pinned_oci_image(
                "web",
                format!("ghcr.io/example/web@{digest}"),
                "linux/amd64",
            ))
            .staging_command(AdapterCommand::new("docker", ["pull"]))
            .stale_reason("pinned OCI declaration changed")
            .build();
        assert_eq!(
            mixed,
            Err(AdapterPlanBuildError::ExternalArtifactPlanConflict)
        );
    }

    #[test]
    fn v2_builder_preserves_typed_dependency_semantics() {
        let plan = AdapterPlanV2::builder("application", "runtime")
            .build_requires("compiler")
            .runtime_requires("database")
            .binds_after("network")
            .invalidates_with("configuration")
            .mounted_command(AdapterCommand::new("initializer", ["prepare"]))
            .output(AdapterOutput::writable_private("state", "state", "state"))
            .stale_reason("typed dependency changed")
            .build()
            .unwrap();

        assert_eq!(plan.dependency_edges.len(), 4);
        assert_eq!(
            plan.dependency_edges[0],
            AdapterDependency::new("compiler", AdapterDependencyType::BuildRequires)
        );
        let decoded: AdapterPlanV2 =
            serde_cbor::from_slice(&serde_cbor::to_vec(&plan).unwrap()).unwrap();
        assert_eq!(decoded, plan);

        let duplicate = AdapterPlanV2::builder("application", "runtime")
            .dependency("compiler")
            .build_requires("compiler")
            .mounted_command(AdapterCommand::new("initializer", ["prepare"]))
            .output(AdapterOutput::writable_private("state", "state", "state"))
            .stale_reason("typed dependency changed")
            .build();
        assert!(matches!(
            duplicate,
            Err(AdapterPlanBuildError::DuplicateValue { .. })
        ));
    }

    #[test]
    fn v2_builder_declares_deterministic_host_owned_caches() {
        let plan = AdapterPlanV2::builder("generated", "generated")
            .cache(
                AdapterCache::host_exclusive("package-store", AdapterCacheProtocol::ContentStore)
                    .compatibility_dimension("tool", "generator@1")
                    .environment_variable("GENERATOR_CACHE", ".")
                    .environment_variable("GENERATOR_INDEX", "index"),
            )
            .staging_command(AdapterCommand::new("generator", ["build"]))
            .output(AdapterOutput::immutable_seed_private(
                "generated",
                "generated",
                "generated",
            ))
            .stale_reason("generator inputs changed")
            .build()
            .unwrap();

        assert_eq!(plan.caches.len(), 1);
        assert_eq!(plan.caches[0].access, AdapterCacheAccess::HostExclusive);
        assert_eq!(plan.caches[0].environment["GENERATOR_INDEX"], "index");
        let decoded: AdapterPlanV2 =
            serde_cbor::from_slice(&serde_cbor::to_vec(&plan).unwrap()).unwrap();
        assert_eq!(decoded, plan);
    }

    #[test]
    fn v2_builder_rejects_mounted_and_ambiguous_cache_bindings() {
        let mounted_only = AdapterPlanV2::builder("generated", "generated")
            .cache(
                AdapterCache::host_exclusive("cache", AdapterCacheProtocol::ContentStore)
                    .environment_variable("GENERATOR_CACHE", "."),
            )
            .mounted_command(AdapterCommand::new("generator", ["build"]))
            .output(AdapterOutput::writable_private(
                "generated",
                "generated",
                "generated",
            ))
            .stale_reason("generator inputs changed")
            .build();
        assert_eq!(
            mounted_only,
            Err(AdapterPlanBuildError::MissingField {
                field: "cache staging action"
            })
        );

        let ambiguous = AdapterPlanV2::builder("generated", "generated")
            .caches([
                AdapterCache::host_exclusive("first", AdapterCacheProtocol::ContentStore)
                    .environment_variable("GENERATOR_CACHE", "first"),
                AdapterCache::host_exclusive("second", AdapterCacheProtocol::LockedIndex)
                    .environment_variable("GENERATOR_CACHE", "second"),
            ])
            .staging_command(AdapterCommand::new("generator", ["build"]))
            .output(AdapterOutput::immutable_seed_private(
                "generated",
                "generated",
                "generated",
            ))
            .stale_reason("generator inputs changed")
            .build();
        assert!(matches!(
            ambiguous,
            Err(AdapterPlanBuildError::DuplicateValue {
                field: "caches.environment",
                ..
            })
        ));
    }

    #[test]
    fn legacy_package_metadata_defaults_to_protocol_v1() {
        let manifest: AdapterPackageManifest = toml::from_str(
            r#"schema = "trail.environment-adapter-package/v1"
[adapter]
canonical_identity = "example/test@1"
implementation_version = "1"
selectors = ["example/test@1"]
kind = "generated"
layer_adapter_name = "test"
discovery_markers = ["test.adapter"]
stability = "experimental"
description = "test"
[executable]
path = "adapter"
sha256 = "sha256:00"
"#,
        )
        .unwrap();
        assert_eq!(manifest.adapter.protocols, [PROTOCOL_V1]);
        let encoded = serde_cbor::to_vec(&manifest).unwrap();
        let value: serde_cbor::Value = serde_cbor::from_slice(&encoded).unwrap();
        let serde_cbor::Value::Map(package) = value else {
            panic!("package did not encode as a CBOR map");
        };
        let adapter = package
            .get(&serde_cbor::Value::Text("adapter".to_string()))
            .unwrap();
        let serde_cbor::Value::Map(adapter) = adapter else {
            panic!("adapter metadata did not encode as a CBOR map");
        };
        assert!(!adapter.contains_key(&serde_cbor::Value::Text("protocols".to_string())));
    }

    #[test]
    fn plan_builder_canonicalizes_sets_and_preserves_wire_shape() {
        let plan = AdapterPlan::builder("generated.client", "generated")
            .dependencies(["schema", "compiler"])
            .identity_inputs(["schema.json", "generator.toml"])
            .semantic_input("strategy", "client-v1")
            .command(
                AdapterCommand::new("generator", ["--out", "generated"])
                    .in_directory("project")
                    .environment_variable("GENERATOR_COLOR", "never"),
            )
            .output(AdapterOutput::immutable_seed_private(
                "client",
                "generated",
                "src/generated",
            ))
            .stale_reason("schema, generator, or strategy changed")
            .build()
            .unwrap();

        assert_eq!(plan.dependencies, ["compiler", "schema"]);
        assert_eq!(plan.identity_inputs, ["generator.toml", "schema.json"]);
        assert_eq!(plan.command.working_directory, "project");
        assert_eq!(plan.command.environment["GENERATOR_COLOR"], "never");
        assert_eq!(
            plan.outputs[0].policy,
            AdapterOutputPolicy::ImmutableSeedPrivate
        );
        let decoded: AdapterPlan =
            serde_cbor::from_slice(&serde_cbor::to_vec(&plan).unwrap()).unwrap();
        assert_eq!(decoded, plan);
    }

    #[test]
    fn plan_builder_rejects_duplicate_and_self_dependencies() {
        let duplicate = AdapterPlan::builder("generated", "generated")
            .dependencies(["schema", "schema"])
            .command(AdapterCommand::new("generator", Vec::<String>::new()))
            .output(AdapterOutput::writable_private(
                "generated",
                "generated",
                "generated",
            ))
            .stale_reason("inputs changed")
            .build();
        assert_eq!(
            duplicate,
            Err(AdapterPlanBuildError::DuplicateValue {
                field: "dependencies",
                value: "schema".to_string(),
            })
        );

        let self_dependency = AdapterPlan::builder("generated", "generated")
            .dependency("generated")
            .command(AdapterCommand::new("generator", Vec::<String>::new()))
            .output(AdapterOutput::writable_private(
                "generated",
                "generated",
                "generated",
            ))
            .stale_reason("inputs changed")
            .build();
        assert_eq!(
            self_dependency,
            Err(AdapterPlanBuildError::SelfDependency {
                component_id: "generated".to_string(),
            })
        );
    }

    #[test]
    fn plan_builder_requires_command_output_and_stale_reason() {
        assert_eq!(
            AdapterPlan::builder("generated", "generated").build(),
            Err(AdapterPlanBuildError::MissingField { field: "command" })
        );
        assert_eq!(
            AdapterPlan::builder("generated", "generated")
                .command(AdapterCommand::new("generator", Vec::<String>::new()))
                .stale_reason("inputs changed")
                .build(),
            Err(AdapterPlanBuildError::OutputCount { actual: 0 })
        );
        assert_eq!(
            AdapterPlan::builder("generated", "generated")
                .command(AdapterCommand::new("generator", Vec::<String>::new()))
                .output(AdapterOutput::immutable_seed_private(
                    "generated",
                    "generated",
                    "generated",
                ))
                .build(),
            Err(AdapterPlanBuildError::MissingField {
                field: "stale_reason",
            })
        );
    }
}
