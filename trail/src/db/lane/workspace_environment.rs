use super::workdir::{ViewPathClass, ViewUpperLayout};
use super::workspace_layer::make_tree_writable;
use super::*;
use std::ffi::OsString;
use std::process::Stdio;
use std::thread;

/// One repository file that the host projects into an adapter-owned staging
/// directory. Adapters describe the mapping; they never receive writable
/// access to the lane source view.
#[derive(Clone, Debug)]
pub(crate) struct WorkspaceEnvironmentInput {
    pub(crate) source_path: String,
    pub(crate) staging_path: String,
    pub(crate) entry: FileEntry,
}

/// A command plan is deliberately argv-based. Trail owns the working
/// directory, environment injection, staging tree, and publication boundary.
#[derive(Clone, Debug)]
pub(crate) struct WorkspaceEnvironmentCommand {
    pub(crate) program: String,
    pub(crate) resolved_program: PathBuf,
    pub(crate) executable_identity: String,
    pub(crate) args: Vec<String>,
    pub(crate) working_directory: String,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) remove_environment: Vec<String>,
    /// Names of host-owned cache declarations used by this action. Commands
    /// never acquire arbitrary writable host paths by supplying directories.
    pub(crate) cache_names: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum WorkspaceEnvironmentCacheProtocol {
    ContentStore,
    CompilerCache,
    LockedIndex,
}

impl WorkspaceEnvironmentCacheProtocol {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ContentStore => "content_store",
            Self::CompilerCache => "compiler_cache",
            Self::LockedIndex => "locked_index",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum WorkspaceEnvironmentCacheAccess {
    /// The ecosystem tool is certified to coordinate concurrent writers and
    /// recover or evict partial entries without changing correctness.
    ToolConcurrent,
    /// Trail serializes all commands using the namespace.
    HostExclusive,
}

impl WorkspaceEnvironmentCacheAccess {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ToolConcurrent => "tool_concurrent",
            Self::HostExclusive => "host_exclusive",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceEnvironmentCache {
    pub(crate) name: String,
    pub(crate) namespace_id: String,
    pub(crate) storage_path: PathBuf,
    pub(crate) protocol: WorkspaceEnvironmentCacheProtocol,
    pub(crate) access: WorkspaceEnvironmentCacheAccess,
    pub(crate) compatibility: BTreeMap<String, String>,
}

/// Immutable identity owned by an external provider rather than Trail's
/// filesystem layer store. Trail records and composes the identity but never
/// deletes provider-owned content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceEnvironmentExternalArtifact {
    pub(crate) name: String,
    pub(crate) artifact_type: String,
    pub(crate) provider: String,
    pub(crate) reference: String,
    pub(crate) digest: String,
    pub(crate) platform: String,
    pub(crate) cleanup_owner: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceEnvironmentRuntimeResource {
    pub(crate) name: String,
    pub(crate) runtime_type: String,
    pub(crate) provider: String,
    pub(crate) artifact_name: String,
    pub(crate) container_port: u16,
    pub(crate) protocol: String,
    pub(crate) health_type: String,
    pub(crate) health_timeout_ms: u64,
    pub(crate) restart_policy: String,
    pub(crate) cleanup_owner: String,
    pub(crate) volume_target: Option<String>,
    pub(crate) secrets: Vec<WorkspaceEnvironmentSecretReference>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkspaceEnvironmentSecretReference {
    pub(crate) name: String,
    pub(crate) provider: String,
    pub(crate) reference: String,
    pub(crate) version: Option<String>,
    pub(crate) purpose: String,
    pub(crate) injection: String,
    pub(crate) target: String,
    pub(crate) environment: Option<String>,
    pub(crate) required: bool,
}

/// One independently mounted directory produced by a component action.
///
/// A component has one canonical key. Immutable-seeded outputs share one
/// content-addressed lower layer, while writable-private outputs live only in
/// the owning lane's generated upper. Every output binding still activates as
/// part of one environment generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum WorkspaceEnvironmentOutputPolicy {
    ImmutableSeedPrivate,
    WritablePrivate,
}

impl WorkspaceEnvironmentOutputPolicy {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ImmutableSeedPrivate => "immutable_seed_private",
            Self::WritablePrivate => "writable_private",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceEnvironmentOutput {
    pub(crate) name: String,
    pub(crate) output_path: String,
    pub(crate) mount_path: String,
    pub(crate) policy: WorkspaceEnvironmentOutputPolicy,
    pub(crate) create_if_missing: bool,
}

/// Host-owned semantics for one directed component relationship.
///
/// Every edge participates in graph validation and deterministic ordering. Only
/// identity-bearing edges enter the target component's canonical artifact key;
/// runtime and binding-order edges are instead retained in generation
/// provenance so changing them does not manufacture a different artifact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum WorkspaceEnvironmentEdgeType {
    BuildRequires,
    RuntimeRequires,
    BindsAfter,
    InvalidatesWith,
}

impl WorkspaceEnvironmentEdgeType {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::BuildRequires => "build_requires",
            Self::RuntimeRequires => "runtime_requires",
            Self::BindsAfter => "binds_after",
            Self::InvalidatesWith => "invalidates_with",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "build_requires" | "ordering_invalidation" => Ok(Self::BuildRequires),
            "runtime_requires" => Ok(Self::RuntimeRequires),
            "binds_after" => Ok(Self::BindsAfter),
            "invalidates_with" => Ok(Self::InvalidatesWith),
            other => Err(Error::InvalidInput(format!(
                "unknown environment edge type `{other}`; expected build_requires, runtime_requires, binds_after, or invalidates_with"
            ))),
        }
    }

    fn identity_input_name(self, component_id: &str) -> Option<String> {
        match self {
            // Preserve the schema-v8 key shape for legacy `depends_on` edges.
            Self::BuildRequires => Some(format!("dependency:{component_id}")),
            Self::InvalidatesWith => Some(format!("dependency:invalidates_with:{component_id}")),
            Self::RuntimeRequires | Self::BindsAfter => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct WorkspaceEnvironmentDependency {
    pub(crate) component_id: String,
    pub(crate) edge_type: WorkspaceEnvironmentEdgeType,
}

impl WorkspaceEnvironmentDependency {
    pub(crate) fn build_requires(component_id: impl Into<String>) -> Self {
        Self {
            component_id: component_id.into(),
            edge_type: WorkspaceEnvironmentEdgeType::BuildRequires,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedWorkspaceEnvironmentDependency {
    pub(crate) component_id: String,
    pub(crate) component_key: String,
    pub(crate) edge_type: WorkspaceEnvironmentEdgeType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceEnvironmentSandboxPolicy {
    /// Existing first-party adapters whose ecosystem-specific execution policy
    /// is maintained with Trail. These remain open-world until migrated onto
    /// individually certified capability profiles.
    TrustedBuiltin,
    /// Repository-authored argv command. The host must execute it inside a
    /// supported kernel sandbox or fail closed before launching the tool.
    RestrictedRecipe,
    /// An installed protocol-v2 plugin staging action with host-owned cache
    /// namespaces. Cache access is projected into the deny-by-default native
    /// sandbox and is never available to planning or mounted actions.
    RestrictedPluginStaging,
    /// An installed protocol-v2 plugin requested mounted initialization. The
    /// host authorizes the typed action only after normalizing the authenticated
    /// package plan, then applies the same native deny-by-default sandbox while
    /// the command runs in an ephemeral candidate lane view.
    RestrictedPluginMounted,
}

/// Normalized plan emitted by every built-in environment adapter.
///
/// The layer key and output contract are data so the host can validate and
/// execute the plan without giving adapters publication or database access.
#[derive(Clone, Debug)]
pub(crate) struct WorkspaceEnvironmentPlan {
    pub(crate) component_id: String,
    pub(crate) adapter_identity: String,
    pub(crate) adapter_version: u32,
    pub(crate) implementation_version: String,
    pub(crate) distribution_digest: String,
    pub(crate) kind: String,
    /// Stable logical component relationships. Trail validates and resolves
    /// these edges; adapters never execute or attach dependencies themselves.
    pub(crate) dependencies: Vec<WorkspaceEnvironmentDependency>,
    /// Populated only by host graph finalization. This retains the exact
    /// upstream keys for both identity-bearing and generation-only edges.
    pub(crate) resolved_dependencies: Vec<ResolvedWorkspaceEnvironmentDependency>,
    pub(crate) layer_key: WorkspaceLayerKeyV1,
    pub(crate) inputs: Vec<WorkspaceEnvironmentInput>,
    /// Optional complete pinned source projection for adapters such as Cargo
    /// whose build graph can include arbitrary workspace source files. The
    /// host streams this root in bounded chunks during explicit sync.
    pub(crate) source_projection: Option<(ObjectId, String)>,
    pub(crate) pre_commands: Vec<WorkspaceEnvironmentCommand>,
    /// Optional host-executed initialization. Writable-private adapters may
    /// provision an empty persistent directory and defer path-sensitive tools
    /// (for example CMake configure) to execution inside the mounted lane.
    pub(crate) command: Option<WorkspaceEnvironmentCommand>,
    /// Commands that must observe the lane's stable mountpoint. Trail runs
    /// them against an ephemeral candidate view and persists only declared
    /// writable-private outputs before normal atomic activation.
    pub(crate) mounted_commands: Vec<WorkspaceEnvironmentCommand>,
    /// Performance-only host cache namespaces. Eviction may make execution
    /// slower but must never change component correctness or readiness.
    pub(crate) caches: Vec<WorkspaceEnvironmentCache>,
    /// Provider-owned immutable identities such as pinned OCI manifests.
    /// These have no manufactured filesystem output and survive cache GC as
    /// generation provenance only.
    pub(crate) external_artifacts: Vec<WorkspaceEnvironmentExternalArtifact>,
    /// Per-lane runtime resources derived from immutable external artifacts.
    /// The declaration is identity-bearing; provider allocation IDs and host
    /// ports are assigned only after the generation commits.
    pub(crate) runtime_resources: Vec<WorkspaceEnvironmentRuntimeResource>,
    pub(crate) sandbox_policy: WorkspaceEnvironmentSandboxPolicy,
    pub(crate) outputs: Vec<WorkspaceEnvironmentOutput>,
    pub(crate) stale_reason: String,
}

struct PreparedPrivateEnvironmentOutputs {
    _staging: tempfile::TempDir,
    paths: Vec<PathBuf>,
}

enum PreparedEnvironmentArtifacts {
    Immutable(WorkspaceLayerReport),
    WritablePrivate(Option<PreparedPrivateEnvironmentOutputs>),
    MetadataOnly,
}

/// Static, side-effect-free facts used to register and discover an adapter.
///
/// Keeping these facts outside `detect` means the host can build a catalog and
/// scan only relevant manifest names without invoking ecosystem tooling.
#[derive(Clone, Debug)]
pub(crate) struct WorkspaceEnvironmentAdapterMetadata {
    pub(crate) canonical_identity: &'static str,
    pub(crate) namespace: &'static str,
    pub(crate) name: &'static str,
    pub(crate) contract_major: u32,
    pub(crate) implementation_version: &'static str,
    pub(crate) distribution_digest: &'static str,
    pub(crate) selectors: &'static [&'static str],
    pub(crate) kind: &'static str,
    pub(crate) layer_adapter_name: &'static str,
    pub(crate) discovery_markers: &'static [&'static str],
    pub(crate) supported_operating_systems: &'static [&'static str],
    pub(crate) supported_architectures: &'static [&'static str],
    pub(crate) stability: &'static str,
    pub(crate) description: &'static str,
}

/// Ecosystem code is restricted to discovery and deterministic planning.
/// Trail remains responsible for executing, validating, publishing, binding,
/// persisting, and reporting the resulting environment.
pub(crate) trait WorkspaceEnvironmentAdapter: Sync {
    fn metadata(&self) -> &'static WorkspaceEnvironmentAdapterMetadata;

    fn identity(&self) -> &'static str {
        self.metadata().canonical_identity
    }

    fn kind(&self) -> &'static str {
        self.metadata().kind
    }

    /// Stable legacy/storage name used by WorkspaceLayerKeyV1. Keeping this
    /// separate lets public component reports use the canonical contract
    /// identity without invalidating already-published layer keys.
    fn layer_adapter_name(&self) -> &'static str {
        self.metadata().layer_adapter_name
    }

    fn accepts_selector(&self, selector: &str) -> bool {
        self.metadata().selectors.contains(&selector)
    }

    fn component_id(&self, component_root: &str) -> Result<String>;

    fn detect(&self, db: &Trail, source_root: &ObjectId, component_root: &str) -> Result<bool>;

    fn plan(
        &self,
        db: &Trail,
        source_root: &ObjectId,
        component_root: &str,
    ) -> Result<WorkspaceEnvironmentPlan>;
}

fn builtin_environment_adapters() -> [&'static dyn WorkspaceEnvironmentAdapter; 6] {
    [
        &super::workspace_node::NODE_WORKSPACE_ADAPTER,
        &super::workspace_cargo::CARGO_TARGET_SEED_ADAPTER,
        &super::workspace_cmake::CMAKE_BUILD_TREE_ADAPTER,
        &super::workspace_go::GO_VENDOR_ADAPTER,
        &super::workspace_python::PYTHON_VENV_ADAPTER,
        &super::workspace_oci::OCI_IMAGE_ADAPTER,
    ]
}

pub(super) fn registered_environment_adapter_metadata(
) -> Vec<&'static WorkspaceEnvironmentAdapterMetadata> {
    let mut metadata = builtin_environment_adapters()
        .into_iter()
        .map(WorkspaceEnvironmentAdapter::metadata)
        .collect::<Vec<_>>();
    metadata.push(&super::workspace_recipe::COMMAND_RECIPE_ADAPTER_METADATA);
    metadata
}

fn builtin_environment_adapter_for_selector(
    selector: &str,
) -> Option<&'static dyn WorkspaceEnvironmentAdapter> {
    builtin_environment_adapters()
        .into_iter()
        .find(|adapter| adapter.accepts_selector(selector))
}

impl Trail {
    /// Return the adapters compiled into this Trail host.
    ///
    /// This is intentionally side-effect free: listing adapters never probes
    /// tools, reads repository files, or executes adapter code beyond static
    /// metadata access.
    pub fn workspace_environment_adapters(&self) -> Result<EnvironmentAdapterCatalogReport> {
        let mut adapters = registered_environment_adapter_metadata()
            .into_iter()
            .map(|metadata| EnvironmentAdapterCatalogEntryReport {
                identity: EnvironmentAdapterIdentityReport {
                    namespace: metadata.namespace.to_string(),
                    name: metadata.name.to_string(),
                    contract_major: metadata.contract_major,
                    implementation_version: metadata.implementation_version.to_string(),
                    distribution_digest: Some(metadata.distribution_digest.to_string()),
                },
                canonical_identity: metadata.canonical_identity.to_string(),
                selectors: metadata
                    .selectors
                    .iter()
                    .map(|selector| (*selector).to_string())
                    .collect(),
                kind: metadata.kind.to_string(),
                layer_adapter_name: metadata.layer_adapter_name.to_string(),
                discovery_markers: metadata
                    .discovery_markers
                    .iter()
                    .map(|marker| (*marker).to_string())
                    .collect(),
                protocols: Vec::new(),
                supported_operating_systems: metadata
                    .supported_operating_systems
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
                supported_architectures: metadata
                    .supported_architectures
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
                source: if metadata.canonical_identity
                    == super::workspace_recipe::COMMAND_RECIPE_ADAPTER_METADATA.canonical_identity
                {
                    "recipe"
                } else {
                    "builtin"
                }
                .to_string(),
                publisher: Some("trail".to_string()),
                publisher_key_id: None,
                trust: "builtin".to_string(),
                certification_tier: format!("builtin-{}", metadata.stability),
                stability: metadata.stability.to_string(),
                description: metadata.description.to_string(),
            })
            .collect::<Vec<_>>();
        let mut selectors = adapters
            .iter()
            .flat_map(|adapter| {
                adapter
                    .selectors
                    .iter()
                    .map(move |selector| (selector.clone(), adapter.canonical_identity.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        for plugin in self.installed_environment_plugins()? {
            let metadata = plugin.manifest.adapter;
            let (namespace, name, contract_major) =
                super::workspace_plugin::validate_plugin_identity(&metadata.canonical_identity)?;
            for selector in &metadata.selectors {
                if let Some(other) =
                    selectors.insert(selector.clone(), metadata.canonical_identity.clone())
                {
                    return Err(Error::Corrupt(format!(
                        "adapter selector `{selector}` is claimed by both `{other}` and `{}`",
                        metadata.canonical_identity
                    )));
                }
            }
            adapters.push(EnvironmentAdapterCatalogEntryReport {
                identity: EnvironmentAdapterIdentityReport {
                    namespace,
                    name,
                    contract_major,
                    implementation_version: metadata.implementation_version,
                    distribution_digest: Some(plugin.distribution_digest),
                },
                canonical_identity: metadata.canonical_identity,
                selectors: metadata.selectors,
                kind: metadata.kind,
                layer_adapter_name: metadata.layer_adapter_name,
                discovery_markers: metadata.discovery_markers,
                protocols: metadata.protocols,
                supported_operating_systems: metadata.supported_operating_systems,
                supported_architectures: metadata.supported_architectures,
                source: "plugin".to_string(),
                publisher: plugin.publisher,
                publisher_key_id: plugin.publisher_key_id,
                trust: plugin.trust,
                certification_tier: plugin.certification_tier,
                stability: metadata.stability,
                description: metadata.description,
            });
        }
        adapters.sort_by(|left, right| left.canonical_identity.cmp(&right.canonical_identity));
        Ok(EnvironmentAdapterCatalogReport {
            contract_major: 1,
            adapters,
        })
    }

    /// Recover synchronization records whose owning process no longer exists.
    ///
    /// Component rows retain their predecessor `attached_key` while building,
    /// so recovery can distinguish a stale-but-usable predecessor from a first
    /// build that produced no usable environment.
    pub(crate) fn recover_workspace_environment_sync_attempts(&self) -> Result<()> {
        let running = {
            let mut stmt = self.conn.prepare(
                "SELECT attempt_id, view_id, owner_pid, owner_start_token
                 FROM environment_sync_attempts WHERE status = 'running'
                 ORDER BY started_at, attempt_id",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        for (attempt_id, view_id, owner_pid, owner_start_token) in running {
            if u32::try_from(owner_pid)
                .ok()
                .is_some_and(|pid| process_matches_start_token(pid, &owner_start_token))
            {
                continue;
            }
            let building_count = self.conn.query_row(
                "SELECT COUNT(*) FROM environment_component_states
                 WHERE view_id = ?1 AND status = 'building'",
                params![view_id],
                |row| row.get::<_, i64>(0),
            )?;
            let recovered_status = if building_count == 0 {
                "succeeded"
            } else {
                "abandoned"
            };
            let reason = if building_count == 0 {
                "synchronization owner exited after component activation; recovered as complete"
            } else {
                "synchronization owner exited before activation; the predecessor environment remains authoritative"
            };
            self.conn
                .execute_batch("SAVEPOINT trail_environment_attempt_recovery")?;
            let recovery = (|| -> Result<()> {
                if building_count > 0 {
                    self.conn.execute(
                        "UPDATE environment_component_states
                         SET status = CASE WHEN attached_key IS NULL THEN 'failed' ELSE 'stale' END,
                             reason = ?1,
                             updated_at = ?2
                         WHERE view_id = ?3 AND status = 'building'",
                        params![reason, now_ts(), view_id],
                    )?;
                    self.conn.execute(
                        "UPDATE workspace_environment_states
                         SET status = CASE WHEN attached_key IS NULL THEN 'failed' ELSE 'stale' END,
                             reason = ?1,
                             updated_at = ?2
                         WHERE view_id = ?3 AND status = 'building'",
                        params![reason, now_ts(), view_id],
                    )?;
                }
                self.conn.execute(
                    "UPDATE environment_sync_attempts
                     SET status = ?1, reason = ?2, updated_at = ?3, finished_at = ?3
                     WHERE attempt_id = ?4 AND status = 'running'",
                    params![recovered_status, reason, now_ts(), attempt_id],
                )?;
                Ok(())
            })();
            match recovery {
                Ok(()) => self
                    .conn
                    .execute_batch("RELEASE SAVEPOINT trail_environment_attempt_recovery")?,
                Err(err) => {
                    let _ = self.conn.execute_batch(
                        "ROLLBACK TO SAVEPOINT trail_environment_attempt_recovery;
                         RELEASE SAVEPOINT trail_environment_attempt_recovery",
                    );
                    return Err(err);
                }
            }
            self.cleanup_mounted_environment_candidates(&attempt_id);
        }
        Ok(())
    }

    fn cleanup_mounted_environment_candidates(&self, attempt_id: &str) {
        let staging = self.db_dir.join("cache/staging");
        let prefix = format!("mounted-environment-{attempt_id}-");
        let Ok(entries) = fs::read_dir(staging) else {
            return;
        };
        for entry in entries.filter_map(std::result::Result::ok) {
            if !entry.file_name().to_string_lossy().starts_with(&prefix) {
                continue;
            }
            let path = entry.path();
            make_tree_writable(&path);
            let _ = fs::remove_dir_all(path);
        }
    }

    fn begin_workspace_environment_sync_attempt(
        &self,
        view_id: &str,
        source_root: &ObjectId,
        mode: &str,
    ) -> Result<String> {
        self.recover_workspace_environment_sync_attempts()?;
        if let Some((attempt_id, owner_pid)) = self
            .conn
            .query_row(
                "SELECT attempt_id, owner_pid FROM environment_sync_attempts
                 WHERE view_id = ?1 AND status = 'running'",
                params![view_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?
        {
            return Err(Error::InvalidInput(format!(
                "workspace view `{view_id}` is already synchronizing in attempt `{attempt_id}` owned by process {owner_pid}"
            )));
        }
        let owner_start_token = current_process_start_token();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let digest = sha256_hex(
            format!(
                "{view_id}:{}:{}:{owner_start_token}:{nonce}:{mode}",
                source_root.0,
                std::process::id()
            )
            .as_bytes(),
        );
        let attempt_id = format!("envsync_{}", &digest[..24]);
        self.conn.execute(
            "INSERT INTO environment_sync_attempts
             (attempt_id, view_id, source_root, mode, owner_pid, owner_start_token, status, reason, started_at, updated_at, finished_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'running', NULL, ?7, ?7, NULL)",
            params![
                attempt_id,
                view_id,
                source_root.0,
                mode,
                i64::from(std::process::id()),
                owner_start_token,
                now_ts()
            ],
        )?;
        Ok(attempt_id)
    }

    fn finish_workspace_environment_sync_attempt(
        &self,
        attempt_id: &str,
        status: &str,
        reason: Option<&str>,
    ) -> Result<()> {
        if !matches!(status, "succeeded" | "failed") {
            return Err(Error::InvalidInput(format!(
                "invalid environment synchronization attempt status `{status}`"
            )));
        }
        let updated = self.conn.execute(
            "UPDATE environment_sync_attempts
             SET status = ?1, reason = ?2, updated_at = ?3, finished_at = ?3
             WHERE attempt_id = ?4 AND status = 'running'",
            params![status, reason, now_ts(), attempt_id],
        )?;
        if updated != 1 {
            return Err(Error::Corrupt(format!(
                "environment synchronization attempt `{attempt_id}` was not running"
            )));
        }
        Ok(())
    }

    /// Discover built-in environment components without invoking package
    /// managers, compilers, network providers, or repository code.
    pub fn discover_workspace_environment(
        &self,
        lane: &str,
        component_root: Option<&str>,
    ) -> Result<EnvironmentDiscoveryReport> {
        let branch = self.lane_branch(lane)?;
        let head = self.get_ref(&branch.ref_name)?;
        let mut roots = BTreeSet::new();
        let plugins = self.installed_environment_plugins()?;
        if !plugins.is_empty() {
            // This also rejects selectors that collide with built-ins before
            // any external code is invoked.
            let _ = self.workspace_environment_adapters()?;
        }
        let mut discovery_markers = builtin_environment_adapters()
            .into_iter()
            .flat_map(|adapter| adapter.metadata().discovery_markers.iter().copied())
            .map(str::to_string)
            .collect::<BTreeSet<_>>();
        discovery_markers.extend(
            plugins
                .iter()
                .flat_map(|plugin| plugin.manifest.adapter.discovery_markers.iter().cloned()),
        );
        if let Some(component_root) = component_root {
            roots.insert(if component_root.trim_matches('/').is_empty() {
                String::new()
            } else {
                normalize_relative_path(component_root)?
            });
        } else {
            self.for_each_root_file_chunk(&head.root_id, 1024, |chunk| {
                for (path, _) in chunk {
                    let file_name = path.rsplit('/').next().unwrap_or(path.as_str());
                    if !discovery_markers.contains(file_name) {
                        continue;
                    }
                    roots.insert(
                        path.rsplit_once('/')
                            .map(|(parent, _)| parent.to_string())
                            .unwrap_or_default(),
                    );
                }
                Ok(())
            })?;
        }

        let mut components = Vec::new();
        let mut conflicts = Vec::new();
        for root in roots {
            for adapter in builtin_environment_adapters() {
                if adapter.detect(self, &head.root_id, &root)? {
                    components.push(EnvironmentDiscoveredComponentReport {
                        component_id: adapter.component_id(&root)?,
                        component_root: root.clone(),
                        kind: adapter.kind().to_string(),
                        adapter_identity: adapter.identity().to_string(),
                    });
                }
            }
            for plugin in &plugins {
                if !super::workspace_plugin::environment_plugin_supports_current_host(plugin) {
                    continue;
                }
                if let Some(component) =
                    self.discover_environment_plugin_component(plugin, &head.root_id, &root)?
                {
                    components.push(component);
                }
            }
        }
        components.extend(self.command_recipe_discovery(&head.root_id, component_root)?);
        components.sort_by(|left, right| {
            (
                &left.component_root,
                &left.component_id,
                &left.adapter_identity,
            )
                .cmp(&(
                    &right.component_root,
                    &right.component_id,
                    &right.adapter_identity,
                ))
        });
        for duplicate in components.windows(2) {
            if duplicate[0].component_id == duplicate[1].component_id {
                conflicts.push(EnvironmentDiscoveryConflictReport {
                    component_root: duplicate[0].component_root.clone(),
                    adapter_identities: vec![
                        duplicate[0].adapter_identity.clone(),
                        duplicate[1].adapter_identity.clone(),
                    ],
                    reason: format!(
                        "multiple adapters proposed logical component `{}`",
                        duplicate[0].component_id
                    ),
                });
            }
        }
        Ok(EnvironmentDiscoveryReport {
            source_root: head.root_id,
            components,
            conflicts,
        })
    }

    /// Return the complete desired ordering/invalidation graph without
    /// executing adapter commands or mutating lane state.
    pub fn workspace_environment_graph(
        &self,
        lane: &str,
        discovery_root: Option<&str>,
    ) -> Result<EnvironmentGraphReport> {
        let discovery = self.discover_workspace_environment(lane, discovery_root)?;
        if !discovery.conflicts.is_empty() {
            return Err(Error::InvalidInput(format!(
                "environment discovery found {} unresolved component identity conflict(s); inspect `trail env discover {lane}`",
                discovery.conflicts.len()
            )));
        }
        let roots = discovery
            .components
            .iter()
            .map(|component| {
                (
                    component.component_id.clone(),
                    component.component_root.clone(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let finalized =
            self.plan_discovered_environment_graph(&discovery.source_root, &discovery.components)?;
        let keys = finalized
            .iter()
            .map(|(plan, component_key)| (plan.component_id.clone(), component_key.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut nodes = Vec::with_capacity(finalized.len());
        let mut edges = Vec::new();
        for (topological_index, (plan, component_key)) in finalized.into_iter().enumerate() {
            let external_artifacts = environment_external_artifact_activations(&plan);
            let runtime_resources = environment_runtime_resource_activations(&plan);
            for dependency in &plan.dependencies {
                edges.push(EnvironmentGraphEdgeReport {
                    source_component_id: dependency.component_id.clone(),
                    source_component_key: keys.get(&dependency.component_id).cloned().ok_or_else(
                        || {
                            Error::Corrupt(format!(
                                "finalized environment graph lost key for dependency `{}`",
                                dependency.component_id
                            ))
                        },
                    )?,
                    target_component_id: plan.component_id.clone(),
                    edge_type: dependency.edge_type.as_str().to_string(),
                });
            }
            nodes.push(EnvironmentGraphNodeReport {
                topological_index: topological_index as u64,
                component_root: roots.get(&plan.component_id).cloned().ok_or_else(|| {
                    Error::Corrupt(format!(
                        "finalized environment graph lost root for component `{}`",
                        plan.component_id
                    ))
                })?,
                component_id: plan.component_id,
                kind: plan.kind,
                adapter_identity: plan.adapter_identity,
                component_key,
                dependencies: plan
                    .dependencies
                    .into_iter()
                    .map(|dependency| dependency.component_id)
                    .collect(),
                caches: plan.caches.iter().map(environment_cache_report).collect(),
                external_artifacts,
                runtime_resources,
                outputs: plan
                    .outputs
                    .into_iter()
                    .map(|output| EnvironmentPlanOutputReport {
                        name: output.name,
                        output_path: output.output_path,
                        mount_path: output.mount_path,
                        policy: output.policy.as_str().to_string(),
                    })
                    .collect(),
            });
        }
        Ok(EnvironmentGraphReport {
            source_root: discovery.source_root,
            total_nodes: nodes.len() as u64,
            total_edges: edges.len() as u64,
            offset: 0,
            next_offset: None,
            nodes,
            edges,
        })
    }

    pub fn workspace_environment_graph_page(
        &self,
        lane: &str,
        discovery_root: Option<&str>,
        offset: u64,
        limit: u64,
    ) -> Result<EnvironmentGraphReport> {
        if limit == 0 || limit > 1_000 {
            return Err(Error::InvalidInput(
                "environment graph limit must be between 1 and 1000".to_string(),
            ));
        }
        let graph = self.workspace_environment_graph(lane, discovery_root)?;
        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(graph.nodes.len());
        let end = start.saturating_add(limit as usize).min(graph.nodes.len());
        let target_ids = graph.nodes[start..end]
            .iter()
            .map(|node| node.component_id.clone())
            .collect::<BTreeSet<_>>();
        let next_offset = (end < graph.nodes.len()).then_some(end as u64);
        Ok(EnvironmentGraphReport {
            source_root: graph.source_root,
            total_nodes: graph.total_nodes,
            total_edges: graph.total_edges,
            offset,
            next_offset,
            nodes: graph.nodes[start..end].to_vec(),
            edges: graph
                .edges
                .into_iter()
                .filter(|edge| target_ids.contains(&edge.target_component_id))
                .collect(),
        })
    }

    /// Resolve and normalize one component without executing its commands or
    /// mutating environment state.
    pub fn plan_workspace_environment(
        &self,
        lane: &str,
        adapter_selector: &str,
        component_root: Option<&str>,
    ) -> Result<EnvironmentPlanReport> {
        self.plan_workspace_environment_component(lane, adapter_selector, component_root, None)
    }

    pub fn plan_workspace_environment_component(
        &self,
        lane: &str,
        adapter_selector: &str,
        component_root: Option<&str>,
        component_id: Option<&str>,
    ) -> Result<EnvironmentPlanReport> {
        let (source_root, plan) = self.resolve_workspace_environment_plan(
            lane,
            adapter_selector,
            component_root.unwrap_or(""),
            component_id,
        )?;
        let plan = self.finalize_workspace_environment_plan_preview(lane, &source_root, plan)?;
        let component_key = self.workspace_layer_cache_key(&plan.layer_key)?;
        let output_path = plan
            .outputs
            .first()
            .map(|output| output.output_path.clone())
            .unwrap_or_default();
        let mount_path = plan
            .outputs
            .first()
            .map(|output| output.mount_path.clone())
            .unwrap_or_default();
        let commands = plan
            .pre_commands
            .iter()
            .chain(plan.command.iter())
            .map(|command| EnvironmentPlanCommandReport {
                phase: "staging".to_string(),
                program: command.program.clone(),
                resolved_program: command.resolved_program.to_string_lossy().into_owned(),
                executable_identity: command.executable_identity.clone(),
                args: command.args.clone(),
                working_directory: command.working_directory.clone(),
                environment_names: command.environment.keys().cloned().collect(),
            })
            .chain(
                plan.mounted_commands
                    .iter()
                    .map(|command| EnvironmentPlanCommandReport {
                        phase: "mounted_initialization".to_string(),
                        program: command.program.clone(),
                        resolved_program: command.resolved_program.to_string_lossy().into_owned(),
                        executable_identity: command.executable_identity.clone(),
                        args: command.args.clone(),
                        working_directory: command.working_directory.clone(),
                        environment_names: command.environment.keys().cloned().collect(),
                    }),
            )
            .collect::<Vec<_>>();
        let process = commands
            .iter()
            .map(|command| command.resolved_program.clone())
            .collect::<Vec<_>>();
        let has_runtime_secrets = plan
            .runtime_resources
            .iter()
            .any(|resource| !resource.secrets.is_empty());
        let capabilities = if !plan.external_artifacts.is_empty()
            || !plan.runtime_resources.is_empty()
        {
            EnvironmentCapabilityReport {
                filesystem_read: Vec::new(),
                filesystem_write: Vec::new(),
                process: Vec::new(),
                network: "none".to_string(),
                shell: "none".to_string(),
                scripts: "none".to_string(),
                secrets: if has_runtime_secrets {
                    "opaque-references-only; host-runtime-file-handle-resolution".to_string()
                } else {
                    "none".to_string()
                },
                sandbox: "not-applicable-metadata-only".to_string(),
            }
        } else {
            match plan.sandbox_policy {
                WorkspaceEnvironmentSandboxPolicy::RestrictedRecipe
                | WorkspaceEnvironmentSandboxPolicy::RestrictedPluginStaging
                | WorkspaceEnvironmentSandboxPolicy::RestrictedPluginMounted => {
                    EnvironmentCapabilityReport {
                        filesystem_read: plan
                            .inputs
                            .iter()
                            .map(|input| input.source_path.clone())
                            .collect(),
                        filesystem_write: plan
                            .outputs
                            .iter()
                            .map(|output| {
                                if plan.sandbox_policy
                                    == WorkspaceEnvironmentSandboxPolicy::RestrictedPluginMounted
                                {
                                    output.mount_path.clone()
                                } else {
                                    output.output_path.clone()
                                }
                            })
                            .chain(
                                plan.caches
                                    .iter()
                                    .map(|cache| cache.storage_path.to_string_lossy().into_owned()),
                            )
                            .collect(),
                        process,
                        network: "deny".to_string(),
                        shell: "deny".to_string(),
                        scripts: "deny".to_string(),
                        secrets: "deny".to_string(),
                        sandbox: restricted_recipe_sandbox_name().to_string(),
                    }
                }
                WorkspaceEnvironmentSandboxPolicy::TrustedBuiltin => EnvironmentCapabilityReport {
                    filesystem_read: if plan.source_projection.is_some() {
                        vec!["pinned-source-root/**".to_string()]
                    } else {
                        plan.inputs
                            .iter()
                            .map(|input| input.source_path.clone())
                            .collect()
                    },
                    filesystem_write: plan
                        .outputs
                        .iter()
                        .map(|output| output.output_path.clone())
                        .chain(
                            plan.caches
                                .iter()
                                .map(|cache| cache.storage_path.to_string_lossy().into_owned()),
                        )
                        .collect(),
                    process,
                    network: "adapter-managed-open-world".to_string(),
                    shell: "argv-direct; child-process policy adapter-managed".to_string(),
                    scripts: "adapter-managed".to_string(),
                    secrets: "none-declared".to_string(),
                    sandbox: "trusted-builtin".to_string(),
                },
            }
        };
        let tools = plan.layer_key.tool_versions.clone();
        let external_artifacts = environment_external_artifact_activations(&plan);
        let runtime_resources = environment_runtime_resource_activations(&plan);
        Ok(EnvironmentPlanReport {
            source_root,
            component_id: plan.component_id,
            adapter_identity: plan.adapter_identity,
            kind: plan.kind,
            component_key,
            dependencies: plan
                .dependencies
                .iter()
                .map(|dependency| dependency.component_id.clone())
                .collect(),
            dependency_edges: plan
                .resolved_dependencies
                .iter()
                .map(|dependency| EnvironmentGenerationDependencyReport {
                    component_id: dependency.component_id.clone(),
                    component_key: dependency.component_key.clone(),
                    edge_type: dependency.edge_type.as_str().to_string(),
                })
                .collect(),
            caches: plan.caches.iter().map(environment_cache_report).collect(),
            external_artifacts,
            runtime_resources,
            inputs: plan
                .inputs
                .into_iter()
                .map(|input| EnvironmentPlanInputReport {
                    source_path: input.source_path,
                    staging_path: input.staging_path,
                    content_hash: input.entry.content_hash,
                    size_bytes: input.entry.size_bytes,
                })
                .collect(),
            tools,
            commands,
            outputs: plan
                .outputs
                .iter()
                .map(|output| EnvironmentPlanOutputReport {
                    name: output.name.clone(),
                    output_path: output.output_path.clone(),
                    mount_path: output.mount_path.clone(),
                    policy: output.policy.as_str().to_string(),
                })
                .collect(),
            output_path,
            mount_path,
            portability_scope: plan.layer_key.portability_scope,
            capabilities,
        })
    }

    fn resolve_workspace_environment_plan(
        &self,
        lane: &str,
        adapter_selector: &str,
        component_root: &str,
        component_id: Option<&str>,
    ) -> Result<(ObjectId, WorkspaceEnvironmentPlan)> {
        let branch = self.lane_branch(lane)?;
        let head = self.get_ref(&branch.ref_name)?;
        let command_metadata = &super::workspace_recipe::COMMAND_RECIPE_ADAPTER_METADATA;
        let (plan, builtin_adapter, plugin_adapter, expected_component_id) = if adapter_selector
            == "auto"
        {
            let discovery = self.discover_workspace_environment(lane, Some(component_root))?;
            if !discovery.conflicts.is_empty() {
                return Err(Error::InvalidInput(format!(
                    "environment discovery found {} unresolved component conflict(s) at `{}`",
                    discovery.conflicts.len(),
                    if component_root.is_empty() {
                        "."
                    } else {
                        component_root
                    }
                )));
            }
            let candidates = discovery
                .components
                .iter()
                .filter(|component| component_id.is_none_or(|id| component.component_id == id))
                .collect::<Vec<_>>();
            match candidates.as_slice() {
                [component] if component.adapter_identity == command_metadata.canonical_identity => {
                    (
                        self.command_recipe_plan(&head.root_id, &component.component_id)?,
                        None,
                        None,
                        component.component_id.clone(),
                    )
                }
                [component] => {
                    if let Some(adapter) =
                        builtin_environment_adapter_for_selector(&component.adapter_identity)
                    {
                        (
                            adapter.plan(self, &head.root_id, &component.component_root)?,
                            Some(adapter),
                            None,
                            component.component_id.clone(),
                        )
                    } else {
                        let plugin = self
                            .environment_plugin_for_selector(&component.adapter_identity)?
                            .ok_or_else(|| {
                                Error::Corrupt(format!(
                                    "discovered adapter `{}` is no longer installed",
                                    component.adapter_identity
                                ))
                            })?;
                        let plan = self.plan_environment_plugin_component(
                            &plugin,
                            &head.root_id,
                            &component.component_root,
                            &component.component_id,
                        )?;
                        (
                            plan,
                            None,
                            Some(plugin),
                            component.component_id.clone(),
                        )
                    }
                }
                [] => {
                    return Err(Error::InvalidInput(format!(
                        "no workspace environment adapter detected at `{}`; specify --adapter explicitly",
                        if component_root.is_empty() { "." } else { component_root }
                    )))
                }
                components => {
                    return Err(Error::InvalidInput(format!(
                        "multiple workspace environment adapters or components detected at `{}`: {}; specify --adapter explicitly",
                        if component_root.is_empty() { "." } else { component_root },
                        components
                            .iter()
                            .map(|component| component.adapter_identity.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )))
                }
            }
        } else if command_metadata.selectors.contains(&adapter_selector) {
            let plan = if let Some(component_id) = component_id {
                self.command_recipe_plan(&head.root_id, component_id)?
            } else {
                self.command_recipe_plan_for_root(&head.root_id, component_root)?
            };
            let expected = plan.component_id.clone();
            (plan, None, None, expected)
        } else {
            if let Some(adapter) = builtin_environment_adapter_for_selector(adapter_selector) {
                let expected = adapter.component_id(component_root)?;
                if component_id.is_some_and(|component_id| component_id != expected) {
                    return Err(Error::InvalidInput(format!(
                        "adapter `{}` proposes component `{expected}`, not requested component `{}`",
                        adapter.identity(),
                        component_id.unwrap_or_default()
                    )));
                }
                (
                    adapter.plan(self, &head.root_id, component_root)?,
                    Some(adapter),
                    None,
                    expected,
                )
            } else if let Some(plugin) = self.environment_plugin_for_selector(adapter_selector)? {
                let discovered = self
                    .discover_environment_plugin_component(&plugin, &head.root_id, component_root)?
                    .ok_or_else(|| {
                        Error::InvalidInput(format!(
                            "adapter `{}` did not detect a component at `{}`",
                            plugin.manifest.adapter.canonical_identity,
                            if component_root.is_empty() {
                                "."
                            } else {
                                component_root
                            }
                        ))
                    })?;
                if component_id.is_some_and(|component_id| component_id != discovered.component_id)
                {
                    return Err(Error::InvalidInput(format!(
                        "adapter `{}` proposes component `{}`, not requested component `{}`",
                        plugin.manifest.adapter.canonical_identity,
                        discovered.component_id,
                        component_id.unwrap_or_default()
                    )));
                }
                let expected = discovered.component_id;
                let plan = self.plan_environment_plugin_component(
                    &plugin,
                    &head.root_id,
                    component_root,
                    &expected,
                )?;
                (plan, None, Some(plugin), expected)
            } else {
                let available = self
                    .workspace_environment_adapters()?
                    .adapters
                    .into_iter()
                    .map(|adapter| adapter.canonical_identity)
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(Error::InvalidInput(format!(
                    "unknown workspace environment adapter `{adapter_selector}`; available adapters: {available}"
                )));
            }
        };
        if let Some(adapter) = builtin_adapter {
            self.validate_workspace_environment_plan(adapter, component_root, &plan)?;
        } else if let Some(plugin) = &plugin_adapter {
            self.validate_environment_plugin_plan(plugin, &expected_component_id, &plan)?;
        } else {
            self.validate_command_recipe_plan(&expected_component_id, &plan)?;
        }
        self.validate_environment_mounts_do_not_shadow_source(&head.root_id, &[&plan])?;
        Ok((head.root_id, plan))
    }

    /// Finalize only the dependency closure needed by a read-only plan. This
    /// makes first-use planning useful without requiring attached state and
    /// avoids invoking unrelated adapters elsewhere in a large monorepo.
    fn finalize_workspace_environment_plan_preview(
        &self,
        lane: &str,
        source_root: &ObjectId,
        selected: WorkspaceEnvironmentPlan,
    ) -> Result<WorkspaceEnvironmentPlan> {
        if selected.dependencies.is_empty() {
            return Ok(selected);
        }
        let selected_id = selected.component_id.clone();
        let discovery = self.discover_workspace_environment(lane, None)?;
        let mut discovered = BTreeMap::new();
        for component in &discovery.components {
            if discovered
                .insert(component.component_id.clone(), component)
                .is_some()
            {
                return Err(Error::InvalidInput(format!(
                    "environment dependency closure has ambiguous component `{}`",
                    component.component_id
                )));
            }
        }
        let mut closure = BTreeMap::from([(selected_id.clone(), selected)]);
        let mut pending = closure[&selected_id]
            .dependencies
            .iter()
            .map(|dependency| dependency.component_id.clone())
            .collect::<BTreeSet<_>>();
        while let Some(component_id) = pending.pop_first() {
            if closure.contains_key(&component_id) {
                continue;
            }
            let component = discovered.get(&component_id).ok_or_else(|| {
                Error::InvalidInput(format!(
                    "environment component `{selected_id}` requires missing component `{component_id}`"
                ))
            })?;
            let plan = self.plan_discovered_environment_component(source_root, component)?;
            pending.extend(
                plan.dependencies
                    .iter()
                    .map(|dependency| dependency.component_id.clone()),
            );
            closure.insert(component_id, plan);
        }
        let plans = closure.into_values().collect::<Vec<_>>();
        self.validate_environment_plan_mount_collisions(&plans)?;
        self.validate_environment_mounts_do_not_shadow_source(
            source_root,
            &plans.iter().collect::<Vec<_>>(),
        )?;
        self.finalize_workspace_environment_plan_graph(plans)?
            .into_iter()
            .find_map(|(plan, _)| (plan.component_id == selected_id).then_some(plan))
            .ok_or_else(|| {
                Error::Corrupt(format!(
                    "environment dependency closure lost selected component `{selected_id}`"
                ))
            })
    }

    /// Resolve one component against the dependency keys that are actually
    /// active in this lane. A single-component sync must never silently build
    /// against a missing or stale predecessor; callers can use `sync-all` to
    /// construct and activate the complete graph atomically.
    fn finalize_single_workspace_environment_plan(
        &self,
        lane: &str,
        mut plan: WorkspaceEnvironmentPlan,
    ) -> Result<WorkspaceEnvironmentPlan> {
        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered workspace view"
            ))
        })?;
        normalize_workspace_environment_dependencies(&plan.component_id, &mut plan.dependencies)?;
        plan.resolved_dependencies.clear();
        for dependency in &plan.dependencies {
            let state = self
                .conn
                .query_row(
                    "SELECT attached_key, status FROM environment_component_states
                     WHERE view_id = ?1 AND component_id = ?2",
                    params![&view.view_id, &dependency.component_id],
                    |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()?;
            let Some((Some(component_key), status)) = state else {
                return Err(Error::InvalidInput(format!(
                    "environment component `{}` requires `{}`, which is not attached; run `trail env sync-all {lane}`",
                    plan.component_id, dependency.component_id
                )));
            };
            if status != "ready" {
                return Err(Error::InvalidInput(format!(
                    "environment component `{}` requires `{}`, which is `{status}`; run `trail env sync-all {lane}`",
                    plan.component_id, dependency.component_id
                )));
            }
            if let Some(input_name) = dependency
                .edge_type
                .identity_input_name(&dependency.component_id)
            {
                plan.layer_key
                    .inputs
                    .insert(input_name, component_key.clone());
            }
            plan.resolved_dependencies
                .push(ResolvedWorkspaceEnvironmentDependency {
                    component_id: dependency.component_id.clone(),
                    component_key,
                    edge_type: dependency.edge_type,
                });
        }
        Ok(plan)
    }

    /// Validate and finalize a complete component DAG. The result is stable
    /// topological order; every downstream canonical key includes the exact
    /// finalized key of each direct dependency.
    fn finalize_workspace_environment_plan_graph(
        &self,
        plans: Vec<WorkspaceEnvironmentPlan>,
    ) -> Result<Vec<(WorkspaceEnvironmentPlan, String)>> {
        let mut by_id = BTreeMap::new();
        for mut plan in plans {
            normalize_workspace_environment_dependencies(
                &plan.component_id,
                &mut plan.dependencies,
            )?;
            plan.resolved_dependencies.clear();
            if by_id.insert(plan.component_id.clone(), plan).is_some() {
                return Err(Error::InvalidInput(
                    "environment graph contains a duplicate component identity".to_string(),
                ));
            }
        }
        for plan in by_id.values() {
            for dependency in &plan.dependencies {
                if !by_id.contains_key(&dependency.component_id) {
                    return Err(Error::InvalidInput(format!(
                        "environment component `{}` requires missing component `{}`",
                        plan.component_id, dependency.component_id
                    )));
                }
            }
        }

        let mut indegree = by_id
            .iter()
            .map(|(component_id, plan)| (component_id.clone(), plan.dependencies.len()))
            .collect::<BTreeMap<_, _>>();
        let mut dependents = BTreeMap::<String, BTreeSet<String>>::new();
        for plan in by_id.values() {
            for dependency in &plan.dependencies {
                dependents
                    .entry(dependency.component_id.clone())
                    .or_default()
                    .insert(plan.component_id.clone());
            }
        }
        let mut ready = indegree
            .iter()
            .filter_map(|(component_id, count)| (*count == 0).then_some(component_id.clone()))
            .collect::<BTreeSet<_>>();
        let mut finalized_keys = BTreeMap::<String, String>::new();
        let mut ordered = Vec::with_capacity(by_id.len());
        while let Some(component_id) = ready.pop_first() {
            let mut plan = by_id.remove(&component_id).ok_or_else(|| {
                Error::Corrupt(format!(
                    "environment graph lost ready component `{component_id}`"
                ))
            })?;
            for dependency in &plan.dependencies {
                let dependency_key =
                    finalized_keys
                        .get(&dependency.component_id)
                        .ok_or_else(|| {
                            Error::Corrupt(format!(
                                "environment graph ordered `{component_id}` before `{}`",
                                dependency.component_id
                            ))
                        })?;
                if let Some(input_name) = dependency
                    .edge_type
                    .identity_input_name(&dependency.component_id)
                {
                    plan.layer_key
                        .inputs
                        .insert(input_name, dependency_key.clone());
                }
                plan.resolved_dependencies
                    .push(ResolvedWorkspaceEnvironmentDependency {
                        component_id: dependency.component_id.clone(),
                        component_key: dependency_key.clone(),
                        edge_type: dependency.edge_type,
                    });
            }
            let key = self.workspace_layer_cache_key(&plan.layer_key)?;
            finalized_keys.insert(component_id.clone(), key.clone());
            ordered.push((plan, key));
            if let Some(children) = dependents.get(&component_id) {
                for child in children {
                    let count = indegree.get_mut(child).ok_or_else(|| {
                        Error::Corrupt(format!("environment graph lost indegree for `{child}`"))
                    })?;
                    *count = count.checked_sub(1).ok_or_else(|| {
                        Error::Corrupt(format!(
                            "environment graph indegree underflow for `{child}`"
                        ))
                    })?;
                    if *count == 0 {
                        ready.insert(child.clone());
                    }
                }
            }
        }
        if !by_id.is_empty() {
            let cycle = environment_dependency_cycle(&by_id)?;
            return Err(Error::InvalidInput(format!(
                "environment component dependency cycle: {}",
                cycle.join(" -> ")
            )));
        }
        Ok(ordered)
    }

    /// Synchronize one adapter component through the host-owned staging and
    /// immutable-layer lifecycle. The legacy Node wrapper delegates here.
    pub fn sync_workspace_environment(
        &self,
        lane: &str,
        adapter_selector: &str,
        component_root: Option<&str>,
    ) -> Result<WorkspaceLayerReport> {
        let (_, plan) = self.resolve_workspace_environment_plan(
            lane,
            adapter_selector,
            component_root.unwrap_or(""),
            None,
        )?;
        if plan.outputs.is_empty() {
            return Err(Error::InvalidInput(format!(
                "adapter `{adapter_selector}` has no filesystem layer output; call the generation-oriented environment sync API instead"
            )));
        }
        if plan
            .outputs
            .iter()
            .any(|output| output.policy != WorkspaceEnvironmentOutputPolicy::ImmutableSeedPrivate)
        {
            return Err(Error::InvalidInput(format!(
                "adapter `{adapter_selector}` uses writable-private outputs; call the generation-oriented environment sync API instead of the legacy layer API"
            )));
        }
        let mut report = self.sync_workspace_environment_component(
            lane,
            adapter_selector,
            component_root,
            None,
        )?;
        if report.layers.len() != 1 {
            return Err(Error::Corrupt(format!(
                "legacy immutable-layer synchronization expected one layer but generation `{}` contains {}",
                report.generation.generation_id,
                report.layers.len()
            )));
        }
        Ok(report.layers.remove(0))
    }

    pub fn sync_workspace_environment_component(
        &self,
        lane: &str,
        adapter_selector: &str,
        component_root: Option<&str>,
        component_id: Option<&str>,
    ) -> Result<EnvironmentSyncReport> {
        let component_root = component_root.unwrap_or("");
        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered workspace view"
            ))
        })?;
        if let (Some(pid), Some(token)) = (view.owner_pid, view.owner_start_token.as_deref()) {
            if process_matches_start_token(pid, token) {
                return Err(Error::InvalidInput(format!(
                    "lane `{lane}` has an active workspace writer in process {pid}; run `trail lane unmount {lane}` before synchronizing its environment"
                )));
            }
        }

        let (source_root, plan) = self.resolve_workspace_environment_plan(
            lane,
            adapter_selector,
            component_root,
            component_id,
        )?;
        let plan = self.finalize_single_workspace_environment_plan(lane, plan)?;
        let cache_key = self.workspace_layer_cache_key(&plan.layer_key)?;
        let predecessor_key = self
            .workspace_environment_rows(lane)?
            .into_iter()
            .find(|state| state.adapter == plan.component_id)
            .and_then(|state| state.attached_key);
        let attempt_id =
            self.begin_workspace_environment_sync_attempt(&view.view_id, &source_root, "single")?;
        let result = (|| -> Result<EnvironmentSyncReport> {
            self.set_workspace_environment_state(
                lane,
                &plan,
                &cache_key,
                predecessor_key.as_deref(),
                "building",
                None,
            )?;
            let prepared =
                self.prepare_workspace_environment_artifacts(&view.view_id, &plan, &cache_key);
            let mut prepared = match prepared {
                Ok(prepared) => prepared,
                Err(err) => {
                    self.set_workspace_environment_state(
                        lane,
                        &plan,
                        &cache_key,
                        predecessor_key.as_deref(),
                        "failed",
                        Some(&err.to_string()),
                    )?;
                    return Err(err);
                }
            };
            if let Err(err) = self.initialize_mounted_workspace_environment_plans(
                lane,
                &attempt_id,
                &source_root,
                &[(&plan, cache_key.as_str())],
                std::slice::from_mut(&mut prepared),
                &[],
            ) {
                self.set_workspace_environment_state(
                    lane,
                    &plan,
                    &cache_key,
                    predecessor_key.as_deref(),
                    "failed",
                    Some(&err.to_string()),
                )?;
                return Err(err);
            }
            let (layer_id, private_paths) = match &prepared {
                PreparedEnvironmentArtifacts::Immutable(layer) => {
                    (Some(layer.layer_id.as_str()), None)
                }
                PreparedEnvironmentArtifacts::WritablePrivate(Some(outputs)) => {
                    (None, Some(outputs.paths.as_slice()))
                }
                PreparedEnvironmentArtifacts::WritablePrivate(None) => (None, None),
                PreparedEnvironmentArtifacts::MetadataOnly => (None, None),
            };
            let activation = EnvironmentLayerActivation {
                layer_id: layer_id.map(str::to_string),
                outputs: environment_output_activations(
                    &plan,
                    layer_id,
                    &cache_key,
                    private_paths,
                )?,
                component_id: plan.component_id.clone(),
                adapter_identity: plan.adapter_identity.clone(),
                adapter_version: plan.adapter_version,
                implementation_version: plan.implementation_version.clone(),
                distribution_digest: plan.distribution_digest.clone(),
                kind: plan.kind.clone(),
                dependencies: environment_dependency_activations(&plan)?,
                caches: environment_cache_activations(&plan),
                external_artifacts: environment_external_artifact_activations(&plan),
                runtime_resources: environment_runtime_resource_activations(&plan),
                expected_key: cache_key.clone(),
                canonical_key: plan.layer_key.clone(),
            };
            self.ensure_workspace_environment_source_root(lane, &source_root)?;
            if let Err(err) =
                self.replace_declared_workspace_layers_at_source(lane, &[activation], &source_root)
            {
                self.set_workspace_environment_state(
                    lane,
                    &plan,
                    &cache_key,
                    predecessor_key.as_deref(),
                    "failed",
                    Some(&err.to_string()),
                )?;
                return Err(err);
            }
            let generation = self.active_environment_generation(lane)?.ok_or_else(|| {
                Error::Corrupt("environment activation committed without a generation".to_string())
            })?;
            let layers = match prepared {
                PreparedEnvironmentArtifacts::Immutable(layer) => vec![layer],
                PreparedEnvironmentArtifacts::WritablePrivate(_) => Vec::new(),
                PreparedEnvironmentArtifacts::MetadataOnly => Vec::new(),
            };
            Ok(EnvironmentSyncReport { generation, layers })
        })();
        match result {
            Ok(report) => {
                self.finish_workspace_environment_sync_attempt(&attempt_id, "succeeded", None)?;
                Ok(report)
            }
            Err(err) => {
                let reason = err.to_string();
                if let Err(finish_err) = self.finish_workspace_environment_sync_attempt(
                    &attempt_id,
                    "failed",
                    Some(&reason),
                ) {
                    return Err(Error::Corrupt(format!(
                        "environment synchronization failed: {reason}; additionally failed to finalize attempt `{attempt_id}`: {finish_err}"
                    )));
                }
                Err(err)
            }
        }
    }

    /// User-facing synchronization semantics: activate the immutable
    /// generation first, then reconcile every declared private runtime
    /// resource and return only after its health contract is satisfied.
    /// Provider failure leaves the generation attached with explicit failed
    /// runtime state so readiness remains fail-closed and retry is idempotent.
    pub fn sync_workspace_environment_component_with_runtime(
        &self,
        lane: &str,
        adapter_selector: &str,
        component_root: Option<&str>,
        component_id: Option<&str>,
    ) -> Result<EnvironmentSyncReport> {
        let mut report = self.sync_workspace_environment_component(
            lane,
            adapter_selector,
            component_root,
            component_id,
        )?;
        if report
            .generation
            .components
            .iter()
            .any(|component| !component.runtime_resources.is_empty())
        {
            report.generation = self.reconcile_workspace_environment_runtime(lane)?;
            report.generation = self.cleanup_retired_workspace_environment_runtime(lane)?;
        }
        Ok(report)
    }

    /// Build every discovered component first, then activate all bindings and
    /// one complete generation atomically. Published but unbound layers remain
    /// harmless cache entries if any build or activation fails.
    pub fn sync_all_workspace_environments(
        &self,
        lane: &str,
        discovery_root: Option<&str>,
    ) -> Result<EnvironmentSyncReport> {
        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered workspace view"
            ))
        })?;
        if let (Some(pid), Some(token)) = (view.owner_pid, view.owner_start_token.as_deref()) {
            if process_matches_start_token(pid, token) {
                return Err(Error::InvalidInput(format!(
                    "lane `{lane}` has an active workspace writer in process {pid}; run `trail lane unmount {lane}` before synchronizing its environment"
                )));
            }
        }
        let discovery = self.discover_workspace_environment(lane, discovery_root)?;
        if !discovery.conflicts.is_empty() {
            return Err(Error::InvalidInput(format!(
                "environment discovery found {} unresolved component identity conflict(s)",
                discovery.conflicts.len()
            )));
        }
        let existing = self
            .workspace_environment_rows(lane)?
            .into_iter()
            .map(|state| (state.adapter, state.attached_key))
            .collect::<BTreeMap<_, _>>();
        if discovery.components.is_empty() && (discovery_root.is_some() || existing.is_empty()) {
            return Err(Error::InvalidInput(format!(
                "no workspace environment components were discovered for lane `{lane}`"
            )));
        }
        let raw_plans = self.plan_discovered_environment_components(
            &discovery.source_root,
            &discovery.components,
        )?;
        self.validate_environment_plan_mount_collisions(&raw_plans)?;
        let planned = self
            .finalize_workspace_environment_plan_graph(raw_plans)?
            .into_iter()
            .map(|(plan, expected_key)| {
                let predecessor = existing.get(&plan.component_id).cloned().flatten();
                (plan, expected_key, predecessor)
            })
            .collect::<Vec<_>>();
        let desired_component_ids = planned
            .iter()
            .map(|(plan, _, _)| plan.component_id.as_str())
            .collect::<BTreeSet<_>>();
        let removed_components = if discovery_root.is_none() {
            existing
                .keys()
                .filter(|component_id| !desired_component_ids.contains(component_id.as_str()))
                .cloned()
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        self.validate_environment_mounts_do_not_shadow_source(
            &discovery.source_root,
            &planned.iter().map(|(plan, _, _)| plan).collect::<Vec<_>>(),
        )?;

        let attempt_id = self.begin_workspace_environment_sync_attempt(
            &view.view_id,
            &discovery.source_root,
            "batch",
        )?;
        let result = (|| -> Result<EnvironmentSyncReport> {
            for (plan, expected_key, predecessor) in &planned {
                self.set_workspace_environment_state(
                    lane,
                    plan,
                    expected_key,
                    predecessor.as_deref(),
                    "building",
                    None,
                )?;
            }
            let mut prepared = Vec::with_capacity(planned.len());
            for (plan, expected_key, _) in &planned {
                match self.prepare_workspace_environment_artifacts(
                    &view.view_id,
                    plan,
                    expected_key,
                ) {
                    Ok(artifacts) => prepared.push(artifacts),
                    Err(err) => {
                        let reason = format!(
                            "atomic environment synchronization aborted before activation: {err}"
                        );
                        for (plan, expected_key, predecessor) in &planned {
                            let _ = self.set_workspace_environment_state(
                                lane,
                                plan,
                                expected_key,
                                predecessor.as_deref(),
                                "failed",
                                Some(&reason),
                            );
                        }
                        return Err(err);
                    }
                }
            }
            let planned_refs = planned
                .iter()
                .map(|(plan, expected_key, _)| (plan, expected_key.as_str()))
                .collect::<Vec<_>>();
            if let Err(err) = self.initialize_mounted_workspace_environment_plans(
                lane,
                &attempt_id,
                &discovery.source_root,
                &planned_refs,
                &mut prepared,
                &removed_components,
            ) {
                let reason = format!(
                    "atomic environment synchronization aborted during mounted initialization: {err}"
                );
                for (plan, expected_key, predecessor) in &planned {
                    let _ = self.set_workspace_environment_state(
                        lane,
                        plan,
                        expected_key,
                        predecessor.as_deref(),
                        "failed",
                        Some(&reason),
                    );
                }
                return Err(err);
            }
            let activations = planned
                .iter()
                .zip(&prepared)
                .map(|((plan, expected_key, _), artifacts)| {
                    let (layer_id, private_paths) = match artifacts {
                        PreparedEnvironmentArtifacts::Immutable(layer) => {
                            (Some(layer.layer_id.as_str()), None)
                        }
                        PreparedEnvironmentArtifacts::WritablePrivate(Some(outputs)) => {
                            (None, Some(outputs.paths.as_slice()))
                        }
                        PreparedEnvironmentArtifacts::WritablePrivate(None) => (None, None),
                        PreparedEnvironmentArtifacts::MetadataOnly => (None, None),
                    };
                    Ok(EnvironmentLayerActivation {
                        layer_id: layer_id.map(str::to_string),
                        outputs: environment_output_activations(
                            plan,
                            layer_id,
                            expected_key,
                            private_paths,
                        )?,
                        component_id: plan.component_id.clone(),
                        adapter_identity: plan.adapter_identity.clone(),
                        adapter_version: plan.adapter_version,
                        implementation_version: plan.implementation_version.clone(),
                        distribution_digest: plan.distribution_digest.clone(),
                        kind: plan.kind.clone(),
                        dependencies: environment_dependency_activations(plan)?,
                        caches: environment_cache_activations(plan),
                        external_artifacts: environment_external_artifact_activations(plan),
                        runtime_resources: environment_runtime_resource_activations(plan),
                        expected_key: expected_key.clone(),
                        canonical_key: plan.layer_key.clone(),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            self.ensure_workspace_environment_source_root(lane, &discovery.source_root)?;
            if let Err(err) = self.replace_declared_workspace_layers_with_removals_at_source(
                lane,
                &activations,
                &removed_components,
                &discovery.source_root,
            ) {
                let reason = format!("atomic environment activation failed: {err}");
                for (plan, expected_key, predecessor) in &planned {
                    let _ = self.set_workspace_environment_state(
                        lane,
                        plan,
                        expected_key,
                        predecessor.as_deref(),
                        "failed",
                        Some(&reason),
                    );
                }
                return Err(err);
            }
            let generation = self.active_environment_generation(lane)?.ok_or_else(|| {
                Error::Corrupt("environment activation committed without a generation".to_string())
            })?;
            let layers = prepared
                .into_iter()
                .filter_map(|artifacts| match artifacts {
                    PreparedEnvironmentArtifacts::Immutable(layer) => Some(layer),
                    PreparedEnvironmentArtifacts::WritablePrivate(_) => None,
                    PreparedEnvironmentArtifacts::MetadataOnly => None,
                })
                .collect();
            Ok(EnvironmentSyncReport { generation, layers })
        })();
        match result {
            Ok(report) => {
                self.finish_workspace_environment_sync_attempt(&attempt_id, "succeeded", None)?;
                Ok(report)
            }
            Err(err) => {
                let reason = err.to_string();
                if let Err(finish_err) = self.finish_workspace_environment_sync_attempt(
                    &attempt_id,
                    "failed",
                    Some(&reason),
                ) {
                    return Err(Error::Corrupt(format!(
                        "atomic environment synchronization failed: {reason}; additionally failed to finalize attempt `{attempt_id}`: {finish_err}"
                    )));
                }
                Err(err)
            }
        }
    }

    pub fn sync_all_workspace_environments_with_runtime(
        &self,
        lane: &str,
        discovery_root: Option<&str>,
    ) -> Result<EnvironmentSyncReport> {
        let mut report = self.sync_all_workspace_environments(lane, discovery_root)?;
        if report
            .generation
            .components
            .iter()
            .any(|component| !component.runtime_resources.is_empty())
        {
            report.generation = self.reconcile_workspace_environment_runtime(lane)?;
            report.generation = self.cleanup_retired_workspace_environment_runtime(lane)?;
        }
        Ok(report)
    }

    fn validate_workspace_environment_plan(
        &self,
        adapter: &dyn WorkspaceEnvironmentAdapter,
        component_root: &str,
        plan: &WorkspaceEnvironmentPlan,
    ) -> Result<()> {
        let expected_component = adapter.component_id(component_root)?;
        if plan.component_id != expected_component {
            return Err(Error::Corrupt(format!(
                "adapter `{}` planned component `{}` but host expected `{expected_component}`",
                adapter.identity(),
                plan.component_id
            )));
        }
        if plan.adapter_identity != adapter.identity()
            || plan.layer_key.adapter != adapter.layer_adapter_name()
            || plan.layer_key.adapter_version != plan.adapter_version
        {
            return Err(Error::Corrupt(format!(
                "component `{}` returned inconsistent adapter identity or version",
                plan.component_id
            )));
        }
        self.validate_workspace_environment_plan_common(plan)
    }

    fn validate_command_recipe_plan(
        &self,
        expected_component_id: &str,
        plan: &WorkspaceEnvironmentPlan,
    ) -> Result<()> {
        let mut tool_identities = BTreeMap::new();
        self.validate_command_recipe_plan_with_tool_cache(
            expected_component_id,
            plan,
            &mut tool_identities,
        )
    }

    fn validate_command_recipe_plan_with_tool_cache(
        &self,
        expected_component_id: &str,
        plan: &WorkspaceEnvironmentPlan,
        tool_identities: &mut BTreeMap<PathBuf, String>,
    ) -> Result<()> {
        let metadata = &super::workspace_recipe::COMMAND_RECIPE_ADAPTER_METADATA;
        if plan.component_id != expected_component_id
            || plan.adapter_identity != metadata.canonical_identity
            || plan.layer_key.adapter != metadata.layer_adapter_name
            || plan.layer_key.adapter_version != metadata.contract_major
            || plan.sandbox_policy != WorkspaceEnvironmentSandboxPolicy::RestrictedRecipe
            || !plan.pre_commands.is_empty()
            || !plan.mounted_commands.is_empty()
            || plan.command.is_none()
        {
            return Err(Error::Corrupt(format!(
                "command recipe `{expected_component_id}` returned an inconsistent identity, policy, or action plan"
            )));
        }
        self.validate_workspace_environment_plan_common_with_tool_cache(plan, tool_identities)
    }

    fn validate_environment_plugin_plan(
        &self,
        plugin: &super::workspace_plugin::InstalledEnvironmentPlugin,
        expected_component_id: &str,
        plan: &WorkspaceEnvironmentPlan,
    ) -> Result<()> {
        let metadata = &plugin.manifest.adapter;
        let protocol = super::workspace_plugin::selected_environment_plugin_protocol(plugin)?;
        let action_policy_is_valid = match protocol {
            trail_environment_adapter_sdk::PROTOCOL_V1 => {
                plan.sandbox_policy == WorkspaceEnvironmentSandboxPolicy::RestrictedRecipe
                    && plan.command.is_some()
                    && plan.mounted_commands.is_empty()
            }
            trail_environment_adapter_sdk::PROTOCOL_V2 if plan.mounted_commands.is_empty() => {
                plan.sandbox_policy
                    == if plan.caches.is_empty() {
                        WorkspaceEnvironmentSandboxPolicy::RestrictedRecipe
                    } else {
                        WorkspaceEnvironmentSandboxPolicy::RestrictedPluginStaging
                    }
                    && plan.command.is_some()
            }
            trail_environment_adapter_sdk::PROTOCOL_V2 => {
                plan.sandbox_policy == WorkspaceEnvironmentSandboxPolicy::RestrictedPluginMounted
            }
            _ => false,
        };
        if plan.component_id != expected_component_id
            || plan.adapter_identity != metadata.canonical_identity
            || plan.adapter_version != 1
            || plan.implementation_version != metadata.implementation_version
            || plan.distribution_digest != plugin.distribution_digest
            || plan.kind != metadata.kind
            || plan.layer_key.adapter != metadata.layer_adapter_name
            || plan.layer_key.adapter_version != 1
            || plan.layer_key.inputs.get("adapter_executable_digest")
                != Some(&plugin.executable_digest)
            || plan.layer_key.inputs.get("protocol") != Some(&protocol.to_string())
            || !action_policy_is_valid
            || !plan.pre_commands.is_empty()
        {
            return Err(Error::Corrupt(format!(
                "plugin component `{expected_component_id}` returned an inconsistent identity, provenance, or policy plan"
            )));
        }
        self.validate_workspace_environment_plan_common(plan)
    }

    fn validate_workspace_environment_plan_common(
        &self,
        plan: &WorkspaceEnvironmentPlan,
    ) -> Result<()> {
        let mut tool_identities = BTreeMap::new();
        self.validate_workspace_environment_plan_common_with_tool_cache(plan, &mut tool_identities)
    }

    fn validate_workspace_environment_plan_common_with_tool_cache(
        &self,
        plan: &WorkspaceEnvironmentPlan,
        tool_identities: &mut BTreeMap<PathBuf, String>,
    ) -> Result<()> {
        validate_environment_component_identity(&plan.component_id)?;
        let mut dependencies = BTreeSet::new();
        for dependency in &plan.dependencies {
            validate_environment_component_identity(&dependency.component_id)?;
            if dependency.component_id == plan.component_id {
                return Err(Error::InvalidInput(format!(
                    "environment component `{}` cannot depend on itself",
                    plan.component_id
                )));
            }
            if !dependencies.insert(&dependency.component_id) {
                return Err(Error::InvalidInput(format!(
                    "environment component `{}` repeats dependency `{}`",
                    plan.component_id, dependency.component_id
                )));
            }
        }
        if !plan.resolved_dependencies.is_empty() {
            return Err(Error::Corrupt(format!(
                "environment component `{}` supplied host-owned resolved dependency state",
                plan.component_id
            )));
        }
        if plan
            .layer_key
            .inputs
            .keys()
            .any(|name| name.starts_with("dependency:"))
        {
            return Err(Error::InvalidInput(format!(
                "environment component `{}` used the host-reserved `dependency:` key namespace",
                plan.component_id
            )));
        }
        if plan.implementation_version.trim().is_empty()
            || plan.distribution_digest.trim().is_empty()
            || plan.layer_key.inputs.get("adapter_implementation")
                != Some(&plan.implementation_version)
            || plan.layer_key.inputs.get("adapter_distribution_digest")
                != Some(&plan.distribution_digest)
        {
            return Err(Error::Corrupt(format!(
                "component `{}` omitted adapter implementation provenance from its layer key",
                plan.component_id
            )));
        }
        if plan.kind != plan.layer_key.kind {
            return Err(Error::Corrupt(format!(
                "component `{}` returned inconsistent layer kind",
                plan.component_id
            )));
        }
        if !matches!(
            plan.kind.as_str(),
            "dependency" | "compiler-results" | "generated" | "build" | "external"
        ) {
            return Err(Error::InvalidInput(format!(
                "component `{}` declared unsupported environment kind `{}`",
                plan.component_id, plan.kind
            )));
        }
        let Some((_, _, contract_major)) = parse_canonical_adapter_identity(&plan.adapter_identity)
        else {
            return Err(Error::InvalidInput(format!(
                "component `{}` declared malformed adapter identity `{}`",
                plan.component_id, plan.adapter_identity
            )));
        };
        if contract_major != plan.adapter_version {
            return Err(Error::InvalidInput(format!(
                "component `{}` adapter identity major does not match adapter_version",
                plan.component_id
            )));
        }
        if plan.outputs.len() > 32 {
            return Err(Error::InvalidInput(format!(
                "component `{}` declares more than 32 outputs",
                plan.component_id
            )));
        }
        let metadata_only = plan.outputs.is_empty();
        if metadata_only {
            if plan.kind != "external"
                || (plan.external_artifacts.is_empty() && plan.runtime_resources.is_empty())
                || plan.command.is_some()
                || !plan.pre_commands.is_empty()
                || !plan.mounted_commands.is_empty()
                || !plan.caches.is_empty()
                || plan.source_projection.is_some()
            {
                return Err(Error::InvalidInput(format!(
                    "component `{}` may omit filesystem outputs only as an action-free external/runtime component",
                    plan.component_id
                )));
            }
        } else if !plan.external_artifacts.is_empty() || !plan.runtime_resources.is_empty() {
            return Err(Error::InvalidInput(format!(
                "component `{}` mixes external/runtime resources with filesystem outputs; split independently owned resources into separate components",
                plan.component_id
            )));
        }
        if plan.external_artifacts.len() > 32 {
            return Err(Error::InvalidInput(format!(
                "component `{}` declares more than 32 external artifacts",
                plan.component_id
            )));
        }
        let mut external_names = BTreeSet::new();
        for artifact in &plan.external_artifacts {
            validate_workspace_environment_external_artifact(artifact)?;
            if !external_names.insert(&artifact.name) {
                return Err(Error::InvalidInput(format!(
                    "component `{}` repeats external artifact `{}`",
                    plan.component_id, artifact.name
                )));
            }
        }
        if !plan.external_artifacts.is_empty() {
            let identity = workspace_external_artifacts_identity(&plan.external_artifacts)?;
            if plan.layer_key.inputs.get("external_artifact_contract") != Some(&identity) {
                return Err(Error::Corrupt(format!(
                    "component `{}` omitted its external artifact contract from the canonical key",
                    plan.component_id
                )));
            }
        }
        if plan.runtime_resources.len() > 32 {
            return Err(Error::InvalidInput(format!(
                "component `{}` declares more than 32 runtime resources",
                plan.component_id
            )));
        }
        let mut runtime_names = BTreeSet::new();
        for resource in &plan.runtime_resources {
            validate_workspace_environment_runtime_resource(resource)?;
            if !runtime_names.insert(&resource.name) {
                return Err(Error::InvalidInput(format!(
                    "component `{}` repeats runtime resource `{}`",
                    plan.component_id, resource.name
                )));
            }
            if !external_names.contains(&resource.artifact_name) {
                return Err(Error::InvalidInput(format!(
                    "component `{}` runtime resource `{}` references missing external artifact `{}`",
                    plan.component_id, resource.name, resource.artifact_name
                )));
            }
        }
        if !plan.runtime_resources.is_empty() {
            let identity = workspace_runtime_resources_identity(&plan.runtime_resources)?;
            if plan.layer_key.inputs.get("runtime_resource_contract") != Some(&identity) {
                return Err(Error::Corrupt(format!(
                    "component `{}` omitted its runtime resource contract from the canonical key",
                    plan.component_id
                )));
            }
        }
        let mut output_names = BTreeSet::new();
        let mut output_policies = BTreeSet::new();
        let mut output_paths = Vec::<(&str, &str)>::new();
        let mut mount_paths = Vec::<(&str, &str)>::new();
        for output in &plan.outputs {
            output_policies.insert(output.policy);
            if output.name.is_empty() || !output_names.insert(output.name.clone()) {
                return Err(Error::InvalidInput(format!(
                    "component `{}` has an empty or duplicate output name `{}`",
                    plan.component_id, output.name
                )));
            }
            normalize_relative_path(&output.mount_path)?;
            normalize_relative_path(&output.output_path)?;
            for (other_name, other_path) in &output_paths {
                if environment_mounts_overlap(&output.output_path, other_path) {
                    return Err(Error::InvalidInput(format!(
                        "component `{}` output `{}` path overlaps output `{other_name}`",
                        plan.component_id, output.name
                    )));
                }
            }
            for (other_name, other_path) in &mount_paths {
                if environment_mounts_overlap(&output.mount_path, other_path) {
                    return Err(Error::InvalidInput(format!(
                        "component `{}` output `{}` mount overlaps output `{other_name}`",
                        plan.component_id, output.name
                    )));
                }
            }
            output_paths.push((&output.name, &output.output_path));
            mount_paths.push((&output.name, &output.mount_path));
        }
        if !metadata_only && output_policies.len() != 1 {
            return Err(Error::InvalidInput(format!(
                "component `{}` mixes immutable-seeded and writable-private outputs; split it into independently keyed components until heterogeneous action publication is available",
                plan.component_id
            )));
        }
        if !plan.caches.is_empty() {
            match plan.sandbox_policy {
                WorkspaceEnvironmentSandboxPolicy::TrustedBuiltin => {}
                WorkspaceEnvironmentSandboxPolicy::RestrictedPluginStaging
                | WorkspaceEnvironmentSandboxPolicy::RestrictedPluginMounted
                    if plan.caches.iter().all(|cache| {
                        cache.access == WorkspaceEnvironmentCacheAccess::HostExclusive
                    }) => {}
                _ => {
                    return Err(Error::InvalidInput(format!(
                        "component `{}` requests host cache namespaces without a certified cache access contract",
                        plan.component_id
                    )));
                }
            }
        }
        let mut cache_names = BTreeSet::new();
        for cache in &plan.caches {
            if !cache_names.insert(cache.name.clone()) {
                return Err(Error::InvalidInput(format!(
                    "component `{}` repeats cache namespace `{}`",
                    plan.component_id, cache.name
                )));
            }
            let expected = self.declare_workspace_environment_cache(
                &plan.adapter_identity,
                &cache.name,
                cache.protocol,
                cache.access,
                cache.compatibility.clone(),
            )?;
            if expected.namespace_id != cache.namespace_id
                || expected.storage_path != cache.storage_path
            {
                return Err(Error::Corrupt(format!(
                    "component `{}` cache `{}` does not match its host-owned namespace identity",
                    plan.component_id, cache.name
                )));
            }
        }
        if plan.mounted_commands.is_empty() {
            if plan.layer_key.inputs.contains_key("mounted_action") {
                return Err(Error::Corrupt(format!(
                    "component `{}` declares a mounted action identity without a mounted action",
                    plan.component_id
                )));
            }
        } else {
            if !matches!(
                plan.sandbox_policy,
                WorkspaceEnvironmentSandboxPolicy::TrustedBuiltin
                    | WorkspaceEnvironmentSandboxPolicy::RestrictedPluginMounted
            ) || output_policies
                != BTreeSet::from([WorkspaceEnvironmentOutputPolicy::WritablePrivate])
            {
                return Err(Error::InvalidInput(format!(
                    "component `{}` may use mounted initialization only as a trusted built-in or authorized protocol-v2 plugin with writable-private outputs",
                    plan.component_id
                )));
            }
            let action_identity = workspace_mounted_commands_identity(&plan.mounted_commands)?;
            if plan.layer_key.inputs.get("mounted_action") != Some(&action_identity) {
                return Err(Error::Corrupt(format!(
                    "component `{}` omitted its mounted initialization action from the canonical key",
                    plan.component_id
                )));
            }
        }
        for command in plan.pre_commands.iter().chain(plan.command.iter()) {
            normalize_relative_path(&command.working_directory)?;
            if command.program.trim().is_empty() {
                return Err(Error::InvalidInput(format!(
                    "component `{}` has an empty build executable",
                    plan.component_id
                )));
            }
            let current_identity =
                if let Some(identity) = tool_identities.get(&command.resolved_program) {
                    identity.clone()
                } else {
                    let identity = workspace_tool_identity_for_path(&command.resolved_program)?;
                    tool_identities.insert(command.resolved_program.clone(), identity.clone());
                    identity
                };
            if !command.resolved_program.is_absolute()
                || current_identity != command.executable_identity
            {
                return Err(Error::InvalidInput(format!(
                    "component `{}` executable identity for `{}` changed after planning",
                    plan.component_id, command.program
                )));
            }
            for name in command.environment.keys() {
                if name.is_empty()
                    || !name
                        .chars()
                        .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
                {
                    return Err(Error::InvalidInput(format!(
                        "component `{}` has invalid environment key `{name}`",
                        plan.component_id
                    )));
                }
            }
            let mut command_caches = BTreeSet::new();
            for cache_name in &command.cache_names {
                if !cache_names.contains(cache_name) || !command_caches.insert(cache_name) {
                    return Err(Error::InvalidInput(format!(
                        "component `{}` command references missing or duplicate cache `{cache_name}`",
                        plan.component_id
                    )));
                }
            }
        }
        for command in &plan.mounted_commands {
            if !command.working_directory.is_empty() {
                normalize_relative_path(&command.working_directory)?;
            }
            if command.program.trim().is_empty() || !command.cache_names.is_empty() {
                return Err(Error::InvalidInput(format!(
                    "component `{}` has an invalid mounted initialization command",
                    plan.component_id
                )));
            }
            let current_identity =
                if let Some(identity) = tool_identities.get(&command.resolved_program) {
                    identity.clone()
                } else {
                    let identity = workspace_tool_identity_for_path(&command.resolved_program)?;
                    tool_identities.insert(command.resolved_program.clone(), identity.clone());
                    identity
                };
            if !command.resolved_program.is_absolute()
                || current_identity != command.executable_identity
            {
                return Err(Error::InvalidInput(format!(
                    "component `{}` mounted executable identity for `{}` changed after planning",
                    plan.component_id, command.program
                )));
            }
            for name in command.environment.keys() {
                if name.is_empty()
                    || !name
                        .chars()
                        .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
                {
                    return Err(Error::InvalidInput(format!(
                        "component `{}` has invalid mounted environment key `{name}`",
                        plan.component_id
                    )));
                }
            }
        }
        let mut staging_paths = BTreeSet::new();
        for input in &plan.inputs {
            normalize_relative_path(&input.source_path)?;
            let staging = normalize_relative_path(&input.staging_path)?;
            if !staging_paths.insert(staging.clone()) {
                return Err(Error::InvalidInput(format!(
                    "component `{}` maps more than one input to `{staging}`",
                    plan.component_id
                )));
            }
        }
        if let Some((_, staging_root)) = &plan.source_projection {
            normalize_relative_path(staging_root)?;
        }
        Ok(())
    }

    fn validate_environment_mounts_do_not_shadow_source(
        &self,
        source_root: &ObjectId,
        plans: &[&WorkspaceEnvironmentPlan],
    ) -> Result<()> {
        let mut mounts = BTreeMap::<String, (String, String, String)>::new();
        for plan in plans {
            for output in &plan.outputs {
                let mount_path = normalize_relative_path(&output.mount_path)?;
                let folded = case_insensitive_path_key(&mount_path);
                let first = folded.split('/').next().unwrap_or_default();
                if matches!(first, ".trail" | ".git") {
                    return Err(Error::InvalidInput(format!(
                        "environment component `{}` output `{}` cannot mount inside reserved path `{}`",
                        plan.component_id, output.name, output.mount_path
                    )));
                }
                mounts.insert(
                    folded,
                    (plan.component_id.clone(), output.name.clone(), mount_path),
                );
            }
        }
        self.for_each_root_file_chunk(source_root, 1024, |chunk| {
            for (path, _) in chunk {
                let folded_path = case_insensitive_path_key(&path);
                if let Some((_, (component_id, output_name, mount))) =
                    environment_path_ancestor(&mounts, &folded_path)
                {
                    return Err(Error::InvalidInput(format!(
                        "environment component `{component_id}` output `{output_name}` mount `{mount}` would shadow pinned source file `{path}`"
                    )));
                }
            }
            Ok(())
        })
    }

    fn prepare_workspace_environment_artifacts(
        &self,
        view_id: &str,
        plan: &WorkspaceEnvironmentPlan,
        component_key: &str,
    ) -> Result<PreparedEnvironmentArtifacts> {
        let Some(policy) = plan.outputs.first().map(|output| output.policy) else {
            if !plan.external_artifacts.is_empty() {
                return Ok(PreparedEnvironmentArtifacts::MetadataOnly);
            }
            return Err(Error::Corrupt(format!(
                "component `{}` reached execution without outputs or external artifacts",
                plan.component_id
            )));
        };
        match policy {
            WorkspaceEnvironmentOutputPolicy::ImmutableSeedPrivate => self
                .build_workspace_layer_singleflight(&plan.layer_key, |build_dir| {
                    self.execute_workspace_environment_plan(plan, build_dir)
                })
                .map(PreparedEnvironmentArtifacts::Immutable),
            WorkspaceEnvironmentOutputPolicy::WritablePrivate => {
                if self.writable_private_outputs_are_compatible(view_id, plan, component_key)? {
                    return Ok(PreparedEnvironmentArtifacts::WritablePrivate(None));
                }
                let outputs = self.execute_writable_private_environment_plan(plan)?;
                Ok(PreparedEnvironmentArtifacts::WritablePrivate(Some(outputs)))
            }
        }
    }

    /// Run path-sensitive initializers at the lane's final mountpoint without
    /// exposing the current generation to partial output.
    ///
    /// The candidate view uses the pinned source root and desired immutable
    /// bindings, but every writable class has a temporary upper. Existing
    /// private outputs are presented as read-only lower directories. After all
    /// commands succeed, only newly prepared writable-private outputs are
    /// copied back into host staging; normal generation activation remains the
    /// sole mutation of the real lane upper and SQLite bindings.
    fn initialize_mounted_workspace_environment_plans(
        &self,
        lane: &str,
        attempt_id: &str,
        source_root: &ObjectId,
        planned: &[(&WorkspaceEnvironmentPlan, &str)],
        prepared: &mut [PreparedEnvironmentArtifacts],
        removed_components: &[String],
    ) -> Result<()> {
        if planned.len() != prepared.len() {
            return Err(Error::Corrupt(
                "mounted environment initialization lost plan/artifact alignment".to_string(),
            ));
        }
        let needs_initialization =
            planned
                .iter()
                .zip(prepared.iter())
                .any(|((plan, _), artifacts)| {
                    !plan.mounted_commands.is_empty()
                        && matches!(
                            artifacts,
                            PreparedEnvironmentArtifacts::WritablePrivate(Some(_))
                        )
                });
        if !needs_initialization {
            return Ok(());
        }

        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered workspace view"
            ))
        })?;
        let branch = self.lane_branch(lane)?;
        let record = self.lane_record(&branch.lane_id)?;
        let mode = self.lane_workdir_mode_for(&record, &branch)?;
        if !matches!(mode, LaneWorkdirMode::OverlayCow | LaneWorkdirMode::NfsCow) {
            return Err(Error::InvalidInput(format!(
                "lane `{lane}` uses `{}` and cannot run mounted environment initialization",
                mode.as_str()
            )));
        }

        let candidate_root = self.db_dir.join("cache/staging");
        fs::create_dir_all(&candidate_root)?;
        let candidate = tempfile::Builder::new()
            .prefix(&format!("mounted-environment-{attempt_id}-"))
            .tempdir_in(&candidate_root)?;
        let candidate_source_upper = candidate.path().join("view/source-upper");
        let candidate_layout = ViewUpperLayout::from_source_upper(candidate_source_upper.clone());
        candidate_layout.ensure()?;
        let real_layout = ViewUpperLayout::from_source_upper(PathBuf::from(&view.source_upper));

        let replaced_components = planned
            .iter()
            .map(|(plan, _)| plan.component_id.clone())
            .chain(removed_components.iter().cloned())
            .collect::<BTreeSet<_>>();
        let desired_mounts = planned
            .iter()
            .flat_map(|(plan, _)| {
                plan.outputs
                    .iter()
                    .map(|output| (plan.component_id.as_str(), output.mount_path.as_str()))
            })
            .collect::<Vec<_>>();
        let mut current_output_stmt = self.conn.prepare(
            "SELECT component_id, mount_path
             FROM environment_component_output_bindings
             WHERE view_id = ?1",
        )?;
        let current_outputs = current_output_stmt
            .query_map(params![&view.view_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (owner, existing_mount) in &current_outputs {
            if replaced_components.contains(owner) {
                continue;
            }
            for (component, desired_mount) in &desired_mounts {
                if environment_mounts_overlap(existing_mount, desired_mount) {
                    return Err(Error::InvalidInput(format!(
                        "environment component `{component}` mounted initializer would overlap active component `{owner}` at `{existing_mount}`"
                    )));
                }
            }
        }
        let retired_mounts =
            current_outputs
                .iter()
                .filter_map(|(component, mount)| {
                    replaced_components
                        .contains(component)
                        .then_some(mount.clone())
                })
                .chain(planned.iter().flat_map(|(plan, _)| {
                    plan.outputs.iter().map(|output| output.mount_path.clone())
                }))
                .collect::<BTreeSet<_>>();

        let mut bindings =
            self.workspace_layer_bindings_for_source_upper(&real_layout.source_upper)?;
        for binding in &mut bindings {
            if binding.layer_id.is_none() {
                let class = environment_upper_class(&binding.kind)?;
                binding.storage_path = Some(safe_join(
                    real_layout.upper_for_class(class),
                    &binding.mount_path,
                )?);
            }
        }
        bindings.retain(|binding| !retired_mounts.contains(&binding.mount_path));

        let mut candidate_outputs = Vec::<(PathBuf, PathBuf, String, String)>::new();
        let mut persisted_mounts = Vec::<(ViewPathClass, String)>::new();
        for ((plan, component_key), artifacts) in planned.iter().zip(prepared.iter()) {
            let (layer_id, private_paths) = match artifacts {
                PreparedEnvironmentArtifacts::Immutable(layer) => {
                    (Some(layer.layer_id.as_str()), None)
                }
                PreparedEnvironmentArtifacts::WritablePrivate(Some(outputs)) => {
                    (None, Some(outputs.paths.as_slice()))
                }
                PreparedEnvironmentArtifacts::WritablePrivate(None) => (None, None),
                PreparedEnvironmentArtifacts::MetadataOnly => (None, None),
            };
            let outputs =
                environment_output_activations(plan, layer_id, component_key, private_paths)?;
            for (output_index, output) in outputs.iter().enumerate() {
                let class = environment_upper_class(&plan.kind)?;
                let storage_path = match artifacts {
                    PreparedEnvironmentArtifacts::Immutable(layer) => {
                        let root = Path::new(&layer.storage_path);
                        Some(if output.layer_subpath.is_empty() {
                            root.to_path_buf()
                        } else {
                            safe_join(root, &output.layer_subpath)?
                        })
                    }
                    PreparedEnvironmentArtifacts::WritablePrivate(Some(private)) => {
                        let seed = private.paths.get(output_index).ok_or_else(|| {
                            Error::Corrupt(format!(
                                "component `{}` lost writable-private seed `{}`",
                                plan.component_id, output.name
                            ))
                        })?;
                        let destination =
                            safe_join(candidate_layout.upper_for_class(class), &output.mount_path)?;
                        copy_dir_recursive(seed, &destination)?;
                        candidate_outputs.push((
                            seed.clone(),
                            destination,
                            plan.component_id.clone(),
                            output.name.clone(),
                        ));
                        persisted_mounts.push((class, output.mount_path.clone()));
                        None
                    }
                    PreparedEnvironmentArtifacts::WritablePrivate(None) => Some(safe_join(
                        real_layout.upper_for_class(class),
                        &output.mount_path,
                    )?),
                    PreparedEnvironmentArtifacts::MetadataOnly => None,
                };
                bindings.push(WorkspaceLayerBinding {
                    binding_identity: output.binding_identity.clone(),
                    layer_id: layer_id.map(str::to_string),
                    mount_path: output.mount_path.clone(),
                    storage_path,
                    kind: plan.kind.clone(),
                    priority: 100,
                });
            }
        }
        bindings.sort_by(|left, right| {
            right
                .mount_path
                .len()
                .cmp(&left.mount_path.len())
                .then_with(|| right.priority.cmp(&left.priority))
                .then_with(|| left.mount_path.cmp(&right.mount_path))
        });

        let run = || -> Result<()> {
            let mut command_index = 0usize;
            for ((plan, _), artifacts) in planned.iter().zip(prepared.iter()) {
                if !matches!(
                    artifacts,
                    PreparedEnvironmentArtifacts::WritablePrivate(Some(_))
                ) {
                    continue;
                }
                for command in &plan.mounted_commands {
                    let process_root = candidate
                        .path()
                        .join("process")
                        .join(format!("{command_index:04}"));
                    self.run_mounted_workspace_environment_command(
                        lane,
                        plan,
                        command,
                        Path::new(&view.mountpoint),
                        &process_root,
                    )?;
                    command_index += 1;
                }
            }
            Ok(())
        };

        let run_result = match mode {
            LaneWorkdirMode::OverlayCow => {
                let mount = self.mount_overlay_cow_workdir_for_lane_with_ephemeral_bindings(
                    lane,
                    candidate_source_upper.clone(),
                    source_root.clone(),
                    bindings,
                )?;
                let result = run();
                drop(mount);
                result
            }
            LaneWorkdirMode::NfsCow => {
                let mount = self.mount_nfs_cow_workdir_for_lane_with_ephemeral_bindings(
                    lane,
                    candidate_source_upper,
                    source_root.clone(),
                    bindings,
                )?;
                let result = run();
                drop(mount);
                result
            }
            _ => unreachable!(),
        };
        run_result?;
        test_crash_point("environment_after_mounted_initialization");

        validate_mounted_environment_candidate_writes(&candidate_layout, &persisted_mounts)?;
        for (seed, output, component_id, output_name) in candidate_outputs {
            if !output.is_dir() {
                return Err(Error::InvalidInput(format!(
                    "mounted initialization for component `{component_id}` did not produce output `{output_name}`"
                )));
            }
            make_tree_writable(&seed);
            if seed.exists() {
                fs::remove_dir_all(&seed)?;
            }
            copy_dir_recursive(&output, &seed)?;
        }
        Ok(())
    }

    fn ensure_workspace_environment_source_root(
        &self,
        lane: &str,
        expected: &ObjectId,
    ) -> Result<()> {
        let branch = self.lane_branch(lane)?;
        let current = self.get_ref(&branch.ref_name)?.root_id;
        if &current != expected {
            return Err(Error::InvalidInput(format!(
                "lane `{lane}` advanced from pinned source root `{expected}` to `{current}` during environment synchronization; retry against the new lane head"
            )));
        }
        Ok(())
    }

    fn run_mounted_workspace_environment_command(
        &self,
        lane: &str,
        plan: &WorkspaceEnvironmentPlan,
        command_plan: &WorkspaceEnvironmentCommand,
        mountpoint: &Path,
        process_root: &Path,
    ) -> Result<()> {
        let working_directory = if command_plan.working_directory.is_empty() {
            mountpoint.to_path_buf()
        } else {
            safe_join(mountpoint, &command_plan.working_directory)?
        };
        if !working_directory.is_dir() {
            return Err(Error::InvalidInput(format!(
                "mounted initialization working directory `{}` for component `{}` does not exist",
                command_plan.working_directory, plan.component_id
            )));
        }
        let isolated_home = process_root.join("home");
        let isolated_tmp = process_root.join("tmp");
        fs::create_dir_all(&isolated_home)?;
        fs::create_dir_all(&isolated_tmp)?;
        let current_identity = workspace_tool_identity_for_path(&command_plan.resolved_program)?;
        if current_identity != command_plan.executable_identity {
            return Err(Error::InvalidInput(format!(
                "mounted initialization executable `{}` changed after the component key was computed",
                command_plan.program
            )));
        }
        let (launcher, launcher_args) = match plan.sandbox_policy {
            WorkspaceEnvironmentSandboxPolicy::TrustedBuiltin => (
                command_plan.resolved_program.clone(),
                command_plan.args.iter().map(OsString::from).collect(),
            ),
            WorkspaceEnvironmentSandboxPolicy::RestrictedPluginMounted => {
                // Reuse the native recipe sandbox with the candidate mount as
                // its read root and the declared final mount targets as its
                // only writable paths. The adapter never learns candidate
                // upper paths or receives mount authority.
                let mut mounted_plan = plan.clone();
                for output in &mut mounted_plan.outputs {
                    output.output_path.clone_from(&output.mount_path);
                }
                for input in &mut mounted_plan.inputs {
                    input.staging_path.clone_from(&input.source_path);
                }
                let executable_directory = isolated_home.join("tool");
                fs::create_dir_all(&executable_directory)?;
                let executable_name =
                    command_plan.resolved_program.file_name().ok_or_else(|| {
                        Error::Corrupt(format!(
                            "mounted plugin executable `{}` has no file name",
                            command_plan.resolved_program.display()
                        ))
                    })?;
                let staged_executable = executable_directory.join(executable_name);
                fs::copy(&command_plan.resolved_program, &staged_executable)?;
                fs::set_permissions(
                    &staged_executable,
                    fs::metadata(&command_plan.resolved_program)?.permissions(),
                )?;
                let expected_digest = command_plan
                    .executable_identity
                    .rsplit_once(":sha256:")
                    .map(|(_, digest)| digest)
                    .ok_or_else(|| {
                        Error::Corrupt(format!(
                            "mounted plugin executable identity for `{}` has no digest",
                            command_plan.program
                        ))
                    })?;
                let staged_digest = sha256_hex(&fs::read(&staged_executable)?);
                if staged_digest != expected_digest {
                    return Err(Error::Corrupt(format!(
                        "staged mounted plugin executable `{}` failed identity verification",
                        command_plan.program
                    )));
                }
                let mut sandboxed_command = command_plan.clone();
                sandboxed_command.resolved_program = staged_executable;
                self.restricted_recipe_launcher(
                    &mounted_plan,
                    &sandboxed_command,
                    mountpoint,
                    &isolated_home,
                    &isolated_tmp,
                )?
            }
            WorkspaceEnvironmentSandboxPolicy::RestrictedRecipe
            | WorkspaceEnvironmentSandboxPolicy::RestrictedPluginStaging => {
                return Err(Error::Corrupt(format!(
                    "repository command component `{}` unexpectedly requested mounted initialization",
                    plan.component_id
                )));
            }
        };
        let mut command = Command::new(launcher);
        command
            .args(launcher_args)
            .current_dir(&working_directory)
            .env_clear()
            .envs(&command_plan.environment)
            .env("HOME", &isolated_home)
            .env("TMPDIR", &isolated_tmp)
            .env("TMP", &isolated_tmp)
            .env("TEMP", &isolated_tmp);
        if plan.sandbox_policy == WorkspaceEnvironmentSandboxPolicy::TrustedBuiltin {
            command
                .env("TRAIL_WORKSPACE", self.workspace_root())
                .env("TRAIL_LANE", lane)
                .env("TRAIL_ENVIRONMENT_COMPONENT", &plan.component_id)
                .env("TRAIL_ENVIRONMENT_INITIALIZATION", "1");
        }
        if let Some(path) = std::env::var_os("PATH") {
            command.env("PATH", path);
        }
        #[cfg(windows)]
        for name in ["SystemRoot", "ComSpec", "PATHEXT"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        for name in &command_plan.remove_environment {
            command.env_remove(name);
        }
        let status =
            if plan.sandbox_policy == WorkspaceEnvironmentSandboxPolicy::RestrictedPluginMounted {
                run_supervised_mounted_plugin_process(&mut command).map_err(|err| {
                    Error::InvalidInput(format!(
                        "failed to supervise mounted initializer `{}` for component `{}`: {err}",
                        command_plan.program, plan.component_id
                    ))
                })?
            } else {
                command.status().map_err(|err| {
                    Error::InvalidInput(format!(
                        "failed to launch mounted initializer `{}` for component `{}`: {err}",
                        command_plan.program, plan.component_id
                    ))
                })?
            };
        if !status.success() {
            return Err(Error::InvalidInput(format!(
                "mounted initialization for component `{}` failed with {status}; the previous environment generation remains active",
                plan.component_id
            )));
        }
        Ok(())
    }

    fn writable_private_outputs_are_compatible(
        &self,
        view_id: &str,
        plan: &WorkspaceEnvironmentPlan,
        component_key: &str,
    ) -> Result<bool> {
        let mut stmt = self.conn.prepare(
            "SELECT output_name, mount_path, policy, binding_identity
             FROM environment_component_output_bindings
             WHERE view_id = ?1 AND component_id = ?2
             ORDER BY output_name",
        )?;
        let actual = stmt
            .query_map(params![view_id, &plan.component_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut expected = plan
            .outputs
            .iter()
            .map(|output| {
                (
                    output.name.clone(),
                    output.mount_path.clone(),
                    output.policy.as_str().to_string(),
                    writable_private_binding_identity(
                        &plan.component_id,
                        &output.name,
                        component_key,
                    ),
                )
            })
            .collect::<Vec<_>>();
        expected.sort();
        if actual != expected {
            return Ok(false);
        }
        let source_upper = self.conn.query_row(
            "SELECT source_upper FROM workspace_views WHERE view_id = ?1",
            params![view_id],
            |row| row.get::<_, String>(0),
        )?;
        let layout =
            super::workdir::ViewUpperLayout::from_source_upper(PathBuf::from(source_upper));
        let class = match plan.kind.as_str() {
            "dependency" => super::workdir::ViewPathClass::Dependency,
            "compiler-results" | "generated" | "build" => super::workdir::ViewPathClass::Generated,
            _ => return Ok(false),
        };
        for output in &plan.outputs {
            let upper = safe_join(layout.upper_for_class(class), &output.mount_path)?;
            if !upper.is_dir() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn execute_writable_private_environment_plan(
        &self,
        plan: &WorkspaceEnvironmentPlan,
    ) -> Result<PreparedPrivateEnvironmentOutputs> {
        if matches!(
            plan.sandbox_policy,
            WorkspaceEnvironmentSandboxPolicy::RestrictedRecipe
                | WorkspaceEnvironmentSandboxPolicy::RestrictedPluginStaging
                | WorkspaceEnvironmentSandboxPolicy::RestrictedPluginMounted
        ) {
            ensure_restricted_recipe_sandbox_available()?;
        }
        #[cfg(target_os = "windows")]
        let staging_parent = std::env::temp_dir();
        #[cfg(not(target_os = "windows"))]
        let staging_parent = PathBuf::from("/tmp");
        if !staging_parent.is_dir() {
            return Err(Error::InvalidInput(
                "writable-private environment staging requires an available host temporary directory"
                    .to_string(),
            ));
        }
        let staging = tempfile::Builder::new()
            .prefix("trail-private-environment-")
            .tempdir_in(staging_parent)?;
        let root = fs::canonicalize(staging.path())?;
        let restricted = matches!(
            plan.sandbox_policy,
            WorkspaceEnvironmentSandboxPolicy::RestrictedRecipe
                | WorkspaceEnvironmentSandboxPolicy::RestrictedPluginStaging
                | WorkspaceEnvironmentSandboxPolicy::RestrictedPluginMounted
        );
        let paths =
            self.execute_workspace_environment_plan_in_directory(plan, &root, restricted)?;
        if restricted {
            let mut entries = 0usize;
            for output in &paths {
                self.validate_restricted_recipe_output(plan, output, &mut entries)?;
            }
        }
        Ok(PreparedPrivateEnvironmentOutputs {
            _staging: staging,
            paths,
        })
    }

    fn execute_workspace_environment_plan(
        &self,
        plan: &WorkspaceEnvironmentPlan,
        build_dir: &Path,
    ) -> Result<PathBuf> {
        if matches!(
            plan.sandbox_policy,
            WorkspaceEnvironmentSandboxPolicy::RestrictedRecipe
                | WorkspaceEnvironmentSandboxPolicy::RestrictedPluginStaging
                | WorkspaceEnvironmentSandboxPolicy::RestrictedPluginMounted
        ) {
            ensure_restricted_recipe_sandbox_available()?;
            #[cfg(target_os = "windows")]
            let sandbox_parent = std::env::temp_dir();
            #[cfg(not(target_os = "windows"))]
            let sandbox_parent = Path::new("/tmp");
            if !sandbox_parent.is_dir() {
                return Err(Error::InvalidInput(
                    "restricted command recipes require an available host temporary directory"
                        .to_string(),
                ));
            }
            let sandbox = tempfile::Builder::new()
                .prefix("trail-environment-")
                .tempdir_in(&sandbox_parent)?;
            let sandbox_root = fs::canonicalize(sandbox.path())?;
            let outputs =
                self.execute_workspace_environment_plan_in_directory(plan, &sandbox_root, true)?;
            let mut output_entries = 0usize;
            for output in &outputs {
                self.validate_restricted_recipe_output(plan, output, &mut output_entries)?;
            }
            if let [output] = outputs.as_slice() {
                let destination = build_dir.join("recipe-output");
                if destination.exists() {
                    return Err(Error::Corrupt(format!(
                        "restricted recipe publication destination `{}` already exists",
                        destination.display()
                    )));
                }
                copy_dir_recursive(output, &destination)?;
                return Ok(destination);
            }
            return package_workspace_environment_outputs(build_dir, "recipe-outputs", &outputs);
        }
        let outputs =
            self.execute_workspace_environment_plan_in_directory(plan, build_dir, false)?;
        package_workspace_environment_outputs(build_dir, "published-outputs", &outputs)
    }

    fn validate_restricted_recipe_output(
        &self,
        plan: &WorkspaceEnvironmentPlan,
        output: &Path,
        entries: &mut usize,
    ) -> Result<()> {
        for entry in walkdir::WalkDir::new(output).follow_links(false) {
            let entry = entry.map_err(|err| {
                Error::InvalidInput(format!(
                    "cannot inspect output for command component `{}`: {err}",
                    plan.component_id
                ))
            })?;
            *entries += 1;
            if *entries > 1_000_000 {
                return Err(Error::InvalidInput(format!(
                    "command component `{}` output exceeds one million filesystem entries",
                    plan.component_id
                )));
            }
            let metadata = fs::symlink_metadata(entry.path())?;
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                return Err(Error::InvalidInput(format!(
                    "command component `{}` output contains unsupported symlink `{}`",
                    plan.component_id,
                    entry.path().display()
                )));
            }
            if !metadata.is_dir() && !metadata.is_file() {
                return Err(Error::InvalidInput(format!(
                    "command component `{}` output contains unsupported filesystem entry `{}`",
                    plan.component_id,
                    entry.path().display()
                )));
            }
            #[cfg(unix)]
            if metadata.is_file() {
                use std::os::unix::fs::MetadataExt;
                if metadata.nlink() != 1 {
                    return Err(Error::InvalidInput(format!(
                        "command component `{}` output contains hard-linked file `{}`",
                        plan.component_id,
                        entry.path().display()
                    )));
                }
                if metadata.mode() & 0o6000 != 0 {
                    return Err(Error::InvalidInput(format!(
                        "command component `{}` output contains set-id file `{}`",
                        plan.component_id,
                        entry.path().display()
                    )));
                }
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;

                if metadata.file_attributes() & winapi::um::winnt::FILE_ATTRIBUTE_REPARSE_POINT != 0
                {
                    return Err(Error::InvalidInput(format!(
                        "command component `{}` output contains unsupported reparse point `{}`",
                        plan.component_id,
                        entry.path().display()
                    )));
                }
                if metadata.is_file() && windows_file_identity(entry.path())?.number_of_links != 1 {
                    return Err(Error::InvalidInput(format!(
                        "command component `{}` output contains hard-linked or unverifiable file `{}`",
                        plan.component_id,
                        entry.path().display()
                    )));
                }
            }
        }
        Ok(())
    }

    fn validate_environment_plan_mount_collisions(
        &self,
        plans: &[WorkspaceEnvironmentPlan],
    ) -> Result<()> {
        let mut mounts = BTreeMap::<String, (&str, &str, &str)>::new();
        for plan in plans {
            for output in &plan.outputs {
                let folded = case_insensitive_path_key(&output.mount_path);
                if let Some((_, (component_id, output_name, mount_path))) =
                    environment_path_overlap_in_map(&mounts, &folded)
                {
                    return Err(Error::InvalidInput(format!(
                        "environment component `{}` output `{}` mount `{}` overlaps component `{component_id}` output `{output_name}` mount `{mount_path}`",
                        plan.component_id, output.name, output.mount_path
                    )));
                }
                mounts.insert(
                    folded,
                    (&plan.component_id, &output.name, &output.mount_path),
                );
            }
        }
        Ok(())
    }

    fn execute_workspace_environment_plan_in_directory(
        &self,
        plan: &WorkspaceEnvironmentPlan,
        build_dir: &Path,
        create_output_before_command: bool,
    ) -> Result<Vec<PathBuf>> {
        if let Some((source_root, staging_root)) = &plan.source_projection {
            self.for_each_root_file_chunk(source_root, 1024, |chunk| {
                for (path, entry) in chunk {
                    let staging_path = format!("{staging_root}/{path}");
                    self.materialize_workspace_environment_input(build_dir, &staging_path, &entry)?;
                }
                Ok(())
            })?;
        }
        for input in &plan.inputs {
            self.materialize_workspace_environment_input(
                build_dir,
                &input.staging_path,
                &input.entry,
            )?;
        }
        let mut outputs = Vec::with_capacity(plan.outputs.len());
        for declaration in &plan.outputs {
            let output = safe_join(build_dir, &declaration.output_path)?;
            if create_output_before_command && declaration.create_if_missing && !output.exists() {
                fs::create_dir_all(&output)?;
            }
            outputs.push(output);
        }
        for command in &plan.pre_commands {
            self.run_workspace_environment_command(plan, command, build_dir)?;
        }
        if let Some(command) = &plan.command {
            self.run_workspace_environment_command(plan, command, build_dir)?;
        }
        for (declaration, output) in plan.outputs.iter().zip(&outputs) {
            if declaration.create_if_missing && !output.exists() {
                fs::create_dir_all(output)?;
            }
            if !output.is_dir() {
                return Err(Error::InvalidInput(format!(
                    "environment build for component `{}` did not produce declared output `{}` at `{}`",
                    plan.component_id, declaration.name, declaration.output_path
                )));
            }
        }
        Ok(outputs)
    }

    fn acquire_workspace_environment_cache_uses(
        &self,
        plan: &WorkspaceEnvironmentPlan,
        command: &WorkspaceEnvironmentCommand,
    ) -> Result<Vec<WorkspaceEnvironmentCacheUseGuard>> {
        let by_name = plan
            .caches
            .iter()
            .map(|cache| (cache.name.as_str(), cache))
            .collect::<BTreeMap<_, _>>();
        let mut names = command.cache_names.clone();
        names.sort();
        names.dedup();
        let mut guards = Vec::with_capacity(names.len());
        for name in names {
            let cache = by_name.get(name.as_str()).ok_or_else(|| {
                Error::Corrupt(format!(
                    "component `{}` command lost cache declaration `{name}`",
                    plan.component_id
                ))
            })?;
            if let Ok(metadata) = fs::symlink_metadata(&cache.storage_path) {
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return Err(Error::InvalidInput(format!(
                        "environment cache namespace `{}` is not a real directory",
                        cache.namespace_id
                    )));
                }
            }
            fs::create_dir_all(&cache.storage_path)?;
            let namespace_parent = fs::canonicalize(self.db_dir.join("cache/namespaces"))?;
            let canonical_storage = fs::canonicalize(&cache.storage_path)?;
            if canonical_storage != namespace_parent.join(&cache.namespace_id) {
                return Err(Error::InvalidInput(format!(
                    "environment cache namespace `{}` escapes host cache storage",
                    cache.namespace_id
                )));
            }
            let compatibility_json = serde_json::to_vec(&cache.compatibility)?;
            self.conn.execute(
                "INSERT OR IGNORE INTO environment_cache_namespaces
                 (namespace_id, adapter_identity, cache_name, protocol, access, authority, scope, compatibility_json, storage_path, last_used_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'performance_only', 'workspace', ?6, ?7, ?8, ?8)",
                params![
                    &cache.namespace_id,
                    &plan.adapter_identity,
                    &cache.name,
                    cache.protocol.as_str(),
                    cache.access.as_str(),
                    &compatibility_json,
                    cache.storage_path.to_string_lossy(),
                    now_ts()
                ],
            )?;
            let changed = self.conn.execute(
                "UPDATE environment_cache_namespaces SET last_used_at = ?1
                 WHERE namespace_id = ?2 AND adapter_identity = ?3 AND cache_name = ?4
                   AND protocol = ?5 AND access = ?6 AND authority = 'performance_only'
                   AND scope = 'workspace' AND compatibility_json = ?7 AND storage_path = ?8",
                params![
                    now_ts(),
                    &cache.namespace_id,
                    &plan.adapter_identity,
                    &cache.name,
                    cache.protocol.as_str(),
                    cache.access.as_str(),
                    &compatibility_json,
                    cache.storage_path.to_string_lossy()
                ],
            )?;
            if changed != 1 {
                return Err(Error::Corrupt(format!(
                    "environment cache namespace `{}` has conflicting provenance",
                    cache.namespace_id
                )));
            }

            let token = format!("{}:{}", std::process::id(), current_process_start_token());
            let lease_dir = self
                .db_dir
                .join("cache/namespace-leases")
                .join(&cache.namespace_id);
            fs::create_dir_all(&lease_dir)?;
            let maintenance_dir = self.db_dir.join("cache/namespace-maintenance");
            fs::create_dir_all(&maintenance_dir)?;
            let maintenance_path = maintenance_dir.join(format!("{}.lock", cache.namespace_id));
            let lease_deadline = Instant::now() + Duration::from_secs(300);
            let lease_path = loop {
                if maintenance_path.exists() {
                    if environment_cache_owner_file_is_stale(&maintenance_path)? {
                        let _ = fs::remove_file(&maintenance_path);
                        continue;
                    }
                    if Instant::now() >= lease_deadline {
                        return Err(Error::InvalidInput(format!(
                            "timed out waiting for environment cache maintenance `{}`",
                            cache.name
                        )));
                    }
                    thread::sleep(Duration::from_millis(50));
                    continue;
                }
                let lease_path = lease_dir.join(format!(
                    "lease_{}",
                    crate::ids::short_hash(
                        format!("{token}:{}:{:?}", now_nanos(), thread::current().id()).as_bytes(),
                        24
                    )
                ));
                let mut lease = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&lease_path)?;
                lease.write_all(token.as_bytes())?;
                lease.sync_all()?;
                if maintenance_path.exists() {
                    let _ = fs::remove_file(&lease_path);
                    continue;
                }
                break lease_path;
            };

            let mut use_guard = WorkspaceEnvironmentCacheUseGuard {
                lease_path,
                exclusive: None,
            };
            let exclusive = if cache.access == WorkspaceEnvironmentCacheAccess::HostExclusive {
                let lock_dir = self.db_dir.join("cache/namespace-locks");
                fs::create_dir_all(&lock_dir)?;
                let lock_path = lock_dir.join(format!("{}.lock", cache.namespace_id));
                let deadline = Instant::now() + Duration::from_secs(300);
                loop {
                    match OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&lock_path)
                    {
                        Ok(mut lock) => {
                            lock.write_all(token.as_bytes())?;
                            lock.sync_all()?;
                            break Some((lock_path, token.clone()));
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                            if environment_cache_owner_file_is_stale(&lock_path)? {
                                let stale = lock_path.with_extension(format!(
                                    "stale.{}",
                                    crate::ids::short_hash(
                                        format!("{}:{}", now_nanos(), token).as_bytes(),
                                        16
                                    )
                                ));
                                let _ = fs::rename(&lock_path, stale);
                                continue;
                            }
                            if Instant::now() >= deadline {
                                return Err(Error::InvalidInput(format!(
                                    "timed out waiting for exclusive environment cache `{}`",
                                    cache.name
                                )));
                            }
                            thread::sleep(Duration::from_millis(50));
                        }
                        Err(error) => return Err(Error::Io(error)),
                    }
                }
            } else {
                None
            };
            use_guard.exclusive = exclusive;
            guards.push(use_guard);
        }
        Ok(guards)
    }

    fn run_workspace_environment_command(
        &self,
        plan: &WorkspaceEnvironmentPlan,
        command_plan: &WorkspaceEnvironmentCommand,
        build_dir: &Path,
    ) -> Result<()> {
        let _cache_uses = self.acquire_workspace_environment_cache_uses(plan, command_plan)?;
        let working_directory = safe_join(build_dir, &command_plan.working_directory)?;
        let isolated_home = build_dir.join(".trail-home");
        let isolated_tmp = build_dir.join(".trail-tmp");
        fs::create_dir_all(&working_directory)?;
        fs::create_dir_all(&isolated_home)?;
        fs::create_dir_all(&isolated_tmp)?;
        let current_identity = workspace_tool_identity_for_path(&command_plan.resolved_program)?;
        if current_identity != command_plan.executable_identity {
            return Err(Error::InvalidInput(format!(
                "environment build executable `{}` changed after the component key was computed",
                command_plan.program
            )));
        }
        let (launcher, launcher_args) = match plan.sandbox_policy {
            WorkspaceEnvironmentSandboxPolicy::TrustedBuiltin => (
                command_plan.resolved_program.clone(),
                command_plan.args.iter().map(OsString::from).collect(),
            ),
            WorkspaceEnvironmentSandboxPolicy::RestrictedRecipe
            | WorkspaceEnvironmentSandboxPolicy::RestrictedPluginStaging => self
                .restricted_recipe_launcher(
                    plan,
                    command_plan,
                    build_dir,
                    &isolated_home,
                    &isolated_tmp,
                )?,
            WorkspaceEnvironmentSandboxPolicy::RestrictedPluginMounted => self
                .restricted_recipe_launcher(
                    plan,
                    command_plan,
                    build_dir,
                    &isolated_home,
                    &isolated_tmp,
                )?,
        };
        let mut command = Command::new(launcher);
        command
            .args(launcher_args)
            .current_dir(&working_directory)
            .env_clear()
            .envs(&command_plan.environment)
            .env("HOME", &isolated_home)
            .env("TMPDIR", &isolated_tmp)
            .env("TMP", &isolated_tmp)
            .env("TEMP", &isolated_tmp);
        if let Some(path) = std::env::var_os("PATH") {
            command.env("PATH", path);
        }
        #[cfg(windows)]
        for name in ["SystemRoot", "ComSpec", "PATHEXT"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        for name in &command_plan.remove_environment {
            command.env_remove(name);
        }
        let status = command.status().map_err(|err| {
            Error::InvalidInput(format!(
                "failed to launch `{}` for component `{}`: {err}",
                command_plan.program, plan.component_id
            ))
        })?;
        if !status.success() {
            return Err(Error::InvalidInput(format!(
                "environment build for component `{}` failed with {status}",
                plan.component_id
            )));
        }
        Ok(())
    }

    fn restricted_recipe_launcher(
        &self,
        plan: &WorkspaceEnvironmentPlan,
        command_plan: &WorkspaceEnvironmentCommand,
        build_dir: &Path,
        isolated_home: &Path,
        isolated_tmp: &Path,
    ) -> Result<(PathBuf, Vec<OsString>)> {
        let cache_paths = command_plan
            .cache_names
            .iter()
            .map(|name| {
                plan.caches
                    .iter()
                    .find(|cache| &cache.name == name)
                    .map(|cache| cache.storage_path.clone())
                    .ok_or_else(|| {
                        Error::Corrupt(format!(
                            "component `{}` lost cache declaration `{name}` before sandbox launch",
                            plan.component_id
                        ))
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        #[cfg(target_os = "macos")]
        {
            let launcher = PathBuf::from("/usr/bin/sandbox-exec");
            if !launcher.is_file() {
                return Err(Error::InvalidInput(
                    "restricted command recipes require `/usr/bin/sandbox-exec` on macOS"
                        .to_string(),
                ));
            }
            let build_dir = fs::canonicalize(build_dir)?;
            let outputs = plan
                .outputs
                .iter()
                .map(|output| -> Result<PathBuf> {
                    let path = safe_join(&build_dir, &output.output_path)?;
                    Ok(fs::canonicalize(path)?)
                })
                .collect::<Result<Vec<_>>>()?;
            let caches = cache_paths
                .iter()
                .map(fs::canonicalize)
                .collect::<std::io::Result<Vec<_>>>()?;
            let readable_inputs = plan
                .inputs
                .iter()
                .map(|input| -> Result<PathBuf> {
                    Ok(fs::canonicalize(safe_join(
                        &build_dir,
                        &input.staging_path,
                    )?)?)
                })
                .collect::<Result<Vec<_>>>()?;
            let working_directory = if command_plan.working_directory.is_empty() {
                build_dir.clone()
            } else {
                fs::canonicalize(safe_join(&build_dir, &command_plan.working_directory)?)?
            };
            let isolated_home = fs::canonicalize(isolated_home)?;
            let isolated_tmp = fs::canonicalize(isolated_tmp)?;
            let executable = command_plan.resolved_program.clone();
            let canonical_executable = fs::canonicalize(&command_plan.resolved_program)?;
            let executable_rules = if canonical_executable == executable {
                format!("(literal \"{}\")", sandbox_profile_escape(&executable))
            } else {
                format!(
                    "(literal \"{}\") (literal \"{}\")",
                    sandbox_profile_escape(&executable),
                    sandbox_profile_escape(&canonical_executable)
                )
            };
            let output_rules = outputs
                .iter()
                .chain(&caches)
                .map(|output| format!("(subpath \"{}\")", sandbox_profile_escape(output)))
                .collect::<Vec<_>>()
                .join(" ");
            let input_rules = readable_inputs
                .iter()
                .map(|input| format!("(literal \"{}\")", sandbox_profile_escape(input)))
                .collect::<Vec<_>>()
                .join(" ");
            let profile = format!(
                "(version 1)\n\
                 (deny default)\n\
                 (import \"system.sb\")\n\
                 (deny mach-lookup)\n\
                 (deny mach-register)\n\
                 (deny process-fork)\n\
                 (deny process-exec)\n\
                 (allow process-exec {})\n\
                 (deny file-write*)\n\
                 (allow file-write* {} (subpath \"{}\") (subpath \"{}\"))\n\
                 (deny file-read* (subpath \"/Users\") (subpath \"/Volumes\") (subpath \"/private/etc\") (subpath \"/private/var\"))\n\
                 (allow file-read-metadata (subpath \"{}\"))\n\
                 (allow file-read* (literal \"{}\") {} {} (subpath \"{}\") (subpath \"{}\") (literal \"{}\") (literal \"{}\") (subpath \"/bin\") (subpath \"/usr\") (subpath \"/System\") (subpath \"/Library\") (subpath \"/opt/homebrew\") (subpath \"/nix/store\"))\n\
                 (deny network*)",
                executable_rules,
                output_rules,
                sandbox_profile_escape(&isolated_home),
                sandbox_profile_escape(&isolated_tmp),
                sandbox_profile_escape(&build_dir),
                sandbox_profile_escape(&working_directory),
                input_rules,
                output_rules,
                sandbox_profile_escape(&isolated_home),
                sandbox_profile_escape(&isolated_tmp),
                sandbox_profile_escape(&executable),
                sandbox_profile_escape(&canonical_executable),
            );
            let mut args = vec![
                OsString::from("-p"),
                OsString::from(profile),
                executable.as_os_str().to_owned(),
            ];
            args.extend(command_plan.args.iter().map(OsString::from));
            Ok((launcher, args))
        }
        #[cfg(all(target_os = "linux", not(test)))]
        {
            let launcher = std::env::current_exe()?;
            let build_dir = fs::canonicalize(build_dir)?;
            let outputs = plan
                .outputs
                .iter()
                .map(|output| -> Result<PathBuf> {
                    let path = safe_join(&build_dir, &output.output_path)?;
                    Ok(fs::canonicalize(path)?)
                })
                .collect::<Result<Vec<_>>>()?;
            let caches = cache_paths
                .iter()
                .map(fs::canonicalize)
                .collect::<std::io::Result<Vec<_>>>()?;
            let readable_inputs = plan
                .inputs
                .iter()
                .map(|input| -> Result<PathBuf> {
                    Ok(fs::canonicalize(safe_join(
                        &build_dir,
                        &input.staging_path,
                    )?)?)
                })
                .collect::<Result<Vec<_>>>()?;
            let isolated_home = fs::canonicalize(isolated_home)?;
            let isolated_tmp = fs::canonicalize(isolated_tmp)?;
            let executable = fs::canonicalize(&command_plan.resolved_program)?;
            let mut args = vec![
                OsString::from("__environment-sandbox"),
                OsString::from("--root"),
                build_dir.into_os_string(),
            ];
            for input in readable_inputs {
                args.push(OsString::from("--read"));
                args.push(input.into_os_string());
            }
            for output in outputs {
                args.push(OsString::from("--output"));
                args.push(output.into_os_string());
            }
            for cache in caches {
                args.push(OsString::from("--cache"));
                args.push(cache.into_os_string());
            }
            args.extend([
                OsString::from("--home"),
                isolated_home.into_os_string(),
                OsString::from("--tmp"),
                isolated_tmp.into_os_string(),
                OsString::from("--program"),
                executable.into_os_string(),
                OsString::from("--"),
            ]);
            args.extend(command_plan.args.iter().map(OsString::from));
            Ok((launcher, args))
        }
        #[cfg(all(target_os = "windows", not(test)))]
        {
            let launcher = std::env::current_exe()?;
            let build_dir = fs::canonicalize(build_dir)?;
            let outputs = plan
                .outputs
                .iter()
                .map(|output| -> Result<PathBuf> {
                    let path = safe_join(&build_dir, &output.output_path)?;
                    Ok(fs::canonicalize(path)?)
                })
                .collect::<Result<Vec<_>>>()?;
            let caches = cache_paths
                .iter()
                .map(fs::canonicalize)
                .collect::<std::io::Result<Vec<_>>>()?;
            let readable_inputs = plan
                .inputs
                .iter()
                .map(|input| -> Result<PathBuf> {
                    Ok(fs::canonicalize(safe_join(
                        &build_dir,
                        &input.staging_path,
                    )?)?)
                })
                .collect::<Result<Vec<_>>>()?;
            let isolated_home = fs::canonicalize(isolated_home)?;
            let isolated_tmp = fs::canonicalize(isolated_tmp)?;
            let executable = fs::canonicalize(&command_plan.resolved_program)?;
            let mut args = vec![
                OsString::from("__environment-sandbox"),
                OsString::from("--root"),
                build_dir.into_os_string(),
            ];
            for input in readable_inputs {
                args.push(OsString::from("--read"));
                args.push(input.into_os_string());
            }
            for output in outputs {
                args.push(OsString::from("--output"));
                args.push(output.into_os_string());
            }
            for cache in caches {
                args.push(OsString::from("--cache"));
                args.push(cache.into_os_string());
            }
            args.extend([
                OsString::from("--home"),
                isolated_home.into_os_string(),
                OsString::from("--tmp"),
                isolated_tmp.into_os_string(),
                OsString::from("--program"),
                executable.into_os_string(),
                OsString::from("--"),
            ]);
            args.extend(command_plan.args.iter().map(OsString::from));
            Ok((launcher, args))
        }
        #[cfg(any(
            all(target_os = "linux", test),
            all(target_os = "windows", test),
            not(any(target_os = "macos", target_os = "linux", target_os = "windows"))
        ))]
        {
            let _ = (
                plan,
                command_plan,
                build_dir,
                isolated_home,
                isolated_tmp,
                cache_paths,
            );
            Err(Error::InvalidInput(format!(
                "restricted command recipe sandboxing is unavailable on {}; Trail refuses to run the repository command without kernel enforcement",
                std::env::consts::OS
            )))
        }
    }

    fn materialize_workspace_environment_input(
        &self,
        build_dir: &Path,
        staging_path: &str,
        entry: &FileEntry,
    ) -> Result<()> {
        let destination = safe_join(build_dir, staging_path)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let projection = self.project_entry_file(entry)?;
        fs::copy(projection, &destination)?;
        let mut permissions = fs::metadata(&destination)?.permissions();
        permissions.set_readonly(false);
        #[cfg(unix)]
        permissions.set_mode(if entry.executable {
            0o755
        } else {
            entry.mode & 0o666
        });
        fs::set_permissions(&destination, permissions)?;
        // Mounted immutable source entries deliberately expose a stable epoch
        // mtime. Match it in staging so build systems such as Cargo do not
        // reject a target seed solely because host projection happened later.
        OpenOptions::new()
            .write(true)
            .open(&destination)?
            .set_modified(SystemTime::UNIX_EPOCH)?;
        Ok(())
    }

    pub fn workspace_environment_status(
        &self,
        lane: &str,
    ) -> Result<Vec<WorkspaceEnvironmentReport>> {
        self.workspace_environment_rows(lane)
    }

    /// Component-oriented status surface. Unlike the legacy dependency report,
    /// this keeps the logical component and versioned adapter identities
    /// separate so multiple components can use the same adapter safely.
    pub fn environment_component_status(
        &self,
        lane: &str,
    ) -> Result<Vec<EnvironmentComponentStateReport>> {
        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered workspace view"
            ))
        })?;
        let mut stmt = self.conn.prepare(
            "SELECT view_id, component_id, adapter_identity, adapter_version, implementation_version, distribution_digest, kind, expected_key, attached_key, status, reason, updated_at FROM environment_component_states WHERE view_id = ?1 ORDER BY component_id",
        )?;
        let reports = stmt
            .query_map(params![view.view_id], |row| {
                let component_id = row.get::<_, String>(1)?;
                let adapter_identity = row.get::<_, String>(2)?;
                let adapter_version = row.get::<_, u32>(3)?;
                let (namespace, name, contract_major) =
                    split_adapter_identity(&adapter_identity, adapter_version);
                Ok(EnvironmentComponentStateReport {
                    view_id: row.get(0)?,
                    component: EnvironmentComponentIdentityReport {
                        component_id,
                        kind: row.get(6)?,
                    },
                    adapter: EnvironmentAdapterIdentityReport {
                        namespace,
                        name,
                        contract_major,
                        implementation_version: row.get(4)?,
                        distribution_digest: row.get(5)?,
                    },
                    expected_key: row.get(7)?,
                    attached_key: row.get(8)?,
                    status: row.get(9)?,
                    reason: row.get(10)?,
                    updated_at: row.get(11)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(reports)
    }

    pub fn active_environment_generation(
        &self,
        lane: &str,
    ) -> Result<Option<EnvironmentGenerationReport>> {
        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered workspace view"
            ))
        })?;
        let generation = self
            .conn
            .query_row(
                "SELECT g.generation_id, g.view_id, g.generation_sequence, g.source_root,
                        g.specification_digest, g.predecessor_generation_id, g.state,
                        g.created_at, g.activated_at, g.retired_at
                 FROM environment_view_generations a
                 JOIN environment_generations g ON g.generation_id = a.generation_id
                 WHERE a.view_id = ?1",
                params![&view.view_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, u64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, Option<i64>>(8)?,
                        row.get::<_, Option<i64>>(9)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            generation_id,
            view_id,
            generation_sequence,
            source_root,
            specification_digest,
            predecessor_generation_id,
            state,
            created_at,
            activated_at,
            retired_at,
        )) = generation
        else {
            return Ok(None);
        };
        let mut stmt = self.conn.prepare(
            "SELECT component_id, adapter_identity, kind, component_key, layer_id, mount_path
             FROM environment_generation_components
             WHERE generation_id = ?1 ORDER BY component_id",
        )?;
        let mut components = stmt
            .query_map(params![&generation_id], |row| {
                Ok(EnvironmentGenerationComponentReport {
                    component_id: row.get(0)?,
                    adapter_identity: row.get(1)?,
                    kind: row.get(2)?,
                    component_key: row.get(3)?,
                    layer_id: row.get(4)?,
                    mount_path: row.get(5)?,
                    dependencies: Vec::new(),
                    outputs: Vec::new(),
                    caches: Vec::new(),
                    external_artifacts: Vec::new(),
                    runtime_resources: Vec::new(),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for component in &mut components {
            let mut dependency_stmt = self.conn.prepare(
                "SELECT dependency_component_id, dependency_component_key, edge_type
                 FROM environment_generation_edges
                 WHERE generation_id = ?1 AND component_id = ?2
                 ORDER BY dependency_component_id",
            )?;
            component.dependencies = dependency_stmt
                .query_map(params![&generation_id, &component.component_id], |row| {
                    Ok(EnvironmentGenerationDependencyReport {
                        component_id: row.get(0)?,
                        component_key: row.get(1)?,
                        edge_type: row.get(2)?,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let mut output_stmt = self.conn.prepare(
                "SELECT output_name, policy, storage_identity, layer_id, mount_path, layer_subpath
                 FROM environment_generation_outputs
                 WHERE generation_id = ?1 AND component_id = ?2
                 ORDER BY output_name",
            )?;
            component.outputs = output_stmt
                .query_map(params![&generation_id, &component.component_id], |row| {
                    Ok(EnvironmentGenerationOutputReport {
                        name: row.get(0)?,
                        policy: row.get(1)?,
                        storage_identity: row.get(2)?,
                        layer_id: row.get(3)?,
                        mount_path: row.get(4)?,
                        layer_subpath: row.get(5)?,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let mut cache_stmt = self.conn.prepare(
                "SELECT cache_name, namespace_id, protocol, access, compatibility_json
                 FROM environment_generation_caches
                 WHERE generation_id = ?1 AND component_id = ?2
                 ORDER BY cache_name",
            )?;
            component.caches = cache_stmt
                .query_map(params![&generation_id, &component.component_id], |row| {
                    let compatibility = row.get::<_, Vec<u8>>(4)?;
                    let compatibility =
                        serde_json::from_slice(&compatibility).map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                4,
                                rusqlite::types::Type::Blob,
                                Box::new(error),
                            )
                        })?;
                    Ok(EnvironmentCacheReport {
                        name: row.get(0)?,
                        namespace_id: row.get(1)?,
                        protocol: row.get(2)?,
                        access: row.get(3)?,
                        authority: "performance_only".to_string(),
                        scope: "workspace".to_string(),
                        compatibility,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let mut external_stmt = self.conn.prepare(
                "SELECT artifact_name, artifact_type, provider, reference, digest, platform, cleanup_owner
                 FROM environment_generation_external_artifacts
                 WHERE generation_id = ?1 AND component_id = ?2
                 ORDER BY artifact_name",
            )?;
            component.external_artifacts = external_stmt
                .query_map(params![&generation_id, &component.component_id], |row| {
                    Ok(EnvironmentExternalArtifactReport {
                        name: row.get(0)?,
                        artifact_type: row.get(1)?,
                        provider: row.get(2)?,
                        reference: row.get(3)?,
                        digest: row.get(4)?,
                        platform: row.get(5)?,
                        cleanup_owner: row.get(6)?,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let mut runtime_stmt = self.conn.prepare(
                "SELECT resource_name, runtime_type, provider, artifact_name,
                        image_reference, image_digest, image_platform, container_port,
                        protocol, health_type, health_timeout_ms, restart_policy,
                        cleanup_owner, volume_target, allocation_id, provider_resource_id,
                        container_name, network_name, volume_name, host_port, status,
                        health_status, reason, created_at, updated_at, started_at, stopped_at
                 FROM environment_generation_runtime_resources
                 WHERE generation_id = ?1 AND component_id = ?2
                 ORDER BY resource_name",
            )?;
            component.runtime_resources = runtime_stmt
                .query_map(params![&generation_id, &component.component_id], |row| {
                    Ok(EnvironmentRuntimeResourceReport {
                        declaration: EnvironmentRuntimeDeclarationReport {
                            name: row.get(0)?,
                            runtime_type: row.get(1)?,
                            provider: row.get(2)?,
                            artifact_name: row.get(3)?,
                            container_port: row.get(7)?,
                            protocol: row.get(8)?,
                            health_type: row.get(9)?,
                            health_timeout_ms: row.get(10)?,
                            restart_policy: row.get(11)?,
                            cleanup_owner: row.get(12)?,
                            volume_target: row.get(13)?,
                            secrets: Vec::new(),
                        },
                        image_reference: row.get(4)?,
                        image_digest: row.get(5)?,
                        image_platform: row.get(6)?,
                        allocation_id: row.get(14)?,
                        provider_resource_id: row.get(15)?,
                        container_name: row.get(16)?,
                        network_name: row.get(17)?,
                        volume_name: row.get(18)?,
                        host_port: row.get(19)?,
                        status: row.get(20)?,
                        health_status: row.get(21)?,
                        reason: row.get(22)?,
                        created_at: row.get(23)?,
                        updated_at: row.get(24)?,
                        started_at: row.get(25)?,
                        stopped_at: row.get(26)?,
                        secret_statuses: Vec::new(),
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            for resource in &mut component.runtime_resources {
                let mut secret_stmt = self.conn.prepare(
                    "SELECT secret_name, provider, reference, version, purpose, injection,
                            target, environment, required, status, reason, resolved_at, updated_at
                     FROM environment_generation_runtime_secrets
                     WHERE generation_id = ?1 AND component_id = ?2 AND resource_name = ?3
                     ORDER BY secret_name",
                )?;
                let statuses = secret_stmt
                    .query_map(
                        params![
                            &generation_id,
                            &component.component_id,
                            &resource.declaration.name
                        ],
                        |row| {
                            let reference = EnvironmentSecretReferenceReport {
                                name: row.get(0)?,
                                provider: row.get(1)?,
                                reference: row.get(2)?,
                                version: row.get(3)?,
                                purpose: row.get(4)?,
                                injection: row.get(5)?,
                                target: row.get(6)?,
                                environment: row.get(7)?,
                                required: row.get(8)?,
                            };
                            Ok(EnvironmentSecretStatusReport {
                                reference,
                                status: row.get(9)?,
                                reason: row.get(10)?,
                                resolved_at: row.get(11)?,
                                updated_at: row.get(12)?,
                            })
                        },
                    )?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                resource.declaration.secrets = statuses
                    .iter()
                    .map(|status| status.reference.clone())
                    .collect();
                resource.secret_statuses = statuses;
            }
        }
        Ok(Some(EnvironmentGenerationReport {
            generation_id,
            view_id,
            generation_sequence,
            source_root: ObjectId(source_root),
            specification_digest,
            predecessor_generation_id,
            state,
            components,
            created_at,
            activated_at,
            retired_at,
        }))
    }

    pub(crate) fn workspace_environment_rows(
        &self,
        lane: &str,
    ) -> Result<Vec<WorkspaceEnvironmentReport>> {
        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered workspace view"
            ))
        })?;
        let mut stmt = self.conn.prepare(
            "SELECT view_id, adapter, expected_key, attached_key, status, reason, updated_at FROM workspace_environment_states WHERE view_id = ?1 ORDER BY adapter",
        )?;
        let reports = stmt
            .query_map(params![view.view_id], |row| {
                Ok(WorkspaceEnvironmentReport {
                    view_id: row.get(0)?,
                    adapter: row.get(1)?,
                    expected_key: row.get(2)?,
                    attached_key: row.get(3)?,
                    status: row.get(4)?,
                    reason: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(reports)
    }

    pub fn explain_workspace_environment_staleness(
        &self,
        lane: &str,
        component_id: &str,
    ) -> Result<EnvironmentStaleExplanationReport> {
        self.explain_workspace_environment_staleness_page(lane, component_id, 0, 256)
    }

    pub fn explain_workspace_environment_staleness_page(
        &self,
        lane: &str,
        component_id: &str,
        offset: u64,
        limit: u64,
    ) -> Result<EnvironmentStaleExplanationReport> {
        if limit == 0 || limit > 1_000 {
            return Err(Error::InvalidInput(
                "environment explanation limit must be between 1 and 1000".to_string(),
            ));
        }
        let state = self
            .environment_component_status(lane)?
            .into_iter()
            .find(|state| state.component.component_id == component_id)
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "environment component `{component_id}` is not recorded for lane `{lane}`"
                ))
            })?;
        let branch = self.lane_branch(lane)?;
        let head = self.get_ref(&branch.ref_name)?;
        let discovery = self.discover_workspace_environment(lane, None)?;
        if !discovery.conflicts.is_empty() {
            return Err(Error::InvalidInput(format!(
                "environment discovery found {} conflict(s) while explaining staleness",
                discovery.conflicts.len()
            )));
        }
        let Some(component) = discovery
            .components
            .iter()
            .find(|component| component.component_id == component_id)
        else {
            return paginate_stale_explanation(
                component_id,
                "stale",
                state.expected_key,
                state.attached_key,
                true,
                vec![EnvironmentStaleChangeReport {
                    dimension: "component".to_string(),
                    name: component_id.to_string(),
                    change: "removed_or_adapter_unavailable".to_string(),
                }],
                offset,
                limit,
            );
        };
        let finalized =
            self.plan_discovered_environment_graph(&head.root_id, &discovery.components)?;
        let (plan, expected_key) = finalized
            .into_iter()
            .find(|(plan, _)| plan.component_id == component.component_id)
            .ok_or_else(|| {
                Error::Corrupt(format!(
                    "environment graph lost discovered component `{component_id}`"
                ))
            })?;
        let (complete, changes) = match state.attached_key.as_deref() {
            Some(attached_key) if attached_key == expected_key => (true, Vec::new()),
            Some(attached_key) => match self.workspace_layer_key_by_cache_key(attached_key)? {
                Some(previous) => (true, diff_workspace_layer_keys(&previous, &plan.layer_key)),
                None => (
                    false,
                    vec![EnvironmentStaleChangeReport {
                        dimension: "provenance".to_string(),
                        name: "canonical_layer_key".to_string(),
                        change: "unavailable_for_legacy_or_missing_layer".to_string(),
                    }],
                ),
            },
            None => (
                true,
                vec![EnvironmentStaleChangeReport {
                    dimension: "attachment".to_string(),
                    name: component_id.to_string(),
                    change: "not_attached".to_string(),
                }],
            ),
        };
        let status = if state.attached_key.as_deref() == Some(expected_key.as_str()) {
            "ready".to_string()
        } else {
            "stale".to_string()
        };
        paginate_stale_explanation(
            component_id,
            &status,
            expected_key,
            state.attached_key,
            complete,
            changes,
            offset,
            limit,
        )
    }

    pub fn refresh_workspace_environment_staleness(&self, lane: &str) -> Result<()> {
        let branch = self.lane_branch(lane)?;
        let head = self.get_ref(&branch.ref_name)?;
        let discovery = self.discover_workspace_environment(lane, None)?;
        if !discovery.conflicts.is_empty() {
            return Err(Error::InvalidInput(format!(
                "environment discovery found {} conflict(s) while refreshing staleness",
                discovery.conflicts.len()
            )));
        }
        let discovered = discovery
            .components
            .iter()
            .map(|component| (component.component_id.clone(), component))
            .collect::<BTreeMap<_, _>>();
        let finalized =
            self.plan_discovered_environment_graph(&head.root_id, &discovery.components)?;
        let mut finalized = finalized
            .into_iter()
            .map(|(plan, key)| (plan.component_id.clone(), (plan, key)))
            .collect::<BTreeMap<_, _>>();
        let mut planned_states = Vec::new();
        for state in self.environment_component_status(lane)? {
            let component_id = state.component.component_id;
            if !discovered.contains_key(&component_id) {
                self.mark_environment_component_stale_without_plan(
                    lane,
                    &component_id,
                    "component is no longer discovered or its adapter is not installed",
                )?;
                continue;
            }
            let (plan, expected_key) = finalized.remove(&component_id).ok_or_else(|| {
                Error::Corrupt(format!(
                    "environment graph lost discovered component `{component_id}`"
                ))
            })?;
            planned_states.push((state.attached_key, plan, expected_key));
        }
        for (attached_key, plan, expected_key) in planned_states {
            let ready = attached_key.as_deref() == Some(expected_key.as_str());
            let exact_reason = if ready {
                None
            } else if let Some(attached_key) = attached_key.as_deref() {
                self.workspace_layer_key_by_cache_key(attached_key)?
                    .map(|previous| {
                        format_stale_changes(&diff_workspace_layer_keys(&previous, &plan.layer_key))
                    })
                    .filter(|reason| !reason.is_empty())
            } else {
                Some("component has no attached environment artifact".to_string())
            };
            let reason = (!ready).then(|| {
                exact_reason
                    .as_deref()
                    .unwrap_or(plan.stale_reason.as_str())
            });
            self.set_workspace_environment_state(
                lane,
                &plan,
                &expected_key,
                attached_key.as_deref(),
                if ready { "ready" } else { "stale" },
                reason,
            )?;
        }
        Ok(())
    }

    fn plan_discovered_environment_component(
        &self,
        source_root: &ObjectId,
        component: &EnvironmentDiscoveredComponentReport,
    ) -> Result<WorkspaceEnvironmentPlan> {
        if component.adapter_identity
            == super::workspace_recipe::COMMAND_RECIPE_ADAPTER_METADATA.canonical_identity
        {
            let plan = self.command_recipe_plan(source_root, &component.component_id)?;
            self.validate_command_recipe_plan(&component.component_id, &plan)?;
            return Ok(plan);
        }
        if let Some(adapter) = builtin_environment_adapter_for_selector(&component.adapter_identity)
        {
            let plan = adapter.plan(self, source_root, &component.component_root)?;
            self.validate_workspace_environment_plan(adapter, &component.component_root, &plan)?;
            return Ok(plan);
        }
        let plugin = self
            .environment_plugin_for_selector(&component.adapter_identity)?
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "adapter `{}` is no longer installed",
                    component.adapter_identity
                ))
            })?;
        let plan = self.plan_environment_plugin_component(
            &plugin,
            source_root,
            &component.component_root,
            &component.component_id,
        )?;
        self.validate_environment_plugin_plan(&plugin, &component.component_id, &plan)?;
        Ok(plan)
    }

    fn plan_discovered_environment_components(
        &self,
        source_root: &ObjectId,
        components: &[EnvironmentDiscoveredComponentReport],
    ) -> Result<Vec<WorkspaceEnvironmentPlan>> {
        let recipe_identity =
            super::workspace_recipe::COMMAND_RECIPE_ADAPTER_METADATA.canonical_identity;
        let recipe_ids = components
            .iter()
            .filter(|component| component.adapter_identity == recipe_identity)
            .map(|component| component.component_id.clone())
            .collect::<BTreeSet<_>>();
        let mut recipe_plans = if recipe_ids.is_empty() {
            BTreeMap::new()
        } else {
            self.command_recipe_plans(source_root, &recipe_ids)?
        };
        let mut tool_identities = BTreeMap::new();
        let mut plans = Vec::with_capacity(components.len());
        for component in components {
            let plan = if component.adapter_identity == recipe_identity {
                let plan = recipe_plans
                    .remove(&component.component_id)
                    .ok_or_else(|| {
                        Error::Corrupt(format!(
                            "batch recipe planner lost component `{}`",
                            component.component_id
                        ))
                    })?;
                self.validate_command_recipe_plan_with_tool_cache(
                    &component.component_id,
                    &plan,
                    &mut tool_identities,
                )?;
                plan
            } else {
                self.plan_discovered_environment_component(source_root, component)?
            };
            plans.push(plan);
        }
        Ok(plans)
    }

    pub(super) fn plan_discovered_environment_graph(
        &self,
        source_root: &ObjectId,
        components: &[EnvironmentDiscoveredComponentReport],
    ) -> Result<Vec<(WorkspaceEnvironmentPlan, String)>> {
        let raw_plans = self.plan_discovered_environment_components(source_root, components)?;
        self.validate_environment_plan_mount_collisions(&raw_plans)?;
        self.validate_environment_mounts_do_not_shadow_source(
            source_root,
            &raw_plans.iter().collect::<Vec<_>>(),
        )?;
        self.finalize_workspace_environment_plan_graph(raw_plans)
    }

    fn mark_environment_component_stale_without_plan(
        &self,
        lane: &str,
        component_id: &str,
        reason: &str,
    ) -> Result<()> {
        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered workspace view"
            ))
        })?;
        let changed = self.conn.execute(
            "UPDATE environment_component_states SET status = 'stale', reason = ?1, updated_at = ?2 WHERE view_id = ?3 AND component_id = ?4",
            params![reason, now_ts(), &view.view_id, component_id],
        )?;
        if changed != 1 {
            return Err(Error::Corrupt(format!(
                "environment component `{component_id}` disappeared during stale refresh"
            )));
        }
        self.conn.execute(
            "UPDATE workspace_environment_states SET status = 'stale', reason = ?1, updated_at = ?2 WHERE view_id = ?3 AND adapter = ?4",
            params![reason, now_ts(), &view.view_id, component_id],
        )?;
        Ok(())
    }

    pub(crate) fn set_workspace_environment_state(
        &self,
        lane: &str,
        plan: &WorkspaceEnvironmentPlan,
        expected_key: &str,
        attached_key: Option<&str>,
        status: &str,
        reason: Option<&str>,
    ) -> Result<()> {
        if !matches!(status, "building" | "ready" | "stale" | "failed") {
            return Err(Error::InvalidInput(format!(
                "invalid workspace environment status `{status}`"
            )));
        }
        let view = self.lane_workspace_view(lane)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{lane}` does not have a layered workspace view"
            ))
        })?;
        // Keep the legacy projection and normalized component state in one
        // SQLite savepoint. Readers can therefore never observe one identity
        // model advancing without the other.
        self.conn
            .execute_batch("SAVEPOINT trail_environment_state")?;
        let writes = (|| -> Result<()> {
            self.conn.execute(
                "INSERT OR REPLACE INTO workspace_environment_states (view_id, adapter, expected_key, attached_key, status, reason, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    view.view_id,
                    plan.component_id,
                    expected_key,
                    attached_key,
                    status,
                    reason,
                    now_ts()
                ],
            )?;
            self.conn.execute(
            "INSERT OR REPLACE INTO environment_component_states (view_id, component_id, adapter_identity, adapter_version, implementation_version, distribution_digest, kind, expected_key, attached_key, status, reason, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                view.view_id,
                plan.component_id,
                plan.adapter_identity,
                plan.adapter_version,
                plan.implementation_version,
                plan.distribution_digest,
                plan.kind,
                expected_key,
                attached_key,
                status,
                reason,
                now_ts()
            ],
            )?;
            Ok(())
        })();
        match writes {
            Ok(()) => {
                self.conn
                    .execute_batch("RELEASE SAVEPOINT trail_environment_state")?;
                Ok(())
            }
            Err(err) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO SAVEPOINT trail_environment_state; RELEASE SAVEPOINT trail_environment_state",
                );
                Err(err)
            }
        }
    }
}

fn run_supervised_mounted_plugin_process(
    command: &mut Command,
) -> Result<std::process::ExitStatus> {
    let mut child = command.spawn()?;
    let child_pid = child.id();
    let child_start_token = match process_start_token(child_pid) {
        Some(token) => token,
        None => {
            if let Some(status) = child.try_wait()? {
                return Ok(status);
            }
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::InvalidInput(format!(
                "cannot authenticate mounted plugin process {child_pid} for parent-death supervision"
            )));
        }
    };
    let mut watchdog = match Command::new(std::env::current_exe()?)
        .arg("__process-watchdog")
        .arg(std::process::id().to_string())
        .arg(child_pid.to_string())
        .arg(&child_start_token)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(watchdog) => watchdog,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Io(error));
        }
    };
    loop {
        if let Some(status) = child.try_wait()? {
            let watchdog_status = watchdog.wait()?;
            if !watchdog_status.success() {
                return Err(Error::InvalidInput(format!(
                    "mounted plugin process watchdog failed with {watchdog_status}"
                )));
            }
            return Ok(status);
        }
        if let Some(watchdog_status) = watchdog.try_wait()? {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::InvalidInput(format!(
                "mounted plugin process watchdog exited early with {watchdog_status}"
            )));
        }
        thread::sleep(Duration::from_millis(20));
    }
}

pub(crate) fn workspace_mounted_commands_identity(
    commands: &[WorkspaceEnvironmentCommand],
) -> Result<String> {
    let commands = commands
        .iter()
        .map(|command| {
            let mut remove_environment = command.remove_environment.clone();
            remove_environment.sort();
            serde_json::json!({
                "program": command.program,
                "resolved_program": command.resolved_program.to_string_lossy(),
                "executable_identity": command.executable_identity,
                "args": command.args,
                "working_directory": command.working_directory,
                "environment": command.environment,
                "remove_environment": remove_environment,
            })
        })
        .collect::<Vec<_>>();
    Ok(sha256_hex(&serde_json::to_vec(&commands)?))
}

fn environment_upper_class(kind: &str) -> Result<ViewPathClass> {
    match kind {
        "dependency" => Ok(ViewPathClass::Dependency),
        "compiler-results" | "generated" | "build" => Ok(ViewPathClass::Generated),
        other => Err(Error::InvalidInput(format!(
            "environment kind `{other}` cannot own a mounted writable output"
        ))),
    }
}

fn validate_mounted_environment_candidate_writes(
    layout: &ViewUpperLayout,
    persisted_mounts: &[(ViewPathClass, String)],
) -> Result<()> {
    let generated_mounts = persisted_mounts
        .iter()
        .filter_map(|(class, mount)| {
            matches!(class, ViewPathClass::Dependency | ViewPathClass::Generated)
                .then_some(mount.as_str())
        })
        .collect::<Vec<_>>();
    for (label, root, allowed, ignore_internal) in [
        (
            "source",
            layout.source_upper.as_path(),
            Vec::<&str>::new(),
            true,
        ),
        (
            "generated",
            layout.generated_upper.as_path(),
            generated_mounts,
            false,
        ),
        (
            "scratch",
            layout.scratch_upper.as_path(),
            Vec::<&str>::new(),
            false,
        ),
    ] {
        for entry in walkdir::WalkDir::new(root).follow_links(false) {
            let entry = entry.map_err(|err| {
                Error::InvalidInput(format!(
                    "cannot inspect mounted initialization {label} upper: {err}"
                ))
            })?;
            if entry.path() == root {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(|_| Error::Corrupt("candidate upper escaped its root".to_string()))?
                .to_string_lossy()
                .replace('\\', "/");
            if ignore_internal && (relative == ".trail" || relative.starts_with(".trail/")) {
                continue;
            }
            if allowed
                .iter()
                .any(|mount| environment_mounts_overlap(&relative, mount))
            {
                continue;
            }
            return Err(Error::InvalidInput(format!(
                "mounted environment initialization wrote undeclared {label} path `{relative}`; only newly prepared writable-private outputs may persist"
            )));
        }
    }
    Ok(())
}

fn split_adapter_identity(identity: &str, fallback_major: u32) -> (String, String, u32) {
    let Some((namespace, remainder)) = identity.split_once('/') else {
        return ("legacy".to_string(), identity.to_string(), fallback_major);
    };
    let Some((name, major)) = remainder.rsplit_once('@') else {
        return (namespace.to_string(), remainder.to_string(), fallback_major);
    };
    (
        namespace.to_string(),
        name.to_string(),
        major.parse().unwrap_or(fallback_major),
    )
}

fn parse_canonical_adapter_identity(identity: &str) -> Option<(String, String, u32)> {
    let (namespace, remainder) = identity.split_once('/')?;
    let (name, major) = remainder.rsplit_once('@')?;
    if namespace.is_empty() || name.is_empty() || namespace.contains('@') || name.contains('/') {
        return None;
    }
    Some((namespace.to_string(), name.to_string(), major.parse().ok()?))
}

fn environment_mounts_overlap(left: &str, right: &str) -> bool {
    left == right
        || left
            .strip_prefix(right)
            .is_some_and(|rest| rest.starts_with('/'))
        || right
            .strip_prefix(left)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn environment_path_ancestor<'a, T>(
    paths: &'a BTreeMap<String, T>,
    path: &str,
) -> Option<(&'a str, &'a T)> {
    if let Some((stored, value)) = paths.get_key_value(path) {
        return Some((stored, value));
    }
    let mut prefix = String::new();
    let mut segments = path.split('/').peekable();
    while let Some(segment) = segments.next() {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(segment);
        if segments.peek().is_some() {
            if let Some((stored, value)) = paths.get_key_value(&prefix) {
                return Some((stored, value));
            }
        }
    }
    None
}

fn environment_path_overlap_in_map<'a, T>(
    paths: &'a BTreeMap<String, T>,
    path: &str,
) -> Option<(&'a str, &'a T)> {
    environment_path_ancestor(paths, path).or_else(|| {
        paths
            .range(path.to_string()..)
            .next()
            .filter(|(stored, _)| stored.starts_with(&format!("{path}/")))
            .map(|(stored, value)| (stored.as_str(), value))
    })
}

fn diff_workspace_layer_keys(
    previous: &WorkspaceLayerKeyV1,
    current: &WorkspaceLayerKeyV1,
) -> Vec<EnvironmentStaleChangeReport> {
    let mut changes = Vec::new();
    diff_layer_key_map("input", &previous.inputs, &current.inputs, &mut changes);
    diff_layer_key_map(
        "tool",
        &previous.tool_versions,
        &current.tool_versions,
        &mut changes,
    );
    for (name, previous, current) in [
        ("kind", previous.kind.as_str(), current.kind.as_str()),
        (
            "adapter",
            previous.adapter.as_str(),
            current.adapter.as_str(),
        ),
        (
            "platform",
            previous.platform.as_str(),
            current.platform.as_str(),
        ),
        (
            "architecture",
            previous.architecture.as_str(),
            current.architecture.as_str(),
        ),
        (
            "portability_scope",
            previous.portability_scope.as_str(),
            current.portability_scope.as_str(),
        ),
        (
            "strategy",
            previous.strategy.as_str(),
            current.strategy.as_str(),
        ),
    ] {
        if previous != current {
            changes.push(EnvironmentStaleChangeReport {
                dimension: "policy".to_string(),
                name: name.to_string(),
                change: "modified".to_string(),
            });
        }
    }
    if previous.adapter_version != current.adapter_version {
        changes.push(EnvironmentStaleChangeReport {
            dimension: "policy".to_string(),
            name: "adapter_version".to_string(),
            change: "modified".to_string(),
        });
    }
    if changes.is_empty() && previous != current {
        changes.push(EnvironmentStaleChangeReport {
            dimension: "canonical_key".to_string(),
            name: "serialized_value".to_string(),
            change: "modified".to_string(),
        });
    }
    changes
}

fn diff_layer_key_map(
    dimension: &str,
    previous: &BTreeMap<String, String>,
    current: &BTreeMap<String, String>,
    changes: &mut Vec<EnvironmentStaleChangeReport>,
) {
    for name in previous
        .keys()
        .chain(current.keys())
        .collect::<BTreeSet<_>>()
    {
        let change = match (previous.get(name), current.get(name)) {
            (None, Some(_)) => Some("added"),
            (Some(_), None) => Some("removed"),
            (Some(left), Some(right)) if left != right => Some("modified"),
            _ => None,
        };
        if let Some(change) = change {
            changes.push(EnvironmentStaleChangeReport {
                dimension: dimension.to_string(),
                name: if dimension == "input" {
                    name.strip_prefix("input:").unwrap_or(name).to_string()
                } else {
                    name.clone()
                },
                change: change.to_string(),
            });
        }
    }
}

fn format_stale_changes(changes: &[EnvironmentStaleChangeReport]) -> String {
    const MAX_NAMES: usize = 12;
    if changes.is_empty() {
        return String::new();
    }
    let mut rendered = changes
        .iter()
        .take(MAX_NAMES)
        .map(|change| format!("{}:{} {}", change.dimension, change.name, change.change))
        .collect::<Vec<_>>();
    if changes.len() > MAX_NAMES {
        rendered.push(format!("and {} more", changes.len() - MAX_NAMES));
    }
    format!("environment identity changed: {}", rendered.join(", "))
}

#[allow(clippy::too_many_arguments)]
fn paginate_stale_explanation(
    component_id: &str,
    status: &str,
    expected_key: String,
    attached_key: Option<String>,
    provenance_complete: bool,
    changes: Vec<EnvironmentStaleChangeReport>,
    offset: u64,
    limit: u64,
) -> Result<EnvironmentStaleExplanationReport> {
    let total_changes = changes.len() as u64;
    let start = usize::try_from(offset)
        .unwrap_or(usize::MAX)
        .min(changes.len());
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let end = start.saturating_add(limit).min(changes.len());
    let next_offset = (end < changes.len()).then_some(end as u64);
    Ok(EnvironmentStaleExplanationReport {
        component_id: component_id.to_string(),
        status: status.to_string(),
        expected_key,
        attached_key,
        complete: provenance_complete && start == 0 && next_offset.is_none(),
        provenance_complete,
        total_changes,
        offset,
        next_offset,
        changes: changes[start..end].to_vec(),
    })
}

fn environment_output_activations(
    plan: &WorkspaceEnvironmentPlan,
    layer_id: Option<&str>,
    component_key: &str,
    private_paths: Option<&[PathBuf]>,
) -> Result<Vec<EnvironmentLayerOutputActivation>> {
    if private_paths.is_some_and(|paths| paths.len() != plan.outputs.len()) {
        return Err(Error::Corrupt(format!(
            "component `{}` produced a different number of private outputs than it declared",
            plan.component_id
        )));
    }
    let package_outputs = layer_id.is_some() && plan.outputs.len() > 1;
    plan.outputs
        .iter()
        .enumerate()
        .map(|(index, output)| {
            let binding_identity = match output.policy {
                WorkspaceEnvironmentOutputPolicy::ImmutableSeedPrivate => layer_id
                    .ok_or_else(|| {
                        Error::Corrupt(format!(
                            "component `{}` immutable output `{}` has no prepared layer",
                            plan.component_id, output.name
                        ))
                    })?
                    .to_string(),
                WorkspaceEnvironmentOutputPolicy::WritablePrivate => {
                    if layer_id.is_some() {
                        return Err(Error::Corrupt(format!(
                            "component `{}` writable-private output `{}` unexpectedly has a layer",
                            plan.component_id, output.name
                        )));
                    }
                    writable_private_binding_identity(
                        &plan.component_id,
                        &output.name,
                        component_key,
                    )
                }
            };
            Ok(EnvironmentLayerOutputActivation {
                name: output.name.clone(),
                mount_path: output.mount_path.clone(),
                policy: output.policy.as_str().to_string(),
                binding_identity,
                private_seed: private_paths.map(|paths| paths[index].clone()),
                layer_subpath: if package_outputs {
                    format!("outputs/{index:04}")
                } else {
                    String::new()
                },
            })
        })
        .collect()
}

fn environment_dependency_activations(
    plan: &WorkspaceEnvironmentPlan,
) -> Result<Vec<(String, String, String)>> {
    if plan.dependencies.len() != plan.resolved_dependencies.len() {
        return Err(Error::Corrupt(format!(
            "finalized environment component `{}` lost resolved dependency provenance",
            plan.component_id
        )));
    }
    plan.resolved_dependencies
        .iter()
        .map(|dependency| {
            if let Some(input_name) = dependency
                .edge_type
                .identity_input_name(&dependency.component_id)
            {
                if plan.layer_key.inputs.get(&input_name) != Some(&dependency.component_key) {
                    return Err(Error::Corrupt(format!(
                        "finalized environment component `{}` omitted identity edge `{}`",
                        plan.component_id, input_name
                    )));
                }
            }
            Ok((
                dependency.component_id.clone(),
                dependency.component_key.clone(),
                dependency.edge_type.as_str().to_string(),
            ))
        })
        .collect()
}

fn environment_cache_report(cache: &WorkspaceEnvironmentCache) -> EnvironmentCacheReport {
    EnvironmentCacheReport {
        name: cache.name.clone(),
        namespace_id: cache.namespace_id.clone(),
        protocol: cache.protocol.as_str().to_string(),
        access: cache.access.as_str().to_string(),
        authority: "performance_only".to_string(),
        scope: "workspace".to_string(),
        compatibility: cache.compatibility.clone(),
    }
}

fn environment_cache_activations(plan: &WorkspaceEnvironmentPlan) -> Vec<EnvironmentCacheReport> {
    plan.caches.iter().map(environment_cache_report).collect()
}

fn environment_external_artifact_report(
    artifact: &WorkspaceEnvironmentExternalArtifact,
) -> EnvironmentExternalArtifactReport {
    EnvironmentExternalArtifactReport {
        name: artifact.name.clone(),
        artifact_type: artifact.artifact_type.clone(),
        provider: artifact.provider.clone(),
        reference: artifact.reference.clone(),
        digest: artifact.digest.clone(),
        platform: artifact.platform.clone(),
        cleanup_owner: artifact.cleanup_owner.clone(),
    }
}

fn environment_external_artifact_activations(
    plan: &WorkspaceEnvironmentPlan,
) -> Vec<EnvironmentExternalArtifactReport> {
    let mut artifacts = plan
        .external_artifacts
        .iter()
        .map(environment_external_artifact_report)
        .collect::<Vec<_>>();
    artifacts.sort_by(|left, right| left.name.cmp(&right.name));
    artifacts
}

pub(super) fn workspace_external_artifacts_identity(
    artifacts: &[WorkspaceEnvironmentExternalArtifact],
) -> Result<String> {
    let mut reports = artifacts
        .iter()
        .map(environment_external_artifact_report)
        .collect::<Vec<_>>();
    reports.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(serde_json::to_string(&reports)?)
}

fn environment_runtime_resource_report(
    resource: &WorkspaceEnvironmentRuntimeResource,
) -> EnvironmentRuntimeDeclarationReport {
    EnvironmentRuntimeDeclarationReport {
        name: resource.name.clone(),
        runtime_type: resource.runtime_type.clone(),
        provider: resource.provider.clone(),
        artifact_name: resource.artifact_name.clone(),
        container_port: resource.container_port,
        protocol: resource.protocol.clone(),
        health_type: resource.health_type.clone(),
        health_timeout_ms: resource.health_timeout_ms,
        restart_policy: resource.restart_policy.clone(),
        cleanup_owner: resource.cleanup_owner.clone(),
        volume_target: resource.volume_target.clone(),
        secrets: resource
            .secrets
            .iter()
            .map(|secret| EnvironmentSecretReferenceReport {
                name: secret.name.clone(),
                provider: secret.provider.clone(),
                reference: secret.reference.clone(),
                version: secret.version.clone(),
                purpose: secret.purpose.clone(),
                injection: secret.injection.clone(),
                target: secret.target.clone(),
                environment: secret.environment.clone(),
                required: secret.required,
            })
            .collect(),
    }
}

fn environment_runtime_resource_activations(
    plan: &WorkspaceEnvironmentPlan,
) -> Vec<EnvironmentRuntimeDeclarationReport> {
    let mut resources = plan
        .runtime_resources
        .iter()
        .map(environment_runtime_resource_report)
        .collect::<Vec<_>>();
    resources.sort_by(|left, right| left.name.cmp(&right.name));
    resources
}

pub(super) fn workspace_runtime_resources_identity(
    resources: &[WorkspaceEnvironmentRuntimeResource],
) -> Result<String> {
    let mut reports = resources
        .iter()
        .map(environment_runtime_resource_report)
        .collect::<Vec<_>>();
    reports.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(serde_json::to_string(&reports)?)
}

fn validate_workspace_environment_runtime_resource(
    resource: &WorkspaceEnvironmentRuntimeResource,
) -> Result<()> {
    validate_environment_runtime_declaration_report(&environment_runtime_resource_report(resource))
}

pub(super) fn validate_environment_runtime_declaration_report(
    resource: &EnvironmentRuntimeDeclarationReport,
) -> Result<()> {
    if resource.name.is_empty()
        || resource.name.len() > 128
        || !resource.name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
        || resource.runtime_type != "container"
        || resource.provider != "oci"
        || resource.artifact_name.is_empty()
        || resource.container_port == 0
        || resource.protocol != "tcp"
        || resource.health_type != "tcp"
        || !(1_000..=300_000).contains(&resource.health_timeout_ms)
        || !matches!(
            resource.restart_policy.as_str(),
            "never" | "on_failure" | "always"
        )
        || resource.cleanup_owner != "trail"
    {
        return Err(Error::InvalidInput(format!(
            "invalid runtime resource declaration `{}`",
            resource.name
        )));
    }
    if let Some(target) = resource.volume_target.as_deref() {
        if target.len() > 4096
            || !target.starts_with('/')
            || target.contains('\\')
            || target.chars().any(char::is_control)
            || target
                .split('/')
                .skip(1)
                .any(|segment| segment.is_empty() || segment == "." || segment == "..")
            || ["/proc", "/sys", "/dev", "/run", "/etc"]
                .iter()
                .any(|reserved| target == *reserved || target.starts_with(&format!("{reserved}/")))
        {
            return Err(Error::InvalidInput(format!(
                "runtime resource `{}` has invalid or reserved volume target `{target}`",
                resource.name
            )));
        }
    }
    if resource.secrets.len() > 16 {
        return Err(Error::InvalidInput(format!(
            "runtime resource `{}` declares more than 16 secret references",
            resource.name
        )));
    }
    let mut secret_names = BTreeSet::new();
    let mut secret_targets = BTreeSet::new();
    for secret in &resource.secrets {
        if secret.name.is_empty()
            || secret.name.len() > 128
            || !secret.name.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
            })
            || !secret_names.insert(secret.name.clone())
            || !matches!(secret.provider.as_str(), "file" | "environment_file")
            || secret.reference.is_empty()
            || secret.reference.len() > 4096
            || secret.reference.chars().any(char::is_control)
            || secret.purpose.is_empty()
            || secret.purpose.len() > 256
            || secret.purpose.chars().any(char::is_control)
            || secret.injection != "file"
            || !secret.target.starts_with("/run/secrets/")
            || secret.target.len() > 4096
            || secret.target.contains('\\')
            || secret.target.chars().any(char::is_control)
            || secret
                .target
                .split('/')
                .skip(1)
                .any(|segment| segment.is_empty() || segment == "." || segment == "..")
            || !secret_targets.insert(secret.target.clone())
            || secret.environment.as_deref().is_some_and(|name| {
                name.is_empty()
                    || name.len() > 128
                    || name.as_bytes()[0].is_ascii_digit()
                    || !name.bytes().all(|byte| {
                        byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'
                    })
            })
            || secret.version.as_deref().is_some_and(|version| {
                version.is_empty() || version.len() > 256 || version.chars().any(char::is_control)
            })
            || secret.provider == "file" && !secret.reference.starts_with('/')
            || secret.provider == "environment_file"
                && !secret.reference.chars().all(|character| {
                    character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
                })
        {
            return Err(Error::InvalidInput(format!(
                "runtime resource `{}` has invalid secret reference `{}`",
                resource.name, secret.name
            )));
        }
    }
    Ok(())
}

fn validate_workspace_environment_external_artifact(
    artifact: &WorkspaceEnvironmentExternalArtifact,
) -> Result<()> {
    validate_environment_external_artifact_report(&environment_external_artifact_report(artifact))
}

pub(super) fn validate_environment_external_artifact_report(
    artifact: &EnvironmentExternalArtifactReport,
) -> Result<()> {
    if artifact.name.is_empty()
        || artifact.name.len() > 128
        || !artifact.name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
        || artifact.artifact_type != "oci_image"
        || artifact.provider != "oci"
        || artifact.cleanup_owner != "external"
    {
        return Err(Error::InvalidInput(format!(
            "invalid external artifact declaration `{}`",
            artifact.name
        )));
    }
    let digest = artifact.digest.strip_prefix("sha256:").ok_or_else(|| {
        Error::InvalidInput(format!(
            "external artifact `{}` requires a sha256 digest",
            artifact.name
        ))
    })?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(Error::InvalidInput(format!(
            "external artifact `{}` has a malformed sha256 digest",
            artifact.name
        )));
    }
    let (repository, reference_digest) = artifact.reference.rsplit_once('@').ok_or_else(|| {
        Error::InvalidInput(format!(
            "external OCI artifact `{}` must use a digest-pinned reference",
            artifact.name
        ))
    })?;
    if repository.is_empty()
        || repository.len() > 2048
        || repository.contains('@')
        || repository.starts_with('/')
        || repository.ends_with('/')
        || !repository
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-/:".contains(&byte))
        || reference_digest != artifact.digest
    {
        return Err(Error::InvalidInput(format!(
            "external OCI artifact `{}` has an invalid or mismatched reference",
            artifact.name
        )));
    }
    let Some((operating_system, architecture)) = artifact.platform.split_once('/') else {
        return Err(Error::InvalidInput(format!(
            "external OCI artifact `{}` requires an os/architecture platform",
            artifact.name
        )));
    };
    if !matches!(operating_system, "linux" | "windows")
        || !matches!(architecture, "amd64" | "arm64")
    {
        return Err(Error::InvalidInput(format!(
            "external OCI artifact `{}` uses unsupported platform `{}`",
            artifact.name, artifact.platform
        )));
    }
    Ok(())
}

struct WorkspaceEnvironmentCacheUseGuard {
    lease_path: PathBuf,
    exclusive: Option<(PathBuf, String)>,
}

impl Drop for WorkspaceEnvironmentCacheUseGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lease_path);
        if let Some((path, token)) = &self.exclusive {
            if fs::read_to_string(path)
                .ok()
                .is_some_and(|owner| owner == *token)
            {
                let _ = fs::remove_file(path);
            }
        }
    }
}

fn environment_cache_owner_file_is_stale(path: &Path) -> Result<bool> {
    let value = match fs::read_to_string(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(Error::Io(error)),
    };
    let Some((pid, start_token)) = value.split_once(':') else {
        return environment_cache_malformed_owner_is_stale(path);
    };
    let Ok(pid) = pid.parse::<u32>() else {
        return environment_cache_malformed_owner_is_stale(path);
    };
    Ok(!process_matches_start_token(pid, start_token))
}

fn environment_cache_malformed_owner_is_stale(path: &Path) -> Result<bool> {
    let modified = fs::metadata(path)?.modified()?;
    Ok(SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default()
        >= Duration::from_secs(5))
}

pub(super) fn environment_cache_namespace_has_live_leases(
    db_dir: &Path,
    namespace_id: &str,
    cleanup_stale: bool,
) -> Result<bool> {
    let lease_dir = db_dir.join("cache/namespace-leases").join(namespace_id);
    let entries = match fs::read_dir(&lease_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(Error::Io(error)),
    };
    let mut live = false;
    for entry in entries {
        let path = entry?.path();
        if environment_cache_owner_file_is_stale(&path)? {
            if cleanup_stale {
                let _ = fs::remove_file(path);
            }
        } else {
            live = true;
        }
    }
    if !live && cleanup_stale {
        let _ = fs::remove_dir(&lease_dir);
    }
    Ok(live)
}

fn normalize_workspace_environment_dependencies(
    component_id: &str,
    dependencies: &mut Vec<WorkspaceEnvironmentDependency>,
) -> Result<()> {
    dependencies.sort_by(|left, right| left.component_id.cmp(&right.component_id));
    for pair in dependencies.windows(2) {
        if pair[0].component_id == pair[1].component_id {
            return Err(Error::InvalidInput(format!(
                "environment component `{component_id}` declares multiple edge types for dependency `{}`",
                pair[0].component_id
            )));
        }
    }
    Ok(())
}

fn writable_private_binding_identity(
    component_id: &str,
    output_name: &str,
    component_key: &str,
) -> String {
    format!(
        "private_{}",
        &sha256_hex(format!("{component_id}\0{output_name}\0{component_key}").as_bytes())[..32]
    )
}

fn package_workspace_environment_outputs(
    build_dir: &Path,
    package_name: &str,
    outputs: &[PathBuf],
) -> Result<PathBuf> {
    match outputs {
        [] => Err(Error::Corrupt(
            "environment execution produced no normalized outputs".to_string(),
        )),
        [output] => Ok(output.clone()),
        outputs => {
            let destination = build_dir.join(format!(".trail-{package_name}"));
            if destination.exists() {
                return Err(Error::Corrupt(format!(
                    "environment output package destination `{}` already exists",
                    destination.display()
                )));
            }
            for (index, output) in outputs.iter().enumerate() {
                copy_dir_recursive(output, &destination.join(format!("outputs/{index:04}")))?;
            }
            Ok(destination)
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedWorkspaceTool {
    pub(crate) path: PathBuf,
    pub(crate) identity: String,
}

pub(crate) fn resolve_workspace_tool_executable(program: &str) -> Result<ResolvedWorkspaceTool> {
    let path = std::env::var_os("PATH").ok_or_else(|| {
        Error::InvalidInput(format!("PATH is unavailable while resolving `{program}`"))
    })?;
    #[cfg(not(windows))]
    let names = vec![program.to_string()];
    #[cfg(windows)]
    let names = if Path::new(program).extension().is_none() {
        vec![
            program.to_string(),
            format!("{program}.exe"),
            format!("{program}.cmd"),
        ]
    } else {
        vec![program.to_string()]
    };
    for directory in std::env::split_paths(&path) {
        for name in &names {
            let candidate = directory.join(name);
            if !candidate.is_file() {
                continue;
            }
            // Preserve the selected executable path when invoking it. Tools
            // such as rustup dispatch from the `cargo`/`rustc` shim name; using
            // the canonical rustup target directly changes argv[0] semantics.
            // Identity still hashes the canonical file behind the shim.
            let executable_path = if candidate.is_absolute() {
                candidate
            } else {
                std::env::current_dir()?.join(candidate)
            };
            let identity = workspace_tool_identity_for_path(&executable_path)?;
            return Ok(ResolvedWorkspaceTool {
                path: executable_path,
                identity,
            });
        }
    }
    Err(Error::InvalidInput(format!(
        "required tool `{program}` was not found on PATH"
    )))
}

impl Trail {
    pub(crate) fn declare_workspace_environment_cache(
        &self,
        adapter_identity: &str,
        name: &str,
        protocol: WorkspaceEnvironmentCacheProtocol,
        access: WorkspaceEnvironmentCacheAccess,
        compatibility: BTreeMap<String, String>,
    ) -> Result<WorkspaceEnvironmentCache> {
        if name.is_empty()
            || name.len() > 128
            || !name.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
        {
            return Err(Error::InvalidInput(format!(
                "invalid environment cache name `{name}`"
            )));
        }
        if compatibility.is_empty() || compatibility.len() > 32 {
            return Err(Error::InvalidInput(format!(
                "environment cache `{name}` requires 1-32 compatibility dimensions"
            )));
        }
        for (dimension, value) in &compatibility {
            if dimension.is_empty()
                || dimension.len() > 128
                || value.is_empty()
                || value.len() > 4096
                || dimension.contains('\0')
                || value.contains('\0')
            {
                return Err(Error::InvalidInput(format!(
                    "environment cache `{name}` has an invalid compatibility dimension"
                )));
            }
        }
        if contains_sensitive_json(&serde_json::to_value(&compatibility)?) {
            return Err(Error::InvalidInput(format!(
                "environment cache `{name}` compatibility contains secret-like data; use only non-secret version or tool identities"
            )));
        }
        let contract = serde_json::json!({
            "schema": "trail.environment-cache/v1",
            "adapter_identity": adapter_identity,
            "name": name,
            "protocol": protocol.as_str(),
            "access": access.as_str(),
            "authority": "performance_only",
            "scope": "workspace",
            "compatibility": &compatibility,
        });
        let namespace_id = format!("cache_{}", sha256_hex(&serde_json::to_vec(&contract)?));
        let storage_path = self.db_dir.join("cache/namespaces").join(&namespace_id);
        Ok(WorkspaceEnvironmentCache {
            name: name.to_string(),
            namespace_id,
            storage_path,
            protocol,
            access,
            compatibility,
        })
    }
}

/// Return one deterministic full cycle from the Kahn remainder without
/// recursion. Every remaining node has at least one dependency in the
/// remainder, so repeatedly following the lexicographically first edge must
/// eventually revisit a node.
fn environment_dependency_cycle(
    remaining: &BTreeMap<String, WorkspaceEnvironmentPlan>,
) -> Result<Vec<String>> {
    let mut current = remaining
        .keys()
        .next()
        .cloned()
        .ok_or_else(|| Error::Corrupt("empty dependency-cycle remainder".to_string()))?;
    let mut path = Vec::<String>::new();
    let mut positions = BTreeMap::<String, usize>::new();
    loop {
        if let Some(position) = positions.get(&current).copied() {
            let mut cycle = path[position..].to_vec();
            cycle.push(current);
            return Ok(cycle);
        }
        positions.insert(current.clone(), path.len());
        path.push(current.clone());
        let plan = remaining.get(&current).ok_or_else(|| {
            Error::Corrupt(format!(
                "dependency-cycle traversal lost component `{current}`"
            ))
        })?;
        current = plan
            .dependencies
            .iter()
            .filter(|dependency| remaining.contains_key(&dependency.component_id))
            .map(|dependency| &dependency.component_id)
            .min()
            .cloned()
            .ok_or_else(|| {
                Error::Corrupt(format!(
                    "dependency-cycle remainder component `{current}` has no remaining dependency"
                ))
            })?;
    }
}

pub(super) fn validate_environment_component_identity(component_id: &str) -> Result<()> {
    if component_id.is_empty()
        || component_id.len() > 256
        || component_id.starts_with('/')
        || component_id.ends_with('/')
        || !component_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | ':' | '/')
        })
        || component_id
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(Error::InvalidInput(format!(
            "invalid environment component identity `{component_id}`"
        )));
    }
    Ok(())
}

fn workspace_tool_identity_for_path(path: &Path) -> Result<String> {
    let canonical = fs::canonicalize(path)?;
    if !canonical.is_file() {
        return Err(Error::InvalidInput(format!(
            "workspace tool `{}` is not a regular file",
            canonical.display()
        )));
    }
    Ok(format!(
        "{}:sha256:{}",
        canonical.to_string_lossy(),
        sha256_hex(&fs::read(&canonical)?)
    ))
}

fn restricted_recipe_sandbox_name() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macos-sandbox-exec"
    }
    #[cfg(target_os = "linux")]
    {
        "linux-landlock-seccomp"
    }
    #[cfg(target_os = "windows")]
    {
        "windows-appcontainer-job"
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        "unavailable-fail-closed"
    }
}

fn ensure_restricted_recipe_sandbox_available() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        if Path::new("/usr/bin/sandbox-exec").is_file() {
            Ok(())
        } else {
            Err(Error::InvalidInput(
                "restricted command recipes require `/usr/bin/sandbox-exec` on macOS".to_string(),
            ))
        }
    }
    #[cfg(all(target_os = "linux", not(test)))]
    {
        Ok(())
    }
    #[cfg(all(target_os = "windows", not(test)))]
    {
        Ok(())
    }
    #[cfg(any(
        all(target_os = "linux", test),
        all(target_os = "windows", test),
        not(any(target_os = "macos", target_os = "linux", target_os = "windows"))
    ))]
    {
        Err(Error::InvalidInput(format!(
                "restricted command recipe sandboxing is unavailable on {}; Trail refuses to run the repository command without native enforcement",
            std::env::consts::OS
        )))
    }
}

#[cfg(target_os = "macos")]
pub(super) fn sandbox_profile_escape(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_test_plan(
        db: &Trail,
        access: WorkspaceEnvironmentCacheAccess,
    ) -> (WorkspaceEnvironmentPlan, WorkspaceEnvironmentCommand) {
        let cache = db
            .declare_workspace_environment_cache(
                "trail/test@1",
                "test-cache",
                WorkspaceEnvironmentCacheProtocol::LockedIndex,
                access,
                BTreeMap::from([
                    ("tool".to_string(), "test-v1".to_string()),
                    ("platform".to_string(), std::env::consts::OS.to_string()),
                ]),
            )
            .unwrap();
        let command = WorkspaceEnvironmentCommand {
            program: "test".to_string(),
            resolved_program: std::env::current_exe().unwrap(),
            executable_identity: "test".to_string(),
            args: Vec::new(),
            working_directory: "project".to_string(),
            environment: BTreeMap::new(),
            remove_environment: Vec::new(),
            cache_names: vec![cache.name.clone()],
        };
        let plan = WorkspaceEnvironmentPlan {
            component_id: "cache-test".to_string(),
            adapter_identity: "trail/test@1".to_string(),
            adapter_version: 1,
            implementation_version: "test".to_string(),
            distribution_digest: "builtin:test".to_string(),
            kind: "generated".to_string(),
            dependencies: Vec::new(),
            resolved_dependencies: Vec::new(),
            layer_key: WorkspaceLayerKeyV1 {
                kind: "generated".to_string(),
                adapter: "test".to_string(),
                adapter_version: 1,
                inputs: BTreeMap::new(),
                tool_versions: BTreeMap::new(),
                platform: std::env::consts::OS.to_string(),
                architecture: std::env::consts::ARCH.to_string(),
                portability_scope: "host".to_string(),
                strategy: "cache-test".to_string(),
            },
            inputs: Vec::new(),
            source_projection: None,
            pre_commands: Vec::new(),
            command: Some(command.clone()),
            mounted_commands: Vec::new(),
            caches: vec![cache],
            external_artifacts: Vec::new(),
            runtime_resources: Vec::new(),
            sandbox_policy: WorkspaceEnvironmentSandboxPolicy::TrustedBuiltin,
            outputs: vec![WorkspaceEnvironmentOutput {
                name: "output".to_string(),
                output_path: "project/output".to_string(),
                mount_path: "output".to_string(),
                policy: WorkspaceEnvironmentOutputPolicy::WritablePrivate,
                create_if_missing: true,
            }],
            stale_reason: "test".to_string(),
        };
        (plan, command)
    }

    #[test]
    fn cache_namespace_identity_is_deterministic_and_compatibility_scoped() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let compatibility = BTreeMap::from([
            ("tool".to_string(), "v1".to_string()),
            ("platform".to_string(), "test".to_string()),
        ]);
        let first = db
            .declare_workspace_environment_cache(
                "trail/test@1",
                "packages",
                WorkspaceEnvironmentCacheProtocol::ContentStore,
                WorkspaceEnvironmentCacheAccess::ToolConcurrent,
                compatibility.clone(),
            )
            .unwrap();
        let repeated = db
            .declare_workspace_environment_cache(
                "trail/test@1",
                "packages",
                WorkspaceEnvironmentCacheProtocol::ContentStore,
                WorkspaceEnvironmentCacheAccess::ToolConcurrent,
                compatibility,
            )
            .unwrap();
        assert_eq!(first, repeated);
        let changed = db
            .declare_workspace_environment_cache(
                "trail/test@1",
                "packages",
                WorkspaceEnvironmentCacheProtocol::ContentStore,
                WorkspaceEnvironmentCacheAccess::ToolConcurrent,
                BTreeMap::from([
                    ("tool".to_string(), "v2".to_string()),
                    ("platform".to_string(), "test".to_string()),
                ]),
            )
            .unwrap();
        assert_ne!(first.namespace_id, changed.namespace_id);
        assert_eq!(
            first.storage_path,
            db.db_dir.join("cache/namespaces").join(&first.namespace_id)
        );
        assert!(!first.storage_path.exists());
        assert!(db
            .declare_workspace_environment_cache(
                "trail/test@1",
                "packages",
                WorkspaceEnvironmentCacheProtocol::ContentStore,
                WorkspaceEnvironmentCacheAccess::ToolConcurrent,
                BTreeMap::from([("registry_token".to_string(), "do-not-persist".to_string())]),
            )
            .is_err());
    }

    #[test]
    fn host_exclusive_cache_serializes_users_and_gc_respects_live_leases() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let (plan, command) = cache_test_plan(&db, WorkspaceEnvironmentCacheAccess::HostExclusive);
        let first = db
            .acquire_workspace_environment_cache_uses(&plan, &command)
            .unwrap();
        fs::write(plan.caches[0].storage_path.join("entry"), "cached\n").unwrap();

        let (sender, receiver) = std::sync::mpsc::channel();
        let workspace_path = workspace.path().to_path_buf();
        let concurrent_plan = plan.clone();
        let concurrent_command = command.clone();
        let worker = thread::spawn(move || {
            let concurrent = Trail::open(workspace_path).unwrap();
            let guard = concurrent
                .acquire_workspace_environment_cache_uses(&concurrent_plan, &concurrent_command)
                .unwrap();
            sender.send(()).unwrap();
            drop(guard);
        });
        assert!(receiver.recv_timeout(Duration::from_millis(150)).is_err());
        let live_gc = db.workspace_cache_gc(false, Some(0)).unwrap();
        assert!(!live_gc
            .deleted
            .iter()
            .any(|entry| entry.id == plan.caches[0].namespace_id));
        assert!(plan.caches[0].storage_path.is_dir());
        drop(first);
        receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        worker.join().unwrap();

        let preview = db.workspace_cache_gc(true, Some(0)).unwrap();
        assert!(preview
            .candidates
            .iter()
            .any(|entry| entry.kind == "environment_cache"
                && entry.id == plan.caches[0].namespace_id));
        assert!(plan.caches[0].storage_path.is_dir());
        let collected = db.workspace_cache_gc(false, Some(0)).unwrap();
        assert!(collected
            .deleted
            .iter()
            .any(|entry| entry.kind == "environment_cache"
                && entry.id == plan.caches[0].namespace_id));
        assert!(!plan.caches[0].storage_path.exists());
    }

    #[test]
    fn ordered_environment_path_lookup_finds_ancestors_and_descendants() {
        let descendants = BTreeMap::from([("generated/nested".to_string(), "child")]);
        assert_eq!(
            environment_path_overlap_in_map(&descendants, "generated"),
            Some(("generated/nested", &"child"))
        );
        let ancestors = BTreeMap::from([("generated".to_string(), "parent")]);
        assert_eq!(
            environment_path_overlap_in_map(&ancestors, "generated/nested"),
            Some(("generated", &"parent"))
        );
        assert!(environment_path_overlap_in_map(&ancestors, "generated-sibling").is_none());
    }

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn failed_mounted_initializer_preserves_previous_generation_and_real_uppers() {
        #[cfg(target_os = "linux")]
        if std::env::var_os("TRAIL_RUN_FUSE_COW_TESTS").as_deref()
            != Some(std::ffi::OsStr::new("1"))
        {
            return;
        }
        #[cfg(target_os = "macos")]
        if std::env::var_os("TRAIL_RUN_NFS_COW_TESTS").as_deref() != Some(std::ffi::OsStr::new("1"))
        {
            return;
        }
        #[cfg(windows)]
        if std::env::var_os("TRAIL_RUN_DOKAN_COW_TESTS").as_deref()
            != Some(std::ffi::OsStr::new("1"))
        {
            return;
        }
        #[cfg(windows)]
        let tool = resolve_workspace_tool_executable("python")
            .or_else(|_| resolve_workspace_tool_executable("python3"));
        #[cfg(not(windows))]
        let tool = resolve_workspace_tool_executable("python3")
            .or_else(|_| resolve_workspace_tool_executable("python"));
        let Ok(tool) = tool else {
            return;
        };

        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "mounted-failure",
            Some("main"),
            if cfg!(target_os = "macos") {
                LaneWorkdirMode::NfsCow
            } else {
                LaneWorkdirMode::OverlayCow
            },
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();

        let component_id = "mounted-failure".to_string();
        let mount_path = ".candidate-output".to_string();
        let baseline_key = WorkspaceLayerKeyV1 {
            kind: "generated".to_string(),
            adapter: "test-mounted".to_string(),
            adapter_version: 1,
            inputs: BTreeMap::from([("revision".to_string(), "baseline".to_string())]),
            tool_versions: BTreeMap::new(),
            platform: std::env::consts::OS.to_string(),
            architecture: std::env::consts::ARCH.to_string(),
            portability_scope: "host".to_string(),
            strategy: "mounted-failure-baseline".to_string(),
        };
        let baseline_cache_key = db.workspace_layer_cache_key(&baseline_key).unwrap();
        let baseline_seed = tempfile::tempdir().unwrap();
        fs::write(baseline_seed.path().join("keep.txt"), "previous\n").unwrap();
        db.replace_declared_workspace_layers(
            "mounted-failure",
            &[EnvironmentLayerActivation {
                layer_id: None,
                outputs: vec![EnvironmentLayerOutputActivation {
                    name: "output".to_string(),
                    mount_path: mount_path.clone(),
                    policy: "writable_private".to_string(),
                    binding_identity: writable_private_binding_identity(
                        &component_id,
                        "output",
                        &baseline_cache_key,
                    ),
                    private_seed: Some(baseline_seed.path().to_path_buf()),
                    layer_subpath: String::new(),
                }],
                component_id: component_id.clone(),
                adapter_identity: "test/mounted@1".to_string(),
                adapter_version: 1,
                implementation_version: "test".to_string(),
                distribution_digest: "builtin:test-mounted-baseline".to_string(),
                kind: "generated".to_string(),
                dependencies: Vec::new(),
                caches: Vec::new(),
                external_artifacts: Vec::new(),
                runtime_resources: Vec::new(),
                expected_key: baseline_cache_key.clone(),
                canonical_key: baseline_key,
            }],
        )
        .unwrap();
        let baseline_generation = db
            .active_environment_generation("mounted-failure")
            .unwrap()
            .unwrap()
            .generation_id;

        let mounted_command = WorkspaceEnvironmentCommand {
            program: "python".to_string(),
            resolved_program: tool.path,
            executable_identity: tool.identity.clone(),
            args: vec![
                "-c".to_string(),
                "from pathlib import Path; out=Path('.candidate-output'); out.mkdir(exist_ok=True); (out/'partial.txt').write_text('partial'); Path('source-leak.txt').write_text('leak'); raise SystemExit(23)".to_string(),
            ],
            working_directory: String::new(),
            environment: BTreeMap::new(),
            remove_environment: Vec::new(),
            cache_names: Vec::new(),
        };
        let action_identity =
            workspace_mounted_commands_identity(std::slice::from_ref(&mounted_command)).unwrap();
        let plan = WorkspaceEnvironmentPlan {
            component_id: component_id.clone(),
            adapter_identity: "test/mounted@1".to_string(),
            adapter_version: 1,
            implementation_version: "test".to_string(),
            distribution_digest: "builtin:test-mounted-failure".to_string(),
            kind: "generated".to_string(),
            dependencies: Vec::new(),
            resolved_dependencies: Vec::new(),
            layer_key: WorkspaceLayerKeyV1 {
                kind: "generated".to_string(),
                adapter: "test-mounted".to_string(),
                adapter_version: 1,
                inputs: BTreeMap::from([
                    ("revision".to_string(), "failure".to_string()),
                    ("mounted_action".to_string(), action_identity),
                ]),
                tool_versions: BTreeMap::from([("python".to_string(), tool.identity)]),
                platform: std::env::consts::OS.to_string(),
                architecture: std::env::consts::ARCH.to_string(),
                portability_scope: "host".to_string(),
                strategy: "mounted-failure-candidate".to_string(),
            },
            inputs: Vec::new(),
            source_projection: None,
            pre_commands: Vec::new(),
            command: None,
            mounted_commands: vec![mounted_command],
            caches: Vec::new(),
            external_artifacts: Vec::new(),
            runtime_resources: Vec::new(),
            sandbox_policy: WorkspaceEnvironmentSandboxPolicy::TrustedBuiltin,
            outputs: vec![WorkspaceEnvironmentOutput {
                name: "output".to_string(),
                output_path: "private/output".to_string(),
                mount_path: mount_path.clone(),
                policy: WorkspaceEnvironmentOutputPolicy::WritablePrivate,
                create_if_missing: true,
            }],
            stale_reason: "test mounted initializer changed".to_string(),
        };
        let candidate_key = db.workspace_layer_cache_key(&plan.layer_key).unwrap();
        let source_root = db
            .get_ref(&db.lane_branch("mounted-failure").unwrap().ref_name)
            .unwrap()
            .root_id;
        let private = db.execute_writable_private_environment_plan(&plan).unwrap();
        let mut artifacts = [PreparedEnvironmentArtifacts::WritablePrivate(Some(private))];
        let error = db
            .initialize_mounted_workspace_environment_plans(
                "mounted-failure",
                "test-mounted-failure",
                &source_root,
                &[(&plan, candidate_key.as_str())],
                &mut artifacts,
                &[],
            )
            .unwrap_err();
        assert!(error.to_string().contains("failed with"));

        let mut leaking_plan = plan.clone();
        leaking_plan.mounted_commands[0].args[1] = "from pathlib import Path; out=Path('.candidate-output'); out.mkdir(exist_ok=True); (out/'partial.txt').write_text('partial'); Path('source-leak.txt').write_text('leak')".to_string();
        leaking_plan
            .layer_key
            .inputs
            .insert("revision".to_string(), "undeclared-write".to_string());
        leaking_plan.layer_key.inputs.insert(
            "mounted_action".to_string(),
            workspace_mounted_commands_identity(&leaking_plan.mounted_commands).unwrap(),
        );
        let leaking_key = db
            .workspace_layer_cache_key(&leaking_plan.layer_key)
            .unwrap();
        let private = db
            .execute_writable_private_environment_plan(&leaking_plan)
            .unwrap();
        let mut artifacts = [PreparedEnvironmentArtifacts::WritablePrivate(Some(private))];
        let error = db
            .initialize_mounted_workspace_environment_plans(
                "mounted-failure",
                "test-mounted-leak",
                &source_root,
                &[(&leaking_plan, leaking_key.as_str())],
                &mut artifacts,
                &[],
            )
            .unwrap_err();
        assert!(error.to_string().contains("undeclared source path"));

        let active_generation = db
            .active_environment_generation("mounted-failure")
            .unwrap()
            .unwrap();
        assert_eq!(active_generation.generation_id, baseline_generation);
        let paths = db.workspace_view_paths_for_lane("mounted-failure").unwrap();
        assert_eq!(
            fs::read_to_string(paths.generated_upper.join(&mount_path).join("keep.txt")).unwrap(),
            "previous\n"
        );
        assert!(!paths
            .generated_upper
            .join(&mount_path)
            .join("partial.txt")
            .exists());
        assert!(!paths.source_upper.join("source-leak.txt").exists());
    }

    #[test]
    fn dead_environment_sync_attempt_recovers_predecessors_and_first_builds() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else {
            LaneWorkdirMode::OverlayCow
        };
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "recover-env",
            Some("main"),
            mode,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let view = db.lane_workspace_view("recover-env").unwrap().unwrap();
        for (component, attached) in [("with-predecessor", Some("old-key")), ("first", None)] {
            db.conn
                .execute(
                    "INSERT INTO environment_component_states
                     (view_id, component_id, adapter_identity, adapter_version, implementation_version, distribution_digest, kind, expected_key, attached_key, status, reason, updated_at)
                     VALUES (?1, ?2, 'trail/test@1', 1, 'test', 'builtin:test', 'dependency', 'new-key', ?3, 'building', NULL, ?4)",
                    params![view.view_id, component, attached, now_ts()],
                )
                .unwrap();
            db.conn
                .execute(
                    "INSERT INTO workspace_environment_states
                     (view_id, adapter, expected_key, attached_key, status, reason, updated_at)
                     VALUES (?1, ?2, 'new-key', ?3, 'building', NULL, ?4)",
                    params![view.view_id, component, attached, now_ts()],
                )
                .unwrap();
        }
        db.conn
            .execute(
                "INSERT INTO environment_sync_attempts
                 (attempt_id, view_id, source_root, mode, owner_pid, owner_start_token, status, reason, started_at, updated_at, finished_at)
                 VALUES ('dead-build', ?1, ?2, 'batch', -1, 'dead', 'running', NULL, ?3, ?3, NULL)",
                params![view.view_id, view.base_root.0, now_ts()],
            )
            .unwrap();
        drop(db);

        let db = Trail::open(workspace.path()).unwrap();
        let rows = {
            let mut stmt = db
                .conn
                .prepare(
                    "SELECT component_id, status, attached_key, reason
                 FROM environment_component_states WHERE view_id = ?1 ORDER BY component_id",
                )
                .unwrap();
            let mapped = stmt
                .query_map(params![view.view_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                })
                .unwrap();
            mapped.collect::<std::result::Result<Vec<_>, _>>().unwrap()
        };
        assert_eq!(rows[0].0, "first");
        assert_eq!(rows[0].1, "failed");
        assert_eq!(rows[0].2, None);
        assert_eq!(rows[1].0, "with-predecessor");
        assert_eq!(rows[1].1, "stale");
        assert_eq!(rows[1].2.as_deref(), Some("old-key"));
        assert!(rows
            .iter()
            .all(|row| row.3.as_deref().unwrap().contains("predecessor")));
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT status FROM environment_sync_attempts WHERE attempt_id = 'dead-build'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "abandoned"
        );

        db.conn
            .execute(
                "INSERT INTO environment_sync_attempts
                 (attempt_id, view_id, source_root, mode, owner_pid, owner_start_token, status, reason, started_at, updated_at, finished_at)
                 VALUES ('dead-after-activation', ?1, ?2, 'single', -1, 'dead', 'running', NULL, ?3, ?3, NULL)",
                params![view.view_id, view.base_root.0, now_ts()],
            )
            .unwrap();
        drop(db);
        let db = Trail::open(workspace.path()).unwrap();
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT status FROM environment_sync_attempts WHERE attempt_id = 'dead-after-activation'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "succeeded"
        );
    }

    #[test]
    fn adapter_identity_parser_preserves_contract_major_and_legacy_names() {
        assert_eq!(
            split_adapter_identity("trail/node@1", 9),
            ("trail".to_string(), "node".to_string(), 1)
        );
        assert_eq!(
            split_adapter_identity("legacy/custom", 0),
            ("legacy".to_string(), "custom".to_string(), 0)
        );
    }

    #[test]
    fn canonical_layer_key_diff_reports_edges_without_exposing_values() {
        let previous = WorkspaceLayerKeyV1 {
            kind: "dependency".to_string(),
            adapter: "example".to_string(),
            adapter_version: 1,
            inputs: BTreeMap::from([
                ("removed.lock".to_string(), "old-secret-value".to_string()),
                ("shared.lock".to_string(), "old-hash".to_string()),
            ]),
            tool_versions: BTreeMap::from([("tool".to_string(), "1".to_string())]),
            platform: "linux".to_string(),
            architecture: "x86_64".to_string(),
            portability_scope: "platform".to_string(),
            strategy: "old".to_string(),
        };
        let mut current = previous.clone();
        current.inputs.remove("removed.lock");
        current
            .inputs
            .insert("added.lock".to_string(), "new-secret-value".to_string());
        current
            .inputs
            .insert("shared.lock".to_string(), "new-hash".to_string());
        current
            .tool_versions
            .insert("tool".to_string(), "2".to_string());
        current.strategy = "new".to_string();

        let changes = diff_workspace_layer_keys(&previous, &current);
        assert!(changes.iter().any(|change| {
            change.dimension == "input" && change.name == "added.lock" && change.change == "added"
        }));
        assert!(changes.iter().any(|change| {
            change.dimension == "input"
                && change.name == "removed.lock"
                && change.change == "removed"
        }));
        assert!(changes.iter().any(|change| {
            change.dimension == "input"
                && change.name == "shared.lock"
                && change.change == "modified"
        }));
        assert!(changes.iter().any(|change| {
            change.dimension == "tool" && change.name == "tool" && change.change == "modified"
        }));
        assert!(changes.iter().any(|change| {
            change.dimension == "policy" && change.name == "strategy" && change.change == "modified"
        }));
        let rendered = format_stale_changes(&changes);
        assert!(!rendered.contains("secret"));
        assert!(!rendered.contains("old-hash"));
        assert!(!rendered.contains("new-hash"));
        let first = paginate_stale_explanation(
            "test",
            "stale",
            "expected".to_string(),
            Some("attached".to_string()),
            true,
            changes.clone(),
            0,
            2,
        )
        .unwrap();
        assert!(!first.complete);
        assert!(first.provenance_complete);
        assert_eq!(first.total_changes, changes.len() as u64);
        assert_eq!(first.changes.len(), 2);
        assert_eq!(first.next_offset, Some(2));
        let remainder = paginate_stale_explanation(
            "test",
            "stale",
            "expected".to_string(),
            Some("attached".to_string()),
            true,
            changes.clone(),
            2,
            1_000,
        )
        .unwrap();
        assert_eq!(remainder.changes.len(), changes.len() - 2);
        assert!(remainder.next_offset.is_none());
    }

    #[test]
    fn dependency_graph_finalization_scales_without_recursive_traversal() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let count = 10_000usize;
        let plans = (0..count)
            .rev()
            .map(|index| {
                let component_id = format!("component-{index:05}");
                WorkspaceEnvironmentPlan {
                    component_id: component_id.clone(),
                    adapter_identity: "trail/test@1".to_string(),
                    adapter_version: 1,
                    implementation_version: "test".to_string(),
                    distribution_digest: "builtin:test".to_string(),
                    kind: "generated".to_string(),
                    dependencies: (index > 0)
                        .then(|| format!("component-{:05}", index - 1))
                        .into_iter()
                        .map(WorkspaceEnvironmentDependency::build_requires)
                        .collect(),
                    resolved_dependencies: Vec::new(),
                    layer_key: WorkspaceLayerKeyV1 {
                        kind: "generated".to_string(),
                        adapter: "test".to_string(),
                        adapter_version: 1,
                        inputs: BTreeMap::from([("component".to_string(), component_id.clone())]),
                        tool_versions: BTreeMap::new(),
                        platform: "test".to_string(),
                        architecture: "test".to_string(),
                        portability_scope: "test".to_string(),
                        strategy: "dependency-scale-test".to_string(),
                    },
                    inputs: Vec::new(),
                    source_projection: None,
                    pre_commands: Vec::new(),
                    command: None,
                    mounted_commands: Vec::new(),
                    caches: Vec::new(),
                    external_artifacts: Vec::new(),
                    runtime_resources: Vec::new(),
                    sandbox_policy: WorkspaceEnvironmentSandboxPolicy::TrustedBuiltin,
                    outputs: vec![WorkspaceEnvironmentOutput {
                        name: "output".to_string(),
                        output_path: format!("staging/{component_id}"),
                        mount_path: format!("generated/{component_id}"),
                        policy: WorkspaceEnvironmentOutputPolicy::WritablePrivate,
                        create_if_missing: true,
                    }],
                    stale_reason: "dependency changed".to_string(),
                }
            })
            .collect();
        let finalized = db.finalize_workspace_environment_plan_graph(plans).unwrap();
        assert_eq!(finalized.len(), count);
        assert_eq!(finalized[0].0.component_id, "component-00000");
        assert_eq!(finalized[count - 1].0.component_id, "component-09999");
        assert_eq!(
            finalized[count - 1].0.layer_key.inputs["dependency:component-09998"],
            finalized[count - 2].1
        );
    }

    #[test]
    fn typed_dependency_edges_separate_identity_from_generation_ordering() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let plan = |component_id: &str,
                    revision: &str,
                    dependencies: Vec<WorkspaceEnvironmentDependency>| {
            WorkspaceEnvironmentPlan {
                component_id: component_id.to_string(),
                adapter_identity: "trail/test@1".to_string(),
                adapter_version: 1,
                implementation_version: "test".to_string(),
                distribution_digest: "builtin:test".to_string(),
                kind: "generated".to_string(),
                dependencies,
                resolved_dependencies: Vec::new(),
                layer_key: WorkspaceLayerKeyV1 {
                    kind: "generated".to_string(),
                    adapter: "test".to_string(),
                    adapter_version: 1,
                    inputs: BTreeMap::from([("revision".to_string(), revision.to_string())]),
                    tool_versions: BTreeMap::new(),
                    platform: "test".to_string(),
                    architecture: "test".to_string(),
                    portability_scope: "test".to_string(),
                    strategy: "typed-edge-test".to_string(),
                },
                inputs: Vec::new(),
                source_projection: None,
                pre_commands: Vec::new(),
                command: None,
                mounted_commands: Vec::new(),
                caches: Vec::new(),
                external_artifacts: Vec::new(),
                runtime_resources: Vec::new(),
                sandbox_policy: WorkspaceEnvironmentSandboxPolicy::TrustedBuiltin,
                outputs: vec![WorkspaceEnvironmentOutput {
                    name: "output".to_string(),
                    output_path: format!("staging/{component_id}"),
                    mount_path: format!("generated/{component_id}"),
                    policy: WorkspaceEnvironmentOutputPolicy::WritablePrivate,
                    create_if_missing: true,
                }],
                stale_reason: "typed edge changed".to_string(),
            }
        };
        let dependencies = vec![
            WorkspaceEnvironmentDependency {
                component_id: "build".to_string(),
                edge_type: WorkspaceEnvironmentEdgeType::BuildRequires,
            },
            WorkspaceEnvironmentDependency {
                component_id: "runtime".to_string(),
                edge_type: WorkspaceEnvironmentEdgeType::RuntimeRequires,
            },
            WorkspaceEnvironmentDependency {
                component_id: "binding".to_string(),
                edge_type: WorkspaceEnvironmentEdgeType::BindsAfter,
            },
            WorkspaceEnvironmentDependency {
                component_id: "configuration".to_string(),
                edge_type: WorkspaceEnvironmentEdgeType::InvalidatesWith,
            },
        ];
        let finalize = |revisions: [&str; 4]| {
            db.finalize_workspace_environment_plan_graph(vec![
                plan("build", revisions[0], Vec::new()),
                plan("runtime", revisions[1], Vec::new()),
                plan("binding", revisions[2], Vec::new()),
                plan("configuration", revisions[3], Vec::new()),
                plan("application", "1", dependencies.clone()),
            ])
            .unwrap()
            .into_iter()
            .find(|(plan, _)| plan.component_id == "application")
            .unwrap()
        };

        let baseline = finalize(["1", "1", "1", "1"]);
        assert!(baseline.0.layer_key.inputs.contains_key("dependency:build"));
        assert!(baseline
            .0
            .layer_key
            .inputs
            .contains_key("dependency:invalidates_with:configuration"));
        assert!(!baseline
            .0
            .layer_key
            .inputs
            .keys()
            .any(|key| key.contains("runtime") || key.contains("binding")));
        assert_ne!(finalize(["2", "1", "1", "1"]).1, baseline.1);
        assert_ne!(finalize(["1", "1", "1", "2"]).1, baseline.1);
        assert_eq!(finalize(["1", "2", "1", "1"]).1, baseline.1);
        assert_eq!(finalize(["1", "1", "2", "1"]).1, baseline.1);
        assert_eq!(baseline.0.resolved_dependencies.len(), 4);
    }

    #[cfg(unix)]
    #[test]
    fn executor_rejects_a_tool_binary_changed_after_component_planning() {
        use std::os::unix::fs::PermissionsExt;

        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let build = tempfile::tempdir().unwrap();
        let tool = build.path().join("tool");
        fs::write(&tool, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
        let identity = workspace_tool_identity_for_path(&tool).unwrap();
        let command = WorkspaceEnvironmentCommand {
            program: "tool".to_string(),
            resolved_program: tool.clone(),
            executable_identity: identity,
            args: Vec::new(),
            working_directory: "project".to_string(),
            environment: BTreeMap::new(),
            remove_environment: Vec::new(),
            cache_names: Vec::new(),
        };
        let plan = WorkspaceEnvironmentPlan {
            component_id: "test".to_string(),
            adapter_identity: "trail/test@1".to_string(),
            adapter_version: 1,
            implementation_version: "test".to_string(),
            distribution_digest: "builtin:test".to_string(),
            kind: "dependency".to_string(),
            dependencies: Vec::new(),
            resolved_dependencies: Vec::new(),
            layer_key: WorkspaceLayerKeyV1 {
                kind: "dependency".to_string(),
                adapter: "test".to_string(),
                adapter_version: 1,
                inputs: BTreeMap::new(),
                tool_versions: BTreeMap::new(),
                platform: std::env::consts::OS.to_string(),
                architecture: std::env::consts::ARCH.to_string(),
                portability_scope: "host".to_string(),
                strategy: "test".to_string(),
            },
            inputs: Vec::new(),
            source_projection: None,
            pre_commands: Vec::new(),
            command: Some(command.clone()),
            mounted_commands: Vec::new(),
            caches: Vec::new(),
            external_artifacts: Vec::new(),
            runtime_resources: Vec::new(),
            sandbox_policy: WorkspaceEnvironmentSandboxPolicy::TrustedBuiltin,
            outputs: vec![WorkspaceEnvironmentOutput {
                name: "primary".to_string(),
                output_path: "project/output".to_string(),
                mount_path: "output".to_string(),
                policy: WorkspaceEnvironmentOutputPolicy::ImmutableSeedPrivate,
                create_if_missing: false,
            }],
            stale_reason: "test".to_string(),
        };
        fs::write(&tool, "#!/bin/sh\nexit 1\n").unwrap();
        let error = db
            .run_workspace_environment_command(&plan, &command, build.path())
            .unwrap_err();
        assert!(error.to_string().contains("changed after"));
    }

    #[test]
    fn automatic_detection_rejects_ambiguous_polyglot_roots() {
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join("src")).unwrap();
        fs::write(
            workspace.path().join("package.json"),
            r#"{"name":"polyglot","version":"1.0.0"}"#,
        )
        .unwrap();
        fs::write(
            workspace.path().join("package-lock.json"),
            r#"{"name":"polyglot","version":"1.0.0","lockfileVersion":3,"packages":{}}"#,
        )
        .unwrap();
        fs::write(
            workspace.path().join("Cargo.toml"),
            "[package]\nname = \"polyglot\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(workspace.path().join("src/lib.rs"), "pub fn value() {}\n").unwrap();
        let cargo_lock_ready = Command::new("cargo")
            .args(["generate-lockfile", "--offline"])
            .current_dir(workspace.path())
            .output()
            .is_ok_and(|output| output.status.success());
        if !cargo_lock_ready {
            fs::write(workspace.path().join("Cargo.lock"), "version = 4\n").unwrap();
        }
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else {
            LaneWorkdirMode::OverlayCow
        };
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "polyglot",
            Some("main"),
            mode,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let discovery = db.discover_workspace_environment("polyglot", None).unwrap();
        assert_eq!(
            discovery
                .components
                .iter()
                .map(|component| component.adapter_identity.as_str())
                .collect::<Vec<_>>(),
            vec!["trail/cargo-target-seed@1", "trail/node@1"]
        );
        assert!(discovery.conflicts.is_empty());
        let error = db
            .sync_workspace_environment("polyglot", "auto", None)
            .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("multiple workspace environment adapters"));
        assert!(message.contains("trail/node@1"));
        assert!(message.contains("trail/cargo-target-seed@1"));
        if cargo_lock_ready
            && Command::new("cargo").arg("--version").output().is_ok()
            && Command::new("rustc").arg("--version").output().is_ok()
            && Command::new("npm").arg("--version").output().is_ok()
            && Command::new("node").arg("--version").output().is_ok()
        {
            let synchronized = db
                .sync_all_workspace_environments("polyglot", None)
                .unwrap();
            assert_eq!(synchronized.layers.len(), 2);
            assert_eq!(synchronized.generation.generation_sequence, 1);
            assert_eq!(
                synchronized
                    .generation
                    .components
                    .iter()
                    .map(|component| component.component_id.as_str())
                    .collect::<Vec<_>>(),
                vec!["cargo-target-seed", "node"]
            );
        }
    }

    #[test]
    fn environment_mount_validation_rejects_tracked_source_shadowing_and_reserved_paths() {
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join("generated")).unwrap();
        fs::write(
            workspace.path().join("generated/owned-by-source.txt"),
            "source\n",
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let source_root = db.get_ref("refs/branches/main").unwrap().root_id;
        let mut plan = WorkspaceEnvironmentPlan {
            component_id: "generated-test".to_string(),
            adapter_identity: "trail/test@1".to_string(),
            adapter_version: 1,
            implementation_version: "test".to_string(),
            distribution_digest: "builtin:test".to_string(),
            kind: "generated".to_string(),
            dependencies: Vec::new(),
            resolved_dependencies: Vec::new(),
            layer_key: WorkspaceLayerKeyV1 {
                kind: "generated".to_string(),
                adapter: "test".to_string(),
                adapter_version: 1,
                inputs: BTreeMap::new(),
                tool_versions: BTreeMap::new(),
                platform: std::env::consts::OS.to_string(),
                architecture: std::env::consts::ARCH.to_string(),
                portability_scope: "host".to_string(),
                strategy: "test".to_string(),
            },
            inputs: Vec::new(),
            source_projection: None,
            pre_commands: Vec::new(),
            command: Some(WorkspaceEnvironmentCommand {
                program: "test".to_string(),
                resolved_program: std::env::current_exe().unwrap(),
                executable_identity: "test".to_string(),
                args: Vec::new(),
                working_directory: "project".to_string(),
                environment: BTreeMap::new(),
                remove_environment: Vec::new(),
                cache_names: Vec::new(),
            }),
            mounted_commands: Vec::new(),
            caches: Vec::new(),
            external_artifacts: Vec::new(),
            runtime_resources: Vec::new(),
            sandbox_policy: WorkspaceEnvironmentSandboxPolicy::TrustedBuiltin,
            outputs: vec![WorkspaceEnvironmentOutput {
                name: "primary".to_string(),
                output_path: "project/generated".to_string(),
                mount_path: "GENERATED".to_string(),
                policy: WorkspaceEnvironmentOutputPolicy::ImmutableSeedPrivate,
                create_if_missing: true,
            }],
            stale_reason: "test".to_string(),
        };

        let error = db
            .validate_environment_mounts_do_not_shadow_source(&source_root, &[&plan])
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("would shadow pinned source file"));

        plan.outputs[0].mount_path = ".trail/generated".to_string();
        let error = db
            .validate_environment_mounts_do_not_shadow_source(&source_root, &[&plan])
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("cannot mount inside reserved path"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_recipe_publication_rejects_hard_link_aliases() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let staging = tempfile::tempdir().unwrap();
        let output = staging.path().join("output");
        fs::create_dir(&output).unwrap();
        let outside = staging.path().join("outside.txt");
        fs::write(&outside, "host canary\n").unwrap();
        fs::hard_link(&outside, output.join("alias.txt")).unwrap();

        let command = WorkspaceEnvironmentCommand {
            program: "test".to_string(),
            resolved_program: std::env::current_exe().unwrap(),
            executable_identity: "test".to_string(),
            args: Vec::new(),
            working_directory: "project".to_string(),
            environment: BTreeMap::new(),
            remove_environment: Vec::new(),
            cache_names: Vec::new(),
        };
        let plan = WorkspaceEnvironmentPlan {
            component_id: "windows-output-validation".to_string(),
            adapter_identity: "trail/test@1".to_string(),
            adapter_version: 1,
            implementation_version: "test".to_string(),
            distribution_digest: "builtin:test".to_string(),
            kind: "generated".to_string(),
            dependencies: Vec::new(),
            resolved_dependencies: Vec::new(),
            layer_key: WorkspaceLayerKeyV1 {
                kind: "generated".to_string(),
                adapter: "test".to_string(),
                adapter_version: 1,
                inputs: BTreeMap::new(),
                tool_versions: BTreeMap::new(),
                platform: "windows".to_string(),
                architecture: std::env::consts::ARCH.to_string(),
                portability_scope: "host".to_string(),
                strategy: "test".to_string(),
            },
            inputs: Vec::new(),
            source_projection: None,
            pre_commands: Vec::new(),
            command,
            mounted_commands: Vec::new(),
            caches: Vec::new(),
            external_artifacts: Vec::new(),
            runtime_resources: Vec::new(),
            sandbox_policy: WorkspaceEnvironmentSandboxPolicy::RestrictedRecipe,
            outputs: vec![WorkspaceEnvironmentOutput {
                name: "primary".to_string(),
                output_path: "project/output".to_string(),
                mount_path: "output".to_string(),
                policy: WorkspaceEnvironmentOutputPolicy::ImmutableSeedPrivate,
                create_if_missing: true,
            }],
            stale_reason: "test".to_string(),
        };
        let mut entries = 0;
        let error = db
            .validate_restricted_recipe_output(&plan, &output, &mut entries)
            .unwrap_err();
        assert!(error.to_string().contains("hard-linked or unverifiable"));
    }
}
